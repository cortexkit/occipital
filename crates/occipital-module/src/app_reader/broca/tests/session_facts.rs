//! The session facts the apps' replies take from Broca's `session.head`: the next
//! send's model, whether the app may send, the log incarnation that claims bind,
//! and `session.details`. Every reply here goes through the module's own handlers
//! over a fake Broca route.

use super::*;
use crate::app_reader::{resolve::Entorhinal, AppReader, DETAILS, TRANSCRIPT_PAGE};
use cortexkit_role_llm_runner::capabilities::HEAD_SENDER;
use occipital_app_reader::source::SENDER_UNKNOWN;

const SAMPLES: &str = "phone-session-facts-v3";

fn last_run() -> Value {
    json!({"runId":"r1", "state":"active"})
}

#[tokio::test]
async fn last_run_facts_are_codes_only_on_pages_claims_details_and_watches() {
    let route = FakeRoute::new();
    *route.run_state.lock().unwrap() = Value::Null;
    let app = sending(&route);
    let without = page(&app, params()).await;
    for reply in [
        without.clone(),
        details(&app).await,
        page(&app, claim(&without, "m20", "m21")).await,
    ] {
        assert!(reply.get("lastRun").is_none(), "{reply}");
    }
    for (state, reason) in [
        ("error", Some("provider_error")),
        ("future_state", Some("future_reason")),
        ("completed", None),
    ] {
        *route.run_state.lock().unwrap() = json!({"run_id":"failed-run", "state":state, "reason":reason,
            "message":"PRIVATE MESSAGE", "provider_body":"PRIVATE BODY",
            "error":{"class":"permanent", "retry_after_secs":10, "message":"PRIVATE ERROR"}});
        let mut expected = json!({"runId":"failed-run", "state":state});
        if let Some(reason) = reason {
            expected["reason"] = json!(reason);
        }
        let newest = page(&app, params()).await;
        let request = claim(&newest, "m20", "m21");
        let watch: Value = serde_json::from_slice(
            &crate::app_reader::tests::serve(&app, crate::app_reader::TRANSCRIPT_WATCH, &request)
                .await,
        )
        .unwrap();
        for reply in [
            newest,
            page(&app, request).await,
            details(&app).await,
            watch,
        ] {
            let reply = reply.get("result").unwrap_or(&reply);
            assert_eq!(reply["lastRun"], expected, "{reply}");
            assert!(!reply.to_string().contains("PRIVATE"), "{reply}");
        }
    }
}

pub(super) async fn assert_last_run_phone_bytes(
    state: Value,
    error: Option<Value>,
    expected: &str,
) {
    let route = FakeRoute::new();
    *route.run_state.lock().unwrap() = state;
    if let Some(error) = error {
        set_head(&route, "last_run_error", error);
    }
    let app = sending(&route);
    let newest = page(&app, params()).await;
    let request = claim(&newest, "m20", "m21");
    for (method, params) in [
        (TRANSCRIPT_PAGE, params()),
        (TRANSCRIPT_PAGE, request.clone()),
        (DETAILS, json!({"session":"alfonso:s1", "harness":"broca"})),
        (crate::app_reader::TRANSCRIPT_WATCH, request),
    ] {
        let bytes = crate::app_reader::tests::serve(&app, method, &params).await;
        let body = std::str::from_utf8(&bytes).unwrap();
        let reply: Value = serde_json::from_slice(&bytes).unwrap();
        let reply = reply.get("result").unwrap_or(&reply);
        assert_eq!(
            reply["lastRun"],
            serde_json::from_str::<Value>(expected).unwrap(),
            "{method}: {body}"
        );
        assert!(
            body.contains(&format!("\"lastRun\":{expected}")),
            "{method}: {body}"
        );
        assert!(!body.contains("PRIVATE"), "{method}: {body}");
    }
}

#[tokio::test]
async fn reported_last_run_error_reaches_the_phone_bytes_without_diagnostics() {
    assert_last_run_phone_bytes(
        json!({"run_id":"r-error", "state":"error", "reason":"provider_error"}),
        Some(json!({"class":"transient", "retry_after_secs":0.125,
            "resets_at_ms":18446744073709551615_u64,
            "message":"PRIVATE MESSAGE", "provider_code":"PRIVATE CODE", "account":"PRIVATE ACCOUNT"})),
        r#"{"error":{"class":"transient","resetsAtMs":18446744073709551615,"retryAfterSecs":0.125},"reason":"provider_error","runId":"r-error","state":"error"}"#,
    ).await;
    assert_last_run_phone_bytes(
        json!({"run_id":"r-error", "state":"error", "reason":"provider_error"}),
        Some(json!({"class":"future_provider_class"})),
        r#"{"error":{"class":"future_provider_class"},"reason":"provider_error","runId":"r-error","state":"error"}"#,
    ).await;
}

/// A run that ended on an account wait names its contract code on the head
/// (runner contract 0.8.41's `provider_code`), and the apps get it as
/// `providerCode` beside the wait, so they can say the account is at its limit
/// until the reset. A code the contract doesn't name stays out, as above.
///
/// No runner served `provider_code` in its `session.head` reply (the session
/// summary that names the last run and its error) when these were cut, so the
/// replies are synthetic, each the run error of Broca's recorded queued case for that
/// code (`tests/data/broca-account-wait-1112/5_queued_*`), and the
/// `lastRun` each gives is a file of `phone-reader-account-wait-v2`. With
/// `OCCIPITAL_WRITE_SAMPLES` set to a folder, the files are written there instead.
#[tokio::test]
async fn an_account_wait_run_end_names_its_code_to_the_apps() {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for (sample, error, expected) in [
        (
            "09-run-end-limit",
            json!({"class":"transient", "retry_after_secs":120.0,
                "resets_at_ms":4102444800000_u64, "provider_code":"account_limit_exhausted"}),
            r#"{"error":{"class":"transient","providerCode":"account_limit_exhausted","resetsAtMs":4102444800000,"retryAfterSecs":120.0},"reason":"provider_error","runId":"r-wait","state":"error"}"#,
        ),
        (
            "10-run-end-selection",
            json!({"class":"transient", "retry_after_secs":30.0,
                "provider_code":"account_selection_refused"}),
            r#"{"error":{"class":"transient","providerCode":"account_selection_refused","retryAfterSecs":30.0},"reason":"provider_error","runId":"r-wait","state":"error"}"#,
        ),
        (
            "11-run-end-no-timing",
            json!({"class":"transient", "provider_code":"account_limit_exhausted"}),
            r#"{"error":{"class":"transient","providerCode":"account_limit_exhausted"},"reason":"provider_error","runId":"r-wait","state":"error"}"#,
        ),
        (
            "12-run-end-final",
            json!({"class":"permanent", "provider_code":"account_selection_final"}),
            r#"{"error":{"class":"permanent","providerCode":"account_selection_final"},"reason":"provider_error","runId":"r-wait","state":"error"}"#,
        ),
    ] {
        assert_last_run_phone_bytes(
            json!({"run_id":"r-wait", "state":"error", "reason":"provider_error"}),
            Some(error),
            expected,
        )
        .await;
        let app = json!({"lastRun": serde_json::from_str::<Value>(expected).unwrap()});
        let file = format!("{sample}.json");
        if let Some(folder) = std::env::var_os("OCCIPITAL_WRITE_SAMPLES") {
            let folder = std::path::PathBuf::from(folder).join("phone-reader-account-wait-v2");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join(file), serde_json::to_vec(&app).unwrap()).unwrap();
            continue;
        }
        let published: Value = serde_json::from_slice(
            &std::fs::read(
                manifest
                    .join("../occipital-app-reader/test-vectors/phone-reader-account-wait-v2")
                    .join(&file),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(app, published, "{file}");
    }
}

/// A run the runner paused because its account is at its limit, to resume
/// itself (runner contract 0.8.42's `account_limit` and
/// `resume_not_before_ms`), reaches the apps with the reason and the time.
#[tokio::test]
async fn an_account_paused_run_names_when_it_resumes() {
    assert_last_run_phone_bytes(
        json!({"run_id":"r-paused", "state":"paused", "reason":"account_limit",
            "resume_not_before_ms": 1794450000000_u64}),
        None,
        r#"{"reason":"account_limit","resumeNotBeforeMs":1794450000000,"runId":"r-paused","state":"paused"}"#,
    )
    .await;
}

#[tokio::test]
async fn unreported_last_run_error_stays_absent_from_the_phone_bytes() {
    assert_last_run_phone_bytes(
        json!({"run_id":"r-error", "state":"error", "reason":"provider_error"}),
        None,
        r#"{"reason":"provider_error","runId":"r-error","state":"error"}"#,
    )
    .await;
}

#[tokio::test]
async fn completed_last_run_has_no_error_in_the_phone_bytes() {
    assert_last_run_phone_bytes(
        json!({"run_id":"r-completed", "state":"completed"}),
        None,
        r#"{"runId":"r-completed","state":"completed"}"#,
    )
    .await;
}

fn model() -> Value {
    json!({"provider":"anthropic", "model":"claude-opus-4-1", "variant":"thinking"})
}

fn grant() -> Value {
    json!({"modes":["tool_result", "turn_system", "steer", "idle_start"], "interrupt":true})
}

fn usage() -> Value {
    json!({"since":"run-first", "steps":3, "unreported_steps":0, "input_tokens":5120,
        "cached_input_tokens":4096, "output_tokens":812, "by_model":[
        {"provider":"anthropic", "model":"claude-opus-4-1", "variant":"thinking", "steps":3,
         "unreported_steps":0, "input_tokens":5120, "cached_input_tokens":4096, "output_tokens":812}]})
}

fn set_head(route: &FakeRoute, key: &str, value: Value) {
    route.head_extra.lock().unwrap().insert(key.into(), value);
}

fn clear_head(route: &FakeRoute, key: &str) {
    route.head_extra.lock().unwrap().remove(key);
}

/// A send path that exists but is never used: these tests only read.
struct Unused(SessionRef);

#[async_trait]
impl SendTarget for Unused {
    fn reference(&self) -> &SessionRef {
        &self.0
    }
    async fn call(&self, method: &'static str, _: Value) -> Result<Value, RunnerError> {
        panic!("a read called {method} on the send path")
    }
}

/// Routes whose session this module can also send to, as the daemon's routes are.
struct SendingRoutes(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for SendingRoutes {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
    fn sender(&self, resolved: &Resolved) -> Option<Arc<dyn SendTarget>> {
        Some(Arc::new(Unused(resolved.reference.clone())))
    }
}

/// An Entorhinal answer whose network policy changes from yes at 700 to no at 710.
struct ChangingNetwork;

#[async_trait]
impl Entorhinal for ChangingNetwork {
    async fn request(&self, method: &str, _: Value) -> Result<Value, ReadError> {
        // A session named under another harness: Entorhinal knows none here.
        if method == "agent.session_for_harness" {
            return Ok(json!({"code":"not_found", "detail":null, "message":"no agent session"}));
        }
        Ok(
            json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref", "incarnation":"entorhinal-test", "ended_at_ms":null,
            "runner":"broca", "session":"alfonso:s1", "harness":"broca", "project_root":"/workspace/resolved-project",
            "network":"no", "network_history":[{"network":"yes", "since_ms":700}, {"network":"no", "since_ms":710}]}}),
        )
    }
}

fn app(routes: Arc<dyn BrocaRoutes>, lookup: Arc<dyn Entorhinal>) -> Arc<AppReader> {
    Arc::new(AppReader::new(Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(lookup)),
        routes,
    ))))
}

fn sending(route: &Arc<FakeRoute>) -> Arc<AppReader> {
    app(
        Arc::new(SendingRoutes(route.clone())),
        Arc::new(ChangingNetwork),
    )
}

/// The module's own routes in these tests have no send path, so this module refuses
/// the app's sends to the session.
fn read_only(route: &Arc<FakeRoute>) -> Arc<AppReader> {
    app(Arc::new(Routes(route.clone())), Arc::new(Lookup))
}

async fn page(app: &AppReader, params: Value) -> Value {
    app.read(TRANSCRIPT_PAGE, params).await.unwrap()
}

async fn details(app: &AppReader) -> Value {
    app.read(DETAILS, json!({"session":"alfonso:s1", "harness":"broca"}))
        .await
        .unwrap()
}

fn calls(route: &FakeRoute, method: &str) -> usize {
    route
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call["method"] == method)
        .count()
}

fn claim(page: &Value, floor: &str, since: &str) -> Value {
    let mut request = params();
    request["claim"] =
        json!({"heldFloorMessageId":floor, "sinceMessageId":since, "heldVersion":page["version"]});
    request
}

/// Neither member is ever `null`, and a reply never carries both.
fn assert_send_members(name: &str, reply: &Value) {
    if let Some(sender) = reply.get("sender") {
        assert!(sender.is_object(), "{name}: sender is an object: {reply}");
        assert!(
            reply.get(SENDER_UNKNOWN).is_none(),
            "{name}: sender and {SENDER_UNKNOWN} together: {reply}"
        );
    }
    if let Some(flag) = reply.get(SENDER_UNKNOWN) {
        assert_eq!(flag, &json!(true), "{name}: {reply}");
    }
}

#[tokio::test]
async fn model_rides_on_the_newest_page_and_turn_carrying_claims_only() {
    let route = FakeRoute::new();
    let app = sending(&route);
    let unmodelled = calls(&route, "session.head");
    let without = page(&app, params()).await;
    assert!(without.get("model").is_none(), "absent means unknown");
    let heads_without = calls(&route, "session.head") - unmodelled;

    set_head(&route, "model", model());
    set_head(&route, "service_tier", json!("fast"));
    let before = calls(&route, "session.head");
    let newest = page(&app, params()).await;
    assert_eq!(newest["model"], model());
    assert_eq!(newest["serviceTier"], "fast");
    // The model comes from the head the page already reads, not another call.
    assert_eq!(calls(&route, "session.head") - before, heads_without);

    let mut cached = params();
    cached["activityToken"] = newest["activityToken"].clone();
    let unchanged = page(&app, cached).await;
    assert_eq!(unchanged["unchanged"], true);
    for member in ["model", "serviceTier", "sender", SENDER_UNKNOWN] {
        assert!(unchanged.get(member).is_none(), "{member}: {unchanged}");
    }

    let mut older = params();
    older["beforeOrdinal"] = json!(20);
    let older = page(&app, older).await;
    assert!(older.get("model").is_none(), "{older}");
    assert!(older.get("serviceTier").is_none(), "{older}");

    let matched = page(&app, claim(&newest, "m20", "m21")).await;
    assert_eq!(matched["claim"], "matched");
    assert_eq!(matched["model"], model());
    assert_eq!(matched["serviceTier"], "fast");

    route.history.store(8, Ordering::Relaxed);
    let expired = page(&app, claim(&newest, "m20", "m21")).await;
    assert_eq!(expired["claim"], "cursor_expired");
    assert_eq!(expired["model"], model());
    assert_eq!(expired["serviceTier"], "fast");
    assert!(expired["resync"].get("model").is_none());
}

#[tokio::test]
async fn session_details_reads_one_head_and_no_messages() {
    let route = FakeRoute::new();
    for (key, value) in [
        ("model", model()),
        ("service_tier", json!("fast")),
        ("usage", usage()),
        ("sender", grant()),
    ] {
        set_head(&route, key, value);
    }
    *route.incarnation.lock().unwrap() = "log-7f3a".into();
    let app = sending(&route);
    let reply = details(&app).await;
    assert_eq!(
        reply,
        json!({"model":model(), "serviceTier":"fast", "usage":usage(), "sender":grant(), "incarnation":"log-7f3a", "lastRun":last_run()})
    );
    let methods: Vec<_> = route
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|call| call["method"].clone())
        .collect();
    assert_eq!(methods, ["session.head"]);
    assert_eq!(route.attaches.load(Ordering::Relaxed), 0);

    // A head naming no tier gets none: Occipital passes the head's own word.
    for key in ["model", "service_tier", "usage", "sender"] {
        clear_head(&route, key);
    }
    let heads = calls(&route, "session.head");
    assert_eq!(
        details(&app).await,
        json!({SENDER_UNKNOWN: true, "incarnation":"log-7f3a", "lastRun":last_run()})
    );
    assert_eq!(calls(&route, "session.head"), heads + 1);
    assert_eq!(calls(&route, "session.read"), 0);

    // Counts a phone can't hold exactly are left out as unknown.
    let mut huge = usage();
    huge["steps"] = json!(1_u64 << 53);
    set_head(&route, "usage", huge);
    assert_eq!(
        details(&app).await,
        json!({SENDER_UNKNOWN: true, "incarnation":"log-7f3a", "lastRun":last_run()})
    );
    // A log identity outside 1 to 128 bytes breaks the contract's head rule.
    *route.incarnation.lock().unwrap() = "x".repeat(129);
    assert_eq!(
        app.read(DETAILS, json!({"session":"alfonso:s1", "harness":"broca"}))
            .await
            .unwrap_err()
            .code,
        "invalid_provider_reply"
    );
    *route.incarnation.lock().unwrap() = "log-7f3a".into();

    let refused = |params: Value| {
        let app = app.clone();
        async move { app.read(DETAILS, params).await.unwrap_err().code }
    };
    assert_eq!(
        refused(json!({"session":"alfonso:s1", "harness":"opencode2"})).await,
        "session_unknown"
    );
    assert_eq!(refused(json!({"session":" "})).await, "invalid_request");
    assert_eq!(refused(json!({"harness":"broca"})).await, "invalid_request");
    assert_eq!(calls(&route, "session.read"), 0);
}

#[tokio::test]
async fn the_send_grant_is_the_runners_sender_unknown_without_one_and_absent_when_refused() {
    let route = FakeRoute::new();
    let app = sending(&route);
    set_head(&route, "sender", grant());
    let newest = page(&app, params()).await;
    let matched = page(&app, claim(&newest, "m20", "m21")).await;
    for (name, reply) in [
        ("newest", &newest),
        ("matched", &matched),
        ("details", &details(&app).await),
    ] {
        assert_send_members(name, reply);
        assert_eq!(reply["sender"], grant(), "{name}");
    }

    // A runner that leaves `sender` out without declaring `head_sender` may
    // predate the member, so its silence is unknown, never "may not send".
    clear_head(&route, "sender");
    let newest = page(&app, params()).await;
    let matched = page(&app, claim(&newest, "m20", "m21")).await;
    for (name, reply) in [
        ("newest", &newest),
        ("matched", &matched),
        ("details", &details(&app).await),
    ] {
        assert_send_members(name, reply);
        assert!(reply.get("sender").is_none(), "{name}");
        assert_eq!(reply[SENDER_UNKNOWN], true, "{name}");
    }

    // A runner declaring `head_sender` that reports no `sender` grants nothing.
    *route.describe.lock().unwrap() = Some(describe_with(&[HEAD_SENDER]));
    let app = sending(&route);
    let newest = page(&app, params()).await;
    let matched = page(&app, claim(&newest, "m20", "m21")).await;
    for (name, reply) in [
        ("newest", &newest),
        ("matched", &matched),
        ("details", &details(&app).await),
    ] {
        assert_send_members(name, reply);
        assert!(reply.get("sender").is_none(), "{name}: {reply}");
        assert!(reply.get(SENDER_UNKNOWN).is_none(), "{name}: {reply}");
    }

    // An unreadable describe leaves no group to read, so the silence is unknown.
    *route.describe.lock().unwrap() = None;
    let app = sending(&route);
    let reply = details(&app).await;
    assert_eq!(reply[SENDER_UNKNOWN], true, "{reply}");
    let newest = page(&app, params()).await;
    assert_eq!(newest[SENDER_UNKNOWN], true, "{newest}");

    // This module has no send path to the session: the app may not send, whatever
    // the runner grants.
    set_head(&route, "sender", grant());
    let app = read_only(&route);
    let newest = page(&app, params()).await;
    for (name, reply) in [("newest", &newest), ("details", &details(&app).await)] {
        assert!(reply.get("sender").is_none(), "{name}: {reply}");
        assert!(reply.get(SENDER_UNKNOWN).is_none(), "{name}: {reply}");
    }
}

#[tokio::test]
async fn role_describe_is_read_once_per_session_and_again_after_its_route_is_lost() {
    let route = FakeRoute::new();
    let sources = Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(ChangingNetwork))),
        Arc::new(SendingRoutes(route.clone())),
    ));
    let app = AppReader::new(sources.clone());
    for _ in 0..3 {
        page(&app, params()).await;
        details(&app).await;
    }
    assert_eq!(calls(&route, "role.describe"), 1);
    // A head that names the grant needs no describe at all.
    set_head(&route, "sender", grant());
    details(&app).await;
    assert_eq!(calls(&route, "role.describe"), 1);
    clear_head(&route, "sender");
    let reference = SessionRef {
        runner: "broca".into(),
        session: "alfonso:s1".into(),
    };
    sources.source(&reference).await.unwrap().lost();
    details(&app).await;
    page(&app, params()).await;
    assert_eq!(calls(&route, "role.describe"), 2);
}

/// The page of up to two messages before `ordinal`.
async fn before(app: &AppReader, ordinal: u64) -> Result<Value, ReadError> {
    let mut request = params();
    request["limit"] = json!(2);
    request["beforeOrdinal"] = json!(ordinal);
    app.read(TRANSCRIPT_PAGE, request).await
}

/// A compaction cut moves where the model's input starts, and nothing the apps
/// read: the same history counter and log, so a held claim still matches and the
/// messages before the cut still page back.
#[tokio::test]
async fn a_compaction_cut_keeps_a_claim_and_older_pages() {
    let route = FakeRoute::new();
    let app = sending(&route);
    let newest = page(&app, params()).await;
    set_head(&route, "first_ordinal", json!(22));
    let matched = page(&app, claim(&newest, "m20", "m22")).await;
    assert_eq!(matched["claim"], "matched", "{matched}");
    assert_eq!(matched["version"], newest["version"]);
    route
        .entries
        .lock()
        .unwrap()
        .push(entry("m23", Some(23), 740));
    let matched = page(&app, claim(&newest, "m20", "m22")).await;
    assert_eq!(matched["claim"], "matched", "{matched}");
    assert_eq!(matched["newTurns"][0]["messageId"], "m23");
    let older = before(&app, 20).await.unwrap();
    assert_eq!(older["messages"][0]["messageId"], "old-a", "{older}");
}

#[tokio::test]
async fn a_recreated_log_expires_claims_and_cursors_even_at_history_zero() {
    let route = FakeRoute::new();
    route.history.store(0, Ordering::Relaxed);
    *route.entries.lock().unwrap() = vec![
        entry("m1", Some(1), 700),
        entry("m2", Some(2), 710),
        entry("m3", Some(3), 720),
    ];
    *route.incarnation.lock().unwrap() = "log-1".into();
    let app = sending(&route);
    let old = page(&app, params()).await;
    // The log is created again under the same key: the same counter and recycled
    // mids, with different content.
    *route.incarnation.lock().unwrap() = "log-2".into();
    route.entries.lock().unwrap()[0]["message"]["content"][0]["text"] =
        json!("unrelated replacement");
    let reply = page(&app, claim(&old, "m1", "m2")).await;
    assert_eq!(reply["claim"], "cursor_expired", "{reply}");
    assert_ne!(reply["version"], old["version"]);
}

/// A Broca entry stamped with the runner's per-entry `written_at`, or with no write
/// time at all.
fn written(mid: &str, ordinal: u64, at: Option<i64>) -> Value {
    let mut entry = json!({"mid":mid, "ordinal":ordinal,
        "message":{"role":"assistant", "content":[{"type":"text", "text":format!("reply {mid}")}]}});
    if let Some(at) = at {
        entry["written_at"] = json!(at);
    }
    entry
}

fn sample_route() -> Arc<FakeRoute> {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = vec![
        written("m20", 20, Some(705)),
        written("m21", 21, Some(706)),
        written("m22", 22, Some(708)),
    ];
    route
}

/// Each sample, by file name, as the module's handlers answer it now.
async fn samples() -> Vec<(&'static str, Value)> {
    let mut samples = Vec::new();

    let route = sample_route();
    set_head(&route, "model", model());
    set_head(&route, "sender", grant());
    let app = sending(&route);
    let newest = page(&app, params()).await;
    route
        .entries
        .lock()
        .unwrap()
        .push(written("m23", 23, Some(709)));
    let matched = page(&app, claim(&newest, "m20", "m22")).await;
    assert_eq!(matched["claim"], "matched");
    samples.push(("newest-page-model.json", newest));
    samples.push(("matched-claim-model.json", matched));

    let route = sample_route();
    for (key, value) in [("model", model()), ("usage", usage()), ("sender", grant())] {
        set_head(&route, key, value);
    }
    *route.incarnation.lock().unwrap() = "6a6ce2eb-b065-4df0-806e-44bcf531e128".into();
    samples.push(("details-usage.json", details(&sending(&route)).await));
    clear_head(&route, "usage");
    clear_head(&route, "sender");
    samples.push(("details-no-usage.json", details(&sending(&route)).await));
    *route.describe.lock().unwrap() = Some(describe_with(&[HEAD_SENDER]));
    samples.push(("details-no-grant.json", details(&sending(&route)).await));
    set_head(&route, "sender", grant());
    samples.push((
        "details-sender-refused.json",
        details(&read_only(&route)).await,
    ));

    // The network policy is yes from 700 and no from 710.
    let route = sample_route();
    *route.entries.lock().unwrap() = vec![
        written("m20", 20, Some(705)),
        written("m21", 21, Some(715)),
        written("m22", 22, None),
    ];
    let network = page(&sending(&route), params()).await;
    let values: Vec<_> = network["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["network"].clone())
        .collect();
    assert_eq!(values, ["yes", "no", "unknown"]);
    samples.push(("newest-page-network.json", network));
    samples
}

/// The session fact samples the apps pin, `phone-session-facts-v3`, are what the
/// handlers answer; `served_samples` says how the set is checked and rewritten.
#[tokio::test]
async fn the_session_fact_samples_are_what_the_handlers_answer() {
    let mut served = super::served_samples::Served::new(SAMPLES, sending(&sample_route()));
    for (name, reply) in samples().await {
        assert_send_members(name, &reply);
        served.keep_file(name, serde_json::to_vec(&reply).unwrap());
    }
    served.check();
}

/// The sample set the apps pin for a run Broca paused because its account is
/// at its limit, `phone-reader-account-pause-v1`; its README says what each
/// file shows. The paused run's state is Broca's recorded `last_run_state`
/// (`tests/data/broca-runner-0842-1112/`), on chat details, the newest page
/// and a claim, and then the same chat once the run has resumed and finished.
#[tokio::test]
async fn the_account_pause_samples_are_what_the_handlers_answer() {
    let recorded = |name: &str| -> Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/broca-runner-0842-1112")
            .join(name);
        serde_json::from_slice::<Value>(&std::fs::read(path).unwrap()).unwrap()["last_run_state"]
            .clone()
    };
    let route = sample_route();
    set_head(&route, "model", model());
    set_head(&route, "sender", grant());
    *route.incarnation.lock().unwrap() = "6a6ce2eb-b065-4df0-806e-44bcf531e128".into();
    *route.run_state.lock().unwrap() = recorded("session-head-account-limit-after.json");
    let app = sending(&route);
    let mut served =
        super::served_samples::Served::new("phone-reader-account-pause-v1", app.clone());
    let details_request =
        json!({"method": DETAILS, "params": {"session":"alfonso:s1", "harness":"broca"}});
    let page_request = json!({"method": TRANSCRIPT_PAGE, "params": params()});
    let paused = json!({"reason": "account_limit", "resumeNotBeforeMs": 1_790_000_600_000_u64,
        "runId": "run-1", "state": "paused"});
    let last_run =
        |bytes: &[u8]| super::served_samples::decoded(bytes)["result"]["lastRun"].clone();

    let reply = served.read(1, &details_request).await;
    assert_eq!(last_run(&reply), paused);
    served.keep("01-details-paused", &details_request, reply);
    let newest = served.read(2, &page_request).await;
    assert_eq!(last_run(&newest), paused);
    let claim_request = json!({"method": TRANSCRIPT_PAGE,
        "params": claim(&super::served_samples::decoded(&newest)["result"], "m20", "m22")});
    served.keep("02-newest-page-paused", &page_request, newest);
    let claimed = served.read(3, &claim_request).await;
    assert_eq!(last_run(&claimed), paused);
    assert_eq!(
        super::served_samples::decoded(&claimed)["result"]["claim"],
        "matched"
    );
    served.keep("03-claim-paused", &claim_request, claimed);

    *route.run_state.lock().unwrap() = recorded("session-head-completed.json");
    let reply = served.read(4, &details_request).await;
    assert_eq!(
        last_run(&reply),
        json!({"runId": "run-1", "state": "completed"})
    );
    served.keep("04-details-resumed-and-finished", &details_request, reply);
    served.check();
}

/// The sample set the apps pin for a run that ended because the account's key
/// is out of credit, `phone-reader-out-of-credit-v1`; its README says what each
/// file shows. The run's state and error are Broca's recorded `session.head`
/// bodies (`tests/data/broca-head-provider-code-1112/`): `billing_exhausted`
/// on chat details, the newest page and a claim, then a permanent error that
/// names no code, for contrast.
#[tokio::test]
async fn the_out_of_credit_samples_are_what_the_handlers_answer() {
    let recorded = |name: &str| -> Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/broca-head-provider-code-1112/after")
            .join(name);
        serde_json::from_slice::<Value>(&std::fs::read(path).unwrap()).unwrap()["result"].clone()
    };
    let route = sample_route();
    set_head(&route, "model", model());
    set_head(&route, "sender", grant());
    *route.incarnation.lock().unwrap() = "6a6ce2eb-b065-4df0-806e-44bcf531e128".into();
    let use_head = |head: &Value| {
        *route.run_state.lock().unwrap() = head["last_run_state"].clone();
        set_head(&route, "last_run_error", head["last_run_error"].clone());
    };
    let billing = recorded("store.billing_exhausted.json");
    use_head(&billing);
    let app = sending(&route);
    let mut served =
        super::served_samples::Served::new("phone-reader-out-of-credit-v1", app.clone());
    let details_request =
        json!({"method": DETAILS, "params": {"session":"alfonso:s1", "harness":"broca"}});
    let page_request = json!({"method": TRANSCRIPT_PAGE, "params": params()});
    let last_run =
        |bytes: &[u8]| super::served_samples::decoded(bytes)["result"]["lastRun"].clone();
    let out_of_credit = json!({"error": {"class": "permanent", "providerCode": "billing_exhausted"},
        "reason": "error", "runId": billing["last_run_state"]["run_id"], "state": "error"});

    let reply = served.read(1, &details_request).await;
    assert_eq!(last_run(&reply), out_of_credit);
    served.keep("01-details-out-of-credit", &details_request, reply);
    let newest = served.read(2, &page_request).await;
    assert_eq!(last_run(&newest), out_of_credit);
    let claim_request = json!({"method": TRANSCRIPT_PAGE,
        "params": claim(&super::served_samples::decoded(&newest)["result"], "m20", "m22")});
    served.keep("02-newest-page-out-of-credit", &page_request, newest);
    let claimed = served.read(3, &claim_request).await;
    assert_eq!(last_run(&claimed), out_of_credit);
    served.keep("03-claim-out-of-credit", &claim_request, claimed);

    let no_code = recorded("store.permanent_no_code.json");
    use_head(&no_code);
    let reply = served.read(4, &details_request).await;
    assert_eq!(
        last_run(&reply),
        json!({"error": {"class": "permanent"}, "reason": "error",
            "runId": no_code["last_run_state"]["run_id"], "state": "error"})
    );
    served.keep("04-details-permanent-no-code", &details_request, reply);
    served.check();
}

/// The sample set the apps pin for a head's service tier,
/// `phone-session-tier-v1`; its README says what each file shows.
/// `served_samples` says how the set is checked and rewritten.
#[tokio::test]
async fn the_service_tier_samples_are_what_the_handlers_answer() {
    let route = sample_route();
    for (key, value) in [
        ("model", model()),
        ("service_tier", json!("fast")),
        ("sender", grant()),
    ] {
        set_head(&route, key, value);
    }
    *route.incarnation.lock().unwrap() = "6a6ce2eb-b065-4df0-806e-44bcf531e128".into();
    let app = sending(&route);
    let mut served = super::served_samples::Served::new("phone-session-tier-v1", app.clone());
    let details_request =
        json!({"method": DETAILS, "params": {"session":"alfonso:s1", "harness":"broca"}});
    let page_request = json!({"method": TRANSCRIPT_PAGE, "params": params()});

    let tiered = served.read(1, &details_request).await;
    assert_eq!(
        super::served_samples::decoded(&tiered)["result"]["serviceTier"],
        "fast"
    );
    served.keep("01-details-fast", &details_request, tiered);
    let page = served.read(2, &page_request).await;
    assert_eq!(
        super::served_samples::decoded(&page)["result"]["serviceTier"],
        "fast"
    );
    served.keep("02-newest-page-fast", &page_request, page);

    clear_head(&route, "service_tier");
    let untiered = served.read(3, &details_request).await;
    assert!(super::served_samples::decoded(&untiered)["result"]
        .get("serviceTier")
        .is_none());
    served.keep("03-details-no-tier", &details_request, untiered);
    served.check();
}
