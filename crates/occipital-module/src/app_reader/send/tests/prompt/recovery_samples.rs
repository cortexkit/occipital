use super::*;
use crate::app_reader::{
    broca::tests::{recovery_chat, served_samples::Served},
    AppReader, SEND, TRANSCRIPT_PAGE,
};
use std::time::Duration;

async fn keep(served: &mut Served, name: &str, method: &str, params: Value) -> Value {
    let request = json!({"method":method, "params":params});
    served.send(1, &request).await;
    let (_, bytes) = served.next(1, Duration::from_secs(5)).await.unwrap();
    let reply = serde_json::from_slice(&bytes).unwrap();
    served.keep(name, &request, bytes);
    reply
}

#[tokio::test]
async fn the_recovery_samples_are_what_the_reader_serves() {
    let f = granted_fixture(Kind::Head);
    let p = Prompt::new();
    let app = Arc::new(AppReader::with_sender(p.resolver.clone(), f.sender.clone()));
    let mut served = Served::new("phone-reader-recovery-v1", app).on_route(samples::phone_route());
    f.runner.scripted.guard().push_back(Ok(
        json!({"state":"finished","reason":"error","run_id":"r1"}),
    ));
    let failed = keep(
        &mut served,
        "01-finished-without-delivery",
        SEND,
        message("one", "hello"),
    )
    .await;
    assert_eq!(failed["result"]["code"], "not_delivered");
    f.runner.captured.guard().extend([
        (
            "role.describe",
            json!({"result":{"capabilities":["run_cancel_by_interrupt"]}}),
        ),
        ("run.cancel", json!({"result":"not_active"})),
        ("run.status", json!({"code":"transient","message":"retry"})),
    ]);
    let stopped = keep(
        &mut served,
        "02-stop-status-unavailable",
        "session.stop",
        json!({"session":"s","runId":"r1","clientRequestId":"stop"}),
    )
    .await;
    assert_eq!(stopped["code"], "provider_unavailable");
    let upload = p.upload("image/png", b"image").await;
    p.runner.describe_offline.store(true, Ordering::SeqCst);
    served.serve_with(Arc::new(
        AppReader::with_sender(p.resolver.clone(), p.sender.clone())
            .with_uploads(Some(p.uploads.clone())),
    ));
    let prompt = keep(
        &mut served,
        "03-prompt-description-unavailable",
        SEND,
        with_files("two", "look", &[&upload]),
    )
    .await;
    assert_eq!(prompt["code"], "provider_unavailable");
    let (app, session) = recovery_chat(false);
    served.serve_with(app);
    let media = keep(
        &mut served,
        "04-media-description-unavailable",
        "session.media",
        json!({"session":session,"sha256":"0".repeat(64),"size":"full"}),
    )
    .await;
    assert_eq!(media["code"], "provider_unavailable");
    let (app, session) = recovery_chat(true);
    served.serve_with(app);
    let page = keep(
        &mut served,
        "05-page-plan-too-large",
        TRANSCRIPT_PAGE,
        json!({"session":session}),
    )
    .await;
    assert_eq!(page["code"], "provider_unavailable");
    let mut unclassified = samples::phone_route();
    unclassified.3 = None;
    served = served.on_route(unclassified);
    let refused = keep(
        &mut served,
        "06-no-device-stamp",
        TRANSCRIPT_PAGE,
        json!({"session":session}),
    )
    .await;
    assert_eq!(refused["code"], "carrier_op_not_permitted");
    served = served.on_route(samples::phone_route());
    let missing = keep(
        &mut served,
        "07-send-without-key",
        SEND,
        json!({"session":"s","text":"hello"}),
    )
    .await;
    assert_eq!(missing["code"], "invalid_request");
    served.check();
}
