//! The sample set the apps pin for choosing how a message is delivered,
//! `phone-reader-send-modes-v1`: each `session.enqueue_user_message` served
//! through the reader's frame loop on a paired phone's route, with the reply bytes
//! the app receives. Beside each send that reaches the runner, a
//! `.runner-send.json` file keeps the `session.send` body Occipital wrote, so the
//! set also shows which delivery each mode becomes. Its README says what each file
//! shows.

use super::*;
use crate::app_reader::{broca::tests::served_samples::Served, AppReader, SourceResolver};
use occipital_app_reader::source::TranscriptSource;
use std::time::Duration;

const SET: &str = "phone-reader-send-modes-v1";
const HEAD: &str = "head-session";
const WORKER: &str = "worker-session";

/// A Broca head and a Broca worker, each with a running turn and the grant its
/// kind gets from core: a head `steer` and `idle_start` with `interrupt`, a
/// worker `tool_result` and `steer` without it.
struct HeadAndWorker {
    head: Arc<FakeRunner>,
    worker: Arc<FakeRunner>,
}

#[async_trait]
impl SendTargets for HeadAndWorker {
    async fn open(&self, session: &str, _: Option<&str>) -> Result<Arc<dyn SendTarget>, ReadError> {
        let runner = match session {
            HEAD => self.head.clone(),
            WORKER => self.worker.clone(),
            other => panic!("the sample sent to {other}"),
        };
        Ok(Arc::new(Target(
            runner,
            SessionRef {
                runner: "broca".into(),
                session: session.into(),
            },
        )))
    }
}

/// Sends need no transcript source.
struct NoSources;

#[async_trait]
impl SourceResolver for NoSources {
    async fn resolve(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Option<Arc<dyn TranscriptSource>>, ReadError> {
        Ok(None)
    }
}

struct Recorder {
    served: Served,
    corr: u64,
}

impl Recorder {
    /// Serve `params` as an app send, keep request and reply as sample `name`,
    /// and answer the reply's `result`, or for a refusal the error body
    /// (`{code, message}`).
    async fn send(&mut self, name: &str, params: Value) -> Value {
        self.corr += 1;
        let request = json!({"method": "session.enqueue_user_message", "params": params});
        self.served.send(self.corr, &request).await;
        let (_, reply) = self
            .served
            .next(self.corr, Duration::from_secs(5))
            .await
            .expect("the module answers");
        self.served.keep(name, &request, reply.clone());
        let reply: Value = serde_json::from_slice(&reply).unwrap();
        if reply.get("result").is_some() {
            reply["result"].clone()
        } else {
            reply
        }
    }

    /// Keep the last `session.send` body `runner` received as `<name>.runner-send.json`.
    fn keep_runner_send(&mut self, name: &str, runner: &FakeRunner) -> Value {
        let body = sends(runner)
            .pop()
            .expect("the runner was sent the message");
        self.served.keep_file(
            &format!("{name}.runner-send.json"),
            body.to_string().into_bytes(),
        );
        body
    }
}

fn message(session: &str, id: &str, text: &str, mode: Option<&str>) -> Value {
    let mut params = json!({"session": session, "text": text, "clientMessageId": id});
    if let Some(mode) = mode {
        params["mode"] = json!(mode);
    }
    params
}

#[tokio::test]
async fn the_send_mode_samples_are_what_the_reader_serves() {
    let head = granted_fixture(Kind::Head);
    let worker = granted_fixture(Kind::Worker);
    // The worker's running turn takes the steer at once, so no follow-up runs.
    worker.runner.lands.store(true, Ordering::SeqCst);
    let clock = head.now.clone();
    let sender = Arc::new(Sender::with_clock(
        Arc::new(HeadAndWorker {
            head: head.runner.clone(),
            worker: worker.runner.clone(),
        }),
        Some(head.receipts.clone()),
        Arc::new(move || clock.load(Ordering::SeqCst)),
    ));
    let app = Arc::new(AppReader::with_sender(Arc::new(NoSources), sender));
    let mut r = Recorder {
        served: Served::new(SET, app),
        corr: 0,
    };

    // A steer while the head's turn runs: it joins that turn at its next step,
    // or starts the next turn if this one ends first, and never expires.
    let steer = message(
        HEAD,
        "5f1d2c3a-7b8e-4c9d-a0e1-2f3a4b5c6d01",
        "Also check the second log",
        Some("steer"),
    );
    let receipt = r.send("01-steer", steer.clone()).await;
    assert_eq!(
        (&receipt["ok"], &receipt["duplicate"]),
        (&json!(true), &json!(false))
    );
    let body = r.keep_runner_send("01-steer", &head.runner);
    assert_eq!(body["delivery"], json!(["steer", "idle_start"]));
    assert!(body.get("expires_at_ms").is_none(), "{body}");

    // The app's retry after a lost reply, with the same mode: the first receipt,
    // and nothing reaches the runner.
    let sent = sends(&head.runner).len();
    let repeat = r.send("02-steer-repeat", steer.clone()).await;
    assert_eq!(repeat["duplicate"], true);
    assert_eq!(repeat["effectId"], receipt["effectId"]);
    assert_eq!(sends(&head.runner).len(), sent);

    // A retry that drops the mode is a different message under the same id.
    let mut queued = steer.clone();
    queued.as_object_mut().unwrap().remove("mode");
    let conflict = r.send("03-repeat-without-mode", queued).await;
    assert_eq!(conflict["code"], "client_message_id_conflict", "{conflict}");
    assert_eq!(sends(&head.runner).len(), sent);

    // No mode is a queue: the message starts the next turn after this one.
    let queue = r
        .send(
            "04-queue",
            message(
                HEAD,
                "5f1d2c3a-7b8e-4c9d-a0e1-2f3a4b5c6d02",
                "Then write the summary",
                None,
            ),
        )
        .await;
    assert_eq!(queue["ok"], true);
    let body = r.keep_runner_send("04-queue", &head.runner);
    assert_eq!(body["delivery"], json!(["idle_start"]));

    // An interrupt ends the running turn and starts one with the message, where
    // the grant has `interrupt: true`.
    let interrupt = r
        .send(
            "05-interrupt",
            message(
                HEAD,
                "5f1d2c3a-7b8e-4c9d-a0e1-2f3a4b5c6d03",
                "Stop that and look at this instead",
                Some("interrupt"),
            ),
        )
        .await;
    assert_eq!(interrupt["ok"], true);
    let body = r.keep_runner_send("05-interrupt", &head.runner);
    assert_eq!(body["delivery"], "interrupt");

    // A worker takes only steers into its running turn, with a 30-minute deadline.
    let steered = r
        .send(
            "06-worker-steer",
            message(
                WORKER,
                "5f1d2c3a-7b8e-4c9d-a0e1-2f3a4b5c6d04",
                "Skip the slow tests",
                Some("steer"),
            ),
        )
        .await;
    assert_eq!(steered["ok"], true);
    let body = r.keep_runner_send("06-worker-steer", &worker.runner);
    assert_eq!(body["delivery"], json!(["steer"]));
    assert_eq!(body["expires_at_ms"], START + 30 * 60_000);

    // A worker's grant has no `interrupt`, so an interrupt is refused and nothing
    // is sent.
    let sent = sends(&worker.runner).len();
    let refused = r
        .send(
            "07-worker-interrupt",
            message(
                WORKER,
                "5f1d2c3a-7b8e-4c9d-a0e1-2f3a4b5c6d05",
                "Stop now",
                Some("interrupt"),
            ),
        )
        .await;
    assert_eq!(refused["code"], "enqueue_not_authorized", "{refused}");
    assert_eq!(sends(&worker.runner).len(), sent);

    r.served.check();
}
