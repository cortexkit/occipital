//! Whether a chat's agent talks to the person through prefrontal's `say`
//! tool. Where it does, its calls are drawn as the agent's messages
//! (`occipital_app_reader::say`), and `session.details` names the tool as
//! `messageTool`, so the apps offer their Simple view: there the agent's own
//! prose isn't drawn, so an agent without the tool would show nothing.
//!
//! The answer comes from the plan in force on the session, which lists every
//! tool the agent is given and the module that provides it. A `say` from any
//! other provider doesn't count.
//!
//! A plan view carries the whole system text, so it isn't read on every page
//! or details read: each session's answer is kept for [`FRESH`]. A persona
//! change, which is what gives or takes the tool, is rare, and the first read
//! after that sees it.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cortexkit_role_llm_runner::plan_view::PlanView;

/// The tool's name, which `session.details` names as the chat's message tool.
pub const SAY: &str = occipital_app_reader::say::TOOL;

/// The module that provides `say`, under both its names: it is being renamed
/// from `prefrontal-core` to `prefrontal`, and a plan composed before the
/// rename names the old one until the session's plan is composed again.
const PROVIDERS: [&str; 2] = ["prefrontal", "prefrontal-core"];

/// How long a session's answer is kept before its plan is read again.
pub(crate) const FRESH: Duration = Duration::from_secs(60);

/// A session's kept answer, with when it was read; shared by every source
/// built for the session, since a plain read builds a fresh one each time.
pub(crate) type MessageToolSlot = Arc<Mutex<Option<(Instant, Option<&'static str>)>>>;

/// Whether `view` gives the agent prefrontal's `say`.
pub(crate) fn from_plan(view: &PlanView) -> Option<&'static str> {
    view.tools
        .iter()
        .any(|tool| tool.name == SAY && PROVIDERS.contains(&tool.provider.as_str()))
        .then_some(SAY)
}

#[cfg(test)]
mod tests;
