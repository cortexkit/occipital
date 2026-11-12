//! `session.request_context`: the plan in force on a session, relayed from its
//! runner's `session.plan_view` in the apps' spelling (`crate::request_context`).
//! The runner here is a test OpenCode 2 one; its replies are the runner
//! contract's own `plan_view` test shape.

use super::*;

/// Routes to the test session, an OpenCode 2 one.
struct OpenCode2(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for OpenCode2 {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        assert_eq!(resolved.harness, "opencode2");
        Ok(self.0.clone())
    }
}

fn app(route: &Arc<FakeRoute>) -> super::super::super::AppReader {
    super::super::super::AppReader::new(Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
            THALAMUS_RUNNER_MODULE_ID,
        ))))),
        Arc::new(OpenCode2(route.clone())),
    )))
}

async fn request_context(route: &Arc<FakeRoute>) -> Result<Value, ReadError> {
    app(route)
        .read(
            super::super::super::REQUEST_CONTEXT,
            json!({"session": "ses_head", "harness": "opencode2"}),
        )
        .await
}

fn plan_view_calls(route: &FakeRoute) -> Vec<Value> {
    route
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call["method"] == "session.plan_view")
        .cloned()
        .collect()
}

/// A plan view with every optional fact, its text holding two- and three-byte
/// characters so the item offsets are bytes, not characters.
fn full_view() -> Value {
    json!({
        "generation": 3,
        "applied_at_ms": 1_794_400_000_000_u64,
        "system": {"text": "é\n猫|ok", "items": [
            {"provider": "first", "preset": "p", "start": 0, "end": 2},
            {"provider": "second", "start": 3, "end": 6},
            {"provider": "third", "start": 7, "end": 9}
        ]},
        "tools": [
            {"name": "bash", "description": "Run a command",
                "input_schema": {"type": "object", "properties": {"command": {"type": "string"}}},
                "provider": "aft", "tokens": {"count": 41, "exact": false}},
            {"name": "read", "description": "Read a file", "input_schema": {"type": "object"},
                "provider": "aft"}
        ],
        "newest_step": {"at_ms": 1_794_400_100_000_u64, "reply_mid": "msg_reply"},
        "pending": {"generation": 4, "notes": ["Your instructions changed.", "The tool `edit` was removed."]},
        "as_planned": true
    })
}

/// Every fact reaches the app under its own name, with the text, the schemas
/// and the byte offsets unchanged and the tools in the runner's order. The
/// runner is asked with the contract's empty request, on the session's route.
#[tokio::test]
async fn the_request_context_is_the_plan_in_force() {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&["plan_view"]));
    *route.plan_view.lock().unwrap() = Some(Ok(full_view()));
    let reply = request_context(&route).await.unwrap();
    assert_eq!(
        reply,
        json!({
            "generation": 3,
            "appliedAtMs": 1_794_400_000_000_u64,
            "newestStep": {"atMs": 1_794_400_100_000_u64, "replyMessageId": "msg_reply"},
            "system": {"text": "é\n猫|ok", "items": [
                {"provider": "first", "preset": "p", "start": 0, "end": 2},
                {"provider": "second", "start": 3, "end": 6},
                {"provider": "third", "start": 7, "end": 9}
            ]},
            "tools": [
                {"name": "bash", "description": "Run a command",
                    "inputSchema": {"type": "object", "properties": {"command": {"type": "string"}}},
                    "provider": "aft", "tokens": {"count": 41, "exact": false}},
                {"name": "read", "description": "Read a file", "inputSchema": {"type": "object"},
                    "provider": "aft"}
            ],
            "pending": {"generation": 4, "notes": ["Your instructions changed.", "The tool `edit` was removed."]},
            "asPlanned": true
        })
    );
    let calls = plan_view_calls(&route);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["params"], json!({}));
}

/// Facts the runner didn't record stay absent, never filled in: no
/// application time, no newest step, no item ranges, no pending plan.
#[tokio::test]
async fn facts_the_runner_has_not_recorded_stay_absent() {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&["plan_view"]));
    *route.plan_view.lock().unwrap() = Some(Ok(json!({
        "generation": 0, "system": {"text": "Be brief."}, "tools": [], "as_planned": true
    })));
    assert_eq!(
        request_context(&route).await.unwrap(),
        json!({"generation": 0, "system": {"text": "Be brief."}, "tools": [], "asPlanned": true})
    );
}

/// A runner that doesn't declare the contract's `plan_view` group isn't asked,
/// and the app is told `unsupported`.
#[tokio::test]
async fn a_runner_without_plan_views_is_not_asked() {
    let route = FakeRoute::new();
    *route.plan_view.lock().unwrap() = Some(Ok(full_view()));
    let error = request_context(&route).await.unwrap_err();
    assert_eq!(error.code, "unsupported");
    assert!(plan_view_calls(&route).is_empty());
}

/// The runner's own answers that asking again can't change reach the app
/// under their own code: no plan in force, and a plan too large for one reply.
#[tokio::test]
async fn no_plan_and_too_large_reach_the_app() {
    for code in ["no_plan", "too_large"] {
        let route = FakeRoute::new();
        *route.describe.lock().unwrap() = Some(describe_with(&["plan_view"]));
        *route.plan_view.lock().unwrap() = Some(Err(ReadError::new(code, "refused")));
        assert_eq!(request_context(&route).await.unwrap_err().code, code);
    }
    let final_code = super::super::final_refusal;
    assert_eq!(
        final_code(ReadOp::PlanView, Some("no_plan")),
        Some("no_plan")
    );
    assert_eq!(
        final_code(ReadOp::PlanView, Some("too_large")),
        Some("too_large")
    );
    assert_eq!(
        final_code(ReadOp::PlanView, Some("scope_owner_mismatch")),
        None
    );
    assert_eq!(final_code(ReadOp::Read, Some("no_plan")), None);
}

/// A plan whose reply to the app wouldn't fit the app reply limit is refused
/// `too_large`, as the runner refuses one above its frame, never cut.
#[tokio::test]
async fn a_plan_above_the_app_reply_limit_is_too_large() {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&["plan_view"]));
    let text = "x".repeat(occipital_app_reader::page::MAX_REPLY_BODY_BYTES);
    *route.plan_view.lock().unwrap() = Some(Ok(json!({
        "generation": 1, "system": {"text": text}, "tools": [], "as_planned": true
    })));
    assert_eq!(request_context(&route).await.unwrap_err().code, "too_large");
}

/// A reply that fails the runner contract's own decoding is never passed on: an
/// item range that ends inside a character, an empty list of notes, and the
/// single `note` that runner contract 0.8.40 replaced with `notes`.
#[tokio::test]
async fn a_reply_outside_the_contract_is_refused() {
    let mut inside_a_character = full_view();
    inside_a_character["system"]["items"][0]["end"] = json!(1);
    let mut no_notes = full_view();
    no_notes["pending"]["notes"] = json!([]);
    let mut old_note = full_view();
    old_note["pending"] = json!({"generation": 4, "note": "Your tools changed."});
    for view in [inside_a_character, no_notes, old_note] {
        let route = FakeRoute::new();
        *route.describe.lock().unwrap() = Some(describe_with(&["plan_view"]));
        *route.plan_view.lock().unwrap() = Some(Ok(view.clone()));
        assert_eq!(
            request_context(&route).await.unwrap_err().code,
            "invalid_provider_reply",
            "{view}"
        );
    }
}

/// The read is admitted like the other app reads, for the desktop too.
#[test]
fn the_request_context_is_a_read_the_desktop_may_make() {
    assert!(super::super::super::METHODS.contains(&super::super::super::REQUEST_CONTEXT));
}
