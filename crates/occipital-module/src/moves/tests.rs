use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use cortexkit_test_support::ScratchDir;
use serde_json::{json, Value};
use subc_client_rs::HandlerOutcome;
use subc_protocol::Principal;

use super::rows::tests::{exported, opened, seed, Opened};
use super::*;

/// One clock for every computer here, so a later step always reads later.
fn tick() -> i64 {
    static NOW: AtomicI64 = AtomicI64::new(1_000);
    NOW.fetch_add(1, Ordering::SeqCst)
}

/// Entorhinal and Engram's session door as a move sees them, shared by the
/// computers of one test. The door keeps each admitted stream, and a read with
/// a cut id serves what was admitted under that handle.
#[derive(Default)]
struct Fleet {
    /// The session refs each cut moves, and the session each ref is bound to.
    plans: Mutex<HashMap<String, Vec<(String, String)>>>,
    paused_cut: Mutex<Option<String>>,
    resume: tokio::sync::Notify,
    bindings: Mutex<HashMap<(String, String), Session>>,
    streams: Mutex<HashMap<String, Vec<u8>>>,
    /// Bytes the door serves in place of a stream's own, to break a read.
    tampered: Mutex<Option<Vec<u8>>>,
    /// Refuse the admission that starts at this offset, once.
    refuse_admit_at: Mutex<Option<u64>>,
}

impl Fleet {
    fn moves(&self, cut_id: &str, sessions: &[Session]) {
        let mut refs = Vec::new();
        for (i, session) in sessions.iter().enumerate() {
            let reference = ("broca-session".to_owned(), format!("{cut_id}-ref-{i}"));
            self.bindings
                .lock()
                .unwrap()
                .insert(reference.clone(), session.clone());
            refs.push(reference);
        }
        self.plans.lock().unwrap().insert(cut_id.into(), refs);
    }
}

fn failure(code: &str) -> CallFailure {
    CallFailure {
        code: Some(code.into()),
        message: code.into(),
    }
}

#[async_trait::async_trait]
impl Remote for Fleet {
    async fn move_refs(
        &self,
        cut_id: &str,
        _agent: &str,
    ) -> Result<Vec<(String, String)>, CallFailure> {
        let paused = self.paused_cut.lock().unwrap().as_deref() == Some(cut_id);
        if paused {
            self.resume.notified().await;
        }
        self.plans
            .lock()
            .unwrap()
            .get(cut_id)
            .cloned()
            .ok_or_else(|| failure("not_found"))
    }

    async fn session_for_ref(
        &self,
        ref_kind: &str,
        reference: &str,
    ) -> Result<Option<Session>, CallFailure> {
        Ok(self
            .bindings
            .lock()
            .unwrap()
            .get(&(ref_kind.to_owned(), reference.to_owned()))
            .cloned())
    }

    async fn door(&self, method: &str, params: Value) -> Result<Value, CallFailure> {
        let handle = params["sessionRef"].as_str().unwrap_or_default().to_owned();
        let mut streams = self.streams.lock().unwrap();
        match method {
            "session.offsets" | "session.cover" => {
                let end = streams.get(&handle).map_or(0, |bytes| bytes.len());
                Ok(json!({"durableOffset": end, "cloudOffset": end}))
            }
            "session.admit" => {
                let stream = streams.entry(handle).or_default();
                if params["fromOffset"].as_u64() != Some(stream.len() as u64) {
                    return Err(failure("offset_mismatch"));
                }
                let mut refuse = self.refuse_admit_at.lock().unwrap();
                if *refuse == Some(stream.len() as u64) {
                    *refuse = None;
                    return Err(failure("unavailable"));
                }
                stream.extend(payload::unhex(params["bytes"].as_str().unwrap()).unwrap());
                Ok(json!({"durableOffset": stream.len()}))
            }
            "session.read" => {
                assert!(params["cutId"].is_string(), "a receiver reads by cut");
                let Some(stream) = streams.get(&handle) else {
                    return Err(failure("cut_item_missing"));
                };
                let digest = payload::hex(&Sha256::digest(stream));
                let served = self
                    .tampered
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or(stream.clone());
                let from = params["fromOffset"].as_u64().unwrap() as usize;
                let limit = params["limit"].as_u64().unwrap() as usize;
                let end = (from + limit).min(served.len());
                Ok(json!({
                    "fromOffset": from,
                    "endOffset": end,
                    "streamEnd": stream.len(),
                    "digest": digest,
                    "bytes": payload::hex(&served[from..end]),
                }))
            }
            other => panic!("the door has no {other}"),
        }
    }
}

/// A participant over fresh stores in `dir`, with no move planned, for tests
/// of how requests reach it.
pub(crate) fn participant(dir: &ScratchDir) -> Arc<Moves> {
    let opened = opened(dir);
    let cuts = Arc::new(Cuts::open(opened.main.data_root()).unwrap());
    Arc::new(Moves::new(
        cuts,
        Arc::new(Fleet::default()),
        Arc::new(Stores {
            display: opened.display.clone(),
            notices: opened.notices.clone(),
            store: opened.shared.clone(),
        }),
        tick,
    ))
}

/// One computer: its stores, and Occipital's participant over them.
struct Computer {
    _dir: ScratchDir,
    opened: Opened,
    cuts: Arc<Cuts>,
    moves: Moves,
}

fn computer(name: &str, fleet: &Arc<Fleet>) -> Computer {
    let dir = ScratchDir::new(name);
    let opened = opened(&dir);
    let cuts = Arc::new(Cuts::open(opened.main.data_root()).unwrap());
    let moves = Moves::new(
        cuts.clone(),
        fleet.clone(),
        Arc::new(Stores {
            display: opened.display.clone(),
            notices: opened.notices.clone(),
            store: opened.shared.clone(),
        }),
        tick,
    );
    Computer {
        _dir: dir,
        opened,
        cuts,
        moves,
    }
}

fn entorhinal() -> Principal {
    Principal::Reserved {
        module_id: "entorhinal".into(),
    }
}

const AGENT: &str = "agent_0000000000000101";

fn cut_id(n: u8) -> String {
    format!("{n:032x}")
}

fn refs() -> Value {
    json!([{"ref_kind": "agent", "ref": AGENT}])
}

impl Computer {
    async fn call_as(
        &self,
        caller: Option<&Principal>,
        method: &str,
        params: Value,
    ) -> Result<Value, (String, Value)> {
        let body = serde_json::to_vec(&json!({"method": method, "params": params})).unwrap();
        match self.moves.serve(caller, &body).await {
            HandlerOutcome::Response(bytes) => {
                let mut reply: Value = serde_json::from_slice(&bytes).unwrap();
                Ok(reply["result"].take())
            }
            HandlerOutcome::Error { code, .. } => Err((code, Value::Null)),
            HandlerOutcome::ErrorWithDetail { code, detail, .. } => Err((code, detail)),
            HandlerOutcome::Streamed => panic!("an agent-data op never streams"),
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, (String, Value)> {
        self.call_as(Some(&entorhinal()), method, params).await
    }

    async fn op(&self, method: &str, cut: &str) -> Result<Value, (String, Value)> {
        self.call(method, json!({"refs": refs(), "cutId": cut}))
            .await
    }

    fn files(&self) -> super::rows::Files<'_> {
        self.opened.files()
    }
}

fn session(name: &str) -> Session {
    Session {
        runner: "broca".into(),
        harness: "broca".into(),
        session: name.into(),
    }
}

/// Only Entorhinal is answered, before the request is read.
#[tokio::test]
async fn only_entorhinal_is_answered() {
    let fleet = Arc::new(Fleet::default());
    let here = computer("moves-caller", &fleet);
    for caller in [
        None,
        Some(Principal::Direct),
        Some(Principal::Reserved {
            module_id: "broca".into(),
        }),
    ] {
        let refused = here
            .call_as(caller.as_ref(), "agent_data.describe", json!({}))
            .await
            .unwrap_err();
        assert_eq!(refused.0, "caller_forbidden", "{caller:?}");
    }
    let described = here.call("agent_data.describe", json!({})).await.unwrap();
    assert_eq!(described["ref_kinds"], json!(["agent"]));
    assert_eq!(described["external"], false);
    assert_eq!(described["optional"], true);
    // A runner's session kind is never Occipital's to take.
    let refused = here
        .call(
            "agent_data.busy",
            json!({"refs": [{"ref_kind": "broca-session", "ref": "x"}]}),
        )
        .await
        .unwrap_err();
    assert_eq!(refused.0, "invalid_request");
}

/// A whole move: the moved session's rows reach the receiver, sends to it are
/// held while it moves and refused naming the new computer once it has moved,
/// and the session that didn't move keeps its rows and its sends.
#[tokio::test]
async fn a_move_carries_the_moved_sessions_rows_and_holds_their_sends() {
    let fleet = Arc::new(Fleet::default());
    let source = computer("moves-source", &fleet);
    let receiver = computer("moves-receiver", &fleet);
    seed(&source.files(), "moved", "broca", "moved-call");
    seed(&source.files(), "stays", "broca", "stays-call");
    let cut = cut_id(1);
    fleet.moves(&cut, &[session("moved")]);
    let before = exported(&source.files(), "moved");

    let prepared = source.op("agent_data.prepare", &cut).await.unwrap();
    let item = &prepared["items"][0];
    assert_eq!(item["ref_kind"], "agent");
    assert_eq!(item["stream"], "reserved:occipital");
    assert_eq!(item["format"], payload::FORMAT);
    assert_eq!(prepared["items"].as_array().unwrap().len(), 1);
    assert_eq!(source.cuts.hold("moved"), Some(Hold::Moving));
    assert_eq!(source.cuts.hold("stays"), None);
    // A repeat answers the first reply.
    assert_eq!(
        source.op("agent_data.prepare", &cut).await.unwrap(),
        prepared
    );

    let exported_reply = source.op("agent_data.export", &cut).await.unwrap();
    assert_eq!(exported_reply["items"][0]["endOffset"], item["endOffset"]);

    let diverged = receiver.op("agent_data.diverged", &cut).await.unwrap();
    assert_eq!(diverged, json!({"refs": [{"state": "at_cut"}]}));
    let imported = receiver.op("agent_data.import", &cut).await.unwrap();
    assert_eq!(imported["items"][0]["digest"], item["digest"]);
    // Nothing is live before activation.
    assert!(exported(&receiver.files(), "moved").is_empty());
    assert_eq!(
        receiver.op("agent_data.activate", &cut).await.unwrap(),
        json!({})
    );
    assert_eq!(exported(&receiver.files(), "moved"), before);
    assert_eq!(
        receiver.op("agent_data.status", &cut).await.unwrap(),
        json!({"step": "activated"})
    );

    // A malformed box id is refused before anything changes.
    let malformed = source
        .call(
            "agent_data.release",
            json!({"refs": refs(), "cutId": cut, "holder": "author-b", "holderBox": "B0X"}),
        )
        .await
        .unwrap_err();
    assert_eq!(malformed.0, "invalid_request");
    let release = json!({"refs": refs(), "cutId": cut, "holder": "author-b",
        "holderBox": HOLDER_BOX});
    let released = source
        .call("agent_data.release", release.clone())
        .await
        .unwrap();
    assert_eq!(released, json!({"enforced": true}));
    assert_eq!(
        source.call("agent_data.release", release).await.unwrap(),
        released
    );
    // The waiting sends are the receiver's to follow now; the display rows
    // stay here for a move back.
    let left: Vec<String> = exported(&source.files(), "moved")
        .into_iter()
        .map(|(table, _)| table)
        .collect();
    assert!(
        !left
            .iter()
            .any(|t| t == "queued_sends" || t == "send_waits"),
        "{left:?}"
    );
    assert_eq!(left.len(), super::rows::TABLES.len() - 2);
    assert_eq!(
        exported(&source.files(), "stays").len(),
        super::rows::TABLES.len()
    );
    // A refused send names the new computer by its box id, never by the
    // `holder` pseudonym.
    assert_eq!(
        source.cuts.hold("moved"),
        Some(Hold::Elsewhere {
            holder: HOLDER_BOX.into()
        })
    );
    let refusal = refusal(&source.cuts.hold("moved").unwrap());
    assert_eq!(refusal.code, SESSION_HELD_ELSEWHERE);
    assert_eq!(refusal.detail, Some(json!({"holder": HOLDER_BOX})));
    // A release naming another holder, or adding, dropping or changing the
    // box id, is another request.
    for other in [
        json!({"holder": "author-c", "holderBox": HOLDER_BOX}),
        json!({"holder": "author-b"}),
        json!({"holder": "author-b", "holderBox": "fedcba9876543210fedcba9876543210"}),
    ] {
        let mut request = json!({"refs": refs(), "cutId": cut});
        request
            .as_object_mut()
            .unwrap()
            .extend(other.as_object().unwrap().clone());
        let refused = source
            .call("agent_data.release", request)
            .await
            .unwrap_err();
        assert_eq!(refused.0, "id_reused", "{other}");
    }
}

/// The new computer's box id, as the driver names it in a release.
const HOLDER_BOX: &str = "0123456789abcdef0123456789abcdef";

/// A release without a box id names no computer to a refused send: the
/// `holder` pseudonym is never shown.
#[tokio::test]
async fn a_release_without_a_box_id_names_no_computer() {
    let fleet = Arc::new(Fleet::default());
    let source = computer("moves-unnamed-source", &fleet);
    let receiver = computer("moves-unnamed-receiver", &fleet);
    seed(&source.files(), "moved", "broca", "moved-call");
    let cut = cut_id(9);
    fleet.moves(&cut, &[session("moved")]);
    source.op("agent_data.prepare", &cut).await.unwrap();
    source.op("agent_data.export", &cut).await.unwrap();
    receiver.op("agent_data.import", &cut).await.unwrap();
    receiver.op("agent_data.activate", &cut).await.unwrap();
    source
        .call(
            "agent_data.release",
            json!({"refs": refs(), "cutId": cut, "holder": "author-b"}),
        )
        .await
        .unwrap();
    let refusal = refusal(&source.cuts.hold("moved").unwrap());
    assert_eq!(refusal.code, SESSION_HELD_ELSEWHERE);
    assert_eq!(refusal.detail, None);
}

/// An agent with no rows here carries no stream: its cut has no item of
/// Occipital's, and the receiver imports nothing, so a computer without
/// Occipital doesn't block its move.
#[tokio::test]
async fn an_agent_with_no_rows_carries_nothing() {
    let fleet = Arc::new(Fleet::default());
    let source = computer("moves-empty-source", &fleet);
    let receiver = computer("moves-empty-receiver", &fleet);
    let cut = cut_id(2);
    fleet.moves(&cut, &[session("quiet")]);
    let prepared = source.op("agent_data.prepare", &cut).await.unwrap();
    assert_eq!(prepared, json!({"items": [], "external": []}));
    // Its sends are held all the same, so none lands after the cut.
    assert_eq!(source.cuts.hold("quiet"), Some(Hold::Moving));
    assert_eq!(
        source.op("agent_data.export", &cut).await.unwrap(),
        json!({"items": []})
    );
    assert_eq!(
        receiver.op("agent_data.import", &cut).await.unwrap(),
        json!({"items": []})
    );
    assert_eq!(
        receiver.op("agent_data.activate", &cut).await.unwrap(),
        json!({})
    );
}

/// Aborting lifts the source's hold; an abort of a cut never seen records it,
/// so a late prepare is refused; an abort after release is refused.
#[tokio::test]
async fn abort_lifts_the_hold_and_closes_the_cut() {
    let fleet = Arc::new(Fleet::default());
    let source = computer("moves-abort", &fleet);
    seed(&source.files(), "moved", "broca", "moved-call");
    let cut = cut_id(3);
    fleet.moves(&cut, &[session("moved")]);
    source.op("agent_data.prepare", &cut).await.unwrap();
    assert_eq!(source.cuts.hold("moved"), Some(Hold::Moving));
    assert_eq!(
        source.op("agent_data.abort", &cut).await.unwrap(),
        json!({})
    );
    assert_eq!(source.cuts.hold("moved"), None);
    assert_eq!(
        exported(&source.files(), "moved").len(),
        super::rows::TABLES.len()
    );

    let unseen = cut_id(4);
    fleet.moves(&unseen, &[session("moved")]);
    assert_eq!(
        source.op("agent_data.abort", &unseen).await.unwrap(),
        json!({})
    );
    let refused = source.op("agent_data.prepare", &unseen).await.unwrap_err();
    assert_eq!(refused.0, "invalid_state");
    assert_eq!(refused.1["step"], "aborted");
}

/// The agent coming back here lifts the hold its move away left, even for a
/// session the incoming stream holds no rows for.
#[tokio::test]
async fn a_move_back_lifts_the_hold() {
    let fleet = Arc::new(Fleet::default());
    let mut home = computer("moves-home", &fleet);
    let away = computer("moves-away", &fleet);
    seed(&home.files(), "moved", "broca", "moved-call");
    let out = cut_id(5);
    fleet.moves(&out, &[session("moved"), session("quiet")]);
    for op in ["agent_data.prepare", "agent_data.export"] {
        home.op(op, &out).await.unwrap();
    }
    away.op("agent_data.import", &out).await.unwrap();
    away.op("agent_data.activate", &out).await.unwrap();
    home.call(
        "agent_data.release",
        json!({"refs": refs(), "cutId": out, "holder": "author-b"}),
    )
    .await
    .unwrap();
    assert!(matches!(
        home.cuts.hold("quiet"),
        Some(Hold::Elsewhere { .. })
    ));

    let back = cut_id(6);
    fleet.moves(&back, &[session("moved"), session("quiet")]);
    for op in ["agent_data.prepare", "agent_data.export"] {
        away.op(op, &back).await.unwrap();
    }
    home.moves.clock = || 0;
    home.op("agent_data.import", &back).await.unwrap();
    home.op("agent_data.activate", &back).await.unwrap();
    assert_eq!(home.cuts.hold("moved"), None);
    assert_eq!(home.cuts.hold("quiet"), None);
    assert_eq!(
        exported(&home.files(), "moved").len(),
        super::rows::TABLES.len()
    );
}

/// A stream longer than one admission goes through the door in pieces; when
/// one is refused, `prepare` is refused and its repeat carries on from where
/// the door's copy ends, so the receiver reads the whole stream once.
#[tokio::test]
async fn a_long_stream_goes_in_pieces_and_resumes() {
    let fleet = Arc::new(Fleet::default());
    let source = computer("moves-long-source", &fleet);
    let receiver = computer("moves-long-receiver", &fleet);
    seed(&source.files(), "moved", "broca", "moved-call");
    let big = vec![b'x'; 3 * DOOR_CHUNK as usize / 2];
    source
        .files()
        .details
        .with_conn_fenced(|tx| {
            tx.execute(
                "INSERT INTO display_writes (key, writer, runner, call_key, kind, schema_pin, body, stored_at_ms)
                 SELECT key || X'00', writer, runner, call_key, 'detail', schema_pin, ?1, stored_at_ms
                 FROM display_writes",
                [&big],
            )
        })
        .unwrap();
    let cut = cut_id(8);
    fleet.moves(&cut, &[session("moved")]);
    *fleet.refuse_admit_at.lock().unwrap() = Some(DOOR_CHUNK);
    let refused = source.op("agent_data.prepare", &cut).await.unwrap_err();
    assert_eq!(refused.0, "unavailable");
    assert_eq!(source.cuts.hold("moved"), Some(Hold::Moving));
    let prepared = source.op("agent_data.prepare", &cut).await.unwrap();
    let end = prepared["items"][0]["endOffset"].as_u64().unwrap();
    assert!(end > 2 * DOOR_CHUNK, "{end}");
    assert_eq!(
        fleet.streams.lock().unwrap().values().next().unwrap().len() as u64,
        end
    );
    source.op("agent_data.export", &cut).await.unwrap();
    receiver.op("agent_data.import", &cut).await.unwrap();
    receiver.op("agent_data.activate", &cut).await.unwrap();
    assert_eq!(
        exported(&receiver.files(), "moved"),
        exported(&source.files(), "moved")
    );
}

/// A stream whose bytes don't hash to the cut's digest is never imported.
#[tokio::test]
async fn a_changed_stream_is_refused() {
    let fleet = Arc::new(Fleet::default());
    let source = computer("moves-tamper-source", &fleet);
    let receiver = computer("moves-tamper-receiver", &fleet);
    seed(&source.files(), "moved", "broca", "moved-call");
    let cut = cut_id(7);
    fleet.moves(&cut, &[session("moved")]);
    source.op("agent_data.prepare", &cut).await.unwrap();
    let mut bytes = fleet
        .streams
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    let last = bytes.len() - 2;
    bytes[last] ^= 1;
    *fleet.tampered.lock().unwrap() = Some(bytes);
    let refused = receiver.op("agent_data.import", &cut).await.unwrap_err();
    assert_eq!(refused.0, "digest_mismatch");
    assert_eq!(
        receiver.op("agent_data.status", &cut).await.unwrap()["step"],
        "none"
    );
}

/// A stream in a format this build doesn't read is refused as needing an
/// upgrade, decided from its first line.
#[test]
fn another_format_is_an_upgrade() {
    let stream = b"{\"agent\":\"a\",\"format\":\"occipital-chat-display/v9\"}\n";
    let dir = ScratchDir::new("moves-format");
    let path = dir.path().join("stream");
    std::fs::write(&path, stream).unwrap();
    let refused = read_stream(&path).err().unwrap();
    assert_eq!(refused.code, "incompatible_version");
    assert_eq!(
        refused.detail,
        Some(json!({"found": "occipital-chat-display/v9", "required": payload::FORMAT}))
    );
}

/// One request and its answer, as Entorhinal sends and reads it.
fn exchange(principal: &Principal, method: &str, params: Value, outcome: HandlerOutcome) -> Value {
    let reply = match outcome {
        HandlerOutcome::Response(bytes) => serde_json::from_slice(&bytes).unwrap(),
        HandlerOutcome::Error { code, message } => {
            json!({"code": code, "message": message, "detail": null})
        }
        HandlerOutcome::ErrorWithDetail {
            code,
            message,
            detail,
        } => json!({"code": code, "message": message, "detail": detail}),
        HandlerOutcome::Streamed => panic!("an agent-data op never streams"),
    };
    json!({
        "op": method,
        "principal": principal,
        "request": {"method": method, "params": params},
        "reply": reply,
    })
}

impl Computer {
    async fn record(&self, principal: &Principal, method: &str, params: Value) -> Value {
        let body = serde_json::to_vec(&json!({"method": method, "params": params})).unwrap();
        let outcome = self.moves.serve(Some(principal), &body).await;
        exchange(principal, method, params, outcome)
    }
}

/// The participant's samples, `test-vectors/agent-data-v1/`: one move of an
/// agent with display rows, as Entorhinal's driver sees it on each computer,
/// plus the refusals a driver must place. Run with
/// `OCCIPITAL_BLESS_VECTORS=1` to rewrite them; without it this fails when a
/// reply's bytes would change.
#[tokio::test]
async fn the_participant_samples_are_what_it_serves() {
    let fleet = Arc::new(Fleet::default());
    let source = computer("moves-samples-source", &fleet);
    let receiver = computer("moves-samples-receiver", &fleet);
    seed(&source.files(), "ses_moved", "broca", "moved-call");
    seed(&source.files(), "ses_stays", "broca", "stays-call");
    let cut = cut_id(9);
    fleet.moves(&cut, &[session("ses_moved")]);
    let ento = entorhinal();
    let at_cut = || json!({"refs": refs(), "cutId": cut});

    let mut files: Vec<(&str, Vec<Value>)> = Vec::new();
    files.push((
        "source.json",
        vec![
            source.record(&ento, "agent_data.describe", json!({})).await,
            source
                .record(&ento, "agent_data.busy", json!({"refs": refs()}))
                .await,
            source.record(&ento, "agent_data.prepare", at_cut()).await,
            source.record(&ento, "agent_data.export", at_cut()).await,
            source.record(&ento, "agent_data.status", at_cut()).await,
        ],
    ));
    files.push((
        "receiver.json",
        vec![
            receiver
                .record(&ento, "agent_data.diverged", at_cut())
                .await,
            receiver.record(&ento, "agent_data.import", at_cut()).await,
            receiver
                .record(&ento, "agent_data.activate", at_cut())
                .await,
            receiver.record(&ento, "agent_data.status", at_cut()).await,
        ],
    ));
    files.push((
        "release.json",
        vec![
            source
                .record(
                    &ento,
                    "agent_data.release",
                    json!({"refs": refs(), "cutId": cut, "holder": "author-b"}),
                )
                .await,
        ],
    ));
    let empty = cut_id(10);
    fleet.moves(&empty, &[session("ses_quiet")]);
    files.push((
        "empty-agent.json",
        vec![
            source
                .record(
                    &ento,
                    "agent_data.prepare",
                    json!({"refs": refs(), "cutId": empty}),
                )
                .await,
            receiver
                .record(
                    &ento,
                    "agent_data.import",
                    json!({"refs": refs(), "cutId": empty}),
                )
                .await,
        ],
    ));
    let broca = Principal::Reserved {
        module_id: "broca".into(),
    };
    let unseen = cut_id(11);
    files.push((
        "refusals.json",
        vec![
            source
                .record(&broca, "agent_data.describe", json!({}))
                .await,
            source
                .record(
                    &ento,
                    "agent_data.busy",
                    json!({"refs": [{"ref_kind": "broca-session", "ref": "ses_moved"}]}),
                )
                .await,
            source
                .record(
                    &ento,
                    "agent_data.export",
                    json!({"refs": refs(), "cutId": unseen}),
                )
                .await,
            source
                .record(
                    &ento,
                    "agent_data.release",
                    json!({"refs": refs(), "cutId": cut, "holder": "author-c"}),
                )
                .await,
        ],
    ));

    let folder =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-vectors/agent-data-v1");
    let bless = std::env::var_os("OCCIPITAL_BLESS_VECTORS").is_some();
    let mut sums = String::new();
    for (name, cases) in &files {
        let mut bytes = serde_json::to_vec_pretty(cases).unwrap();
        bytes.push(b'\n');
        sums.push_str(&format!(
            "{}  {name}\n",
            payload::hex(&Sha256::digest(&bytes))
        ));
        let path = folder.join(name);
        if bless {
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(&path, &bytes).unwrap();
        } else {
            assert_eq!(
                std::fs::read(&path).unwrap_or_default(),
                bytes,
                "{name} changed; rerun with OCCIPITAL_BLESS_VECTORS=1"
            );
        }
    }
    let sums_path = folder.join("SHA256SUMS");
    if bless {
        std::fs::write(&sums_path, &sums).unwrap();
    } else {
        assert_eq!(
            std::fs::read_to_string(&sums_path).unwrap_or_default(),
            sums
        );
    }
}

#[tokio::test]
async fn release_preserves_waiting_sends_written_after_export() {
    let fleet = Arc::new(Fleet::default());
    let source = computer("moves-late-send", &fleet);
    seed(&source.files(), "moved", "broca", "call");
    let cut = cut_id(1);
    fleet.moves(&cut, &[session("moved")]);
    source.op("agent_data.prepare", &cut).await.unwrap();
    source.op("agent_data.export", &cut).await.unwrap();
    source.files().store.with_conn_fenced(|tx| tx.execute("INSERT INTO harness_app_send_queue (session,client_message_id,harness,send_id,accepted_at,state) VALUES ('moved','late','broca','late',8,'queued')", [])).unwrap();
    source
        .call(
            "agent_data.release",
            json!({"refs": refs(), "cutId": cut, "holder":"author-b"}),
        )
        .await
        .unwrap();
    let left = exported(&source.files(), "moved");
    assert_eq!(
        left.iter()
            .filter(|(t, _)| t == "queued_sends")
            .map(|(_, row)| &row[1])
            .collect::<Vec<_>>(),
        [&rusqlite::types::Value::Text("late".into())]
    );
}

#[tokio::test]
async fn partial_activation_refuses_abort_and_can_finish_after_restart() {
    let fleet = Arc::new(Fleet::default());
    let src = computer("moves-partial-src", &fleet);
    let mut dst = computer("moves-partial-dst", &fleet);
    let cut = cut_id(1);
    seed(&src.files(), "s", "broca", "c");
    fleet.moves(&cut, &[session("s")]);
    src.op("agent_data.prepare", &cut).await.unwrap();
    dst.op("agent_data.import", &cut).await.unwrap();
    dst.files().details.with_conn_fenced(|tx| tx.execute_batch("CREATE TRIGGER stop BEFORE INSERT ON display_writes BEGIN SELECT RAISE(ABORT,'busy'); END")).unwrap();
    dst.op("agent_data.activate", &cut).await.unwrap_err();
    assert_eq!(
        dst.opened
            .display
            .call_pin("broca", "c")
            .unwrap()
            .unwrap()
            .0,
        "s"
    );
    dst.moves.cuts = Arc::new(Cuts::open(dst.opened.main.data_root()).unwrap());
    assert_eq!(
        dst.op("agent_data.status", &cut).await.unwrap()["step"],
        "imported"
    );
    assert_eq!(
        dst.op("agent_data.abort", &cut).await.unwrap_err().0,
        "invalid_state"
    );
    dst.files()
        .details
        .with_conn_fenced(|tx| tx.execute_batch("DROP TRIGGER stop"))
        .unwrap();
    dst.op("agent_data.activate", &cut).await.unwrap();
    assert_eq!(exported(&src.files(), "s"), exported(&dst.files(), "s"));
}

#[tokio::test(start_paused = true)]
async fn a_slow_prepare_does_not_block_status_or_other_agents() {
    let fleet = Arc::new(Fleet::default());
    let source = computer("moves-independent", &fleet);
    let cut = cut_id(1);
    let other = cut_id(2);
    fleet.moves(&cut, &[]);
    fleet.moves(&other, &[]);
    *fleet.paused_cut.lock().unwrap() = Some(cut.clone());
    let pending = source.op("agent_data.prepare", &cut);
    tokio::pin!(pending);
    assert!(futures_util::poll!(&mut pending).is_pending());
    let quick = std::time::Duration::from_secs(1);
    tokio::time::timeout(quick, source.op("agent_data.status", &cut))
        .await
        .unwrap()
        .unwrap();
    let same = source.op("agent_data.prepare", &other);
    tokio::pin!(same);
    assert!(futures_util::poll!(&mut same).is_pending());
    let unrelated = cut_id(3);
    fleet.moves(&unrelated, &[]);
    let params =
        json!({"refs":[{"ref_kind":"agent","ref":"agent_0000000000000102"}],"cutId":unrelated});
    tokio::time::timeout(quick, source.call("agent_data.prepare", params))
        .await
        .unwrap()
        .unwrap();
    fleet.resume.notify_one();
    pending.await.unwrap();
    same.await.unwrap();
}

mod admitted;
