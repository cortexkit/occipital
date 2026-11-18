use cortexkit_role_llm_runner::plan_view::{PlanView, System, Tool};
use serde_json::json;

use super::*;

fn plan(tools: &[(&str, &str)]) -> PlanView {
    PlanView::new(
        7,
        System::new("Be brief."),
        tools
            .iter()
            .map(|(name, provider)| Tool::new(*name, "", json!({"type": "object"}), *provider))
            .collect(),
    )
}

/// Only prefrontal's `say`, under either of its names, makes it the chat's
/// message tool.
#[test]
fn only_prefrontal_s_say_counts() {
    for (tools, expected) in [
        (vec![("say", "prefrontal-core"), ("bash", "aft")], Some(SAY)),
        (vec![("say", "prefrontal")], Some(SAY)),
        (vec![("say", "aft")], None),
        (vec![("notify", "prefrontal-core")], None),
        (vec![], None),
    ] {
        assert_eq!(from_plan(&plan(&tools)), expected, "{tools:?}");
    }
}
