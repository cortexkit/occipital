//! Ask two modules the same app-reader question and compare their answers.
//!
//! Occipital took over the apps' chat reader from Thalamus. While both serve it,
//! the same read on the same live session should get the same reply from each,
//! so a difference is either a porting mistake or a change one side made since.
//! This runs as a plain local caller, which both readers admit while the
//! desktop app is in development.
//!
//! ```text
//! compare_readers <method> '<params JSON>' [<module> <module>]
//! ```
//!
//! The modules default to `occipital` and `thalamus-harness`. The daemon is
//! found the way every local client finds it (the SDK's default discovery).
//! It prints each reply's outcome and, when they differ, every JSON path where
//! they do, and exits 1 on a difference. Naming one module twice asks it once
//! and prints the reply, for finding sessions to compare (for example through
//! Entorhinal).
//!
//! A watch (`session.transcript_watch`, `session.display_watch`) is opened on
//! both modules, and the first notice each sends within 20 seconds is compared,
//! or how the watch ended if it sent none. Both watches are then cancelled. A
//! watch named on one module twice instead prints every notice it sends for
//! `COMPARE_STREAM_SECS` seconds (default 60), each with the seconds since the
//! watch opened, for following a live turn.

use std::process::ExitCode;

use std::time::Duration;

use serde_json::Value;
use subc_client_rs::consumer::{
    CallOptions, ConsumerOptions, SubcConsumer, SubscribeOptions, SubscriptionEnd,
};
use subc_protocol::{BindIdentity, RouteTarget};

/// How long a watch may take to send its first notice.
const FIRST_NOTICE: Duration = Duration::from_secs(20);

fn target(module: &str) -> RouteTarget {
    RouteTarget::ManagementSurface {
        module_id: module.into(),
    }
}

fn identity() -> BindIdentity {
    BindIdentity::new("occipital-compare", "compare")
}

/// A watch's first notice, or how it ended without one.
async fn first_notice(
    consumer: &SubcConsumer,
    module: &str,
    request: &Value,
) -> Result<Value, Value> {
    let mut watch = consumer
        .subscribe(
            target(module),
            identity(),
            request.to_string().into_bytes(),
            SubscribeOptions::default(),
        )
        .await
        .map_err(
            |error| serde_json::json!({ "code": error.code(), "message": error.to_string() }),
        )?;
    let notice = tokio::time::timeout(FIRST_NOTICE, watch.events().recv()).await;
    let outcome = match notice {
        Ok(Some(bytes)) => serde_json::from_slice::<Value>(&bytes)
            .map_err(|error| serde_json::json!({ "undecodable": error.to_string() })),
        Ok(None) => Err(match watch.ended().await {
            SubscriptionEnd::Error(body) => serde_json::to_value(body).unwrap_or(Value::Null),
            other => serde_json::json!({ "ended": format!("{other:?}") }),
        }),
        Err(_) => Err(serde_json::json!({ "silent": FIRST_NOTICE.as_secs() })),
    };
    let _ = watch.unsubscribe();
    outcome
}

/// Print every notice one module's watch sends within `span`, then how it ended.
async fn stream(
    consumer: &SubcConsumer,
    module: &str,
    request: &Value,
    span: Duration,
) -> ExitCode {
    let mut watch = match consumer
        .subscribe(
            target(module),
            identity(),
            request.to_string().into_bytes(),
            SubscribeOptions::default(),
        )
        .await
    {
        Ok(watch) => watch,
        Err(error) => {
            println!(
                "{{\"code\": {:?}, \"message\": {:?}}}",
                error.code(),
                error.to_string()
            );
            return ExitCode::from(1);
        }
    };
    let opened = tokio::time::Instant::now();
    let deadline = opened + span;
    loop {
        match tokio::time::timeout_at(deadline, watch.events().recv()).await {
            Ok(Some(bytes)) => println!(
                "{:.1}s {}",
                opened.elapsed().as_secs_f64(),
                String::from_utf8_lossy(&bytes)
            ),
            Ok(None) => {
                println!("ended: {:?}", watch.ended().await);
                return ExitCode::SUCCESS;
            }
            Err(_) => break,
        }
    }
    let _ = watch.unsubscribe();
    ExitCode::SUCCESS
}

/// One module's answer: the reply's `result`, or its refusal body.
async fn ask(consumer: &SubcConsumer, module: &str, request: &Value) -> Result<Value, Value> {
    let reply = consumer
        .call(
            target(module),
            identity(),
            request.to_string().into_bytes(),
            CallOptions::default(),
        )
        .await;
    match reply {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => Ok(value.get("result").cloned().unwrap_or(value)),
            Err(error) => Err(serde_json::json!({ "undecodable": error.to_string() })),
        },
        Err(error) => Err(match &error {
            subc_client_rs::consumer::CallError::Module(body) => {
                serde_json::to_value(body).unwrap_or(Value::Null)
            }
            other => serde_json::json!({ "code": other.code(), "message": other.to_string() }),
        }),
    }
}

/// Every JSON path where `a` and `b` differ, with both values.
fn differences(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let missing = Value::String("<absent>".into());
                differences(
                    &format!("{path}/{key}"),
                    a.get(key).unwrap_or(&missing),
                    b.get(key).unwrap_or(&missing),
                    out,
                );
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                differences(&format!("{path}/{index}"), a, b, out);
            }
        }
        _ if a != b => out.push(format!("{path}: {a} != {b}")),
        _ => {}
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (method, params) = match (args.first(), args.get(1)) {
        (Some(method), Some(params)) => (method.clone(), params.clone()),
        _ => {
            eprintln!("usage: compare_readers <method> '<params JSON>' [<module> <module>]");
            return ExitCode::from(2);
        }
    };
    let params: Value = match serde_json::from_str(&params) {
        Ok(params) => params,
        Err(error) => {
            eprintln!("params are not JSON: {error}");
            return ExitCode::from(2);
        }
    };
    let first = args.get(2).map_or("occipital", String::as_str);
    let second = args.get(3).map_or("thalamus-harness", String::as_str);
    let consumer = match SubcConsumer::connect_default(ConsumerOptions::default()).await {
        Ok(consumer) => consumer,
        Err(error) => {
            eprintln!("cannot reach the daemon: {error}");
            return ExitCode::from(2);
        }
    };
    let request = serde_json::json!({ "method": method, "params": params });
    if first == second && method.ends_with("_watch") {
        let span = std::env::var("COMPARE_STREAM_SECS")
            .ok()
            .and_then(|secs| secs.parse().ok())
            .unwrap_or(60);
        return stream(&consumer, first, &request, Duration::from_secs(span)).await;
    }
    if first == second {
        let reply = ask(&consumer, first, &request).await;
        let (Ok(body) | Err(body)) = &reply;
        println!("{body:#}");
        return if reply.is_ok() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        };
    }
    let (a, b) = if method.ends_with("_watch") {
        tokio::join!(
            first_notice(&consumer, first, &request),
            first_notice(&consumer, second, &request)
        )
    } else {
        tokio::join!(
            ask(&consumer, first, &request),
            ask(&consumer, second, &request)
        )
    };
    let outcome = |reply: &Result<Value, Value>| match reply {
        Ok(_) => "result".to_owned(),
        Err(body) => format!("refused {body}"),
    };
    println!("{first}: {}", outcome(&a));
    println!("{second}: {}", outcome(&b));
    let (a, b) = (a.unwrap_or_else(|body| body), b.unwrap_or_else(|body| body));
    let mut found = Vec::new();
    differences("", &a, &b, &mut found);
    if found.is_empty() {
        println!("identical ({} bytes)", a.to_string().len());
        return ExitCode::SUCCESS;
    }
    println!("{} differences:", found.len());
    for difference in &found {
        println!("  {difference}");
    }
    ExitCode::from(1)
}
