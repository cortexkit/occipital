//! A call moved to another runner with its history keeps its tool row and its
//! riding notices. The runner that now serves the chat names the call with its
//! original key and `call_carrier`, the runner that minted the key (llm-runner
//! `call_identity`), and the call's records and notices stay filed under that
//! runner, so Occipital finds them by the session and key the app asks about.
//! Here a call in an OpenCode 2 chat on Thalamus's runner moves to Broca,
//! which gives the chat a new session id.

use super::tool_display_samples::{pin, reserved, AnyHarness};
use super::*;
use crate::display::app::ToolDisplay;
use crate::display::store::{DisplayStore, ViewState};
use cortexkit_role_tool_display::{DisplayKind, DisplayWrite};
use cortexkit_test_support::ScratchDir;

const NOTICE: &str = "Context is at 62%.";

/// The notice that rode the call, as a read's `notices` fact lists it, from the
/// runner that minted the call or, with `carrier`, from the one it moved to.
fn notice(carrier: Option<&str>) -> Value {
    let mut fact = json!({"part": 0, "call_key": "ck_ls", "send_id": "send-1",
        "sender": "magic-context", "text": NOTICE, "delivered_as": "tool_result"});
    if let Some(carrier) = carrier {
        fact["call_carrier"] = json!(carrier);
    }
    fact
}

/// The call as Thalamus's runner minted and served it, in an OpenCode 2 chat.
fn on_thalamus() -> Vec<Value> {
    vec![json!({"mid": "msg_a", "ordinal": 1,
        "tool_times": [{"tool_call_id": "call_ls", "started_at": 10, "finished_at": 20,
            "call_key": "ck_ls", "schema_pin": pin("bash", 'b')}],
        "notices": [notice(None)],
        "message": {"id": "msg_a", "type": "assistant", "time": {"created": 1_u64, "completed": 30_u64},
            "content": [{"type": "tool", "id": "call_ls", "name": "bash",
                "state": {"status": "completed", "input": {"command": "ls"},
                    "content": [{"type": "text", "text": "x\n"}]}}]}})]
}

/// The same call once Broca imported the chat: its key unchanged, with Thalamus's
/// runner as its carrier.
fn on_broca() -> Vec<Value> {
    vec![
        json!({"mid": "m1", "ordinal": 1,
            "tool_times": [{"tool_call_id": "call_ls", "started_at": 10, "finished_at": 20,
                "call_key": "ck_ls", "call_carrier": THALAMUS_RUNNER_MODULE_ID,
                "schema_pin": pin("bash", 'b')}],
            "message": {"role": "assistant", "content": [
                {"type": "tool_call", "tool_call_id": "call_ls", "name": "bash", "input": {"command": "ls"}}]}}),
        json!({"mid": "m2", "ordinal": 2,
            "notices": [notice(Some(THALAMUS_RUNNER_MODULE_ID))],
            "message": {"role": "tool", "content": [
                {"type": "tool_result", "tool_call_id": "call_ls", "content": "x\n"}]}}),
    ]
}

/// Wait for the background writer to keep the call under `runner`.
async fn kept(store: &DisplayStore, runner: &str, session: &str) {
    for _ in 0..1_000 {
        if store
            .call_pin(runner, "ck_ls")
            .unwrap()
            .is_some_and(|(kept, _)| kept == session)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("the call wasn't kept in {session}");
}

#[tokio::test]
async fn a_moved_call_keeps_its_tool_row_and_notices() {
    let dir = ScratchDir::new("occipital-moved-calls");
    let descriptor = crate::store::tests::descriptor(&dir);
    let main = crate::store::open(&descriptor).unwrap();
    let store = Arc::new(DisplayStore::open(&descriptor, dir.path()).unwrap());
    let file = Arc::new(crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap());
    let (notices, _notice_task) = crate::notices::Notices::start(file.clone());
    let (call_pins, _pin_task) = crate::display::call_pins::start(store.clone(), None);
    let pins = crate::display::call_pins::CallPinSlot::default();
    let _ = pins.set(call_pins);
    let notice_slot = crate::notices::NoticeSlot::default();
    let _ = notice_slot.set(notices.clone());
    let hints = tokio::sync::broadcast::channel(crate::display::app::HINTS).0;
    let tool = Arc::new(ToolDisplay::new(store.clone(), None, hints));

    // AFT serves the call's view, and wrote its record for the runner that ran it.
    let manifest = super::tool_display_samples::view("aft.bash", json!({}), "", "", 0);
    let input_schema = super::tool_display_samples::input_schema(
        "command",
        json!({"command": {"type": "string"}}),
    );
    store
        .keep_view(
            "aft",
            &pin("bash", 'b'),
            &ViewState::View {
                manifest,
                input_schema,
            },
            "1.0",
            1,
        )
        .unwrap();
    let write = DisplayWrite::new(
        reserved(THALAMUS_RUNNER_MODULE_ID),
        "ck_ls",
        DisplayKind::Record,
        pin("bash", 'b'),
        json!({"durationMs": 10, "outcome": "success"})
            .as_object()
            .unwrap()
            .clone(),
    );
    store.write(&reserved("aft"), &write, 1).unwrap();

    // Before the move: Thalamus's runner serves the chat.
    let thalamus = FakeRoute::new();
    *thalamus.entries.lock().unwrap() = on_thalamus();
    *thalamus.run_state.lock().unwrap() = Value::Null;
    let before = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::Hosts(
                THALAMUS_RUNNER_MODULE_ID,
                "opencode2",
            )))),
            Arc::new(AnyHarness(thalamus)),
        )
        .with_call_pins(pins.clone())
        .with_notices(notice_slot.clone()),
    )));
    before.set_tool_display(tool.clone());
    before.set_notices(notices.clone());
    let old = json!({"session": "ses_head", "harness": "opencode2"});
    let mut page = old.clone();
    page["limit"] = json!(50);
    before
        .read(super::super::super::TRANSCRIPT_PAGE, page)
        .await
        .unwrap();
    kept(&store, THALAMUS_RUNNER_MODULE_ID, "ses_head").await;

    // After: Broca serves the chat under a new session id.
    let broca = FakeRoute::new();
    *broca.entries.lock().unwrap() = on_broca();
    let after = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(Lookup))),
            Arc::new(Routes(broca)),
        )
        .with_call_pins(pins)
        .with_notices(notice_slot),
    )));
    after.set_tool_display(tool);
    after.set_notices(notices);
    let page = after
        .read(super::super::super::TRANSCRIPT_PAGE, params())
        .await
        .unwrap();
    let call = &page["messages"][0]["parts"][0];
    assert_eq!(call["callKey"], "ck_ls", "{call}");
    // Which runner minted the call is Occipital's to know; the app asks by key.
    assert!(call.get("callCarrier").is_none(), "{call}");
    // The call follows its chat into the new session, still filed under the
    // runner that minted its key.
    kept(&store, THALAMUS_RUNNER_MODULE_ID, "alfonso:s1").await;
    assert!(store.call_pin("broca", "ck_ls").unwrap().is_none());

    let new = json!({"session": "alfonso:s1", "harness": "broca", "callKeys": ["ck_ls"]});
    let records = after
        .read(crate::display::app::RECORDS, new.clone())
        .await
        .unwrap();
    assert_eq!(
        records["calls"]["ck_ls"]["record"],
        json!({"durationMs": 10, "outcome": "success"}),
        "{records}"
    );
    let mut read = None;
    for _ in 0..1_000 {
        let found = after.read(crate::notices::READ, new.clone()).await.unwrap();
        if found["calls"].get("ck_ls").is_some() {
            read = Some(found);
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    // Seen before and after the move, it's one notice, not two.
    assert_eq!(
        read.expect("the moved call's notice")["calls"]["ck_ls"],
        json!([{"sender": "magic-context", "text": NOTICE, "deliveredAs": "tool_result"}])
    );
    // The old session no longer holds the call or its notice.
    let gone = json!({"session": "ses_head", "harness": "opencode2", "callKeys": ["ck_ls"]});
    for method in [crate::display::app::RECORDS, crate::notices::READ] {
        assert_eq!(
            before.read(method, gone.clone()).await.unwrap()["calls"],
            json!({}),
            "{method}"
        );
    }
}
