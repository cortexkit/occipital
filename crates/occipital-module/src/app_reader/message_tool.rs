//! Whether a chat's agent talks to the person through prefrontal's `say`
//! tool, for `session.details` (`messageTool`). The apps offer their Simple
//! view only where it does: there, the agent's own prose isn't drawn, so an
//! agent without the tool would show nothing.
//!
//! The answer comes from the plan in force on the session, which lists every
//! tool the agent is given and the module that provides it. A `say` from any
//! other provider doesn't count.
//!
//! A plan view carries the whole system text, so it isn't read on every
//! details read: each chat's answer is kept for a minute. A persona change,
//! which is what gives or takes the tool, is rare, and the next read after
//! the minute sees it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cortexkit_role_llm_runner::plan_view::PlanView;

use super::SourceResolver;

/// The tool's name, which `session.details` names as the chat's message tool.
pub const SAY: &str = "say";

/// The module that provides `say`, under both its names while the rename from
/// `prefrontal-core` runs.
const PROVIDERS: [&str; 2] = ["prefrontal", "prefrontal-core"];

/// How long a chat's answer is kept before its plan is read again.
const FRESH: Duration = Duration::from_secs(60);

/// How many chats' answers are kept. Past this, the oldest are dropped.
const KEPT: usize = 1024;

/// Whether `view` gives the agent prefrontal's `say`.
pub fn from_plan(view: &PlanView) -> Option<&'static str> {
    view.tools
        .iter()
        .any(|tool| tool.name == SAY && PROVIDERS.contains(&tool.provider.as_str()))
        .then_some(SAY)
}

/// A chat, as `(session, harness)`.
type Chat = (String, Option<String>);

#[derive(Default)]
pub struct MessageTools {
    /// Each chat's answer, with when it was read.
    kept: Mutex<HashMap<Chat, (Instant, Option<&'static str>)>>,
}

impl MessageTools {
    /// The chat's message tool, from its plan or the answer kept for it. A
    /// plan the runner can't serve (no plan view, no plan yet, the runner
    /// unreachable) answers none, and isn't kept, so the next read asks
    /// again: the apps then simply don't offer the Simple view.
    pub async fn of(
        &self,
        resolver: &dyn SourceResolver,
        session: &str,
        harness: Option<&str>,
    ) -> Option<&'static str> {
        let key = (session.to_owned(), harness.map(str::to_owned));
        let now = Instant::now();
        if let Some((at, tool)) = self.kept.lock().ok()?.get(&key) {
            if now.duration_since(*at) < FRESH {
                return *tool;
            }
        }
        let view = resolver.plan_view(session, harness).await.ok()?;
        let tool = from_plan(&view);
        if let Ok(mut kept) = self.kept.lock() {
            if kept.len() >= KEPT && !kept.contains_key(&key) {
                if let Some(oldest) = kept
                    .iter()
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(key, _)| key.clone())
                {
                    kept.remove(&oldest);
                }
            }
            kept.insert(key, (now, tool));
        }
        tool
    }
}

#[cfg(test)]
mod tests;
