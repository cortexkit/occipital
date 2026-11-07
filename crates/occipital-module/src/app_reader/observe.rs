use crate::app_reader::Guard as _;
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::time::Instant;

// Allow one slow managed call (normally bounded at 30 seconds), but leave ample
// time to retry a read before a device gives up after several minutes.
pub(super) const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(45);

pub(super) async fn deadline<T>(
    method: &str,
    work: impl Future<Output = T>,
) -> Result<T, tokio::time::error::Elapsed> {
    // A send can have committed a side effect; timing it out as a retryable read
    // would invite duplicate delivery. Sends retain their existing outcome rules.
    if method == super::SEND {
        return Ok(work.await);
    }
    tokio::time::timeout(READ_DEADLINE, work).await
}

tokio::task_local! {
    static REQUEST: Arc<Mutex<Trace>>;
}

#[derive(Default)]
struct Trace {
    calls: BTreeMap<String, u64>,
    steps: Vec<Step>,
    outcome: Option<&'static str>,
    /// The refusal's message, for a request refused: what the code alone
    /// doesn't say, such as a runner's own reason.
    refusal: Option<String>,
}

struct Step {
    name: String,
    started: Instant,
    elapsed_ms: Option<u64>,
}

#[derive(Debug)]
struct Timing<'a> {
    step: &'a str,
    elapsed_ms: u64,
    finished: bool,
}

struct RequestLog {
    method: String,
    session: String,
    correlation: String,
    started: Instant,
    trace: Arc<Mutex<Trace>>,
}

impl Drop for RequestLog {
    fn drop(&mut self) {
        let trace = self.trace.guard();
        let timings: Vec<_> = trace
            .steps
            .iter()
            .map(|step| Timing {
                step: &step.name,
                elapsed_ms: step.elapsed_ms.unwrap_or_else(|| millis(step.started)),
                finished: step.elapsed_ms.is_some(),
            })
            .collect();
        let pending = timings.iter().rev().find(|step| !step.finished);
        // Keep the timing fields structured so a stalled nested call can be found
        // without logging transcript bodies, request parameters or project paths.
        // A refusal's message is module text, such as a runner's reason, and
        // carries none of them.
        let steps: Vec<_> = timings
            .iter()
            .map(|step| (step.step, step.elapsed_ms, step.finished))
            .collect();
        let elapsed = self.started.elapsed();
        let outcome = trace.outcome.unwrap_or("cancelled");
        macro_rules! summary {
            ($level:expr) => {
                tracing::event!($level, method = %self.method, session = %self.session, correlation = %self.correlation,
                    duration_ms = millis(self.started), outcome, refusal = trace.refusal.as_deref(),
                    pending_step = pending.map(|step| step.step), remote_calls = ?trace.calls, steps = ?steps,
                    "app reader request finished")
            };
        }
        // Logging may have been installed while the request was running. Use the
        // active logger instead of an earlier empty logging context.
        match summary_level(&self.method, trace.outcome, elapsed) {
            tracing::Level::WARN => summary!(tracing::Level::WARN),
            tracing::Level::INFO => summary!(tracing::Level::INFO),
            _ => summary!(tracing::Level::DEBUG),
        }
    }
}

/// The requests that change something on the person's behalf: a message sent,
/// a run stopped, a file placed in a project or an upload discarded. Each one
/// that succeeds is logged at INFO, so the log can always answer whether an
/// app did it, where a successful read is logged only when slow.
const CONSEQUENTIAL: [&str; 5] = [
    super::SEND,
    super::STOP,
    super::uploads::FINISH,
    super::uploads::ABORT,
    super::uploads::PLACE,
];

fn summary_level(
    method: &str,
    outcome: Option<&str>,
    elapsed: std::time::Duration,
) -> tracing::Level {
    // Deadline failures must remain visible when the fleet logger filters debug.
    if outcome == Some("deadline") {
        return tracing::Level::WARN;
    }
    // An unset outcome means the request was dropped, rather than refused.
    match outcome {
        Some("ok") if CONSEQUENTIAL.contains(&method) => tracing::Level::INFO,
        Some("ok") if elapsed > std::time::Duration::from_secs(2) => tracing::Level::INFO,
        Some("ok") | None => tracing::Level::DEBUG,
        _ => tracing::Level::WARN,
    }
}

fn millis(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

pub(super) async fn request<T>(
    method: &str,
    session: &str,
    correlation: String,
    work: impl Future<Output = T>,
) -> T {
    let trace = Arc::new(Mutex::new(Trace::default()));
    REQUEST
        .scope(trace.clone(), async {
            let log = RequestLog {
                method: method.into(),
                session: session.into(),
                correlation,
                started: Instant::now(),
                trace: trace.clone(),
            };
            let result = note_a_long_wait(&log, work).await;
            trace.guard().outcome.get_or_insert("ok");
            result
        })
        .await
}

/// How long a request may run before the log says it's still running.
const STILL_RUNNING_AFTER: std::time::Duration = std::time::Duration::from_secs(10);

/// Run `work`, and log once if it's still running after
/// [`STILL_RUNNING_AFTER`], naming the step it waits in. A request that never
/// answers otherwise leaves no line at all, since the summary is logged when it
/// ends. A watch is held open by design, so it's never noted.
async fn note_a_long_wait<T>(log: &RequestLog, work: impl Future<Output = T>) -> T {
    let watch = matches!(
        log.method.as_str(),
        super::TRANSCRIPT_WATCH
            | super::DISPLAY_WATCH
            | crate::display::app::RECORDS_WATCH
            | crate::notices::WATCH
    );
    if watch {
        return work.await;
    }
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => return result,
        () = tokio::time::sleep(STILL_RUNNING_AFTER) => {}
    }
    {
        let trace = log.trace.guard();
        let pending = trace
            .steps
            .iter()
            .rev()
            .find(|step| step.elapsed_ms.is_none())
            .map(|step| step.name.as_str());
        tracing::warn!(method = %log.method, session = %log.session, correlation = %log.correlation,
            running_ms = millis(log.started), pending_step = pending, remote_calls = ?trace.calls,
            "app reader request still running");
    }
    work.await
}

pub(super) fn outcome(code: &'static str) {
    let _ = REQUEST.try_with(|trace| trace.guard().outcome = Some(code));
}

/// The most of a refusal's message the request log keeps.
const REFUSAL_CHARS: usize = 300;

/// Record that the request was refused with `code`, and why: the request log
/// names the message too, cut to [`REFUSAL_CHARS`] characters.
pub(super) fn refused(code: &'static str, message: &str) {
    let message: String = message.chars().take(REFUSAL_CHARS).collect();
    let _ = REQUEST.try_with(|trace| {
        let mut trace = trace.guard();
        trace.outcome = Some(code);
        trace.refusal = Some(message);
    });
}

fn begin(name: &str) -> Option<usize> {
    REQUEST
        .try_with(|trace| {
            let mut trace = trace.guard();
            let index = trace.steps.len();
            trace.steps.push(Step {
                name: name.into(),
                started: Instant::now(),
                elapsed_ms: None,
            });
            index
        })
        .ok()
}

fn finish(index: Option<usize>) {
    if let Some(index) = index {
        let _ = REQUEST.try_with(|trace| {
            let mut trace = trace.guard();
            let step = &mut trace.steps[index];
            step.elapsed_ms = Some(millis(step.started));
        });
    }
}

pub(super) async fn step<T>(name: &str, work: impl Future<Output = T>) -> T {
    let index = begin(name);
    let result = work.await;
    finish(index);
    result
}

pub(super) fn synchronous<T>(name: &str, work: impl FnOnce() -> T) -> T {
    let index = begin(name);
    let result = work();
    finish(index);
    result
}

pub(super) async fn remote<T>(provider: &str, method: &str, work: impl Future<Output = T>) -> T {
    let name = format!("{provider}.{method}");
    let _ = REQUEST.try_with(|trace| {
        *trace.guard().calls.entry(name.clone()).or_default() += 1;
    });
    step(&name, work).await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fmt::Write;

    #[derive(Clone, Default)]
    pub(crate) struct Log(Arc<Mutex<Vec<u8>>>);

    impl crate::test_tracing::Capture for Log {
        fn event(&self, event: &tracing::Event<'_>) {
            if event.metadata().target() != "occipital_module::app_reader::observe" {
                return;
            }
            struct Fields(String);
            impl tracing::field::Visit for Fields {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    write!(&mut self.0, "{}={value:?} ", field.name()).unwrap();
                }
            }
            let mut fields = Fields(format!("level={} ", event.metadata().level()));
            event.record(&mut fields);
            self.0
                .lock()
                .unwrap()
                .extend_from_slice(fields.0.as_bytes());
        }
    }

    impl Log {
        pub(crate) fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    pub(crate) fn capture() -> (Log, crate::test_tracing::Guard) {
        let log = Log::default();
        let guard = crate::test_tracing::enter(log.clone());
        (log, guard)
    }

    #[tokio::test(start_paused = true)]
    async fn reader_log_records_step_time_and_cancellation() {
        let (log, _guard) = capture();
        request("session.details", "session", "1:1:1".into(), async {
            remote(
                "broca",
                "session.head",
                tokio::time::sleep(std::time::Duration::from_millis(7)),
            )
            .await;
            synchronous("projection", || ());
        })
        .await;
        let text = log.text();
        assert!(text.contains("duration_ms=7"), "{text}");
        assert!(text.contains("(\"broca.session.head\", 7, true)"), "{text}");
        assert!(text.contains("(\"projection\", 0, true)"), "{text}");
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(11),
            request(
                "session.details",
                "session",
                "1:1:2".into(),
                step("resolution", std::future::pending::<()>()),
            ),
        )
        .await;
        let text = log.text();
        assert_eq!(
            text.matches("app reader request finished").count(),
            2,
            "{text}"
        );
        assert!(text.contains("outcome=\"cancelled\""), "{text}");
        assert!(text.contains("pending_step=\"resolution\""), "{text}");
        assert!(text.contains("(\"resolution\", 11, false)"), "{text}");
        assert_eq!(text.matches("level=DEBUG").count(), 2, "{text}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_still_running_after_ten_seconds_is_logged_once() {
        let (log, _guard) = capture();
        request(
            "session.transcript_page",
            "session",
            "1:1:3".into(),
            async {
                remote(
                    "thalamus-harness",
                    "session.read",
                    tokio::time::sleep(std::time::Duration::from_secs(25)),
                )
                .await;
            },
        )
        .await;
        let text = log.text();
        assert_eq!(
            text.matches("app reader request still running").count(),
            1,
            "{text}"
        );
        assert!(text.contains("running_ms=10000"), "{text}");
        assert!(
            text.contains("pending_step=\"thalamus-harness.session.read\""),
            "{text}"
        );
        assert!(text.contains("correlation=1:1:3"), "{text}");
        // A quick request and a watch held for minutes are never noted.
        request("session.details", "session", "1:1:4".into(), async {
            tokio::time::sleep(std::time::Duration::from_secs(9)).await;
        })
        .await;
        request(
            super::super::DISPLAY_WATCH,
            "session",
            "1:1:5".into(),
            async {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            },
        )
        .await;
        assert_eq!(
            log.text()
                .matches("app reader request still running")
                .count(),
            1
        );
    }

    /// A refused request names the refusal's message beside its code, so a
    /// runner's reason reaches the log; a long one is cut. A request that
    /// wasn't refused has no such field.
    #[tokio::test]
    async fn a_refused_request_logs_why() {
        let (log, _guard) = capture();
        request(
            "session.transcript_page",
            "session",
            "1:1:1".into(),
            async {
                refused(
                    "provider_unavailable",
                    "thalamus-harness: module error transient: bounded runner queue full",
                );
            },
        )
        .await;
        request("session.details", "session", "1:1:2".into(), async {
            refused("provider_unavailable", &"x".repeat(1_000));
        })
        .await;
        request("session.details", "session", "1:1:3".into(), async {
            outcome("ok");
        })
        .await;
        let text = log.text();
        assert!(
            text.contains(
                "refusal=\"thalamus-harness: module error transient: bounded runner queue full\""
            ),
            "{text}"
        );
        assert!(
            text.contains(&format!("refusal=\"{}\"", "x".repeat(REFUSAL_CHARS))),
            "{text}"
        );
        assert!(!text.contains(&"x".repeat(REFUSAL_CHARS + 1)), "{text}");
        assert_eq!(text.matches("refusal=").count(), 2, "{text}");
    }

    #[tokio::test(start_paused = true)]
    async fn the_read_deadline_does_not_reclassify_a_slow_send() {
        let started = Instant::now();
        let duration = READ_DEADLINE + std::time::Duration::from_secs(1);
        deadline(super::super::SEND, tokio::time::sleep(duration))
            .await
            .unwrap();
        assert_eq!(started.elapsed(), duration);
    }

    #[tokio::test(start_paused = true)]
    async fn reader_summary_level_tracks_outcome_and_duration() {
        use std::time::Duration;

        for (code, duration_ms, level) in [
            ("ok", 7, "DEBUG"),
            ("ok", 2_000, "DEBUG"),
            ("ok", 2_001, "INFO"),
            ("provider_unavailable", 7, "WARN"),
            ("invalid_request", 7, "WARN"),
            ("reply_encode_failed", 7, "WARN"),
            ("reply_write_failed", 3_000, "WARN"),
            ("deadline_reply_not_queued", 7, "WARN"),
            ("cancelled", 3_000, "WARN"),
        ] {
            let (log, _guard) = capture();
            request("session.details", "session", "1:1:1".into(), async {
                remote(
                    "broca",
                    "session.head",
                    tokio::time::sleep(Duration::from_millis(duration_ms)),
                )
                .await;
                synchronous("projection", || ());
                outcome(code);
            })
            .await;
            let text = log.text();
            assert_eq!(
                text.matches("app reader request finished").count(),
                1,
                "{text}"
            );
            for field in [
                format!("level={level}"),
                format!("outcome=\"{code}\""),
                format!("duration_ms={duration_ms}"),
                "method=session.details".into(),
                "session=session".into(),
                "correlation=1:1:1".into(),
                "remote_calls={\"broca.session.head\": 1}".into(),
                format!("(\"broca.session.head\", {duration_ms}, true)"),
                "(\"projection\", 0, true)".into(),
            ] {
                assert!(text.contains(&field), "missing {field}: {text}");
            }
        }

        // A successful send, Stop, upload finish, abort or place is always
        // logged, even when fast; an upload chunk is not.
        for (method, level) in [
            (super::super::SEND, "INFO"),
            (super::super::STOP, "INFO"),
            (super::super::uploads::FINISH, "INFO"),
            (super::super::uploads::ABORT, "INFO"),
            (super::super::uploads::PLACE, "INFO"),
            (super::super::uploads::CHUNK, "DEBUG"),
        ] {
            let (log, _guard) = capture();
            request(method, "session", "1:1:2".into(), async {}).await;
            let text = log.text();
            assert!(
                text.contains(&format!("level={level}")) && text.contains("outcome=\"ok\""),
                "{method}: {text}"
            );
        }

        {
            let (log, _guard) = capture();
            let cancelled = tokio::time::timeout(
                Duration::from_secs(3),
                request(
                    "session.details",
                    "session",
                    "1:1:cancelled".into(),
                    remote("broca", "session.head", std::future::pending::<()>()),
                ),
            )
            .await;
            assert!(cancelled.is_err());
            let text = log.text();
            for field in [
                "level=DEBUG",
                "outcome=\"cancelled\"",
                "duration_ms=3000",
                "pending_step=\"broca.session.head\"",
                "(\"broca.session.head\", 3000, false)",
            ] {
                assert!(text.contains(field), "missing {field}: {text}");
            }
        }

        let (log, _guard) = capture();
        request("session.details", "session", "1:1:2".into(), async {
            let expired = deadline(
                "session.details",
                step(
                    "resolution",
                    remote(
                        "entorhinal",
                        "agent.session_for_runner",
                        std::future::pending::<()>(),
                    ),
                ),
            )
            .await;
            assert!(expired.is_err());
            outcome("deadline");
        })
        .await;
        let text = log.text();
        assert_eq!(
            text.matches("app reader request finished").count(),
            1,
            "{text}"
        );
        for field in [
            "level=WARN",
            "outcome=\"deadline\"",
            "duration_ms=45000",
            "pending_step=\"entorhinal.agent.session_for_runner\"",
            "remote_calls={\"entorhinal.agent.session_for_runner\": 1}",
            "(\"resolution\", 45000, false)",
            "(\"entorhinal.agent.session_for_runner\", 45000, false)",
        ] {
            assert!(text.contains(field), "missing {field}: {text}");
        }
    }

    #[tokio::test]
    async fn dropped_request_is_logged_on_another_task_and_thread() {
        let log = Log::default();
        {
            let _collector = crate::test_tracing::enter(log.clone());
        }
        let (held, origin_thread) = tokio::spawn(async {
            let mut held = Box::pin(request(
                "session.details",
                "session",
                "cross-task-drop".into(),
                remote("broca", "session.head", std::future::pending::<()>()),
            ));
            std::future::poll_fn(|cx| {
                assert!(held.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            (held, std::thread::current().id())
        })
        .await
        .unwrap();

        let drop_log = log.clone();
        let drop_thread = tokio::task::spawn_blocking(move || {
            // The shared collector's test observer is thread-local. Production
            // logging is global; explicitly observe the thread performing Drop.
            let _capture = crate::test_tracing::enter(drop_log);
            let thread = std::thread::current().id();
            drop(held);
            thread
        })
        .await
        .unwrap();
        assert_ne!(origin_thread, drop_thread);
        let text = log.text();
        assert_eq!(
            text.matches("app reader request finished").count(),
            1,
            "{text}"
        );
        for field in [
            "level=DEBUG",
            "outcome=\"cancelled\"",
            "correlation=cross-task-drop",
            "pending_step=\"broca.session.head\"",
            "remote_calls={\"broca.session.head\": 1}",
        ] {
            assert!(text.contains(field), "missing {field}: {text}");
        }
    }
}
