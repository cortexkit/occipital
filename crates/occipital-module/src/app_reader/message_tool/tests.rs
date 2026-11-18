use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use cortexkit_role_llm_runner::plan_view::{PlanView, System, Tool};
use occipital_app_reader::source::{ReadError, TranscriptSource};
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

/// A runner whose session `s` has prefrontal's `say`, counting plan reads,
/// and whose session `gone` serves no plan.
#[derive(Default)]
struct Plans {
    reads: AtomicUsize,
}

#[async_trait]
impl SourceResolver for Plans {
    async fn resolve(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Option<Arc<dyn TranscriptSource>>, ReadError> {
        Ok(None)
    }

    async fn plan_view(&self, session: &str, _: Option<&str>) -> Result<PlanView, ReadError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        match session {
            "s" => Ok(plan(&[("say", "prefrontal-core")])),
            _ => Err(ReadError::new("no_plan", "no plan yet")),
        }
    }
}

/// A chat's answer is kept, so a second details read doesn't read the plan
/// again; a plan the runner couldn't serve answers none and isn't kept.
#[tokio::test]
async fn an_answer_is_kept_and_a_failed_read_is_not() {
    let tools = MessageTools::default();
    let plans = Plans::default();
    assert_eq!(tools.of(&plans, "s", Some("broca")).await, Some(SAY));
    assert_eq!(tools.of(&plans, "s", Some("broca")).await, Some(SAY));
    assert_eq!(plans.reads.load(Ordering::SeqCst), 1);
    assert_eq!(tools.of(&plans, "gone", Some("broca")).await, None);
    assert_eq!(tools.of(&plans, "gone", Some("broca")).await, None);
    assert_eq!(plans.reads.load(Ordering::SeqCst), 3);
}
