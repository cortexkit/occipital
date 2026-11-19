use super::*;
use crate::app_reader::{
    resolve::SessionRef,
    send::{RunnerError, SendTarget, SendTargets, Sender},
    send_store::Receipts,
};
use occipital_app_reader::source::ReadError;

struct Target {
    session: SessionRef,
    at_head: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl SendTarget for Target {
    fn reference(&self) -> &SessionRef {
        &self.session
    }
    async fn call(&self, method: &'static str, _: Value) -> Result<Value, RunnerError> {
        Ok(match method {
            "session.head" => {
                self.at_head.notify_one();
                self.resume.notified().await;
                json!({"incarnation":"inc-a", "first_ordinal":1, "history_version":1,"updated_at":1,"last_run_state":{"run_id":"r1","state":"active"},"sender":{"modes":["steer","idle_start"],"interrupt":true}})
            }
            "role.describe" => {
                serde_json::to_value(cortexkit_role_llm_runner::describe::RoleDescribe::default())
                    .unwrap()
            }
            "session.send" | "session.send_status" => {
                json!({"state":"pending","submission_id":"sub-1"})
            }
            other => panic!("unexpected {other}"),
        })
    }
}

struct Targets(Arc<Target>);
#[async_trait::async_trait]
impl SendTargets for Targets {
    async fn open(&self, _: &str, _: Option<&str>) -> Result<Arc<dyn SendTarget>, ReadError> {
        Ok(self.0.clone())
    }
}

async fn paused_send(timeout: bool) {
    let fleet = Arc::new(Fleet::default());
    let source = computer("moves-send-drain", &fleet);
    let cut = cut_id(1);
    fleet.moves(&cut, &[session("s")]);
    let target = Arc::new(Target {
        session: SessionRef {
            runner: "broca".into(),
            session: "s".into(),
        },
        at_head: Default::default(),
        resume: Default::default(),
    });
    let sender = Sender::new(
        Arc::new(Targets(target.clone())),
        Some(Receipts::open(source.opened.shared.clone()).unwrap()),
    );
    sender.set_moves(source.cuts.clone());
    let send = sender.send(json!({"session":"s","clientMessageId":"admitted","text":"hello"}));
    tokio::pin!(send);
    tokio::select! { _ = target.at_head.notified() => {}, result = &mut send => panic!("send did not reach head: {result:?}") }
    let prepare = source.op("agent_data.prepare", &cut);
    tokio::pin!(prepare);
    assert!(futures_util::poll!(&mut prepare).is_pending());
    assert_eq!(source.cuts.hold("s"), Some(Hold::Moving));
    if timeout {
        let refused = prepare.await.unwrap_err();
        assert_eq!(refused.0, "unavailable");
        assert!(!source.cuts.stream_path(&cut).exists());
    } else {
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut prepare)
                .await
                .is_err(),
            "prepare replied while the send was still in session.head"
        );
        target.resume.notify_one();
        send.await.unwrap();
        prepare.await.unwrap();
        let stream = std::fs::read_to_string(source.cuts.stream_path(&cut)).unwrap();
        assert!(stream
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|line| line["table"] == "queued_sends" && line["row"][1] == "admitted"));
    }
}

#[tokio::test]
async fn prepare_includes_a_send_admitted_before_the_hold() {
    paused_send(false).await;
}

#[tokio::test(start_paused = true)]
async fn prepare_times_out_instead_of_cutting_an_admitted_send() {
    paused_send(true).await;
}
