//! The display watch loop; its host owns admission, projections and delivery backpressure.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::display::{
    DisplayProjection, DisplayReadError, DisplayReadReply, DisplayWindowExpired,
    SequencedDisplayFrame,
};

pub const NOTICE_BOUND: usize = 196_608;
pub const DISPLAY_WINDOW: Duration = Duration::from_millis(300);

#[derive(Debug)]
pub struct DisplayWatchStopped;

#[async_trait]
pub trait DisplayWatchHost: Send + Sync {
    /// Validate using the same read handler as polling, without creating a projection.
    async fn read(&self, params: Value) -> Result<DisplayReadReply, DisplayReadError>;
    fn projection(&self, session: &str) -> Option<Arc<DisplayProjection>>;
    fn projection_changes(&self, session: &str) -> watch::Receiver<u64>;
    fn readmit(&self) -> Result<(), DisplayReadError>;
    /// Delivery must return when the watch is stopped, including while its channel is stalled.
    async fn emit(&self, body: Vec<u8>) -> Result<(), DisplayWatchStopped>;
    async fn stopped(&self);
    /// The computer's time in Unix milliseconds, stamped on the tool frames of
    /// each notice.
    fn now_ms(&self) -> u64 {
        crate::display::now_ms()
    }
}

pub fn display_body(reply: &DisplayReadReply) -> Vec<u8> {
    serde_json::to_vec(&serde_json::to_value(reply).expect("a display reply always serializes"))
        .expect("a display notice always serializes")
}

/// If even one frame is too large, the phone resumes after its sequence without re-reading it.
fn display_expired(seq: u64) -> DisplayReadReply {
    DisplayReadReply::Expired {
        seq,
        window_expired: DisplayWindowExpired { oldest_seq: seq },
    }
}

/// The next notice and the sequence the watch advances to; None when there is nothing to send.
pub fn next_display_notice(reply: DisplayReadReply) -> Option<(DisplayReadReply, Option<u64>)> {
    match reply {
        DisplayReadReply::Absent { .. } => None,
        DisplayReadReply::Baseline { seq, frame, tools } => {
            let baseline = DisplayReadReply::Baseline { seq, frame, tools };
            if display_body(&baseline).len() <= NOTICE_BOUND {
                Some((baseline, Some(seq)))
            } else {
                Some((display_expired(seq), Some(seq)))
            }
        }
        // The caller handles an expired window by reading a fresh Baseline.
        DisplayReadReply::Expired { .. } => None,
        DisplayReadReply::Incremental { frames, .. } => {
            let first: &SequencedDisplayFrame = frames.first()?;
            let incremental = |taken: &[SequencedDisplayFrame]| DisplayReadReply::Incremental {
                seq: taken.last().map_or(0, |frame| frame.seq),
                frames: taken.to_vec(),
            };
            if display_body(&incremental(std::slice::from_ref(first))).len() > NOTICE_BOUND {
                return Some((display_expired(first.seq), Some(first.seq)));
            }
            // Split at whole frames; each notice's seq is its last frame's seq.
            let mut taken = 1;
            while taken < frames.len()
                && display_body(&incremental(&frames[..=taken])).len() <= NOTICE_BOUND
            {
                taken += 1;
            }
            let last = frames[taken - 1].seq;
            Some((incremental(&frames[..taken]), Some(last)))
        }
    }
}

pub async fn run<H: DisplayWatchHost>(w: &H, params: Value) -> Result<(), DisplayReadError> {
    w.readmit()?;
    // The read validates the params; its refusal ends the subscription.
    w.read(params.clone()).await?;
    let session = params
        .get("session")
        .and_then(Value::as_str)
        .ok_or_else(|| DisplayReadError {
            code: "invalid_request",
            message: "session.display_watch requires 'session'".into(),
        })?;
    // The read above refused anything but a boolean here.
    let tools = params.get("acceptToolFrames") == Some(&Value::Bool(true));
    let mut projections = w.projection_changes(session);
    let mut since: Option<u64> = None;
    let mut last_notice: Option<Instant> = None;
    loop {
        // Wait out the window, then read everything that arrived meanwhile.
        if let Some(last) = last_notice {
            let wait = (last + DISPLAY_WINDOW).saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                tokio::select! {
                    biased;
                    _ = w.stopped() => return Ok(()),
                    _ = tokio::time::sleep(wait) => {}
                }
            }
        }
        w.readmit()?;
        // Clear the flag before lookup so creation immediately after lookup still wakes us.
        // The host must signal both creation and removal; lookup never creates a projection.
        projections.borrow_and_update();
        let Some(projection) = w.projection(session) else {
            tokio::select! {
                biased;
                _ = w.stopped() => return Ok(()),
                created = projections.changed() => {
                    if created.is_err() {
                        return Ok(());
                    }
                }
            }
            continue;
        };
        // Subscribe before reading so a frame pushed after the read wakes us.
        let mut changes = projection.subscribe_display();
        changes.borrow_and_update();
        let reply = if tools {
            projection.display_read_with_tools(since, w.now_ms())
        } else {
            projection.display_read(since)
        };
        if matches!(reply, DisplayReadReply::Expired { .. }) {
            // The ring dropped unsent frames: the next notice is a fresh Baseline.
            since = None;
            continue;
        }
        if let Some((notice, advanced)) = next_display_notice(reply) {
            since = advanced.or(since);
            last_notice = Some(Instant::now());
            if w.emit(display_body(&notice)).await.is_err() {
                return Ok(());
            }
            // Frames may remain after a split notice; read again.
            continue;
        }
        tokio::select! {
            biased;
            _ = w.stopped() => return Ok(()),
            _ = changes.changed() => {}
            // The session's projection was removed or replaced: look it up again.
            _ = projections.changed() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    };

    use serde_json::json;
    use sha2::{Digest, Sha256};
    use tokio::sync::mpsc;

    use super::*;
    use crate::display::DisplayReadRequest;
    use crate::display_events::test_support::FakeSession;

    struct FakeHost {
        session: Mutex<Option<Arc<FakeSession>>>,
        projections: watch::Sender<u64>,
        stop: watch::Sender<bool>,
        admitted: AtomicBool,
        notices: mpsc::Sender<Vec<u8>>,
    }

    impl FakeHost {
        fn new(capacity: usize) -> (Arc<Self>, mpsc::Receiver<Vec<u8>>) {
            let (notices, receiver) = mpsc::channel(capacity);
            (
                Arc::new(Self {
                    session: Mutex::new(None),
                    projections: watch::channel(0).0,
                    stop: watch::channel(false).0,
                    admitted: AtomicBool::new(true),
                    notices,
                }),
                receiver,
            )
        }

        fn attach(&self, session: &str, capacity: usize) -> Arc<FakeSession> {
            let session = Arc::new(FakeSession::new(session, capacity));
            *self.session.lock().unwrap() = Some(Arc::clone(&session));
            self.projections.send_modify(|generation| *generation += 1);
            session
        }

        fn start(
            self: &Arc<Self>,
            params: Value,
        ) -> tokio::task::JoinHandle<Result<(), DisplayReadError>> {
            let host = Arc::clone(self);
            tokio::spawn(async move { run(host.as_ref(), params).await })
        }

        fn stop(&self) {
            self.stop.send_replace(true);
        }
    }

    #[async_trait]
    impl DisplayWatchHost for FakeHost {
        async fn read(&self, params: Value) -> Result<DisplayReadReply, DisplayReadError> {
            let request = DisplayReadRequest::parse(&params)?;
            Ok(request.read(self.projection(request.session).as_deref()))
        }

        fn projection(&self, session: &str) -> Option<Arc<DisplayProjection>> {
            self.session
                .lock()
                .unwrap()
                .as_ref()
                .filter(|candidate| candidate.session == session)
                .map(|candidate| Arc::clone(&candidate.projection))
        }

        fn projection_changes(&self, _session: &str) -> watch::Receiver<u64> {
            self.projections.subscribe()
        }

        fn readmit(&self) -> Result<(), DisplayReadError> {
            if self.admitted.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(DisplayReadError {
                    code: "carrier_op_not_permitted",
                    message: "route was revoked".into(),
                })
            }
        }

        async fn emit(&self, body: Vec<u8>) -> Result<(), DisplayWatchStopped> {
            tokio::select! {
                biased;
                _ = self.stopped() => Err(DisplayWatchStopped),
                sent = self.notices.send(body) => sent.map_err(|_| DisplayWatchStopped),
            }
        }

        async fn stopped(&self) {
            let mut stopped = self.stop.subscribe();
            let _ = stopped.wait_for(|stopped| *stopped).await;
        }
    }

    async fn settle() {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    async fn quiet(receiver: &mut mpsc::Receiver<Vec<u8>>, duration: Duration) {
        settle().await;
        tokio::time::advance(duration).await;
        settle().await;
        assert!(receiver.try_recv().is_err(), "unexpected display notice");
    }

    async fn next(receiver: &mut mpsc::Receiver<Vec<u8>>) -> (Value, Vec<u8>) {
        let body = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
            .await
            .expect("watch notice timeout")
            .expect("watch remains open");
        (serde_json::from_slice(&body).unwrap(), body)
    }

    fn delta(session: &FakeSession, mid: &str, text: &str) {
        session
            .ingest(json!({"kind":"display","event":{"type":"text_delta","mid":mid,"delta":text}}));
        session.flush();
    }

    fn control(session: &FakeSession, name: &str) {
        session.ingest(json!({"kind":"control","event":{"type":name}}));
    }

    fn vectors_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-watch-v1")
    }

    fn vector(name: &str, notice: &Value) {
        let expected: Value = serde_json::from_slice(
            &std::fs::read(vectors_root().join(format!("session.display_watch/{name}.json")))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(*notice, expected, "display vector {name}");
    }

    /// Only a watch that asked for tool frames gets them, with the running call in
    /// its baseline and each frame stamped with the computer's time. A watch that
    /// didn't ask sees a quiet ring until text arrives, as before tool frames
    /// existed.
    #[tokio::test(start_paused = true)]
    async fn only_a_watch_that_accepts_tool_frames_gets_them() {
        let (plain_host, mut plain) = FakeHost::new(32);
        let (tool_host, mut tools) = FakeHost::new(32);
        let session = plain_host.attach("ses_tools", 64);
        *tool_host.session.lock().unwrap() = Some(Arc::clone(&session));
        session.ingest(
            json!({"kind":"control","event":{"type":"tool_started","mid":"m1",
            "tool_call_id":"c1","name":"bash","arguments":"{}","started_at":5}}),
        );
        let before = crate::display::now_ms();
        let plain_task = plain_host.start(json!({"session":"ses_tools"}));
        let tool_task = tool_host.start(json!({"session":"ses_tools","acceptToolFrames":true}));

        let (baseline, _) = next(&mut tools).await;
        assert_eq!(
            baseline["frame"],
            json!({"kind":"display_gap"}),
            "{baseline}"
        );
        assert_eq!(baseline["tools"][0]["callId"], "c1");
        assert_eq!(baseline["tools"][0]["startedAtMs"], 5);
        assert!(baseline["tools"][0]["asOfMs"].as_u64().unwrap() >= before);
        quiet(&mut plain, Duration::from_millis(400)).await;

        session.ingest(
            json!({"kind":"control","event":{"type":"tool_finished","mid":"m1",
            "tool_call_id":"c1","outcome":"ok","output_bytes":3,"finished_at":9}}),
        );
        delta(&session, "m2", "Done.");
        let mut frames = Vec::new();
        while frames.len() < 2 {
            let (notice, _) = next(&mut tools).await;
            frames.extend(notice["frames"].as_array().unwrap().iter().cloned());
        }
        assert_eq!(frames[0]["frame"]["phase"], "answered", "{frames:?}");
        assert_eq!(frames[0]["frame"]["outputBytes"], 3);
        assert_eq!(frames[1]["frame"]["chunk"], "Done.");
        let (plain_baseline, _) = next(&mut plain).await;
        assert_eq!(
            plain_baseline,
            json!({"seq":1,"frame":{"kind":"display","sessionID":"ses_tools",
                "messageID":"m2","chunk":"Done.","partType":"text"}})
        );

        plain_host.stop();
        tool_host.stop();
        plain_task.await.unwrap().unwrap();
        tool_task.await.unwrap().unwrap();
        let not_a_boolean = json!({"session":"s","acceptToolFrames":"yes"});
        let refused = DisplayReadRequest::parse(&not_a_boolean);
        assert_eq!(refused.unwrap_err().code, "invalid_request");
    }

    #[tokio::test(start_paused = true)]
    async fn display_watch_waits_for_a_projection_then_paces_frames_and_marks_oversize() {
        let (host, mut notices) = FakeHost::new(32);
        let task = host.start(json!({"session":"ses_display","harness":"ck-app"}));
        quiet(&mut notices, Duration::from_millis(800)).await;
        assert!(host.projection("ses_display").is_none());
        let session = host.attach("ses_display", 64);
        quiet(&mut notices, Duration::from_millis(800)).await;
        delta(&session, "msg_one", "Hello");
        let (baseline, body) = next(&mut notices).await;
        assert_eq!(
            body,
            display_body(&host.read(json!({"session":"ses_display"})).await.unwrap())
        );
        vector("baseline", &baseline);

        let mut last_at = Instant::now();
        delta(&session, "msg_one", " there");
        delta(&session, "msg_one", ", watcher");
        control(&session, "run_finished");
        let (incremental, _) = next(&mut notices).await;
        assert!(Instant::now().duration_since(last_at) >= Duration::from_millis(250));
        last_at = Instant::now();
        vector("incremental", &incremental);
        let mut seen = 1;
        for frame in incremental["frames"].as_array().unwrap() {
            assert_eq!(frame["seq"], seen + 1, "frames neither repeat nor skip");
            seen += 1;
        }
        assert_eq!(incremental["seq"], seen);
        quiet(&mut notices, Duration::from_millis(1500)).await;
        control(&session, "run_started");
        delta(&session, "msg_two", "Next");
        let (next_turn, _) = next(&mut notices).await;
        assert_eq!(next_turn["frames"][0]["seq"], seen + 1);
        assert!(Instant::now().duration_since(last_at) >= Duration::from_millis(250));
        control(&session, "run_finished");
        let (ended, _) = next(&mut notices).await;
        assert_eq!(ended["frames"][0]["frame"]["kind"], "display_end");
        seen = ended["seq"].as_u64().unwrap();
        last_at = Instant::now();
        control(&session, "run_started");
        for index in 0..15 {
            delta(&session, &format!("msg_split_{index}"), &"s".repeat(20_000));
        }
        let mut total = 0;
        let mut split = Vec::new();
        while total < 15 {
            let (notice, body) = next(&mut notices).await;
            assert!(body.len() <= NOTICE_BOUND);
            assert!(Instant::now().duration_since(last_at) >= Duration::from_millis(250));
            last_at = Instant::now();
            for frame in notice["frames"].as_array().unwrap() {
                assert_eq!(frame["seq"], seen + 1, "frames neither repeat nor skip");
                seen += 1;
                total += 1;
            }
            assert_eq!(notice["seq"], seen);
            split.push(notice);
        }
        assert!(split.len() >= 2);
        vector("incremental-split", &split[0]);
        host.stop();
        task.await.unwrap().unwrap();

        let (host, mut notices) = FakeHost::new(32);
        let big = host.attach("ses_big", 64);
        for _ in 0..25 {
            delta(&big, "msg_big", &"x".repeat(10 * 1024));
        }
        let task = host.start(json!({"session":"ses_big"}));
        let (expired, _) = next(&mut notices).await;
        vector("expired-oversize", &expired);
        delta(&big, "msg_after", "after");
        let (after, _) = next(&mut notices).await;
        assert_eq!(
            after["frames"][0]["seq"].as_u64(),
            Some(expired["seq"].as_u64().unwrap() + 1)
        );
        host.stop();
        task.await.unwrap().unwrap();
    }

    #[test]
    fn the_phone_watch_vectors_are_complete_and_their_checksums_verify() {
        let root = vectors_root();
        let sums = std::fs::read_to_string(root.join("SHA256SUMS")).unwrap();
        let required = [
            "baseline",
            "expired-oversize",
            "incremental-split",
            "incremental",
        ];
        let mut found = Vec::new();
        for line in sums.lines() {
            let (digest, path) = line.split_once("  ").unwrap();
            let bytes = std::fs::read(root.join(path)).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest, "{path}");
            assert!(bytes.len() <= NOTICE_BOUND, "{path}");
            let _: Value = serde_json::from_slice(&bytes).unwrap();
            found.push(path.to_string());
        }
        let expected: Vec<_> = required
            .iter()
            .map(|name| format!("session.display_watch/{name}.json"))
            .collect();
        assert_eq!(found, expected);
        assert_eq!(
            std::fs::read_dir(root.join("session.display_watch"))
                .unwrap()
                .count(),
            4
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_watch_parks_only_itself_and_resyncs_when_it_resumes() {
        let (stalled, mut held) = FakeHost::new(1);
        let (flowing, mut live) = FakeHost::new(32);
        let session = stalled.attach("ses", 2);
        *flowing.session.lock().unwrap() = Some(Arc::clone(&session));
        let first = stalled.start(json!({"session":"ses"}));
        let second = flowing.start(json!({"session":"ses"}));
        settle().await;
        delta(&session, "m", "a");
        next(&mut live).await;
        settle().await;
        tokio::time::advance(DISPLAY_WINDOW).await;
        delta(&session, "m", "b");
        next(&mut live).await;
        settle().await;
        for text in ["c", "d", "e", "f"] {
            tokio::time::advance(DISPLAY_WINDOW).await;
            delta(&session, "m", text);
            next(&mut live).await;
        }
        next(&mut held).await;
        next(&mut held).await;
        let (fresh, _) = next(&mut held).await;
        assert_eq!(fresh["frame"]["chunk"], "abcdef");
        stalled.stop();
        flowing.stop();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn display_watch_refusals_cancellation_and_projection_replacement() {
        let (host, mut notices) = FakeHost::new(1);
        assert_eq!(
            run(host.as_ref(), json!({"session":"s","unknown":0}))
                .await
                .unwrap_err()
                .code,
            "invalid_request"
        );
        let session = host.attach("s", 2);
        delta(&session, "m", "one");
        let task = host.start(json!({"session":"s"}));
        next(&mut notices).await;
        let replacement = host.attach("s", 2);
        delta(&replacement, "m", "new");
        delta(&replacement, "m", "est");
        delta(&replacement, "m", "!");
        let (fresh, _) = next(&mut notices).await;
        assert_eq!(fresh["frames"][0]["seq"], 2);
        host.admitted.store(false, Ordering::SeqCst);
        delta(&replacement, "m", "revoked");
        assert_eq!(
            task.await.unwrap().unwrap_err().code,
            "carrier_op_not_permitted"
        );

        let (host, _notices) = FakeHost::new(1);
        let task = host.start(json!({"session":"absent"}));
        settle().await;
        host.stop();
        task.await.unwrap().unwrap();
    }

    #[test]
    fn an_oversize_incremental_frame_expires_only_that_frame() {
        let session = FakeSession::new("s", 4);
        delta(&session, "m", &"x".repeat(NOTICE_BOUND));
        delta(&session, "next", "small");
        let (expired, since) =
            next_display_notice(session.projection.display_read(Some(0))).unwrap();
        assert_eq!(since, Some(1));
        assert_eq!(
            serde_json::to_value(expired).unwrap(),
            json!({"seq":1,"windowExpired":{"oldestSeq":1}})
        );
        let (notice, since) = next_display_notice(session.projection.display_read(since)).unwrap();
        assert_eq!(since, Some(2));
        assert_eq!(
            serde_json::to_value(notice).unwrap()["frames"][0]["frame"]["chunk"],
            "small"
        );
    }
}
