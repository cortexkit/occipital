//! The person's commands (`user-commands/v1`): things a module lets the person
//! do from the apps directly, such as Magic Context's flush, with a live
//! preview of what each would do now.
//!
//! The apps ask Occipital, which lists every module that declares the
//! capability and relays preview and run to the module that owns the command.
//! The apps then need one grant and one reply shape, not one per module. The
//! module decides, confirms and records what runs; Occipital never runs
//! anything itself.
//!
//! A module admits these calls only from `reserved:occipital`, so it can't see
//! which app asked. Occipital checks the app's route itself and forwards what
//! it verified as `caller` facts (the app, its admission, the phone's device),
//! which the module records beside Occipital's principal and never treats as
//! authority. The wire shapes are the `cortexkit-role-user-commands` crate's.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cortexkit_role_user_commands as contract;
use occipital_app_reader::source::ReadError;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

pub const LIST: &str = "commands.list";
pub const PREVIEW: &str = "commands.preview";
pub const RUN: &str = "commands.run";
/// The apps' three operations: two reads and the run.
pub const METHODS: [&str; 3] = [LIST, PREVIEW, RUN];

/// How often the daemon's catalog is read again for the modules that declare
/// commands, and their builds.
const CATALOG_CHECK: Duration = Duration::from_secs(10);
/// How long a module may take to describe its commands.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a module may take to preview a command. A preview is a local read,
/// so this is generous.
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a run may take. A module waiting on its own approval (an operator
/// confirmation or an approval card) answers within five minutes; this leaves
/// room for the answer to arrive.
const RUN_TIMEOUT: Duration = Duration::from_secs(330);

/// Why a call to a module failed.
#[derive(Debug)]
pub enum Failure {
    /// The module never received it, so asking again is safe.
    NotSent(String),
    /// It was sent, and no answer came: the module may have acted.
    Unknown(String),
    /// The module refused it.
    Refused(contract::ErrorBody),
}

/// The modules that serve commands, and calls to them. The daemon serves them
/// in production ([`DaemonModules`]); tests use a fake.
#[async_trait]
pub trait Modules: Send + Sync {
    /// Every ready module whose manifest provides `user-commands/v1`, with its
    /// build (its `module_version`, plus `+<commit>` when known).
    async fn declaring(&self) -> Result<BTreeMap<String, String>, String>;

    /// `method` with `params` on `module`'s management surface, within
    /// `timeout`. A refusal, whether an error frame or a reply body carrying
    /// `code`, is [`Failure::Refused`].
    async fn call(
        &self,
        module: &str,
        method: &str,
        params: &Value,
        timeout: Duration,
    ) -> Result<Value, Failure>;
}

pub struct Commands {
    modules: Arc<dyn Modules>,
    state: std::sync::Mutex<State>,
}

#[derive(Default)]
struct State {
    checked: Option<Instant>,
    declaring: BTreeMap<String, String>,
    /// Each module's commands, under the build that described them: a new
    /// build is asked again, since a module only changes its commands when it
    /// is placed again.
    described: HashMap<String, (String, Arc<Vec<contract::Command>>)>,
}

impl Commands {
    pub fn new(modules: Arc<dyn Modules>) -> Self {
        Self {
            modules,
            state: std::sync::Mutex::default(),
        }
    }

    /// `commands.list {subject}`: every declaring module's commands for the
    /// subject's kind, each named by its module. A module that can't describe
    /// itself is left out, so one broken module never hides the others.
    pub async fn list(&self, params: Value) -> Result<Value, ReadError> {
        let request: contract::ListRequest = decode(params)?;
        request.check().map_err(invalid)?;
        let declaring = self.declaring().await;
        let described =
            futures_util::future::join_all(declaring.iter().map(|(module, build)| async move {
                (module, self.described(module, build).await)
            }))
            .await;
        let kind = request.subject.kind();
        let mut listed = Vec::new();
        for (module, commands) in described {
            match commands {
                Ok(commands) => listed.extend(
                    commands
                        .iter()
                        .filter(|command| command.subject == kind)
                        .map(|command| {
                            contract::ListedCommand::new(module.clone(), command.clone())
                        }),
                ),
                Err(error) => {
                    tracing::warn!(module = %module, %error, "a module's commands were left out of the list");
                }
            }
        }
        encode(&contract::ListReply::new(listed))
    }

    /// `commands.preview`: what the command would do now, from its module.
    /// The module's answer is passed on as it came once it reads as a
    /// preview; a fact in it the rules refuse is the app's to show as missing.
    pub async fn preview(
        &self,
        caller: contract::Caller,
        params: Value,
    ) -> Result<Value, ReadError> {
        let request: contract::AppPreviewRequest = decode(params)?;
        let command = self.command(&request.module, &request.id).await?;
        request.check(&command).map_err(invalid)?;
        let forwarded =
            contract::ModulePreviewRequest::new(request.id, request.subject, request.args, caller);
        let reply = self
            .modules
            .call(
                &request.module,
                PREVIEW,
                &encode(&forwarded)?,
                PREVIEW_TIMEOUT,
            )
            .await
            // A preview changes nothing, so an unanswered one is safe to ask again.
            .map_err(|failure| failed(&request.module, PREVIEW, failure, false))?;
        checked_reply::<contract::Preview>(&request.module, PREVIEW, reply)
    }

    /// `commands.run`: the person's run, under the app's `request_id`. It is
    /// never sent twice: an unanswered run is reported as such, and the app
    /// asks again under the same `request_id`, which the module answers from
    /// its stored reply.
    pub async fn run(&self, caller: contract::Caller, params: Value) -> Result<Value, ReadError> {
        let request: contract::AppRunRequest = decode(params)?;
        let command = self.command(&request.module, &request.id).await?;
        request.check(&command).map_err(invalid)?;
        let forwarded = contract::ModuleRunRequest::new(
            contract::ModulePreviewRequest::new(request.id, request.subject, request.args, caller),
            request.preview_version,
            request.request_id,
        );
        let reply = self
            .modules
            .call(&request.module, RUN, &encode(&forwarded)?, RUN_TIMEOUT)
            .await
            .map_err(|failure| failed(&request.module, RUN, failure, true))?;
        checked_reply::<contract::RunReply>(&request.module, RUN, reply)
    }

    /// The command `id` of `module`, from its current description.
    async fn command(&self, module: &str, id: &str) -> Result<contract::Command, ReadError> {
        let unknown = || {
            ReadError::new(
                "unknown_command",
                format!("{module} declares no command {id:?}"),
            )
        };
        let declaring = self.declaring().await;
        let Some(build) = declaring.get(module) else {
            return Err(unknown());
        };
        let commands = self.described(module, build).await.map_err(|error| {
            ReadError::new(
                "command_unavailable",
                format!("{module} can't describe its commands: {error}"),
            )
        })?;
        commands
            .iter()
            .find(|command| command.id == id)
            .cloned()
            .ok_or_else(unknown)
    }

    /// The declaring modules, read again when the last read is older than
    /// [`CATALOG_CHECK`]. A failed read keeps the modules read before.
    async fn declaring(&self) -> BTreeMap<String, String> {
        {
            let state = self.lock();
            if state
                .checked
                .is_some_and(|checked| checked.elapsed() < CATALOG_CHECK)
            {
                return state.declaring.clone();
            }
        }
        let read = self.modules.declaring().await;
        let mut state = self.lock();
        state.checked = Some(Instant::now());
        match read {
            Ok(declaring) => {
                state
                    .described
                    .retain(|module, _| declaring.contains_key(module));
                state.declaring = declaring;
            }
            Err(error) => tracing::debug!(%error, "catalog.list for command modules failed"),
        }
        state.declaring.clone()
    }

    /// `module`'s commands under `build`, asked for when none are kept under
    /// it. A description whose digest doesn't match its commands, or that
    /// isn't JSON, isn't kept, so the next read asks again.
    ///
    /// Within a description that matches its digest, each command is read on
    /// its own: one this build can't decode (a subject kind or arg kind added
    /// to the contract after it) or that fails its checks is left out and
    /// logged, and the module's other commands stay listed. A preview or run
    /// of a left-out command is refused `unknown_command`, because the relay
    /// only relays what it has read. Two commands sharing an id are both left
    /// out, so a preview never reaches the wrong one.
    async fn described(
        &self,
        module: &str,
        build: &str,
    ) -> Result<Arc<Vec<contract::Command>>, String> {
        if let Some((kept_build, commands)) = self.lock().described.get(module) {
            if kept_build == build {
                return Ok(commands.clone());
            }
        }
        let reply = self
            .modules
            .call(module, "commands.describe", &json!({}), DESCRIBE_TIMEOUT)
            .await
            .map_err(|failure| match failure {
                Failure::NotSent(error) | Failure::Unknown(error) => error,
                Failure::Refused(body) => format!("{}: {}", body.code, body.message),
            })?;
        let bytes = serde_json::to_vec(&reply).map_err(|error| format!("describe: {error}"))?;
        let described = contract::DescribeReply::decode_tolerant(&bytes)
            .map_err(|error| format!("describe: {error}"))?;
        for skipped in &described.unreadable {
            tracing::warn!(module = %module, index = skipped.index, id = ?skipped.id, error = %skipped.error, "a command this build can't read was left out");
        }
        let mut readable = Vec::new();
        for (index, command) in described.commands.into_iter().enumerate() {
            match command.check() {
                Ok(()) => readable.push(command),
                Err(error) => {
                    tracing::warn!(module = %module, index, id = %command.id, %error, "a command that fails its checks was left out");
                }
            }
        }
        let mut seen = HashMap::<String, usize>::new();
        for command in &readable {
            *seen.entry(command.id.clone()).or_default() += 1;
        }
        readable.retain(|command| {
            let unique = seen.get(&command.id) == Some(&1);
            if !unique {
                tracing::warn!(module = %module, id = %command.id, "two commands share an id; both were left out");
            }
            unique
        });
        let commands = Arc::new(readable);
        self.lock()
            .described
            .insert(module.to_owned(), (build.to_owned(), commands.clone()));
        Ok(commands)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// An app request's params as `T`. Any shape problem is `invalid_params`.
fn decode<T: DeserializeOwned>(params: Value) -> Result<T, ReadError> {
    serde_json::from_value(params).map_err(|error| {
        ReadError::new("invalid_params", format!("command params: {error}"))
            .with_detail(json!({"field": "params"}))
    })
}

fn encode(value: &impl serde::Serialize) -> Result<Value, ReadError> {
    serde_json::to_value(value).map_err(|error| {
        ReadError::new(
            "internal",
            format!("cannot encode a command message: {error}"),
        )
    })
}

/// A field the contract's checks refused, as the app sees it.
fn invalid(error: contract::ValidationError) -> ReadError {
    ReadError::new(
        "invalid_params",
        format!("invalid command params: {}", error.field),
    )
    .with_detail(json!({"field": error.field}))
}

/// A module's reply, passed on as it came once it reads as `T`.
fn checked_reply<T: DeserializeOwned>(
    module: &str,
    method: &str,
    reply: Value,
) -> Result<Value, ReadError> {
    match serde_json::from_value::<T>(reply.clone()) {
        Ok(_) => Ok(reply),
        Err(error) => Err(ReadError::new(
            "command_reply_invalid",
            format!("{module} answered {method} in a shape the apps can't read: {error}"),
        )),
    }
}

/// A failed call to a module, as the app sees it. Unsent calls and, for a
/// read, unanswered ones are `command_unavailable`, which the app may ask
/// again. An unanswered run is `command_outcome_unknown`: it may have run, so
/// the app asks again only under the same `request_id`. The contract's
/// refusal codes pass through, `unavailable` among them: the module couldn't
/// take the call now and nothing ran, so the app may repeat it, a run under
/// the same `request_id`. Any other refusal is `command_refused`, with the
/// module's code in `detail.moduleCode`.
fn failed(module: &str, method: &str, failure: Failure, mutation: bool) -> ReadError {
    match failure {
        Failure::NotSent(error) => ReadError::new(
            "command_unavailable",
            format!("{module} didn't receive {method}: {error}"),
        ),
        Failure::Unknown(error) if mutation => ReadError::new(
            "command_outcome_unknown",
            format!("{module} didn't answer {method}, which may have run: {error}"),
        ),
        Failure::Unknown(error) => ReadError::new(
            "command_unavailable",
            format!("{module} didn't answer {method}: {error}"),
        ),
        Failure::Refused(body) => {
            let code = match body.code.as_str() {
                contract::INVALID_PARAMS => "invalid_params",
                contract::UNKNOWN_COMMAND => "unknown_command",
                contract::UNKNOWN_SUBJECT => "unknown_subject",
                contract::CALLER_NOT_ADMITTED => "caller_not_admitted",
                // Added to the contract's refusals after 0.1.0, so the crate
                // has no constant for it yet.
                "unavailable" => "unavailable",
                _ => "command_refused",
            };
            let mut error = ReadError::new(
                code,
                format!("{module} refused {method}: {}: {}", body.code, body.message),
            );
            if code == "command_refused" {
                error = error.with_detail(json!({"moduleCode": body.code}));
            } else if let Some(detail) = body.detail {
                error = error.with_detail(detail);
            }
            error
        }
    }
}

/// The daemon's catalog and module calls, as this module, on its one
/// connection for calling other modules.
pub struct DaemonModules {
    calls: Arc<crate::display::daemon::DaemonCalls>,
}

impl DaemonModules {
    pub fn new(calls: Arc<crate::display::daemon::DaemonCalls>) -> Self {
        Self { calls }
    }
}

#[async_trait]
impl Modules for DaemonModules {
    async fn declaring(&self) -> Result<BTreeMap<String, String>, String> {
        let consumer = self
            .calls
            .consumer()
            .await
            .map_err(|failure| format!("catalog.list: {failure}"))?;
        let listed = match consumer.catalog_list().await {
            Ok(listed) => listed,
            Err(error) => {
                self.calls.failed(error.code()).await;
                return Err(format!("catalog.list: {error}"));
            }
        };
        Ok(listed
            .modules
            .into_iter()
            .filter(|module| {
                module.ready
                    && module.capabilities.as_ref().is_some_and(|declared| {
                        declared
                            .provides
                            .iter()
                            .any(|capability| capability == contract::PROVIDES)
                    })
            })
            .map(|module| {
                let version = module.module_version.unwrap_or_default();
                let build = match module
                    .provenance
                    .and_then(|provenance| provenance.build_git_sha)
                {
                    Some(commit) if !commit.is_empty() => format!("{version}+{commit}"),
                    _ => version,
                };
                (module.module_id, build)
            })
            .collect())
    }

    async fn call(
        &self,
        module: &str,
        method: &str,
        params: &Value,
        timeout: Duration,
    ) -> Result<Value, Failure> {
        let consumer = self
            .calls
            .consumer()
            .await
            .map_err(|failure| Failure::NotSent(failure.to_string()))?;
        let defaults = subc_client_rs::CallOptions::default();
        let called = consumer
            .call(
                subc_protocol::RouteTarget::ManagementSurface {
                    module_id: module.to_owned(),
                },
                subc_protocol::BindIdentity::new(crate::MODULE_ID, "commands"),
                json!({"method": method, "params": params})
                    .to_string()
                    .into_bytes(),
                subc_client_rs::CallOptions {
                    consumer_identity: Some(self.calls.identity()),
                    timeout,
                    route_retry_deadline: defaults.route_retry_deadline.min(timeout),
                    ..defaults
                },
            )
            .await;
        let bytes = match called {
            Ok(bytes) => bytes,
            Err(subc_client_rs::CallError::Module(body)) => return Err(Failure::Refused(body)),
            Err(error) => {
                self.calls.failed(error.code()).await;
                let sent = matches!(error, subc_client_rs::CallError::OutcomeUnknown(_))
                    && error.never_forwarded() != Some(true);
                return Err(if sent {
                    Failure::Unknown(error.to_string())
                } else {
                    Failure::NotSent(error.to_string())
                });
            }
        };
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| Failure::Unknown(format!("the reply didn't parse: {error}")))?;
        if let Some(code) = value.get("code").and_then(Value::as_str) {
            let mut body =
                contract::ErrorBody::new(code, value["message"].as_str().unwrap_or_default());
            if let Some(detail) = value.get("detail").filter(|detail| !detail.is_null()) {
                body = body.with_detail(detail.clone());
            }
            return Err(Failure::Refused(body));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests;
