//! Resolve runner/session references without deriving a project's bind identity.

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use cortexkit_agent_identity::{
    AgentSessionForHarnessParams, AgentSessionForHarnessReply, AgentSessionForRefParams,
    AgentSessionForRefReply, AgentSessionForRunnerParams, AgentSessionForRunnerReply,
    SessionNetwork, AGENT_SESSION_FOR_HARNESS,
};
use cortexkit_project_identity::{Outcome, Presence, Refusal, RefusalCode};
use occipital_app_reader::source::ReadError;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use subc_client_rs::consumer::{CallOptions, SubcConsumer};
use subc_client_rs::ConsumerIdentity;
use subc_protocol::{BindIdentity, RouteTarget};

pub const BROCA_RUNNER_MODULE_ID: &str = "broca";
/// Thalamus's runner, which hosts OpenCode sessions.
pub const OPENCODE_RUNNER_MODULE_ID: &str = "thalamus-harness";
/// Thalamus's runner for Pi sessions: the same program as the OpenCode one,
/// registered as its own module.
pub const PI_RUNNER_MODULE_ID: &str = "thalamus-pi";

/// The harness of the sessions `runner` hosts, for every runner this reader
/// serves; `None` for any other.
pub fn served_harness(runner: &str) -> Option<&'static str> {
    match runner {
        BROCA_RUNNER_MODULE_ID => Some("broca"),
        OPENCODE_RUNNER_MODULE_ID => Some("opencode"),
        PI_RUNNER_MODULE_ID => Some("pi"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionRef {
    pub runner: String,
    pub session: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Yes,
    No,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NetworkChange {
    pub network: Network,
    pub since_ms: i64,
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub reference: SessionRef,
    pub harness: String,
    pub project_root: PathBuf,
    pub network: Network,
    pub network_history: Vec<NetworkChange>,
}

impl Resolved {
    /// The identity a route to the session's runner opens with. Runners find an
    /// existing session by harness and session alone, so it names no project.
    pub fn identity(&self) -> BindIdentity {
        BindIdentity::new(&self.harness, &self.reference.session)
    }

    pub fn network_at(&self, written_at: Option<i64>) -> Network {
        let Some(at) = written_at else {
            return Network::Unknown;
        };
        // A current value alone cannot establish what was allowed when a row was written.
        if self
            .network_history
            .windows(2)
            .any(|pair| pair[0].since_ms >= pair[1].since_ms)
        {
            return Network::Unknown;
        }
        self.network_history
            .iter()
            .rev()
            .find(|change| change.since_ms <= at)
            .map_or(Network::Unknown, |change| change.network)
    }
}

#[async_trait]
pub trait Entorhinal: Send + Sync {
    /// Return the published result/refusal envelope, not only its `result` field.
    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError>;
}

pub struct DaemonEntorhinal {
    pub consumer: Arc<SubcConsumer>,
    pub identity: BindIdentity,
    pub principal: ConsumerIdentity,
}

#[async_trait]
impl Entorhinal for DaemonEntorhinal {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        self.request_with_timeout(method, params, CallOptions::default().timeout)
            .await
    }
}

impl DaemonEntorhinal {
    pub(crate) async fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: std::time::Duration,
    ) -> Result<Value, ReadError> {
        let bytes = self
            .consumer
            .call(
                RouteTarget::ManagementSurface {
                    module_id: "entorhinal".into(),
                },
                self.identity.clone(),
                json!({"method": method, "params": params})
                    .to_string()
                    .into_bytes(),
                CallOptions {
                    consumer_identity: Some(self.principal.clone()),
                    timeout,
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| {
                if error.code() == Some("not_found") {
                    ReadError::new("not_found", error.to_string())
                } else {
                    ReadError::new("provider_unavailable", format!("entorhinal: {error}"))
                }
            });
        match bytes {
            Err(error) if error.code == "not_found" => {
                encode(&Refusal::new(RefusalCode::NotFound, error.message, None))
            }
            Err(error) => Err(error),
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| invalid(format!("entorhinal reply: {error}"))),
        }
    }
}

/// Resolution's local view, decoded through the published runner-lookup reply.
#[derive(Deserialize)]
#[serde(try_from = "AgentSessionForRunnerReply")]
struct RunnerBinding {
    runner: String,
    session: String,
    harness: String,
    project_root: Option<PathBuf>,
    network: Network,
    network_history: Vec<NetworkChange>,
    held_on: Option<String>,
}

impl TryFrom<AgentSessionForRunnerReply> for RunnerBinding {
    type Error = &'static str;

    fn try_from(reply: AgentSessionForRunnerReply) -> Result<Self, Self::Error> {
        Ok(Self {
            runner: reply.runner,
            session: reply.session,
            harness: reply.harness.ok_or("session binding has no harness")?,
            project_root: reply.project_root.map(PathBuf::from),
            network: reply.network.into(),
            network_history: reply
                .network_history
                .into_iter()
                .map(|entry| NetworkChange {
                    network: entry.network.into(),
                    since_ms: entry.since_ms,
                })
                .collect(),
            held_on: present(reply.held_on),
        })
    }
}

/// `agent.session_for_harness`'s row: the runner Entorhinal records (`None` for a
/// session no runner hosts) and the session's binding.
#[derive(Deserialize)]
#[serde(try_from = "AgentSessionForHarnessReply")]
struct HarnessBinding {
    runner: Option<String>,
    binding: RunnerBinding,
}

impl TryFrom<AgentSessionForHarnessReply> for HarnessBinding {
    type Error = &'static str;

    fn try_from(reply: AgentSessionForHarnessReply) -> Result<Self, Self::Error> {
        let runner = reply.runner.filter(|runner| !runner.is_empty());
        Ok(Self {
            binding: RunnerBinding {
                runner: runner.clone().unwrap_or_default(),
                session: reply.session,
                harness: reply.harness.ok_or("session binding has no harness")?,
                project_root: reply.project_root.map(PathBuf::from),
                network: reply.network.into(),
                network_history: reply
                    .network_history
                    .into_iter()
                    .map(|entry| NetworkChange {
                        network: entry.network.into(),
                        since_ms: entry.since_ms,
                    })
                    .collect(),
                held_on: present(reply.held_on),
            },
            runner,
        })
    }
}

impl From<SessionNetwork> for Network {
    fn from(network: SessionNetwork) -> Self {
        match network {
            SessionNetwork::Yes => Self::Yes,
            SessionNetwork::No => Self::No,
            _ => Self::Unknown,
        }
    }
}

fn present<T>(value: Presence<T>) -> Option<T> {
    match value {
        Presence::Present(value) => Some(value),
        _ => None,
    }
}

#[derive(Deserialize)]
struct AgentRef {
    agent_id: String,
    ref_kind: String,
    #[serde(rename = "ref")]
    reference: String,
}

/// The legacy lookup binding carries the agent, reference, harness, and session
/// values that the resolver checks against the earlier lookup.
#[derive(Deserialize)]
#[serde(try_from = "AgentSessionForRefReply")]
struct RefBinding {
    agent_id: String,
    ref_kind: String,
    reference: String,
    harness: String,
    session_id: String,
    project_root: Option<PathBuf>,
    held_on: Option<String>,
}

impl TryFrom<AgentSessionForRefReply> for RefBinding {
    type Error = &'static str;

    fn try_from(reply: AgentSessionForRefReply) -> Result<Self, Self::Error> {
        Ok(Self {
            agent_id: reply.agent_id,
            ref_kind: reply.ref_kind,
            reference: reply.session_ref,
            harness: reply.harness.ok_or("session binding has no harness")?,
            session_id: reply
                .session_id
                .ok_or("session binding has no session_id")?,
            project_root: reply.project_root.map(PathBuf::from),
            held_on: present(reply.held_on),
        })
    }
}

pub struct Resolver {
    lookup: Arc<dyn Entorhinal>,
}

impl Resolver {
    pub fn new(lookup: Arc<dyn Entorhinal>) -> Self {
        Self { lookup }
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        super::observe::remote("entorhinal", method, self.lookup.request(method, params)).await
    }

    pub async fn resolve(&self, reference: &SessionRef) -> Result<Resolved, ReadError> {
        if reference.runner.is_empty() || reference.session.is_empty() {
            return Err(unknown(reference));
        }
        let mut primary = self
            .request(
                "agent.session_for_runner",
                encode(&AgentSessionForRunnerParams::new(
                    reference.runner.clone(),
                    reference.session.clone(),
                ))?,
            )
            .await?;
        // Older registrations omitted network metadata; absence still means unknown, not local.
        if let Some(result) = primary.get_mut("result").and_then(Value::as_object_mut) {
            result.entry("network").or_insert_with(|| json!("unknown"));
            result.entry("network_history").or_insert_with(|| json!([]));
        }
        if let Some(binding) = decode::<RunnerBinding>(primary)? {
            if binding.runner != reference.runner || binding.session != reference.session {
                return Err(invalid("entorhinal returned a different runner/session"));
            }
            return resolved(
                reference.clone(),
                binding.harness,
                binding.project_root,
                binding.network,
                binding.network_history,
                binding.held_on,
            );
        }

        // Older registrations omitted runner. They may resolve only through the recorded ref.
        let harness = "broca";
        let session = reference.session.as_str();
        let legacy = self
            .request(
                "agent.for_session",
                json!({
                    "harness": harness, "session_id": session,
                }),
            )
            .await?;
        if let Some(agent) = decode::<AgentRef>(legacy)? {
            let reply = self
                .request(
                    "agent.session_for_ref",
                    encode(&AgentSessionForRefParams::new(
                        agent.ref_kind.clone(),
                        agent.reference.clone(),
                    ))?,
                )
                .await?;
            if let Some(binding) = decode::<RefBinding>(reply)? {
                if binding.agent_id != agent.agent_id
                    || binding.ref_kind != agent.ref_kind
                    || binding.reference != agent.reference
                    || binding.harness != harness
                    || binding.session_id != session
                {
                    return Err(invalid(
                        "entorhinal legacy session reference changed during resolution",
                    ));
                }
                return resolved(
                    reference.clone(),
                    binding.harness,
                    binding.project_root,
                    Network::Unknown,
                    Vec::new(),
                    binding.held_on,
                );
            }
        }
        Err(unknown(reference))
    }

    /// The project folder of a session found by its harness and session id, whatever
    /// runner hosts it or none (Entorhinal's `agent.session_for_harness`), for placing
    /// uploads into a head no runner this module reaches hosts. A session held on another
    /// machine is refused `session_on_other_machine`, and one Entorhinal doesn't know is
    /// `session_unknown` naming no runner.
    pub async fn project_root_by_harness(
        &self,
        harness: &str,
        session: &str,
    ) -> Result<PathBuf, ReadError> {
        let Some((_, binding)) = self.by_harness(harness, session).await? else {
            return Err(unknown_by_harness(harness, session));
        };
        placement(
            binding.held_on,
            binding.project_root,
            format_args!("session {session} under harness {harness}"),
        )
    }

    /// The runner Entorhinal records for a session found by harness and session id:
    /// `None` for a session no runner hosts, and `session_unknown` naming no runner for
    /// one Entorhinal doesn't know.
    pub async fn runner_by_harness(
        &self,
        harness: &str,
        session: &str,
    ) -> Result<Option<String>, ReadError> {
        match self.by_harness(harness, session).await? {
            Some((runner, _)) => Ok(runner),
            None => Err(unknown_by_harness(harness, session)),
        }
    }

    async fn by_harness(
        &self,
        harness: &str,
        session: &str,
    ) -> Result<Option<(Option<String>, RunnerBinding)>, ReadError> {
        if harness.is_empty() || session.is_empty() {
            return Ok(None);
        }
        let params = AgentSessionForHarnessParams::new(harness.into(), session.into());
        let reply = self
            .request(AGENT_SESSION_FOR_HARNESS, encode(&params)?)
            .await?;
        let Some(HarnessBinding { runner, binding }) = decode::<HarnessBinding>(reply)? else {
            return Ok(None);
        };
        if binding.harness != harness || binding.session != session {
            return Err(invalid("entorhinal returned a different harness/session"));
        }
        Ok(Some((runner, binding)))
    }
}

/// A session Entorhinal doesn't know under this harness: `detail.runner` is null, since
/// no runner records it.
fn unknown_by_harness(harness: &str, session: &str) -> ReadError {
    ReadError::new(
        "session_unknown",
        format!("no session {session} under harness {harness}"),
    )
    .with_detail(json!({"runner": null}))
}

/// A session's project folder, unless its data is held on another machine.
fn placement(
    held_on: Option<String>,
    root: Option<PathBuf>,
    what: std::fmt::Arguments<'_>,
) -> Result<PathBuf, ReadError> {
    // Opening the same name locally would expose an empty history or start a new conversation.
    if let Some(machine_id) = held_on {
        return Err(ReadError::new(
            "session_on_other_machine",
            "session data is held on another machine",
        )
        .with_detail(json!({"machine_id": machine_id})));
    }
    root.filter(|root| root.is_absolute())
        .ok_or_else(|| ReadError::new("session_unknown", format!("{what} has no project_root")))
}

fn resolved(
    reference: SessionRef,
    harness: String,
    root: Option<PathBuf>,
    network: Network,
    network_history: Vec<NetworkChange>,
    held_on: Option<String>,
) -> Result<Resolved, ReadError> {
    let project_root = placement(
        held_on,
        root,
        format_args!(
            "session {} on runner {}",
            reference.session, reference.runner
        ),
    )?;
    // Each runner hosts one harness's sessions; a runner this reader doesn't
    // know is read as Broca's, which the harness check then refuses.
    let expected = served_harness(&reference.runner).unwrap_or("broca");
    if harness != expected {
        return Err(invalid(format!(
            "session {} on runner {} is not a {expected} session",
            reference.session, reference.runner
        )));
    }
    Ok(Resolved {
        reference,
        harness,
        project_root,
        network,
        network_history,
    })
}

/// A session Entorhinal doesn't know under the runner the phone named. Broca sessions
/// never change runner, so `detail.runner` is null: no runner records it.
fn unknown(reference: &SessionRef) -> ReadError {
    ReadError::new(
        "session_unknown",
        format!(
            "no session {} on runner {}",
            reference.session, reference.runner
        ),
    )
    .with_detail(json!({"runner": null}))
}

fn invalid(message: impl Into<String>) -> ReadError {
    ReadError::new("invalid_provider_reply", message)
}

/// An Entorhinal request or refusal as JSON. These published types always
/// serialize; a failure is answered as an error rather than a panic all the same.
fn encode(value: &impl Serialize) -> Result<Value, ReadError> {
    serde_json::to_value(value).map_err(|error| {
        ReadError::new(
            "internal",
            format!("cannot encode an Entorhinal value: {error}"),
        )
    })
}

fn decode<T: DeserializeOwned>(mut reply: Value) -> Result<Option<T>, ReadError> {
    // Keep malformed placement distinct from an unsupported harness in the send adapter.
    if let Some(held_on) = reply["result"]
        .get("heldOn")
        .filter(|value| !value.is_null())
    {
        if !held_on
            .as_str()
            .is_some_and(|machine_id| !machine_id.trim().is_empty())
        {
            return Err(invalid("entorhinal heldOn must name a machine")
                .with_detail(json!({"field":"heldOn"})));
        }
    }
    // Older replies used null for local placement; the typed contract represents it by omission.
    if reply["result"]["heldOn"].is_null() {
        if let Some(result) = reply.get_mut("result").and_then(Value::as_object_mut) {
            result.remove("heldOn");
        }
    }
    match serde_json::from_value::<Outcome<T>>(reply.clone())
        .map_err(|error| invalid(format!("entorhinal result: {error}")))?
    {
        Outcome::Success(success) => Ok(Some(success.result)),
        Outcome::Refusal(refusal) if refusal.code == RefusalCode::NotFound => Ok(None),
        _ => Err(invalid(format!(
            "entorhinal refusal or malformed reply: {reply}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{collections::VecDeque, sync::Mutex};

    const RUNNER: &str =
        include_str!("../../tests/fixtures/entorhinal-agent-sessions-v1/session-for-runner.json");
    const SESSION: &str =
        include_str!("../../tests/fixtures/entorhinal-agent-sessions-v1/for-session.json");
    const REFERENCE: &str =
        include_str!("../../tests/fixtures/entorhinal-agent-sessions-v1/session-for-ref.json");
    const HARNESS: &str =
        include_str!("../../tests/fixtures/entorhinal-agent-sessions-v1/session-for-harness.json");

    /// Entorhinal 0.1.49's `agent.session_for_harness` sample, served as published: a
    /// head no runner hosts places its upload in its project folder, a session held
    /// elsewhere is refused by name, and an unknown one is `session_unknown` naming no
    /// runner.
    #[tokio::test]
    async fn uploads_find_a_session_by_harness_whatever_runner_hosts_it() {
        assert_eq!(
            format!("{:x}", Sha256::digest(HARNESS.as_bytes())),
            "32ec4b73a8571b8d534b76c4858d4d0190063c8343fe5aea5f2630e50a2c9544"
        );
        let records: Vec<Value> = serde_json::from_str(HARNESS).unwrap();
        let resolver = |record: &Value| {
            Resolver::new(FakeEntorhinal::new(vec![(
                "agent.session_for_harness",
                record["request"]["params"].clone(),
                record["reply"].clone(),
            )]))
        };
        let params = |record: &Value| {
            let p = &record["request"]["params"];
            (
                p["harness"].as_str().unwrap().to_owned(),
                p["session"].as_str().unwrap().to_owned(),
            )
        };
        // A Broca worker, then a head with `runner: null`: both place into the project.
        for record in &records[..2] {
            let (harness, session) = params(record);
            assert_eq!(
                resolver(record)
                    .project_root_by_harness(&harness, &session)
                    .await
                    .unwrap(),
                PathBuf::from("/fixture/project")
            );
        }
        assert!(records[1]["reply"]["result"]["runner"].is_null());
        let (harness, session) = params(&records[2]);
        let held = resolver(&records[2])
            .project_root_by_harness(&harness, &session)
            .await
            .unwrap_err();
        assert_eq!(held.code, "session_on_other_machine");
        assert_eq!(held.detail.unwrap()["machine_id"], "source-machine");
        let (harness, session) = params(&records[3]);
        let unknown = resolver(&records[3])
            .project_root_by_harness(&harness, &session)
            .await
            .unwrap_err();
        assert_eq!(unknown.code, "session_unknown");
        assert_eq!(unknown.detail.unwrap(), json!({"runner": null}));
    }

    #[test]
    fn published_placement_samples_keep_their_checksums() {
        for (bytes, checksum) in [
            (
                include_str!(
                    "../../tests/fixtures/entorhinal-agent-sessions-v1/sessions-held-on.json"
                ),
                "17df474000ecbd7c148a30c358b101cb947919073159f7f2941fff369b19e174",
            ),
            (
                include_str!(
                    "../../tests/fixtures/entorhinal-agent-sessions-v1/sessions-open.json"
                ),
                "01d785e934526b457d9f958ac94d8a0829526729e0f418e114ea8faab3a86af7",
            ),
        ] {
            assert_eq!(format!("{:x}", Sha256::digest(bytes.as_bytes())), checksum);
        }
    }

    #[test]
    fn published_lookup_samples_decode_and_keep_their_checksums() {
        for (bytes, checksum, method) in [
            (
                RUNNER,
                "b62878e6c91d98cc81ff0f993205edd4a88b5eaf49d649bab81feca02696203e",
                "agent.session_for_runner",
            ),
            (
                SESSION,
                "dd41914934a9b85b35670a815bef17de84752d8da7d05a1bf8833f3edbd1996b",
                "agent.for_session",
            ),
            (
                REFERENCE,
                "3e9e99950c167a2b2c5b3315fca35d353f50bc29e8ad93afe616c15e7cb703f6",
                "agent.session_for_ref",
            ),
        ] {
            assert_eq!(format!("{:x}", Sha256::digest(bytes.as_bytes())), checksum);
            let records: Vec<Value> = serde_json::from_str(bytes).unwrap();
            for record in records {
                assert_eq!(record["request"]["method"], method);
                let found = record["reply"].get("result").is_some();
                let decoded = match method {
                    "agent.session_for_runner" => decode::<RunnerBinding>(record["reply"].clone())
                        .unwrap()
                        .is_some(),
                    "agent.for_session" => decode::<AgentRef>(record["reply"].clone())
                        .unwrap()
                        .is_some(),
                    _ => decode::<RefBinding>(record["reply"].clone())
                        .unwrap()
                        .is_some(),
                };
                assert_eq!(decoded, found);
            }
        }
    }

    /// Entorhinal 0.1.47 adds the binding's recorded `runner` to `agent.session_for_ref`
    /// replies. The reader takes the harness and session from the reply as before, and a
    /// member it doesn't use must never make the lookup fail.
    #[test]
    fn reference_reply_naming_its_runner_still_decodes() {
        let records: Vec<Value> = serde_json::from_str(REFERENCE).unwrap();
        let with_runner = records
            .iter()
            .find(|record| record["reply"]["result"].get("runner").is_some())
            .expect("the published sample carries a reply naming its runner");
        let binding = decode::<RefBinding>(with_runner["reply"].clone())
            .unwrap()
            .expect("a reply naming its runner is still a binding");
        assert_eq!(binding.harness, "broca");
        assert_eq!(binding.session_id, "broca-session-001");
        assert_eq!(binding.reference, "44444444444444444444444444444444");
    }

    pub(super) struct FakeEntorhinal(Mutex<VecDeque<(String, Value, Value)>>);
    impl FakeEntorhinal {
        fn new(calls: Vec<(&str, Value, Value)>) -> Arc<Self> {
            Arc::new(Self(Mutex::new(
                calls
                    .into_iter()
                    .map(|(op, params, reply)| (op.into(), params, reply))
                    .collect(),
            )))
        }
    }
    #[async_trait]
    impl Entorhinal for FakeEntorhinal {
        async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
            let (expected, arguments, reply) = self
                .0
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected lookup");
            assert_eq!(method, expected);
            assert_eq!(params, arguments);
            Ok(reply)
        }
    }
    fn reference() -> SessionRef {
        SessionRef {
            runner: "broca".into(),
            session: "broca-live".into(),
        }
    }
    fn missing() -> Value {
        json!({"code":"not_found", "message":"not registered", "detail":null})
    }
    fn primary_params() -> Value {
        json!({"runner":"broca", "session":"broca-live"})
    }
    fn legacy_reply() -> Value {
        let mut records: Vec<Value> = serde_json::from_str(SESSION).unwrap();
        let mut reply = records.remove(0)["reply"].clone();
        reply["result"]["ref_kind"] = json!("broca-session");
        reply
    }
    fn legacy_params() -> Value {
        json!({"harness":"broca", "session_id":"broca-live"})
    }
    fn ref_params() -> Value {
        json!({"ref_kind":"broca-session", "ref":"0123456789abcdef0123456789abcdef"})
    }

    #[tokio::test]
    async fn runner_lookup_preserves_binding_and_network_history() {
        let records: Vec<Value> = serde_json::from_str(RUNNER).unwrap();
        let fake = FakeEntorhinal::new(vec![(
            "agent.session_for_runner",
            primary_params(),
            records[0]["reply"].clone(),
        )]);
        let resolved = Resolver::new(fake.clone())
            .resolve(&reference())
            .await
            .unwrap();
        assert_eq!(resolved.reference, reference());
        assert_eq!(resolved.harness, "broca");
        assert_eq!(
            resolved.project_root,
            PathBuf::from("/workspace/session-vectors")
        );
        assert_eq!(resolved.network, Network::No);
        assert_eq!(resolved.network_at(Some(699)), Network::Unknown);
        assert_eq!(resolved.network_at(Some(700)), Network::Yes);
        assert_eq!(resolved.network_at(Some(710)), Network::No);
        assert_eq!(resolved.network_at(None), Network::Unknown);
        assert!(fake.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn runner_lookup_without_network_metadata_keeps_unknown_defaults() {
        let records: Vec<Value> = serde_json::from_str(RUNNER).unwrap();
        let mut reply = records[0]["reply"].clone();
        let result = reply["result"].as_object_mut().unwrap();
        result.remove("network");
        result.remove("network_history");
        let fake = FakeEntorhinal::new(vec![("agent.session_for_runner", primary_params(), reply)]);
        let result = Resolver::new(fake).resolve(&reference()).await.unwrap();
        assert_eq!(result.network, Network::Unknown);
        assert!(result.network_history.is_empty());
    }

    #[tokio::test]
    async fn legacy_reference_lookup_supplies_root_without_guessing_network() {
        let records: Vec<Value> = serde_json::from_str(REFERENCE).unwrap();
        let mut reply = records[0]["reply"].clone();
        reply["result"]["harness"] = json!("broca");
        reply["result"]["ref_kind"] = json!("broca-session");
        reply["result"]["session_id"] = json!("broca-live");
        let fake = FakeEntorhinal::new(vec![
            ("agent.session_for_runner", primary_params(), missing()),
            ("agent.for_session", legacy_params(), legacy_reply()),
            ("agent.session_for_ref", ref_params(), reply),
        ]);
        let resolved = Resolver::new(fake.clone())
            .resolve(&reference())
            .await
            .unwrap();
        assert_eq!(
            resolved.project_root,
            PathBuf::from("/workspace/session-vectors/reopened")
        );
        assert_eq!(resolved.network, Network::Unknown);
        assert_eq!(resolved.network_at(Some(1000)), Network::Unknown);
        assert!(fake.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unresolved_session_is_refused_by_name() {
        let fake = FakeEntorhinal::new(vec![
            ("agent.session_for_runner", primary_params(), missing()),
            ("agent.for_session", legacy_params(), missing()),
        ]);
        let error = Resolver::new(fake).resolve(&reference()).await.unwrap_err();
        assert_eq!(error.code, "session_unknown");
        assert!(error.message.contains("broca-live") && error.message.contains("broca"));
    }

    #[tokio::test]
    async fn prefixed_legacy_session_is_looked_up_exactly_as_supplied() {
        let mut reference = reference();
        reference.session = "alfonso:broca-live".into();
        let records: Vec<Value> = serde_json::from_str(REFERENCE).unwrap();
        let mut reply = records[0]["reply"].clone();
        reply["result"]["harness"] = json!("broca");
        reply["result"]["ref_kind"] = json!("broca-session");
        reply["result"]["session_id"] = json!("alfonso:broca-live");
        let fake = FakeEntorhinal::new(vec![
            (
                "agent.session_for_runner",
                json!({"runner":"broca", "session":"alfonso:broca-live"}),
                missing(),
            ),
            (
                "agent.for_session",
                json!({"harness":"broca", "session_id":"alfonso:broca-live"}),
                legacy_reply(),
            ),
            ("agent.session_for_ref", ref_params(), reply),
        ]);
        let result = Resolver::new(fake).resolve(&reference).await.unwrap();
        assert_eq!(result.reference, reference);
    }

    #[tokio::test]
    async fn missing_ref_root_and_provider_refusals_never_fabricate_bind_identity() {
        for reply in [
            missing(),
            json!({"result":{
                "agent_id":"agent_0000000000000101", "ref_kind":"broca-session", "ref":"0123456789abcdef0123456789abcdef",
                "harness":"broca", "session_id":"broca-live", "project_root":null, "incarnation":"entorhinal-test",
            }}),
        ] {
            let fake = FakeEntorhinal::new(vec![
                ("agent.session_for_runner", primary_params(), missing()),
                ("agent.for_session", legacy_params(), legacy_reply()),
                ("agent.session_for_ref", ref_params(), reply),
            ]);
            assert_eq!(
                Resolver::new(fake)
                    .resolve(&reference())
                    .await
                    .unwrap_err()
                    .code,
                "session_unknown"
            );
        }
        let fake = FakeEntorhinal::new(vec![(
            "agent.session_for_runner",
            primary_params(),
            json!({"code":"caller_forbidden"}),
        )]);
        assert_eq!(
            Resolver::new(fake)
                .resolve(&reference())
                .await
                .unwrap_err()
                .code,
            "invalid_provider_reply"
        );
    }
}
