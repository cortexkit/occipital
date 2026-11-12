//! The relay against the contract's samples (`tests/data/user-commands-v1/`,
//! copied unchanged from the `cortexkit-role-user-commands` crate), through a
//! fake catalog and fake modules.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::*;
use crate::app_reader::{command_caller, RouteBinding};
use sha2::{Digest, Sha256};
use subc_protocol::{BindIdentity, Principal, RouteTarget};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/user-commands-v1")
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

/// The body named `name` in a samples file that lists `{name, body}` cases.
fn case(file: &str, name: &str) -> Value {
    sample(file)
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap_or_else(|| panic!("{file} has no case {name}"))["body"]
        .clone()
}

const CONTEXT: &str = "example-context";
const CODE: &str = "example-code-tools";

/// What a fake module answers for one method.
enum Answer {
    Reply(Value),
    NotSent,
    Unknown,
    Refused(&'static str),
}

#[derive(Default)]
struct Fake {
    declaring: Mutex<BTreeMap<String, String>>,
    answers: Mutex<HashMap<(String, String), Answer>>,
    calls: Mutex<Vec<(String, String, Value)>>,
}

impl Fake {
    fn answer(&self, module: &str, method: &str, answer: Answer) {
        self.answers
            .lock()
            .unwrap()
            .insert((module.into(), method.into()), answer);
    }

    fn calls(&self, method: &str) -> Vec<(String, Value)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, called, _)| called == method)
            .map(|(module, _, params)| (module.clone(), params.clone()))
            .collect()
    }
}

#[async_trait]
impl Modules for Fake {
    async fn declaring(&self) -> Result<BTreeMap<String, String>, String> {
        Ok(self.declaring.lock().unwrap().clone())
    }

    async fn call(
        &self,
        module: &str,
        method: &str,
        params: &Value,
        _: Duration,
    ) -> Result<Value, Failure> {
        self.calls
            .lock()
            .unwrap()
            .push((module.into(), method.into(), params.clone()));
        match self
            .answers
            .lock()
            .unwrap()
            .get(&(module.into(), method.into()))
        {
            Some(Answer::Reply(value)) => Ok(value.clone()),
            Some(Answer::NotSent) => Err(Failure::NotSent("route refused".into())),
            Some(Answer::Unknown) => Err(Failure::Unknown("connection lost".into())),
            Some(Answer::Refused(code)) => {
                Err(Failure::Refused(contract::ErrorBody::new(*code, "refused")))
            }
            None => Err(Failure::NotSent(format!("{module} serves no {method}"))),
        }
    }
}

/// Both sample modules declaring, each describing its commands from
/// `declarations.json` (the context module with the sample's own fixed
/// digest), and the given preview and run answers for the context module.
fn modules() -> Arc<Fake> {
    let fake = Arc::new(Fake::default());
    *fake.declaring.lock().unwrap() = BTreeMap::from([
        (CONTEXT.into(), "1.0.0+aaaa".into()),
        (CODE.into(), "2.0.0".into()),
    ]);
    fake.answer(
        CONTEXT,
        "commands.describe",
        Answer::Reply(sample("describe.json")),
    );
    let code_tools: Vec<contract::Command> =
        serde_json::from_value(sample("declarations.json")[CODE].clone()).unwrap();
    fake.answer(
        CODE,
        "commands.describe",
        Answer::Reply(
            serde_json::to_value(contract::DescribeReply::new(code_tools).unwrap()).unwrap(),
        ),
    );
    fake
}

fn caller(app: &str, admission: &str, device: Option<&str>) -> contract::Caller {
    let mut caller = contract::Caller::new(app, admission);
    caller.device = device.map(str::to_owned);
    caller
}

#[test]
fn the_contract_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    // Eight bodies and the README.
    assert_eq!(sums.lines().count(), 9);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// The list for a session merges every declaring module's session commands,
/// each named by its module: the sample's merged list, in module order.
#[tokio::test]
async fn a_session_list_merges_each_modules_commands() {
    let commands = Commands::new(modules());
    let listed = commands
        .list(sample("requests.json")["list"].clone())
        .await
        .unwrap();
    let mut expected = sample("list.json")["commands"].as_array().unwrap().clone();
    expected.sort_by_key(|entry| entry["module"].as_str().unwrap().to_owned());
    assert_eq!(listed["commands"], Value::Array(expected));
}

/// A subject of another kind lists none of the session commands.
#[tokio::test]
async fn the_list_holds_only_commands_for_the_subjects_kind() {
    let commands = Commands::new(modules());
    let listed = commands
        .list(json!({"subject": {"kind": "agent", "agent_id": "agent_1"}}))
        .await
        .unwrap();
    assert_eq!(listed, json!({"commands": []}));
}

/// A module whose description fails its checks, here a digest that doesn't
/// match its commands, is left out and asked again next time; the other
/// module's commands are still listed.
#[tokio::test]
async fn a_module_with_a_bad_description_is_left_out() {
    let fake = modules();
    let mut broken = sample("describe.json");
    broken["digest"] = json!("0".repeat(64));
    fake.answer(CONTEXT, "commands.describe", Answer::Reply(broken));
    let commands = Commands::new(fake.clone());
    let list = sample("requests.json")["list"].clone();
    let listed = commands.list(list.clone()).await.unwrap();
    let modules: Vec<&str> = listed["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["module"].as_str().unwrap())
        .collect();
    assert_eq!(modules, [CODE]);
    commands.list(list).await.unwrap();
    let context_asks = fake
        .calls("commands.describe")
        .iter()
        .filter(|(module, _)| module == CONTEXT)
        .count();
    assert_eq!(context_asks, 2, "a refused description isn't kept");
}

/// A module's commands are kept while its build stays the same, and asked for
/// again once it is placed on another build.
#[tokio::test]
async fn descriptions_are_kept_per_build() {
    let fake = modules();
    let commands = Commands::new(fake.clone());
    let list = sample("requests.json")["list"].clone();
    commands.list(list.clone()).await.unwrap();
    commands.list(list.clone()).await.unwrap();
    assert_eq!(fake.calls("commands.describe").len(), 2, "one each");
    fake.declaring
        .lock()
        .unwrap()
        .insert(CONTEXT.into(), "1.0.1+bbbb".into());
    commands.lock().checked = None;
    commands.list(list).await.unwrap();
    assert_eq!(fake.calls("commands.describe").len(), 3);
}

/// A preview reaches the module as the contract's module request: the app's
/// params with the verified caller added. The module's answer comes back as
/// it was sent.
#[tokio::test]
async fn a_preview_is_relayed_with_the_callers_facts() {
    let fake = modules();
    let available = case("previews.json", "flush_available");
    fake.answer(CONTEXT, PREVIEW, Answer::Reply(available.clone()));
    let commands = Commands::new(fake.clone());
    let requests = sample("requests.json");
    let reply = commands
        .preview(
            caller("desktop", "operator_app", None),
            requests["app_preview"].clone(),
        )
        .await
        .unwrap();
    assert_eq!(reply, available);
    assert_eq!(
        fake.calls(PREVIEW),
        [(CONTEXT.to_owned(), requests["module_preview"].clone())]
    );
}

/// A run reaches the module as the contract's module request, under the app's
/// `request_id`, and its reply comes back as it was sent.
#[tokio::test]
async fn a_run_is_relayed_under_the_apps_request_id() {
    let fake = modules();
    let scheduled = case("run-replies.json", "flush_scheduled");
    fake.answer(CONTEXT, RUN, Answer::Reply(scheduled.clone()));
    let commands = Commands::new(fake.clone());
    let requests = sample("requests.json");
    let reply = commands
        .run(
            caller("phone", "phone", Some("0123456789abcdef")),
            requests["app_run"].clone(),
        )
        .await
        .unwrap();
    assert_eq!(reply, scheduled);
    assert_eq!(
        fake.calls(RUN),
        [(CONTEXT.to_owned(), requests["module_run"].clone())]
    );
}

/// Requests the relay can refuse itself never reach a module: an unknown
/// module or command, a run of a read-only command, a subject of the wrong
/// kind, and an unknown member.
#[tokio::test]
async fn requests_the_relay_refuses_reach_no_module() {
    let fake = modules();
    let commands = Commands::new(fake.clone());
    let requests = sample("requests.json");
    let with = |base: &str, field: &str, value: Value| {
        let mut request = requests[base].clone();
        request[field] = value;
        request
    };
    let desktop = || caller("desktop", "development", None);
    let refused = [
        (
            commands
                .preview(
                    desktop(),
                    with("app_preview", "module", json!("example-nothing")),
                )
                .await,
            "unknown_command",
        ),
        (
            commands
                .preview(desktop(), with("app_preview", "id", json!("compact")))
                .await,
            "unknown_command",
        ),
        (
            commands
                .run(desktop(), with("app_run", "id", json!("status")))
                .await,
            "invalid_params",
        ),
        (
            commands
                .preview(
                    desktop(),
                    with("app_preview", "subject", json!({"kind": "computer"})),
                )
                .await,
            "invalid_params",
        ),
        (
            commands
                .preview(desktop(), with("app_preview", "extra", json!(1)))
                .await,
            "invalid_params",
        ),
    ];
    for (index, (result, code)) in refused.into_iter().enumerate() {
        assert_eq!(result.unwrap_err().code, code, "case {index}");
    }
    assert!(fake.calls(PREVIEW).is_empty() && fake.calls(RUN).is_empty());
}

/// How a module's failure reaches the app. An unanswered run may have run, so
/// it is never reported as safe to send again; an unanswered preview is. The
/// contract's refusal codes pass through, `unavailable` (nothing ran, safe to
/// repeat) among them, and any other is `command_refused` naming the
/// module's code.
#[tokio::test]
async fn module_failures_reach_the_app_by_what_they_prove() {
    let requests = sample("requests.json");
    let run = |answer: Answer| {
        let fake = modules();
        fake.answer(CONTEXT, RUN, answer);
        let request = requests["app_run"].clone();
        async move {
            Commands::new(fake)
                .run(caller("desktop", "development", None), request)
                .await
                .unwrap_err()
        }
    };
    assert_eq!(run(Answer::Unknown).await.code, "command_outcome_unknown");
    assert_eq!(run(Answer::NotSent).await.code, "command_unavailable");
    assert_eq!(
        run(Answer::Refused("unknown_subject")).await.code,
        "unknown_subject"
    );
    assert_eq!(
        run(Answer::Refused("unavailable")).await.code,
        "unavailable"
    );
    let other = run(Answer::Refused("vault_locked")).await;
    assert_eq!(other.code, "command_refused");
    assert_eq!(other.detail, Some(json!({"moduleCode": "vault_locked"})));
    let invalid = run(Answer::Reply(json!({"done": true}))).await;
    assert_eq!(invalid.code, "command_reply_invalid");

    let fake = modules();
    fake.answer(CONTEXT, PREVIEW, Answer::Unknown);
    let preview = Commands::new(fake)
        .preview(
            caller("desktop", "development", None),
            requests["app_preview"].clone(),
        )
        .await
        .unwrap_err();
    assert_eq!(preview.code, "command_unavailable");
}

fn route(principal: Principal, harness: &str) -> RouteBinding {
    (
        Some(principal),
        RouteTarget::ManagementSurface {
            module_id: crate::MODULE_ID.into(),
        },
        None,
        Some(BindIdentity::new(harness, "caller")),
        None,
    )
}

/// The caller facts a module is told come from the route alone: the signed
/// desktop's stamp, the development desktop, or a paired device with its
/// fingerprint. Callosum without a device, another module, or a route on
/// another surface gets no commands.
#[test]
fn caller_facts_come_from_the_route() {
    let callosum = || Principal::Reserved {
        module_id: "callosum".into(),
    };
    let mut stamped = route(Principal::Unverified, "desktop");
    stamped.4 = Some(subc_protocol::peer::OperatorApp::new("alfonso-desktop", 1));
    assert_eq!(
        command_caller(Some(&stamped)),
        Some(caller("desktop", "operator_app", None))
    );
    assert_eq!(
        command_caller(Some(&route(Principal::Direct, "desktop"))),
        Some(caller("desktop", "development", None))
    );
    assert_eq!(
        command_caller(Some(&route(callosum(), "fed-phone:0123456789abcdef"))),
        Some(caller("phone", "phone", Some("0123456789abcdef")))
    );
    let mut elsewhere = route(Principal::Direct, "desktop");
    elsewhere.1 = RouteTarget::ManagementSurface {
        module_id: "another".into(),
    };
    for refused in [
        route(callosum(), "callosum"),
        route(callosum(), "fed-phone:not-hex!"),
        route(
            Principal::Reserved {
                module_id: "magic-context".into(),
            },
            "fed-phone:0123",
        ),
        elsewhere,
    ] {
        assert_eq!(command_caller(Some(&refused)), None, "{refused:?}");
    }
    assert_eq!(command_caller(None), None);
}

/// A description whose commands hold a number (an arg's default) still
/// passes its digest check here, where serde_json keeps each number's text
/// (`arbitrary_precision`): the digest a module computes without that
/// feature, worked out by hand from the commands' canonical JSON (RFC 8785,
/// JCS: sorted keys, no spaces), is the one this module accepts.
#[test]
fn a_number_default_keeps_the_modules_digest() {
    let reply = json!({
        "commands": [{"id": "wait", "title": "Wait", "summary": "Waits a while.",
            "subject": "computer",
            "args": [{"key": "seconds", "kind": "number", "label": "Seconds", "default": 2.5}],
            "effects": ["reversible"], "run": {}}],
        // SHA-256 of `[{"args":[{"default":2.5,"key":"seconds",...}],...}]`,
        // the commands array in canonical JSON (RFC 8785).
        "digest": "33bd98021945ff75d0cf4bf0a067e02deb388ee5fc21c6e6b4aec83f0afe8eff",
    });
    let described: contract::DescribeReply = serde_json::from_value(reply).unwrap();
    described.check().unwrap();
}
