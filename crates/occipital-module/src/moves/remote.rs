//! What a move asks of other modules: Entorhinal, for the sessions a cut
//! moves, and Engram's session door, which carries the stream between
//! computers.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use subc_protocol::RouteTarget;

use super::payload::Session;
use crate::display::daemon::{CallFailure, DaemonCalls};

/// Engram's service that carries session streams between computers.
const DOOR_SERVICE: &str = "session-door";

/// How long a door call may take. `session.cover` uploads whatever the cloud
/// lacks, which can take minutes for a large stream.
pub(crate) const DOOR_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[async_trait]
pub(crate) trait Remote: Send + Sync {
    /// The session refs the move `cut_id` of `agent` carries, from the move's
    /// frozen plan, as `(ref_kind, ref)`.
    async fn move_refs(
        &self,
        cut_id: &str,
        agent: &str,
    ) -> Result<Vec<(String, String)>, CallFailure>;

    /// The session a ref is bound to, if it is bound to a whole one.
    async fn session_for_ref(
        &self,
        ref_kind: &str,
        reference: &str,
    ) -> Result<Option<Session>, CallFailure>;

    /// One call to the session door, answered with its `result`.
    async fn door(&self, method: &str, params: Value) -> Result<Value, CallFailure>;
}

/// [`Remote`] through the daemon, as this module.
pub(crate) struct DaemonRemote(pub Arc<DaemonCalls>);

impl DaemonRemote {
    async fn entorhinal(&self, method: &str, params: Value) -> Result<Value, CallFailure> {
        let reply = self
            .0
            .call(
                RouteTarget::ManagementSurface {
                    module_id: "entorhinal".into(),
                },
                "agent move",
                &json!({"method": method, "params": params}),
                None,
            )
            .await?;
        result(reply)
    }
}

#[async_trait]
impl Remote for DaemonRemote {
    async fn move_refs(
        &self,
        cut_id: &str,
        agent: &str,
    ) -> Result<Vec<(String, String)>, CallFailure> {
        let reply = self
            .entorhinal("agent.move_refs", json!({"cutId": cut_id}))
            .await?;
        move_refs(reply, cut_id, agent)
    }

    async fn session_for_ref(
        &self,
        ref_kind: &str,
        reference: &str,
    ) -> Result<Option<Session>, CallFailure> {
        let reply = self
            .entorhinal(
                "agent.session_for_ref",
                json!({"ref_kind": ref_kind, "ref": reference}),
            )
            .await;
        let reply = match reply {
            Err(error) if error.code.as_deref() == Some("not_found") => return Ok(None),
            other => other?,
        };
        let row: cortexkit_agent_identity::AgentSessionForRefReply = decode(reply)?;
        let runner = match row.runner {
            cortexkit_project_identity::Presence::Present(runner) => Some(runner),
            _ => None,
        };
        Ok(match (runner, row.harness, row.session_id) {
            (Some(runner), Some(harness), Some(session)) => Some(Session {
                runner,
                harness,
                session,
            }),
            // A placeholder binding names no session yet: there is nothing of
            // it here to carry.
            _ => None,
        })
    }

    async fn door(&self, method: &str, params: Value) -> Result<Value, CallFailure> {
        let reply = self
            .0
            .call_timed(
                RouteTarget::InternalService {
                    module_id: "engram".into(),
                    service_id: DOOR_SERVICE.into(),
                },
                "agent move",
                &json!({"method": method, "params": params}),
                DOOR_TIMEOUT,
            )
            .await?;
        result(reply)
    }
}

/// `agent.move_refs`'s `result`, checked to answer for this move of this
/// agent.
pub(crate) fn move_refs(
    result: Value,
    cut_id: &str,
    agent: &str,
) -> Result<Vec<(String, String)>, CallFailure> {
    #[derive(Deserialize)]
    struct Ref {
        ref_kind: String,
        #[serde(rename = "ref")]
        reference: String,
    }
    #[derive(Deserialize)]
    struct Reply {
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "cutId")]
        cut_id: String,
        refs: Vec<Ref>,
    }
    let reply: Reply = decode(result)?;
    if reply.cut_id != cut_id || reply.agent_id != agent {
        return Err(failure(
            "entorhinal answered for another move or another agent",
        ));
    }
    Ok(reply
        .refs
        .into_iter()
        .map(|r| (r.ref_kind, r.reference))
        .collect())
}

fn result(reply: Value) -> Result<Value, CallFailure> {
    match reply {
        Value::Object(mut object) => object
            .remove("result")
            .ok_or_else(|| failure("the reply has no result")),
        _ => Err(failure("the reply is not an object")),
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, CallFailure> {
    serde_json::from_value(value)
        .map_err(|error| failure(format!("the reply didn't decode: {error}")))
}

fn failure(message: impl Into<String>) -> CallFailure {
    CallFailure {
        code: None,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    const SOURCE: &str = include_str!("../../tests/data/entorhinal-move-refs-v1/source.json");
    const RECEIVER: &str = include_str!("../../tests/data/entorhinal-move-refs-v1/receiver.json");

    /// Entorhinal's published `agent.move_refs` replies (Entorhinal
    /// `agent-move-refs-v1`, copied byte for byte) decode to the refs a move
    /// carries, the same on both computers; a reply naming another move or
    /// another agent is refused.
    #[test]
    fn entorhinal_s_move_refs_samples_decode() {
        for (bytes, sum) in [
            (
                SOURCE,
                "c23eb0821edef9071b68c3dbade8e08de7c3410c3ace33a838036af1d28aa220",
            ),
            (
                RECEIVER,
                "c23eb0821edef9071b68c3dbade8e08de7c3410c3ace33a838036af1d28aa220",
            ),
        ] {
            assert_eq!(format!("{:x}", Sha256::digest(bytes.as_bytes())), sum);
            let records: Vec<Value> = serde_json::from_str(bytes).unwrap();
            let record = &records[0];
            assert_eq!(record["principal"]["moduleId"], "occipital");
            let cut = record["request"]["cutId"].as_str().unwrap();
            let reply = result(record["reply"].clone()).unwrap();
            assert_eq!(
                move_refs(reply.clone(), cut, "agent_0123456789abcdef").unwrap(),
                vec![
                    ("pi".into(), "p".into()),
                    ("session".into(), "a".into()),
                    ("session".into(), "z".into()),
                ]
            );
            assert!(move_refs(reply.clone(), cut, "agent_other").is_err());
            assert!(move_refs(reply, &"b".repeat(32), "agent_0123456789abcdef").is_err());
        }
    }
}
