//! Sends kept while the session's runner can't take them (`send::resend`):
//! kept in order, resent under their own send ids, never delivered twice, and
//! given up after the limit or on a final refusal.

use super::*;

/// The session's waiting list, as `(clientMessageId, state, held, reason)`.
async fn listed(f: &Fixture) -> Vec<(Value, Value, bool, Value)> {
    let (list, _) = f.sender.queued("s").await.unwrap_or_default();
    list.as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|entry| {
            (
                entry["clientMessageId"].clone(),
                entry["state"].clone(),
                entry.get("held").is_some(),
                entry.get("reason").cloned().unwrap_or(Value::Null),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_send_the_runner_cant_take_is_kept_and_resent_once() {
    let f = granted_fixture(Kind::Head);
    f.runner.unreachable.store(true, Ordering::SeqCst);
    let kept = f.sender.send(message("m1", "hello")).await.unwrap();
    assert_eq!((&kept["ok"], &kept["held"]), (&json!(true), &json!(true)));
    assert!(f.runner.written().is_empty());
    assert_eq!(
        listed(&f).await,
        [(json!("m1"), json!("queued"), true, Value::Null)]
    );
    // Still down: the resend waits for a later try.
    assert!(!f.sender.resend_kept_now("s").await);
    f.runner.unreachable.store(false, Ordering::SeqCst);
    assert!(f.sender.resend_kept_now("s").await);
    let written = f.runner.written();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0]["send_id"], kept["effectId"]);
    // The runner holds it for the next turn; it is followed as any send.
    assert_eq!(
        listed(&f).await,
        [(json!("m1"), json!("queued"), false, Value::Null)]
    );
    assert!(!f.sender.resend_kept_now("s").await, "nothing left");
}

/// The runner took the send but its answer was lost: the resend, under the
/// same send id, gets that send back and writes nothing new.
#[tokio::test]
async fn a_send_whose_answer_was_lost_is_not_delivered_twice() {
    let f = granted_fixture(Kind::Head);
    f.runner.running.store(false, Ordering::SeqCst);
    f.runner.lose_next_send_reply.store(true, Ordering::SeqCst);
    let kept = f.sender.send(message("m1", "hello")).await.unwrap();
    assert_eq!(kept["held"], true);
    assert_eq!(f.runner.written().len(), 1);
    assert!(f.sender.resend_kept_now("s").await);
    assert_eq!(f.runner.written().len(), 1);
    assert_eq!(f.runner.written()[0]["send_id"], kept["effectId"]);
}

/// A send to a chat with kept sends waits behind them, even once the runner
/// is back, and they go out in the order the app sent them.
#[tokio::test]
async fn later_sends_wait_behind_kept_ones_in_order() {
    let f = granted_fixture(Kind::Head);
    f.runner.unreachable.store(true, Ordering::SeqCst);
    f.sender.send(message("m1", "first")).await.unwrap();
    f.runner.unreachable.store(false, Ordering::SeqCst);
    f.now.fetch_add(1, Ordering::SeqCst);
    let behind = f.sender.send(message("m2", "second")).await.unwrap();
    assert_eq!(behind["held"], true);
    assert!(f.runner.written().is_empty());
    assert!(f.sender.resend_kept_now("s").await);
    assert!(f.sender.resend_kept_now("s").await);
    let sends = f.runner.calls();
    let texts: Vec<&Value> = sends
        .iter()
        .filter(|(method, _)| method == "session.send")
        .map(|(_, body)| &body["prompt"])
        .collect();
    assert_eq!(texts, [&json!("first"), &json!("second")]);
}

/// A steer kept during a restart goes out as the app asked: into the running
/// turn if one runs, else starting one.
#[tokio::test]
async fn a_kept_steer_is_resent_as_a_steer_that_may_start_a_turn() {
    let f = granted_fixture(Kind::Head);
    f.runner.unreachable.store(true, Ordering::SeqCst);
    let mut steer = message("m1", "skip staging");
    steer["mode"] = json!("steer");
    f.sender.send(steer).await.unwrap();
    f.runner.unreachable.store(false, Ordering::SeqCst);
    assert!(f.sender.resend_kept_now("s").await);
    assert_eq!(
        f.runner.written()[0]["delivery"],
        json!(["steer", "idle_start"])
    );
}

#[tokio::test]
async fn a_kept_send_is_given_up_after_the_limit() {
    let f = granted_fixture(Kind::Head);
    f.runner.unreachable.store(true, Ordering::SeqCst);
    f.sender.send(message("m1", "hello")).await.unwrap();
    f.now.fetch_add(
        crate::app_reader::send_retry::RESEND_LIMIT_MS - 1,
        Ordering::SeqCst,
    );
    assert!(!f.sender.resend_kept_now("s").await);
    f.now.fetch_add(1, Ordering::SeqCst);
    assert!(f.sender.resend_kept_now("s").await);
    assert_eq!(
        listed(&f).await,
        [(
            json!("m1"),
            json!("unsent"),
            false,
            json!("runner_unavailable")
        )]
    );
    assert!(f.runner.written().is_empty());
}

/// A refusal the runner says can't pass later ends the send at once, listed
/// unsent with its code; one it says may pass is tried again.
#[tokio::test]
async fn a_final_refusal_on_resend_ends_the_send() {
    let f = granted_fixture(Kind::Head);
    f.runner.unreachable.store(true, Ordering::SeqCst);
    f.sender.send(message("m1", "hello")).await.unwrap();
    f.runner.unreachable.store(false, Ordering::SeqCst);
    f.runner
        .scripted
        .lock()
        .unwrap()
        .push_back(FakeRunner::refuse("transient", None));
    assert!(!f.sender.resend_kept_now("s").await);
    f.runner
        .scripted
        .lock()
        .unwrap()
        .push_back(FakeRunner::refuse("pre_user_unavailable", None));
    assert!(f.sender.resend_kept_now("s").await);
    assert_eq!(
        listed(&f).await,
        [(json!("m1"), json!("unsent"), false, json!("runner_refused"))]
    );
}

/// "Send now" on a kept send is answered `not_queued`, without the runner.
#[tokio::test]
async fn send_now_on_a_kept_send_is_not_queued() {
    let f = granted_fixture(Kind::Head);
    f.runner.unreachable.store(true, Ordering::SeqCst);
    f.sender.send(message("m1", "hello")).await.unwrap();
    let calls = f.runner.calls().len();
    let answer = f
        .sender
        .promote(json!({"session": "s", "clientMessageId": "m1", "clientRequestId": "p1"}))
        .await
        .unwrap();
    assert_eq!(answer, json!({"outcome": "not_queued"}));
    assert_eq!(f.runner.calls().len(), calls);
}

/// A restart of this module keeps the kept sends, and the started sender
/// resends them on its own.
#[tokio::test]
async fn kept_sends_survive_a_restart_and_resend_themselves() {
    let mut f = granted_fixture(Kind::Head);
    f.runner.unreachable.store(true, Ordering::SeqCst);
    f.sender.send(message("m1", "hello")).await.unwrap();
    f.restart();
    f.runner.unreachable.store(false, Ordering::SeqCst);
    let _background = f.sender.start();
    eventually(|| f.runner.written().len() == 1).await;
}

/// A worker's steer is not kept: it may only join the run it was sent into,
/// which a restart ends, so it is answered `not_sent` as before.
#[tokio::test]
async fn a_worker_steer_is_not_kept() {
    let f = granted_fixture(Kind::Worker);
    f.runner.lose_next_send_reply.store(true, Ordering::SeqCst);
    let reply = f.sender.send(message("m1", "hello")).await.unwrap();
    assert_eq!(reply["code"], "not_sent");
    assert!(listed(&f).await.is_empty());
}
