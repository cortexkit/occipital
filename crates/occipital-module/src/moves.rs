//! Occipital's part in moving an agent to another computer: the agent-data
//! participant (`cortexkit-role-agent-data`, served on the `agent-data`
//! internal service to Entorhinal only).
//!
//! A moved agent's chats keep their tool records, details, call pins, riding
//! notices, wake records, message-to-send map and waiting messages on the new
//! computer. Occipital carries those rows for the sessions the move carries:
//!
//! - **prepare** (source): asks Entorhinal which sessions the cut moves, holds
//!   new sends to them (durably, before replying), writes their rows as one
//!   stream (`payload`) and admits it through Engram's session door.
//! - **export** (source): checks the door holds the whole stream.
//! - **import** (receiver): reads the stream back through the door with the
//!   cut's id, checks its digest, and keeps it, unapplied.
//! - **activate** (receiver): inserts the rows, keeping every row already held,
//!   and lifts any hold an earlier move away left on those sessions.
//! - **release** (source, after activation): drops the sessions' waiting
//!   sends, which the new computer follows now, and keeps holding sends to
//!   the sessions as living there. The display rows stay: a record a provider
//!   writes after the cut is kept for a move back.
//! - **abort**: lifts the source's hold, or drops the receiver's stream.
//!
//! The rows are display data only, so Occipital is never busy, and an agent
//! with no rows here carries no stream. Each step is recorded per cut
//! (`state`), so a repeated op gets its first answer.

pub(crate) mod payload;
pub(crate) mod remote;
pub(crate) mod rows;
pub mod state;

use std::collections::BTreeSet;
use std::sync::Arc;

use cortexkit_store::SqliteStore;
use occipital_app_reader::send::SendRefusal;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use subc_client_rs::HandlerOutcome;
use subc_protocol::Principal;

use crate::display::daemon::CallFailure;
use crate::display::store::DisplayStore;
use crate::notices::NoticeStore;
use payload::{Session, FORMAT};
use remote::Remote;
use state::{Cut, Cuts, Hold, Item, Side, Step};

/// The internal service Entorhinal opens to reach a participant.
pub const SERVICE_ID: &str = "agent-data";

/// The one ref kind Occipital takes: the whole agent. It never names a
/// runner's session kind, since covering one makes a move admissible and
/// Occipital can't carry a conversation.
const REF_KIND: &str = "agent";

/// The only caller the contract admits.
const MOVER: &str = "entorhinal";

/// Engram admits at most this many raw bytes in one `session.admit`, and one
/// `session.read` returns at most as many.
const DOOR_CHUNK: u64 = 1_048_576;

/// A send to a session whose agent is moving to another computer now. The app
/// keeps the draft: once the move ends, the chat answers from where its agent
/// lives.
pub const SESSION_MOVING: &str = "session_moving";

/// A send or read for a session whose agent now lives on another computer,
/// which `holder` names when known. The app looks the agent up again and
/// reaches the chat there.
pub const SESSION_HELD_ELSEWHERE: &str = "session_held_elsewhere";

/// The new computer a runner's `session_held_elsewhere` refusal names in its
/// `detail`, or empty when it names none. A `holder` that isn't a box id
/// (32 lowercase hex digits) is left out, so the app never looks up
/// something that can't be a computer.
pub(crate) fn runner_holder(detail: Option<&Value>) -> String {
    detail
        .and_then(|detail| {
            serde_json::from_value::<
                cortexkit_role_llm_runner::session_retention::SessionHeldElsewhereDetail,
            >(detail.clone())
            .ok()
        })
        .and_then(|detail| detail.holder)
        .filter(|holder| {
            holder.len() == 32
                && holder
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .unwrap_or_default()
}

/// The refusal for a send a move holds.
pub(crate) fn refusal(hold: &Hold) -> SendRefusal {
    match hold {
        Hold::Moving => SendRefusal::new(
            SESSION_MOVING,
            "this chat's agent is moving to another computer; send again once the move ends",
        ),
        Hold::Elsewhere { holder } => SendRefusal {
            code: SESSION_HELD_ELSEWHERE,
            message: "this chat's agent lives on another computer now".into(),
            detail: (!holder.is_empty()).then(|| json!({ "holder": holder })),
        },
    }
}

/// The files a move reads and writes.
pub(crate) struct Stores {
    pub display: Arc<DisplayStore>,
    pub notices: Arc<NoticeStore>,
    pub store: Arc<SqliteStore>,
}

impl Stores {
    fn files(&self) -> rows::Files<'_> {
        let (records, details) = self.display.files();
        rows::Files {
            records,
            details,
            notices: self.notices.file(),
            store: &self.store,
        }
    }
}

pub struct Moves {
    cuts: Arc<Cuts>,
    remote: Arc<dyn Remote>,
    stores: Arc<Stores>,
    clock: fn() -> i64,
    cut_locks: crate::key_locks::KeyLocks,
    agent_locks: crate::key_locks::KeyLocks,
}

/// A refusal, as the daemon's error frame carries it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Refusal {
    pub code: &'static str,
    pub message: String,
    pub detail: Option<Value>,
}

fn refuse(code: &'static str, message: impl Into<String>) -> Refusal {
    Refusal {
        code,
        message: message.into(),
        detail: None,
    }
}

fn unavailable(error: impl std::fmt::Display) -> Refusal {
    refuse("unavailable", error.to_string())
}

impl Moves {
    pub(crate) fn new(
        cuts: Arc<Cuts>,
        remote: Arc<dyn Remote>,
        stores: Arc<Stores>,
        clock: fn() -> i64,
    ) -> Self {
        Self {
            cuts,
            remote,
            stores,
            clock,
            cut_locks: Default::default(),
            agent_locks: Default::default(),
        }
    }

    /// Serve one request on the `agent-data` route. `caller` is the route's
    /// attested opener: only Entorhinal is answered, before anything in the
    /// request is read.
    pub async fn serve(&self, caller: Option<&Principal>, body: &[u8]) -> HandlerOutcome {
        let answer = match caller {
            Some(Principal::Reserved { module_id }) if module_id == MOVER => {
                self.answer(body).await
            }
            _ => Err(refuse("caller_forbidden", "only Entorhinal moves agents")),
        };
        match answer {
            Ok(result) => HandlerOutcome::Response(
                serde_json::to_vec(&json!({ "result": result })).unwrap_or_default(),
            ),
            Err(refusal) => match refusal.detail {
                Some(detail) => HandlerOutcome::ErrorWithDetail {
                    code: refusal.code.into(),
                    message: refusal.message,
                    detail,
                },
                None => HandlerOutcome::Error {
                    code: refusal.code.into(),
                    message: refusal.message,
                },
            },
        }
    }

    async fn answer(&self, body: &[u8]) -> Result<Value, Refusal> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Envelope {
            method: String,
            params: Value,
        }
        let envelope: Envelope = serde_json::from_slice(body)
            .map_err(|error| refuse("invalid_request", error.to_string()))?;
        let op = cortexkit_role_agent_data::Op::ALL
            .into_iter()
            .find(|op| op.method() == envelope.method)
            .ok_or_else(|| refuse("invalid_request", "unknown agent-data operation"))?;
        let answered = self.op(&envelope.method, envelope.params).await;
        tracing::info!(
            method = %envelope.method,
            outcome = answered.as_ref().map_or_else(|refusal| refusal.code, |_| "ok"),
            "agent move op answered"
        );
        // A code outside the op's row is this module's bug; the driver retries
        // `unavailable` rather than stopping on a code it can't place.
        answered.map_err(|mut refusal| {
            let code = cortexkit_role_agent_data::errors::RefusalCode::from_code(refusal.code);
            if !cortexkit_role_agent_data::errors::allowed_refusals(op).contains(&code) {
                refusal.code = "unavailable";
            }
            refusal
        })
    }

    async fn op(&self, method: &str, params: Value) -> Result<Value, Refusal> {
        use cortexkit_role_agent_data as contract;
        let decode = |error: serde_json::Error| refuse("invalid_request", error.to_string());
        match method {
            "agent_data.describe" => {
                let _: contract::describe::DescribeRequest =
                    serde_json::from_value(params).map_err(decode)?;
                // Optional: chat display data never blocks a move. A computer
                // that doesn't run Occipital takes the agent without it, and
                // the rows stay on this one for a move back.
                Ok(json!(contract::describe::DescribeReply::new(
                    vec![REF_KIND.into()],
                    vec![contract::describe::Store::new(FORMAT, "agent")],
                    false,
                )
                .with_optional(true)))
            }
            "agent_data.busy" => {
                let request: contract::busy::BusyRequest =
                    serde_json::from_value(params).map_err(decode)?;
                for r in request.refs.as_slice() {
                    agent_of(r)?;
                }
                let idle: Vec<Value> = request
                    .refs
                    .as_slice()
                    .iter()
                    .map(|_| json!({"state": "idle"}))
                    .collect();
                Ok(json!({ "refs": idle }))
            }
            "agent_data.release" => {
                let request: contract::release::ReleaseRequest =
                    serde_json::from_value(params).map_err(decode)?;
                let agent = one_agent(request.refs.as_slice())?;
                let _cut = self.cut_locks.lock(request.cut_id.as_str()).await;
                let _agent = self.agent_locks.lock(agent).await;
                self.release(
                    request.cut_id.as_str(),
                    agent,
                    &request.holder,
                    request.holder_box.as_deref(),
                )
                .await
            }
            _ => {
                let request: contract::request::CutRequest =
                    serde_json::from_value(params).map_err(decode)?;
                let cut_id = request.cut_id.as_str();
                let refs = request.refs.as_slice();
                // Status reads one atomically replaced file. Mutations take
                // the cut first, then the agent so competing cuts are ordered.
                let (_cut, _agent) = match method {
                    "agent_data.status" | "agent_data.diverged" => (None, None),
                    _ => {
                        let cut = self.cut_locks.lock(cut_id).await;
                        let agent = match refs.first() {
                            Some(r) => Some(self.agent_locks.lock(r.r#ref.as_str()).await),
                            None => None,
                        };
                        (Some(cut), agent)
                    }
                };
                match method {
                    "agent_data.status" => {
                        // A status read never refuses for its refs: it can reach a
                        // receiver whose import never ran.
                        let step = self.load(cut_id)?.map_or("none", |cut| cut.step.name());
                        Ok(json!({ "step": step }))
                    }
                    "agent_data.diverged" => {
                        for r in refs {
                            agent_of(r)?;
                        }
                        // Import inserts without replacing, and every row is
                        // immutable, so importing a cut here never loses one.
                        let at_cut: Vec<Value> =
                            refs.iter().map(|_| json!({"state": "at_cut"})).collect();
                        Ok(json!({ "refs": at_cut }))
                    }
                    "agent_data.prepare" => self.prepare(cut_id, one_agent(refs)?).await,
                    "agent_data.export" => self.export(cut_id, one_agent(refs)?).await,
                    "agent_data.import" => self.import(cut_id, one_agent(refs)?).await,
                    "agent_data.activate" => self.activate(cut_id, one_agent(refs)?).await,
                    "agent_data.abort" => self.abort(cut_id, refs).await,
                    _ => Err(refuse("invalid_request", "unknown agent-data operation")),
                }
            }
        }
    }

    fn load(&self, cut_id: &str) -> Result<Option<Cut>, Refusal> {
        self.cuts.load(cut_id).map_err(unavailable)
    }

    fn save(&self, cut_id: &str, cut: &Cut) -> Result<(), Refusal> {
        self.cuts.save(cut_id, cut).map_err(unavailable)
    }

    /// The cut's saved state, checked against this request: it must be the
    /// agent the cut started with, else `invalid_state` naming that ref.
    fn existing(&self, cut_id: &str, agent: &str) -> Result<Option<Cut>, Refusal> {
        let cut = self.load(cut_id)?;
        if let Some(cut) = &cut {
            if cut.agent != agent && cut.side.is_some() {
                return Err(Refusal {
                    code: "invalid_state",
                    message: "the cut started with another agent".into(),
                    detail: Some(json!({
                        "step": cut.step.name(),
                        "refs": [{"ref_kind": REF_KIND, "ref": cut.agent}],
                    })),
                });
            }
        }
        Ok(cut)
    }

    async fn prepare(&self, cut_id: &str, agent: &str) -> Result<Value, Refusal> {
        let mut cut = match self.existing(cut_id, agent)? {
            Some(cut) => match (cut.side, cut.step) {
                (Some(Side::Source), Step::Prepared) if cut.pending => cut,
                (Some(Side::Source), Step::Prepared | Step::Exported | Step::Released) => {
                    return Ok(prepare_reply(agent, &cut))
                }
                (_, step) => return Err(invalid_state(step)),
            },
            None => {
                let sessions = self.moved_sessions(cut_id, agent).await?;
                // Hold new sends first, durably, so none lands after the rows
                // are read.
                let cut = Cut {
                    agent: agent.into(),
                    side: Some(Side::Source),
                    step: Step::Prepared,
                    item: None,
                    sessions,
                    holder: None,
                    holder_box: None,
                    pending: true,
                    at_ms: (self.clock)(),
                };
                self.save(cut_id, &cut)?;
                cut
            }
        };
        // The stream is written once; a repeat after a failed admission
        // carries on admitting the same bytes.
        if cut.item.is_none() {
            let path = self.cuts.stream_path(cut_id);
            let (stores, agent_id, moved) =
                (self.stores.clone(), agent.to_owned(), cut.sessions.clone());
            let written = tokio::task::spawn_blocking(move || {
                write_stream(&stores, &path, &agent_id, &moved)
            })
            .await
            .map_err(unavailable)?
            .map_err(|error| refuse("participant_failed", error))?;
            cut.item = written.map(|(end_offset, digest)| Item {
                session_ref: session_ref(agent, cut_id),
                end_offset,
                digest,
            });
            self.save(cut_id, &cut)?;
        }
        if let Some(item) = &cut.item {
            self.admit(cut_id, item).await?;
        }
        cut.pending = false;
        self.save(cut_id, &cut)?;
        Ok(prepare_reply(agent, &cut))
    }

    async fn export(&self, cut_id: &str, agent: &str) -> Result<Value, Refusal> {
        let mut cut = self
            .existing(cut_id, agent)?
            .ok_or_else(|| refuse("cut_unknown", "prepare this cut first"))?;
        match (cut.side, cut.step) {
            (Some(Side::Source), Step::Prepared) if cut.pending => {
                return Err(unavailable(
                    "this cut's prepare didn't finish; prepare it again",
                ))
            }
            (Some(Side::Source), Step::Prepared) => {
                if let Some(item) = &cut.item {
                    self.cover(item).await?;
                }
                cut.step = Step::Exported;
                cut.at_ms = (self.clock)();
                self.save(cut_id, &cut)?;
            }
            (Some(Side::Source), Step::Exported | Step::Released) => {}
            (_, step) => return Err(invalid_state(step)),
        }
        let items: Vec<Value> = cut
            .item
            .iter()
            .map(|item| json!({"sessionRef": item.session_ref, "endOffset": item.end_offset}))
            .collect();
        Ok(json!({ "items": items }))
    }

    /// The source lets the sessions go to the new computer. `holder_box`, the
    /// new computer's box id when the driver names it, is what a refused send
    /// names; `holder` is a pseudonym, kept only to tell repeats apart. A
    /// repeat naming either differently is another request.
    async fn release(
        &self,
        cut_id: &str,
        agent: &str,
        holder: &str,
        holder_box: Option<&str>,
    ) -> Result<Value, Refusal> {
        let mut cut = self
            .existing(cut_id, agent)?
            .ok_or_else(|| refuse("cut_unknown", "prepare this cut first"))?;
        match (cut.side, cut.step) {
            (Some(Side::Source), Step::Exported) => {
                if let Some(item) = cut.item.clone() {
                    let (stores, path) = (self.stores.clone(), self.cuts.stream_path(cut_id));
                    tokio::task::spawn_blocking(move || {
                        apply_stream(&path, &item, |batch| {
                            stores.files().delete_waiting_sends(batch)
                        })
                    })
                    .await
                    .map_err(unavailable)??;
                }
                cut.step = Step::Released;
                cut.holder = Some(holder.into());
                cut.holder_box = holder_box.map(str::to_owned);
                cut.at_ms = (self.clock)();
                self.save(cut_id, &cut)?;
                let _ = std::fs::remove_file(self.cuts.stream_path(cut_id));
            }
            (Some(Side::Source), Step::Released)
                if cut.holder.as_deref() == Some(holder)
                    && cut.holder_box.as_deref() == holder_box => {}
            (Some(Side::Source), Step::Released) => {
                return Err(refuse(
                    "id_reused",
                    "this cut was released to another holder",
                ))
            }
            (_, step) => return Err(invalid_state(step)),
        }
        Ok(json!({ "enforced": true }))
    }

    async fn import(&self, cut_id: &str, agent: &str) -> Result<Value, Refusal> {
        if let Some(cut) = self.existing(cut_id, agent)? {
            return match (cut.side, cut.step) {
                (Some(Side::Receiver), Step::Imported | Step::Activating | Step::Activated) => {
                    Ok(import_reply(&cut))
                }
                (_, step) => Err(invalid_state(step)),
            };
        }
        let session_ref = session_ref(agent, cut_id);
        let path = self.cuts.stream_path(cut_id);
        let item = match self.download(cut_id, &session_ref, &path).await? {
            Some(item) => item,
            None => {
                let cut = Cut {
                    agent: agent.into(),
                    side: Some(Side::Receiver),
                    step: Step::Imported,
                    item: None,
                    sessions: Vec::new(),
                    holder: None,
                    holder_box: None,
                    pending: false,
                    at_ms: (self.clock)(),
                };
                self.save(cut_id, &cut)?;
                return Ok(import_reply(&cut));
            }
        };
        let read_path = path.clone();
        let read = tokio::task::spawn_blocking(move || read_stream(&read_path))
            .await
            .map_err(unavailable)?;
        let read = match read {
            Ok(read) => read,
            Err(refusal) => {
                let _ = std::fs::remove_file(&path);
                return Err(refusal);
            }
        };
        if read.agent != agent {
            let _ = std::fs::remove_file(&path);
            return Err(refuse(
                "participant_failed",
                "the stream is another agent's",
            ));
        }
        let cut = Cut {
            agent: agent.into(),
            side: Some(Side::Receiver),
            step: Step::Imported,
            item: Some(item),
            sessions: read.sessions,
            holder: None,
            holder_box: None,
            pending: false,
            at_ms: (self.clock)(),
        };
        self.save(cut_id, &cut)?;
        Ok(import_reply(&cut))
    }

    async fn activate(&self, cut_id: &str, agent: &str) -> Result<Value, Refusal> {
        let mut cut = self
            .existing(cut_id, agent)?
            .ok_or_else(|| refuse("cut_unknown", "import this cut first"))?;
        match (cut.side, cut.step) {
            (Some(Side::Receiver), Step::Imported | Step::Activating) => {
                if cut.step == Step::Imported {
                    cut.step = Step::Activating;
                    self.save(cut_id, &cut)?;
                }
                if let Some(item) = cut.item.clone() {
                    let (stores, path) = (self.stores.clone(), self.cuts.stream_path(cut_id));
                    tokio::task::spawn_blocking(move || {
                        apply_stream(&path, &item, |batch| stores.files().import(batch))
                    })
                    .await
                    .map_err(unavailable)??;
                }
                cut.step = Step::Activated;
                cut.at_ms = (self.clock)();
                self.save(cut_id, &cut)?;
                let _ = std::fs::remove_file(self.cuts.stream_path(cut_id));
            }
            (Some(Side::Receiver), Step::Activated) => {}
            (_, step) => return Err(invalid_state(step)),
        }
        Ok(json!({}))
    }

    async fn abort(
        &self,
        cut_id: &str,
        refs: &[cortexkit_role_agent_data::refs::Ref],
    ) -> Result<Value, Refusal> {
        let cut = self.load(cut_id)?;
        let mut cut = match cut {
            Some(cut) => {
                // A cut seen here keeps the agent it started with.
                if cut.side.is_some() {
                    self.existing(cut_id, one_agent(refs)?)?;
                }
                cut
            }
            // An abort of a cut never seen touches nothing, and records the abort
            // so a late prepare or import is refused.
            None => Cut {
                agent: refs
                    .first()
                    .map(|r| r.r#ref.to_string())
                    .unwrap_or_default(),
                side: None,
                step: Step::Aborted,
                item: None,
                sessions: Vec::new(),
                holder: None,
                holder_box: None,
                pending: false,
                at_ms: (self.clock)(),
            },
        };
        match cut.step {
            Step::Activating | Step::Activated | Step::Released => {
                return Err(invalid_state(cut.step))
            }
            Step::Aborted if cut.side.is_none() => {}
            _ => {
                cut.step = Step::Aborted;
                cut.at_ms = (self.clock)();
            }
        }
        self.save(cut_id, &cut)?;
        let _ = std::fs::remove_file(self.cuts.stream_path(cut_id));
        Ok(json!({}))
    }

    /// The sessions the cut moves, from the move's plan: each ref Entorhinal
    /// lists, mapped to its session. A ref bound to no whole session yet holds
    /// nothing here.
    async fn moved_sessions(&self, cut_id: &str, agent: &str) -> Result<Vec<Session>, Refusal> {
        let refs = self
            .remote
            .move_refs(cut_id, agent)
            .await
            .map_err(|error| remote_refusal("entorhinal", error))?;
        let mut sessions = BTreeSet::new();
        for (kind, reference) in refs {
            if let Some(session) = self
                .remote
                .session_for_ref(&kind, &reference)
                .await
                .map_err(|error| remote_refusal("entorhinal", error))?
            {
                sessions.insert(session);
            }
        }
        Ok(sessions.into_iter().collect())
    }

    /// Admit the stream through the door, resuming where it already ends, and
    /// wait until the cloud holds it all.
    async fn admit(&self, cut_id: &str, item: &Item) -> Result<(), Refusal> {
        let path = self.cuts.stream_path(cut_id);
        let mut offset = self.door_end(item).await?;
        while offset < item.end_offset {
            let length = (item.end_offset - offset).min(DOOR_CHUNK);
            let bytes = read_range(&path, offset, length).map_err(unavailable)?;
            let reply = self
                .remote
                .door(
                    "session.admit",
                    json!({
                        "sessionRef": item.session_ref,
                        "fromOffset": offset,
                        "bytes": payload::hex(&bytes),
                    }),
                )
                .await;
            match reply {
                Ok(reply) if reply["durableOffset"].as_u64() == Some(offset + length) => {
                    offset += length;
                }
                // Another attempt admitted part of it already: carry on from the
                // door's end, which `door_end` checks against this stream.
                Err(error) if error.code.as_deref() == Some("offset_mismatch") => {
                    offset = self.door_end(item).await?;
                }
                Ok(other) => {
                    return Err(unavailable(format!("the door admitted to {other}")));
                }
                Err(error) => return Err(remote_refusal("engram", error)),
            }
        }
        self.cover(item).await
    }

    /// Where the door's copy of the stream ends now. A copy longer than the
    /// stream, or one that differs from it, is never this cut's.
    async fn door_end(&self, item: &Item) -> Result<u64, Refusal> {
        let offsets = self
            .remote
            .door("session.offsets", json!({"sessionRef": item.session_ref}))
            .await;
        let end = match offsets {
            Ok(reply) => reply["durableOffset"].as_u64().unwrap_or(0),
            Err(error) if error.code.as_deref() == Some("not_found") => 0,
            Err(error) => return Err(remote_refusal("engram", error)),
        };
        if end > item.end_offset {
            return Err(refuse(
                "participant_failed",
                "the door holds more of this stream than this cut wrote",
            ));
        }
        Ok(end)
    }

    /// Check the door holds the whole stream, durably and in the cloud.
    async fn cover(&self, item: &Item) -> Result<(), Refusal> {
        let cover = self
            .remote
            .door("session.cover", json!({"sessionRef": item.session_ref}))
            .await
            .map_err(|error| remote_refusal("engram", error))?;
        if cover["durableOffset"].as_u64() != Some(item.end_offset)
            || cover["cloudOffset"].as_u64().unwrap_or(0) < item.end_offset
        {
            return Err(unavailable("the stream isn't in the cloud yet"));
        }
        Ok(())
    }

    /// Read the cut's stream through the door into `path`, checking its
    /// digest. `None` when the cut carries no stream of Occipital's: the
    /// source held no rows for the agent.
    async fn download(
        &self,
        cut_id: &str,
        session_ref: &str,
        path: &std::path::Path,
    ) -> Result<Option<Item>, Refusal> {
        use std::io::Write;
        let mut file = std::fs::File::create(path).map_err(unavailable)?;
        let mut hash = Sha256::new();
        let mut offset = 0u64;
        let mut sealed: Option<(u64, String)> = None;
        loop {
            if let Some((end, _)) = &sealed {
                if offset >= *end {
                    break;
                }
            }
            let reply = self
                .remote
                .door(
                    "session.read",
                    json!({
                        "sessionRef": session_ref,
                        "cutId": cut_id,
                        "fromOffset": offset,
                        "limit": DOOR_CHUNK,
                    }),
                )
                .await;
            let reply = match reply {
                Ok(reply) => reply,
                Err(error) if error.code.as_deref() == Some("cut_item_missing") => {
                    drop(file);
                    let _ = std::fs::remove_file(path);
                    return Ok(None);
                }
                Err(error) if error.code.as_deref() == Some("unavailable") => {
                    return Err(refuse("cut_not_ready", error.message));
                }
                Err(error) if error.code.is_none() => return Err(unavailable(error)),
                Err(error) => {
                    return Err(refuse(
                        "participant_failed",
                        format!("the door refused the cut: {error}"),
                    ))
                }
            };
            let (Some(stream_end), Some(digest)) =
                (reply["streamEnd"].as_u64(), reply["digest"].as_str())
            else {
                return Err(digest_mismatch("the door's read names no end or digest"));
            };
            match &sealed {
                None => sealed = Some((stream_end, digest.to_owned())),
                Some((end, sum)) if *end != stream_end || sum != digest => {
                    return Err(digest_mismatch("the cut's end changed between reads"));
                }
                Some(_) => {}
            }
            let bytes = reply["bytes"]
                .as_str()
                .and_then(payload::unhex)
                .ok_or_else(|| digest_mismatch("the door's read holds no bytes"))?;
            if reply["fromOffset"].as_u64() != Some(offset)
                || bytes.is_empty() && offset < stream_end
            {
                return Err(digest_mismatch("the door read from another offset"));
            }
            hash.update(&bytes);
            file.write_all(&bytes).map_err(unavailable)?;
            offset += bytes.len() as u64;
            if offset > stream_end {
                return Err(digest_mismatch("the door read past the cut"));
            }
        }
        file.sync_all().map_err(unavailable)?;
        let Some((end_offset, digest)) = sealed else {
            return Err(digest_mismatch("the door read nothing"));
        };
        if payload::hex(&hash.finalize()) != digest {
            let _ = std::fs::remove_file(path);
            return Err(digest_mismatch(
                "the stream doesn't hash to the cut's digest",
            ));
        }
        Ok(Some(Item {
            session_ref: session_ref.into(),
            end_offset,
            digest,
        }))
    }
}

/// The agent a request's one ref names.
fn one_agent(refs: &[cortexkit_role_agent_data::refs::Ref]) -> Result<&str, Refusal> {
    match refs {
        [only] => agent_of(only),
        _ => Err(refuse(
            "invalid_request",
            "Occipital takes exactly one agent ref",
        )),
    }
}

fn agent_of(r: &cortexkit_role_agent_data::refs::Ref) -> Result<&str, Refusal> {
    if r.ref_kind.as_str() != REF_KIND {
        return Err(refuse(
            "invalid_request",
            format!("Occipital holds no {} refs", r.ref_kind),
        ));
    }
    Ok(r.r#ref.as_str())
}

/// The stream's door handle, fresh for each cut: a stream is written once, so
/// no move ever appends to another's.
fn session_ref(agent: &str, cut_id: &str) -> String {
    payload::hex(&Sha256::digest(format!("{FORMAT}:{agent}:{cut_id}")))[..32].to_owned()
}

fn session_names(sessions: &[Session]) -> BTreeSet<String> {
    sessions.iter().map(|s| s.session.clone()).collect()
}

fn invalid_state(step: Step) -> Refusal {
    Refusal {
        code: "invalid_state",
        message: format!("the cut is {} here", step.name()),
        detail: Some(json!({ "step": step.name() })),
    }
}

fn digest_mismatch(message: &str) -> Refusal {
    Refusal {
        code: "digest_mismatch",
        message: message.into(),
        detail: Some(json!({ "reason": "staging_digest_mismatch" })),
    }
}

/// A failed call to another module: its refusal is that module's, so a move
/// retries it as `unavailable`.
fn remote_refusal(module: &str, error: CallFailure) -> Refusal {
    unavailable(format!("{module}: {error}"))
}

fn prepare_reply(agent: &str, cut: &Cut) -> Value {
    let items: Vec<Value> = cut
        .item
        .iter()
        .map(|item| {
            json!({
                "ref_kind": REF_KIND,
                "ref": agent,
                "sessionRef": item.session_ref,
                "stream": format!("reserved:{}", crate::MODULE_ID),
                "endOffset": item.end_offset,
                "digest": item.digest,
                "format": FORMAT,
                // Sends are held, but a provider may still write a record for
                // a moved call after the cut. That row stays here, kept for a
                // move back.
                "frozen": false,
            })
        })
        .collect();
    json!({ "items": items, "external": [] })
}

fn import_reply(cut: &Cut) -> Value {
    let items: Vec<Value> = cut
        .item
        .iter()
        .map(|item| {
            json!({
                "sessionRef": item.session_ref,
                "endOffset": item.end_offset,
                "digest": item.digest,
            })
        })
        .collect();
    json!({ "items": items })
}

/// Write the rows of `sessions` to `path` as the stream. `None` when there are
/// none: no stream is written, and the cut carries nothing of Occipital's.
fn write_stream(
    stores: &Stores,
    path: &std::path::Path,
    agent: &str,
    sessions: &[Session],
) -> Result<Option<(u64, String)>, String> {
    let names = session_names(sessions);
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut writer =
        payload::Writer::start(std::io::BufWriter::new(file), agent).map_err(|e| e.to_string())?;
    let mut held = BTreeSet::new();
    let counts = stores.files().export(&names, |row| {
        if let Some(session) = row_session(&row) {
            held.insert(session);
        }
        writer.row(&row).map_err(|e| e.to_string())
    })?;
    let total: u64 = counts.values().sum();
    if total == 0 {
        drop(writer);
        let _ = std::fs::remove_file(path);
        return Ok(None);
    }
    // The stream names the moved sessions it holds rows for.
    let carried: Vec<Session> = sessions
        .iter()
        .filter(|s| held.contains(&s.session))
        .cloned()
        .collect();
    let written = writer.finish(carried).map_err(|e| e.to_string())?;
    std::fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    tracing::info!(?counts, "agent move stream written");
    Ok(Some(written))
}

/// The session a row's `session` column names, if its table has one.
fn row_session((table, values): &rows::Row) -> Option<String> {
    let at = table.session_column()?;
    match values.get(at) {
        Some(rusqlite::types::Value::Text(session)) => Some(session.clone()),
        _ => None,
    }
}

fn read_stream(path: &std::path::Path) -> Result<payload::Read, Refusal> {
    let file = std::fs::File::open(path).map_err(unavailable)?;
    visit_stream(file, |_| Ok(()))
}

fn apply_stream(
    path: &std::path::Path,
    item: &Item,
    mut apply: impl FnMut(&[rows::Row]) -> Result<u64, String>,
) -> Result<(), Refusal> {
    use std::io::Seek;
    // Check the staged bytes before deleting any queued send or inserting a row.
    let mut file = std::fs::File::open(path).map_err(unavailable)?;
    let mut hash = Sha256::new();
    let length = std::io::copy(&mut file, &mut hash).map_err(unavailable)?;
    if length != item.end_offset || payload::hex(&hash.finalize()) != item.digest {
        return Err(digest_mismatch("the staged stream changed"));
    }
    file.rewind().map_err(unavailable)?;
    let mut batch = Vec::new();
    let mut bytes = 0;
    visit_stream(file, |row| {
        use rusqlite::types::Value as Sql;
        let size = row
            .1
            .iter()
            .map(|v| match v {
                Sql::Blob(b) => b.len(),
                Sql::Text(t) => t.len(),
                _ => 8,
            })
            .sum::<usize>();
        if !batch.is_empty() && (batch.len() == 128 || bytes + size > 1 << 20) {
            apply(&batch)?;
            batch.clear();
            bytes = 0;
        }
        batch.push(row);
        bytes += size;
        Ok(())
    })?;
    if !batch.is_empty() {
        apply(&batch).map_err(unavailable)?;
    }
    Ok(())
}

fn visit_stream(
    file: std::fs::File,
    each: impl FnMut(rows::Row) -> Result<(), String>,
) -> Result<payload::Read, Refusal> {
    payload::read(std::io::BufReader::new(file), each).map_err(|unreadable| match unreadable {
        payload::Unreadable::Apply(why) => unavailable(why),
        payload::Unreadable::Format(found) => Refusal {
            code: "incompatible_version",
            message: format!("this build reads {FORMAT}, not {found}"),
            detail: Some(json!({ "found": found, "required": FORMAT })),
        },
        payload::Unreadable::Broken(why) => refuse(
            "participant_failed",
            format!("the stream is not one this build wrote: {why}"),
        ),
    })
}

fn read_range(path: &std::path::Path, offset: u64, length: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek};
    let mut file = std::fs::File::open(path)?;
    file.seek(std::io::SeekFrom::Start(offset))?;
    let mut bytes = vec![0; length as usize];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
pub(crate) mod tests;
