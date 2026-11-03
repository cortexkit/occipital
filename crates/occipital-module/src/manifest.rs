//! The subc manifest, built in Rust so the binary and its declaration cannot
//! drift: `ck-occipital --manifest` prints exactly what serving registers.

#[cfg(any(test, feature = "show-ui"))]
use serde_json::{json, Value};
#[cfg(feature = "show-ui")]
use subc_protocol::manifest::ExecutionMode;
use subc_protocol::manifest::{
    build_provenance_from_source, BuildGitShaSource, CapabilityDeclarations, Concurrency,
    GitTreeState, ManifestProvenance, ModuleManifest, ProviderRole, Tool,
};
use subc_protocol::PROTOCOL_VERSION;

/// The id the daemon serves Occipital under. Occipital is installed as a
/// reserved module, so only the process the daemon spawned may register this
/// id, and the daemon derives the module's store path from it. It never
/// changes once deployed.
pub const MODULE_ID: &str = "occipital";

/// "This module serves the app reader's operations." CortexKit core, which
/// composes the plan a runner receives for each session, lists the one module
/// declaring it in that plan: as a sender, allowed to pass the apps' messages
/// and Stop to the session with the same grants as `thalamus-harness`, and as a
/// reader, for runners that admit only the readers a plan lists (see the
/// llm-runner contract's `senders` and `readers`). The apps don't choose their
/// reader by it: each app names Occipital in the release that switches to it.
/// The daemon reserves the capability to this module.
pub const APP_SERVER_CAPABILITY: &str = "app-server/v1";

/// The one tool Occipital declares in its manifest.
#[cfg(feature = "show-ui")]
pub const SHOW_UI: &str = "show_ui";

/// The tools served beside the teaching text in this build.
pub const TOOL_NAMES: &[&str] = &[
    #[cfg(feature = "show-ui")]
    SHOW_UI,
];

/// The version of the `show_ui` records' schema this build writes, reported in
/// the manifest's provenance so an operator can see it without opening the
/// file. The app reader's tables carry their own migration chains.
const STORE_SCHEMA_VERSION: Option<&str> = Some("1");

#[cfg(feature = "show-ui")]
const SHOW_UI_DESCRIPTION: &str = "Show the person an interface: a native program \
     checked against the rich UI language, with optional data from an earlier tool \
     result. Returns the stored record's id, or the checker's diagnostics to repair from.";

/// The commit this binary was built from, as `build.rs` embedded it, or
/// `unknown`.
pub const BUILD_GIT_SHA: &str = env!("OCCIPITAL_BUILD_GIT_SHA");

/// What `build.rs` found about the source tree: `clean`, `dirty`,
/// `no_git_dir` or `never_derived`.
pub const BUILD_GIT_TREE: &str = env!("OCCIPITAL_BUILD_GIT_TREE");

/// The input schema declared to agents for `show_ui`.
#[cfg(feature = "show-ui")]
pub fn show_ui_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": {"type": "string", "enum": ["native", "html"]},
            "title": {"type": "string"},
            "summary": {"type": "string"},
            "program": {"type": "string"},
            "data": {"type": "string", "pattern": "^\\$\\$tc\\((0|[1-9][0-9]*)\\)$"},
            "data_format": {"type": "string", "enum": ["json", "csv"]},
            "data_columns": {
                "type": "object",
                "additionalProperties": {
                    "type": "string",
                    "enum": ["text", "number", "boolean"]
                }
            }
        },
        "required": ["kind", "title", "summary", "program"],
        "additionalProperties": false
    })
}

/// The manifest Occipital registers with.
///
/// It declares a management surface serving the apps' chat operations, and a
/// tool provider serving reply teaching, with `show_ui` only when enabled at
/// build time. It has no `rich-ui-carrier/v1` capability. Core and the
/// phone find a carrier by that capability and then read its targets from the
/// `ui.registry` operation, which this build does not serve, so declaring it
/// would advertise routes that cannot work.
pub fn manifest() -> Result<ModuleManifest, String> {
    let tools: Vec<Tool> = vec![
        #[cfg(feature = "show-ui")]
        Tool {
            name: SHOW_UI.into(),
            description: Some(SHOW_UI_DESCRIPTION.into()),
            // Each successful call stores a record, so retrying an unseen
            // outcome would create a second record.
            execution_mode: ExecutionMode::Mutating,
            schema: show_ui_schema(),
        },
    ];
    Ok(
        ModuleManifest::builder(MODULE_ID, env!("CARGO_PKG_VERSION"))
            .protocol_ver(PROTOCOL_VERSION)
            .provides(vec![
                ProviderRole::ManagementSurface {
                    operations: crate::app_reader::operations(),
                    config_schema: serde_json::json!({"type": "object", "additionalProperties": false}),
                    observability: vec![],
                    // Admission reads only the route's attested principal and the
                    // device label Callosum stamps; the session comes from each
                    // request and is resolved through Entorhinal, never from the
                    // bind identity's project or session.
                    identity_scope: vec![],
                    concurrency: Concurrency::ModuleManaged,
                },
                ProviderRole::ToolProvider {
                    tools,
                    // `show_ui` is specified to take a record's owner from the route's
                    // scope stamp, never from the bind identity's project or session,
                    // so no identity key partitions what the tool does.
                    identity_scope: vec![],
                    concurrency: Concurrency::ModuleManaged,
                    emits_push: false,
                    sub_supervises: false,
                },
            ])
            .operation_delivery(crate::app_reader::operation_delivery())
            // Only `upload.chunk` takes a raw payload.
            .operation_payload(crate::app_reader::uploads::operation_payload())
            .capabilities(Some(CapabilityDeclarations {
                // `system-text-default/v1` promises that a `system_text` item
                // without a preset gets Occipital's default text, the reply
                // teaching. Occipital serves no system-text presets, so a
                // plan composer can only ask for the text when this is
                // declared.
                //
                // `bind-no-project/v1` tells the daemon that no route here
                // reads a project from its bind identity: `show_ui` takes its
                // owner from the scope stamp, and the app reader finds each
                // session's folder through Entorhinal.
                //
                // `app-server/v1`: see [`APP_SERVER_CAPABILITY`].
                //
                // `operator-app/v1` asks the daemon to stamp a route opened by
                // an operator app it recognised, such as the desktop app, which
                // the app reader admits to its reads.
                //
                // `tool-display/v1` makes this the display server: tool
                // providers find it by this capability and send it their calls'
                // records, details and completions (`display.write`), and it
                // fetches their views (see `crate::display`).
                provides: vec![
                    cortexkit_role_tool_provider::PROVIDES.into(),
                    cortexkit_role_tool_provider::SYSTEM_TEXT_DEFAULT.into(),
                    subc_protocol::scope::BIND_NO_PROJECT_CAPABILITY.into(),
                    APP_SERVER_CAPABILITY.into(),
                    subc_protocol::peer::OPERATOR_APP_CAPABILITY.into(),
                    cortexkit_role_tool_display::TOOL_DISPLAY_CAPABILITY.into(),
                ],
                requires: vec![],
                must_never_reach: vec![],
            }))
            .provenance(Some(build_provenance()?))
            .build(),
    )
}

/// The build's provenance for the manifest. A clean tree declares its commit.
/// A dirty tree declares none (subc-protocol records it as declined), and a
/// build without git says which way it found no commit. The launch nonce's
/// source is left unset: subc-client-rs fills it in at HELLO from the nonce it
/// actually read, and `--manifest` never reads one.
pub fn build_provenance() -> Result<ManifestProvenance, String> {
    let source = match (BUILD_GIT_TREE, BUILD_GIT_SHA) {
        ("clean", revision) => BuildGitShaSource::Git {
            revision,
            tree_state: GitTreeState::Clean,
        },
        ("dirty", revision) => BuildGitShaSource::Git {
            revision,
            tree_state: GitTreeState::Dirty,
        },
        ("no_git_dir", _) => BuildGitShaSource::NoGitDir,
        _ => BuildGitShaSource::NeverDerived,
    };
    build_provenance_from_source(source, None, STORE_SCHEMA_VERSION)
        .map_err(|error| format!("the embedded build provenance is malformed: {error}"))
}

/// The manifest as `--manifest` prints it: one line of JSON.
pub fn manifest_json() -> Result<String, String> {
    let manifest = manifest()?;
    serde_json::to_string(&manifest)
        .map_err(|error| format!("cannot serialise the manifest: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_value() -> Value {
        serde_json::from_str(&manifest_json().unwrap()).unwrap()
    }

    fn role<'a>(value: &'a Value, name: &str) -> &'a Value {
        let provides = value["provides"].as_array().unwrap();
        assert_eq!(provides.len(), 2, "{provides:?}");
        provides
            .iter()
            .find(|role| role["role"] == name)
            .unwrap_or_else(|| panic!("no {name} role: {provides:?}"))
    }

    #[test]
    #[cfg(feature = "show-ui")]
    fn the_tool_provider_serves_show_ui_and_nothing_else() {
        let value = manifest_value();
        assert_eq!(value["module_id"], MODULE_ID);
        let tools = role(&value, "tool_provider")["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, [SHOW_UI]);
        assert_eq!(tools[0]["schema"], show_ui_schema());
    }

    #[test]
    #[cfg(not(feature = "show-ui"))]
    fn the_tool_provider_serves_teaching_without_tools() {
        let value = manifest_value();
        assert_eq!(value["module_id"], MODULE_ID);
        assert_eq!(role(&value, "tool_provider")["tools"], json!([]));
    }

    /// The apps address these names, and the three watches must be declared as
    /// watches or the daemon delivers them as unary calls.
    #[test]
    fn the_management_surface_serves_the_app_operations() {
        let value = manifest_value();
        let surface = role(&value, "management_surface");
        let mut names: Vec<(&str, &str)> = surface["operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|op| (op["name"].as_str().unwrap(), op["kind"].as_str().unwrap()))
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                ("display.model_names", "query"),
                ("display.template", "query"),
                ("display.views", "query"),
                ("session.details", "query"),
                ("session.display_read", "query"),
                ("session.display_watch", "query"),
                ("session.enqueue_user_message", "mutate"),
                ("session.media", "query"),
                ("session.stop", "mutate"),
                ("session.tool_detail", "query"),
                ("session.tool_records", "query"),
                ("session.tool_records_watch", "query"),
                ("session.transcript_page", "query"),
                ("session.transcript_watch", "query"),
                ("upload.abort", "mutate"),
                ("upload.begin", "mutate"),
                ("upload.chunk", "mutate"),
                ("upload.finish", "mutate"),
                ("upload.place", "mutate"),
                ("upload.status", "query"),
            ]
        );
        let delivery = value["operation_delivery"].as_object().unwrap();
        let mut watches: Vec<&String> = delivery.keys().collect();
        watches.sort();
        assert_eq!(
            watches,
            [
                "session.display_watch",
                "session.tool_records_watch",
                "session.transcript_watch"
            ]
        );
        let payloads = value["operation_payload"].as_object().unwrap();
        assert_eq!(payloads.keys().collect::<Vec<_>>(), ["upload.chunk"]);
        assert_eq!(surface["identity_scope"], json!([]));
    }

    #[test]
    fn the_manifest_does_not_declare_the_rich_ui_carrier() {
        let value = manifest_value();
        let text = value.to_string();
        assert!(!text.contains("rich-ui-carrier"), "{text}");
        assert_eq!(
            value["capabilities"]["provides"],
            json!([
                cortexkit_role_tool_provider::PROVIDES,
                cortexkit_role_tool_provider::SYSTEM_TEXT_DEFAULT,
                "bind-no-project/v1",
                "app-server/v1",
                "operator-app/v1",
                "tool-display/v1"
            ])
        );
        assert!(value.get("rich_ui").is_none());
    }

    #[test]
    #[cfg(feature = "show-ui")]
    fn show_ui_schema_requires_the_four_members_and_closes_the_object() {
        let schema = show_ui_schema();
        assert_eq!(
            schema["required"],
            json!(["kind", "title", "summary", "program"])
        );
        assert_eq!(schema["additionalProperties"], false);
        let mut members: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
        members.sort();
        assert_eq!(
            members,
            [
                "data",
                "data_columns",
                "data_format",
                "kind",
                "program",
                "summary",
                "title"
            ]
        );
    }

    #[test]
    fn provenance_declares_the_commit_only_for_a_clean_tree() {
        let provenance = build_provenance().unwrap();
        match BUILD_GIT_TREE {
            "clean" => assert_eq!(provenance.build_git_sha.as_deref(), Some(BUILD_GIT_SHA)),
            _ => assert_eq!(provenance.build_git_sha, None),
        }
        assert_eq!(provenance.store_schema_version.as_deref(), Some("1"));
        assert_eq!(provenance.launch_nonce_source, None);
    }
}
