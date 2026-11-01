//! AFT's Bash tool-view templates (`templates/aft/bash/`) check clean against
//! the operations AFT froze for them and every call shape the row and page can
//! be shown for.
//!
//! The registry and host cases in `templates/aft/bash/check/` restate AFT's
//! v1 schema freeze (`bash_status`, `bash_output`, the `aft.bash.view` record
//! and the Bash tool's arguments) in this crate's provisional schema subset.
//! The view schema also admits the running form the host supplies before any
//! record exists: `phase: "running"` with every terminal field null. They are
//! check inputs only: AFT's published manifest is the authority, and a change
//! there must be copied here before the templates are published again.

use std::collections::BTreeMap;

use occipital_lang::{
    check_program, CheckContext, HostValues, OperationRegistry, Profile, ToolInputs, TrialOutcome,
};

const ROW: &str = include_str!("../../../templates/aft/bash/row.ui");
const PAGE: &str = include_str!("../../../templates/aft/bash/page.ui");
const REGISTRY: &str = include_str!("../../../templates/aft/bash/check/registry.json");
const HOSTS: &str = include_str!("../../../templates/aft/bash/check/hosts.json");

fn hosts() -> BTreeMap<String, ToolInputs> {
    serde_json::from_str(HOSTS).expect("the host cases parse")
}

/// Checks `source` under `profile` for every host case and reports each
/// case's diagnostics, so a failure names the case and the codes together.
fn check_all(source: &str, profile: Profile) {
    let mut failures = Vec::new();
    for (name, tool) in hosts() {
        let mut context = CheckContext::new(profile);
        context.registry =
            serde_json::from_str::<OperationRegistry>(REGISTRY).expect("the registry parses");
        context.tool = Some(tool);
        let checked = check_program(source.as_bytes(), &context, &HostValues::default())
            .unwrap_or_else(|error| panic!("{name}: the host inputs were refused: {error:?}"));
        let report = serde_json::to_string_pretty(&checked).expect("a check result serializes");
        println!("== {name}\n{report}");
        // Tool profiles run no trial (§5.7), so a clean check is `ok` with an
        // empty diagnostic list.
        if !checked.ok()
            || checked.trial().outcome != TrialOutcome::NotRun
            || !checked.diagnostics().is_empty()
        {
            failures.push(name);
        }
    }
    assert!(
        failures.is_empty(),
        "cases with diagnostics or a failed trial: {failures:?}"
    );
}

#[test]
fn the_bash_row_checks_clean_for_every_call_shape() {
    check_all(ROW, Profile::ToolRow);
}

#[test]
fn the_bash_page_checks_clean_for_every_call_shape() {
    check_all(PAGE, Profile::ToolPage);
}
