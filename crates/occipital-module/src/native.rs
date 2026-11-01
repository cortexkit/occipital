//! Native checking is isolated from persistence: a result must fit before it commits.

use std::panic::{catch_unwind, AssertUnwindSafe};

use cortexkit_role_tool_provider::result::{ContentPart, ToolResult};
use occipital_lang::{CheckResult, HostError, VersionDescriptor};
use serde_json::{json, Value};
use subc_client_rs::HandlerOutcome;

use crate::{serve::NativeExecutor, show_ui::PreparedCall, store::Store};

pub const RESULT_CAP: usize = 32_768;

/// The three library entry calls share one unwind boundary, including fallback
/// and version discovery, so a dependency panic cannot terminate the server.
pub trait Boundary: Send + Sync {
    fn check(&self, call: &PreparedCall) -> Result<CheckResult, HostError>;
    fn fallback(&self, call: &PreparedCall) -> occipital_lang::FallbackText;
    fn version(&self) -> VersionDescriptor;
}

pub struct Language;

impl Boundary for Language {
    fn check(&self, call: &PreparedCall) -> Result<CheckResult, HostError> {
        occipital_lang::check_program(call.source.as_bytes(), &call.context, &call.host)
    }

    fn fallback(&self, call: &PreparedCall) -> occipital_lang::FallbackText {
        occipital_lang::program_fallback(
            call.source.as_bytes(),
            &call.context.show_ui.title,
            &call.context.show_ui.summary,
        )
    }

    fn version(&self) -> VersionDescriptor {
        VersionDescriptor::current()
    }
}

pub struct Executor<B>(pub B);

/// Models consume text content, so include the check there as well as in the
/// structured result. Version metadata stays structured-only; the cap counts
/// both copies of the diagnostics exactly as sent.
pub fn envelope(content: Value, error_code: Option<&str>) -> Result<Vec<u8>, serde_json::Error> {
    let summary = match content["outcome"].as_str() {
        Some("stored") => "Native interface stored.",
        Some("check_failed") => "Native interface check failed.",
        _ => "Native interface was not stored.",
    };
    let text = if let Some(check) = content.get("check") {
        format!("{summary}\n{}", serde_json::to_string(check)?)
    } else {
        format!(
            "{summary} {}",
            content["message"].as_str().unwrap_or_default()
        )
    };
    let content = content.as_object().cloned().ok_or_else(|| {
        <serde_json::Error as serde::ser::Error>::custom("structured content must be an object")
    })?;
    let mut result =
        ToolResult::new(vec![ContentPart::text(text)]).with_structured_content(content);
    result.is_error = Some(false);
    if let Some(code) = error_code {
        result = result.with_error(code);
    }
    serde_json::to_vec(&result)
}

fn response(content: Value, error_code: Option<&str>) -> HandlerOutcome {
    match envelope(content, error_code) {
        Ok(bytes) => HandlerOutcome::Response(bytes),
        Err(error) => HandlerOutcome::Error {
            code: "internal".into(),
            message: format!("cannot serialize tool result: {error}"),
        },
    }
}

pub fn failed(code: &str, message: &str) -> HandlerOutcome {
    response(
        json!({"schema":"occipital.show_ui/1", "outcome":"failed", "code":code, "message":message}),
        Some(code),
    )
}

pub fn refused(refusal: crate::show_ui::Refusal) -> HandlerOutcome {
    let code = refusal.code;
    let content = crate::show_ui::refusal_content(refusal);
    let encoded = serde_json::to_string(&content).and_then(|text| {
        serde_json::to_vec(
            &ToolResult::new(vec![ContentPart::text(text)])
                .with_structured_content(content)
                .with_error(code),
        )
    });
    match encoded {
        Ok(bytes) => HandlerOutcome::Response(bytes),
        Err(_) => failed("internal", "cannot serialize refusal"),
    }
}

fn host_code(error: &HostError) -> &'static str {
    match error {
        HostError::Rejected(_) | HostError::Refused(_) => "check_refused",
        HostError::InvalidHostValue(_) => "invalid_host_value",
        HostError::Internal(_) => "internal",
    }
}

fn record_id() -> Result<String, getrandom::Error> {
    let mut random = [0; 16];
    getrandom::fill(&mut random)?;
    Ok(format!(
        "r_{}",
        random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}

fn fit(mut content: Value, error: Option<&str>) -> Result<Option<Vec<u8>>, serde_json::Error> {
    loop {
        let bytes = envelope(content.clone(), error)?;
        if bytes.len() <= RESULT_CAP {
            return Ok(Some(bytes));
        }
        let Some(diagnostics) = content["check"]["diagnostics"].as_array_mut() else {
            return Ok(None);
        };
        let Some(index) = diagnostics
            .iter()
            .rposition(|d| d["severity"] == "warning")
            .or_else(|| diagnostics.len().checked_sub(1))
        else {
            return Ok(None);
        };
        diagnostics.remove(index);
        let omitted = content["check"]["omitted"].as_u64().unwrap_or(0);
        content["check"]["omitted"] = Value::from(omitted.saturating_add(1));
    }
}

impl<B: Boundary> NativeExecutor for Executor<B> {
    fn execute(&self, call: PreparedCall, store: Option<&Store>) -> HandlerOutcome {
        let checked = catch_unwind(AssertUnwindSafe(|| {
            let check = self.0.check(&call)?;
            let fallback = self.0.fallback(&call);
            let version = self.0.version();
            Ok::<_, HostError>((check, fallback, version))
        }));
        let (check, fallback, version) = match checked {
            Err(_) => return failed("internal", "language boundary panicked"),
            Ok(Err(error)) => return failed(host_code(&error), "language host rejected the check"),
            Ok(Ok(values)) => values,
        };
        let success = check.ok();
        let id = if success {
            match record_id() {
                Ok(id) => Some(id),
                Err(_) => return failed("internal", "record id randomness unavailable"),
            }
        } else {
            None
        };
        let mut content = json!({
            "schema":"occipital.show_ui/1", "outcome":if success { "stored" } else { "check_failed" },
            "kind":"native", "check":check
        });
        if let Some(id) = &id {
            content["recordId"] = id.clone().into();
            content["version"] = json!(version);
        }
        let bytes = match fit(content, if success { None } else { Some("check_failed") }) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return failed("result_too_large", "tool result exceeds 32768 bytes"),
            Err(_) => return failed("internal", "cannot serialize check result"),
        };
        if let Some(id) = id {
            let Some(store) = store else {
                return failed("store_failed", "store unavailable");
            };
            if store.insert(&id, &call, &version, &fallback).is_err() {
                return failed("store_failed", "native record could not be committed");
            }
        }
        HandlerOutcome::Response(bytes)
    }
}
