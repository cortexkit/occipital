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
    // Ten bodies and the README.
    assert_eq!(sums.lines().count(), 11);
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

/// Project and workspace subjects (contract 0.1.2) list and relay like any
/// other: each list holds only its kind's commands, and a run reaches its
/// module with the app's subject unchanged.
#[tokio::test]
async fn project_and_workspace_commands_list_and_run_under_their_subjects() {
    let fake = modules();
    fake.answer(
        CONTEXT,
        "commands.describe",
        Answer::Reply(sample("describe-project.json")),
    );
    fake.answer(
        CODE,
        "commands.describe",
        Answer::Reply(sample("describe-workspace.json")),
    );
    let scheduled = case("run-replies.json", "flush_scheduled");
    fake.answer(CONTEXT, RUN, Answer::Reply(scheduled.clone()));
    let commands = Commands::new(fake.clone());
    let requests = sample("requests.json");
    let listed = |subject: &Value| {
        let commands = &commands;
        let subject = subject.clone();
        async move {
            commands.list(json!({ "subject": subject })).await.unwrap()["commands"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| {
                    format!(
                        "{}/{}",
                        entry["module"].as_str().unwrap(),
                        entry["command"]["id"].as_str().unwrap()
                    )
                })
                .collect::<Vec<_>>()
        }
    };
    let project = requests["project_run"]["subject"].clone();
    let workspace = requests["workspace_run"]["subject"].clone();
    assert_eq!(listed(&project).await, [format!("{CONTEXT}/edit_project")]);
    assert_eq!(listed(&workspace).await, [format!("{CODE}/edit_workspace")]);
    assert!(listed(&requests["list"]["subject"]).await.is_empty());

    let mut app_run = requests["project_run"].clone();
    let module_run = app_run.clone();
    let caller_facts = app_run.as_object_mut().unwrap().remove("caller").unwrap();
    app_run["module"] = json!(CONTEXT);
    let reply = commands
        .run(serde_json::from_value(caller_facts).unwrap(), app_run)
        .await
        .unwrap();
    assert_eq!(reply, scheduled);
    assert_eq!(fake.calls(RUN), [(CONTEXT.to_owned(), module_run)]);
}

/// A description is read command by command once its digest matches: a
/// command naming a subject kind this build doesn't know, and two commands
/// sharing an id, are left out, the module's other commands stay listed, and
/// a preview of a left-out command reaches no module.
#[tokio::test]
async fn a_command_this_build_cant_read_hides_only_itself() {
    let fake = modules();
    let commands = Commands::new(fake.clone());
    let list = sample("requests.json")["list"].clone();
    let before = commands.list(list.clone()).await.unwrap();

    let mut described = sample("describe.json");
    let entries = described["commands"].as_array_mut().unwrap();
    let extra = |id: &str, subject: &str| {
        json!({"id": id, "title": "Extra", "summary": "An extra command.",
               "subject": subject, "args": [], "effects": ["reversible"], "run": {}})
    };
    entries.push(extra("orbit", "galaxy"));
    entries.push(extra("twin", "session"));
    entries.push(extra("twin", "session"));
    let bytes = serde_jcs::to_vec(&described["commands"]).unwrap();
    described["digest"] = json!(format!("{:x}", Sha256::digest(bytes)));
    fake.answer(CONTEXT, "commands.describe", Answer::Reply(described));
    let commands = Commands::new(fake.clone());
    assert_eq!(commands.list(list).await.unwrap(), before);

    let mut preview = sample("requests.json")["app_preview"].clone();
    for id in ["orbit", "twin"] {
        preview["id"] = json!(id);
        let refused = commands
            .preview(caller("desktop", "development", None), preview.clone())
            .await
            .unwrap_err();
        assert_eq!(refused.code, "unknown_command", "{id}");
    }
    assert!(fake.calls(PREVIEW).is_empty());
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

/// `phone-commands-v1` and `-v2`: what the relay served the apps from the live
/// Magic Context module, before and after it stopped putting token figures in
/// a flush preview's version. Every request is one the contract's app request
/// types take, every answer is one its reply types take, and every fact in a
/// preview, a run's answer or a table row draws through the apps' shared
/// formatter, so the palette never meets a fact it refuses. The steps the
/// palette leans on keep their outcomes: a flush that runs and is
/// `scheduled`, a repeat answered byte for byte from the stored reply, a stale
/// version, and the refusals.
#[test]
fn the_live_command_samples_are_what_the_apps_decode_and_draw() {
    for (set, requests) in [("phone-commands-v1", 16), ("phone-commands-v2", 12)] {
        let read = live_command_set(set, requests);
        let outcome = |name: &str| read(name)["result"]["outcome"].clone();
        assert_eq!(
            read("10-preview-flush-available.json")["result"]["available"],
            true
        );
        assert_eq!(outcome("08-run-flush-stale.json"), "preview_stale");
        assert_eq!(outcome("11-run-flush-scheduled.json"), "scheduled");
        // The repeat is answered from the module's stored reply, byte for byte.
        let bytes = |name: &str| {
            std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../occipital-app-reader/test-vectors")
                    .join(set)
                    .join(name),
            )
            .unwrap()
        };
        assert_eq!(
            bytes("11-run-flush-scheduled.json"),
            bytes("12-run-flush-repeat.json"),
            "{set}"
        );
        assert_eq!(read("09-unknown-module.json")["code"], "unknown_command");
        assert_eq!(read("04-details-opencode1.json")["code"], "invalid_request");
        if set == "phone-commands-v1" {
            // Only v1 holds a flush with nothing it may land.
            assert_eq!(
                read("06-preview-flush.json")["result"]["reason"],
                "queued_in_recent_tail"
            );
        } else {
            assert_eq!(
                read("07-preview-status.json")["result"]["table"]["columns"],
                serde_json::json!(["Now"])
            );
        }
    }
    // v3: AFT's `status` and Magic Context's `wrapup` on two agents' chats
    // (sessions on an OpenCode 2 runner and on Broca), and the answers for a
    // session neither module serves.
    let v3 = live_command_set("phone-commands-v3", 7);
    let ids = |name: &str| -> Vec<String> {
        v3(name)["result"]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                format!(
                    "{}/{}",
                    entry["module"].as_str().unwrap(),
                    entry["command"]["id"].as_str().unwrap()
                )
            })
            .collect()
    };
    let palette = [
        "aft/status",
        "magic-context/flush",
        "magic-context/status",
        "magic-context/wrapup",
    ];
    assert_eq!(ids("01-list-opencode2.json"), palette);
    assert_eq!(ids("02-list-broca.json"), palette);
    for name in [
        "03-preview-aft-status-opencode2.json",
        "04-preview-aft-status-broca.json",
        "06-preview-wrapup-opencode2.json",
    ] {
        assert_eq!(v3(name)["result"]["available"], true, "{name}");
    }
    assert_eq!(
        v3("05-preview-aft-status-not-bound.json")["result"]["reason"],
        "not_bound"
    );
    assert_eq!(
        v3("07-preview-wrapup-not-managed.json")["result"]["reason"],
        "not_managed"
    );
}

/// Checks one recorded set and hands back a reader for its replies.
fn live_command_set(set: &str, requests: usize) -> impl Fn(&str) -> Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../occipital-app-reader/test-vectors")
        .join(set);
    let read = {
        let root = root.clone();
        move |name: &str| -> Value {
            serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap()
        }
    };
    let draws = |fact: &Value, name: &str| {
        occipital_lang::format_fact(fact)
            .unwrap_or_else(|error| panic!("{set}/{name}: {fact}: {error:?}"));
    };
    let mut names: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".request.json"))
        .collect();
    names.sort();
    assert_eq!(names.len(), requests, "{set}");
    for request_name in &names {
        let request = read(request_name);
        let reply_name = request_name.replace(".request.json", ".json");
        let reply = read(&reply_name);
        // The relay's request files are `{method, params}`; the phone's hold
        // only the params, and the step's name says the method.
        let (method, params) = match request["method"].as_str() {
            Some(method) => (method.to_owned(), request["params"].clone()),
            None if request_name.contains("-list") => (LIST.to_owned(), request.clone()),
            None if request_name.contains("-preview") => (PREVIEW.to_owned(), request.clone()),
            None => ("session.details".to_owned(), request.clone()),
        };
        let Some(result) = reply.get("result").cloned() else {
            assert!(reply["code"].is_string(), "{set}/{reply_name}");
            continue;
        };
        match method.as_str() {
            LIST => {
                serde_json::from_value::<contract::ListRequest>(params).unwrap();
                serde_json::from_value::<contract::ListReply>(result).unwrap();
            }
            PREVIEW => {
                serde_json::from_value::<contract::AppPreviewRequest>(params).unwrap();
                serde_json::from_value::<contract::Preview>(result.clone()).unwrap();
                for fact in result["facts"].as_array().into_iter().flatten() {
                    draws(fact, &reply_name);
                }
                for row in result["table"]["rows"].as_array().into_iter().flatten() {
                    for cell in row["cells"].as_array().unwrap() {
                        let mut fact = cell.clone();
                        fact["label"] = row["label"].clone();
                        draws(&fact, &reply_name);
                    }
                }
            }
            RUN => {
                serde_json::from_value::<contract::AppRunRequest>(params).unwrap();
                serde_json::from_value::<contract::RunReply>(result.clone()).unwrap();
                for fact in result["facts"].as_array().into_iter().flatten() {
                    draws(fact, &reply_name);
                }
            }
            _ => assert!(result["subject"].is_object(), "{set}/{reply_name}"),
        }
    }
    let sums = std::fs::read_to_string(root.join("SHA256SUMS")).unwrap();
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            expected,
            "{set}/{name}"
        );
    }
    read
}

/// Magic Context's owner commands (`tests/data/mc-user-commands-samples-v2/`,
/// recorded by the Magic Context module's own command handlers and copied
/// unchanged; its README says how they were made): the
/// relay keeps every one of its 30 declarations, lists each under its
/// subject's kind, and every preview and run reply decodes as the contract's,
/// with facts and table cells the apps can word.
#[tokio::test]
async fn magic_contexts_owner_commands_relay_whole() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/mc-user-commands-samples-v2");
    let read = |name: &str| -> Value {
        serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap()
    };
    let sums = std::fs::read_to_string(root.join("SHA256SUMS")).unwrap();
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }

    let fake = Arc::new(Fake::default());
    *fake.declaring.lock().unwrap() = BTreeMap::from([("magic-context".into(), "0.50.0".into())]);
    fake.answer(
        "magic-context",
        "commands.describe",
        Answer::Reply(read("describe.json")),
    );
    let commands = Commands::new(fake);
    let mut counts = BTreeMap::new();
    for subject in [
        json!({"kind": "computer"}),
        json!({"kind": "workspace", "workspace_id": "ws-1"}),
        json!({"kind": "project", "project_id": "pj-1"}),
        json!({"kind": "agent", "agent_id": "agent_1"}),
        json!({"kind": "session", "session": {"runner": "broca", "harness": "broca", "session": "s"}}),
    ] {
        let kind = subject["kind"].as_str().unwrap().to_owned();
        let listed = commands.list(json!({ "subject": subject })).await.unwrap();
        counts.insert(kind, listed["commands"].as_array().unwrap().len());
    }
    assert_eq!(
        counts,
        BTreeMap::from([
            ("agent".to_owned(), 5),
            ("computer".to_owned(), 7),
            ("project".to_owned(), 10),
            ("session".to_owned(), 3),
            ("workspace".to_owned(), 5),
        ]),
        "all 30 declarations survive the relay's checks"
    );

    let draws = |fact: &Value, name: &str| {
        occipital_lang::format_fact(fact)
            .unwrap_or_else(|error| panic!("{name}: {fact}: {error:?}"));
    };
    let mut replies = 0;
    for line in sums.lines() {
        let name = line.split_once("  ").unwrap().1;
        let reply = read(name);
        if name.ends_with("-preview.json") || name.ends_with("-preview-locked.json") {
            serde_json::from_value::<contract::Preview>(reply.clone()).unwrap();
            for fact in reply["facts"].as_array().into_iter().flatten() {
                draws(fact, name);
            }
            for row in reply["table"]["rows"].as_array().into_iter().flatten() {
                for cell in row["cells"].as_array().unwrap() {
                    let mut fact = cell.clone();
                    fact["label"] = row["label"].clone();
                    draws(&fact, name);
                }
            }
            replies += 1;
        } else if name.ends_with("-run.json") || name == "preview-stale.json" {
            serde_json::from_value::<contract::RunReply>(reply.clone()).unwrap();
            for fact in reply["facts"].as_array().into_iter().flatten() {
                draws(fact, name);
            }
            replies += 1;
        } else if name != "describe.json" {
            assert!(reply["code"].is_string(), "{name}");
        }
    }
    assert_eq!(read("preview-stale.json")["outcome"], "preview_stale");
    assert_eq!(replies, 32);
}
