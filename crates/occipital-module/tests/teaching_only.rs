//! The build without `show-ui` serves stamped reply teaching without interactive tools.

#![cfg(not(feature = "show-ui"))]

use cortexkit_role_tool_provider::{catalog, describe};
use occipital_module::serve::OccipitalHandler;
use serde_json::{json, Value};
use subc_client_rs::test_support::ModuleHarness;
use subc_client_rs::{ModuleHandler, RouteBindRequest, RouteHandle};
use subc_protocol::session::HealthStatus;
use subc_protocol::{FrameType, ModuleHelloAckBody};

/// Without a store HELLO_ACK names, the app reader cannot start, but the
/// teaching is served all the same.
fn handler() -> OccipitalHandler {
    OccipitalHandler::new(None, Box::new(|_| {}))
}

#[tokio::test]
async fn the_real_catalog_lists_no_tools_and_keeps_the_stamped_reply_teaching() {
    let mut harness = ModuleHarness::start(|_| handler(), []).await;
    let sample: Value = serde_json::from_str(include_str!(
        "fixtures/pinned/subc-protocol-0.33.1/module_control_request_route_bind_with_scope.json"
    ))
    .unwrap();
    let request = RouteBindRequest::new(
        RouteHandle::detached(42, 7),
        serde_json::from_value(sample["target"].clone()).unwrap(),
        serde_json::from_value(sample["identity"].clone()).unwrap(),
    );
    let route = harness.bind_route(request).await;

    let mut teaching_digest = None;
    for arguments in [json!({}), json!({"system_text": {}})] {
        let corr = harness
            .send_request(
                route,
                serde_json::to_vec(&json!({"name": "tool.catalog", "arguments": arguments}))
                    .unwrap(),
            )
            .await;
        let reply = harness.read_reply(route, corr).await;
        assert_eq!(reply.header.ty, FrameType::Response);
        let answer: catalog::CatalogAnswer = serde_json::from_slice(&reply.body).unwrap();
        assert!(answer.tools.is_empty());
        if arguments.get("system_text").is_none() {
            assert!(answer.system_text.is_none());
            continue;
        }
        teaching_digest = Some(answer.catalog_digest);
        let text = answer.system_text.unwrap();
        assert_eq!(
            text.text.unwrap().as_bytes(),
            occipital_lang::reply_teaching().text.as_bytes()
        );
        assert_eq!(text.tool_names, Some(vec![]));
        let stamp = text.rich_ui.unwrap();
        assert_eq!(stamp.dialect(), "1.0");
        // Replies keep catalog 1.0's first digest, which every app build knows.
        assert_eq!(
            stamp.catalog_digest(),
            "beb0904e8b402916d84d7609a8b06db30ac308f692dcc048db2dc1138cd0c986"
        );
    }

    for (name, arguments) in [
        ("role.describe", json!({})),
        (
            "tool.catalog",
            json!({"system_text": {}, "digest_only": true}),
        ),
    ] {
        let corr = harness
            .send_request(
                route,
                serde_json::to_vec(&json!({"name": name, "arguments": arguments})).unwrap(),
            )
            .await;
        let reply = harness.read_reply(route, corr).await;
        assert_eq!(reply.header.ty, FrameType::Response);
        let value: Value = serde_json::from_slice(&reply.body).unwrap();
        if name == "role.describe" {
            describe::check_describe(&value).unwrap();
        } else {
            let answer: catalog::CatalogAnswer = serde_json::from_value(value).unwrap();
            assert_eq!(Some(answer.catalog_digest), teaching_digest);
            assert!(answer.system_text.is_none());
            assert!(answer.tools.is_empty());
        }
    }

    for name in ["show_ui", "ui.registry", "session.output_refs"] {
        let corr = harness
            .send_request(
                route,
                serde_json::to_vec(&json!({"name": name, "arguments": {}})).unwrap(),
            )
            .await;
        let reply = harness.read_reply(route, corr).await;
        assert_eq!(reply.header.ty, FrameType::Error, "{name}");
        let value: Value = serde_json::from_slice(&reply.body).unwrap();
        assert_eq!(value["code"], "unknown_tool", "{name}: {value}");
    }
    harness.shutdown().await;
}

/// The app reader's sends and uploads need the store, so a HELLO_ACK naming one
/// opens it and the module reports healthy; without one it stays degraded.
#[tokio::test]
async fn hello_ack_opens_the_store_the_app_reader_needs() {
    let data_home = cortexkit_test_support::ScratchDir::new("occipital-teaching-data");
    let store_path = data_home.path().join("cortexkit/occipital/store.db");
    let storage = json!({
        "module_id": "occipital",
        "storage_namespace": "default",
        "isolation": {"kind": "module"},
        "backend": {"backend": "sqlite", "path": store_path},
    });
    let ack = |storage| ModuleHelloAckBody {
        negotiated_ver: subc_protocol::PROTOCOL_VERSION,
        subc_ops: vec![],
        subc_capabilities: vec![],
        storage,
        machine_id: None,
    };
    let unnamed = handler();
    unnamed.on_hello_ack(&ack(None)).await;
    assert_eq!(unnamed.health().await.status, HealthStatus::Degraded);
    let named = handler();
    named.on_hello_ack(&ack(Some(storage))).await;
    assert_eq!(named.health().await.status, HealthStatus::Ok);
    assert!(store_path.exists());
}
