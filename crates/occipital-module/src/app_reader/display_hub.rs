//! One projection per session, independent of how many phones read its ring.

use crate::app_reader::Guard as _;
use occipital_app_reader::{
    display::{DisplayBatcher, DisplayDeltaIngress, DisplayProjection},
    display_events::DisplayInput,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

/// 1024 entries.
const RING_ENTRIES: std::num::NonZeroUsize = std::num::NonZeroUsize::MIN.saturating_add(1023);

#[derive(Default)]
struct State {
    projections: HashMap<String, Arc<DisplayProjection>>,
    changes: HashMap<String, watch::Sender<u64>>,
    batcher: DisplayBatcher,
}

#[derive(Default)]
pub struct DisplayHub {
    state: Mutex<State>,
}

impl DisplayHub {
    pub fn projection(&self, session: &str) -> Option<Arc<DisplayProjection>> {
        self.state.guard().projections.get(session).cloned()
    }

    pub fn changes(&self, session: &str) -> watch::Receiver<u64> {
        let mut state = self.state.guard();
        state
            .changes
            .retain(|_, signal| signal.receiver_count() > 0);
        state
            .changes
            .entry(session.into())
            .or_insert_with(|| watch::channel(0).0)
            .subscribe()
    }

    pub fn attach(&self, session: &str) -> Arc<DisplayProjection> {
        let mut state = self.state.guard();
        let projection = state
            .projections
            .entry(session.into())
            .or_insert_with(|| Arc::new(DisplayProjection::with_ring(RING_ENTRIES)))
            .clone();
        Self::signal(&state, session);
        projection
    }

    pub fn remove(&self, session: &str) {
        let mut state = self.state.guard();
        state.batcher.discard_session(session);
        state.projections.remove(session);
        Self::signal(&state, session);
    }

    fn signal(state: &State, session: &str) {
        if let Some(signal) = state.changes.get(session) {
            signal.send_modify(|n| *n = n.wrapping_add(1));
        }
    }

    /// Called by the source subscription, never by a polling read or projection lookup.
    pub fn ingest(self: &Arc<Self>, session: &str, input: DisplayInput) {
        let mut state = self.state.guard();
        let projection = state
            .projections
            .entry(session.into())
            .or_insert_with(|| Arc::new(DisplayProjection::with_ring(RING_ENTRIES)))
            .clone();
        Self::signal(&state, session);
        match input {
            DisplayInput::Delta {
                mid,
                text,
                part_type,
            } => {
                let pushed = state.batcher.push(DisplayDeltaIngress {
                    session_id: session.into(),
                    message_id: mid,
                    delta_text: text,
                    part_type,
                });
                for chunk in pushed.flushed {
                    projection.observe_display(chunk);
                }
                if let Some(timer) = pushed.timer {
                    let hub = Arc::downgrade(self);
                    tokio::spawn(async move {
                        tokio::time::sleep(occipital_app_reader::display_watch::DISPLAY_WINDOW)
                            .await;
                        if let Some(hub) = hub.upgrade() {
                            let state = hub.state.guard();
                            if let Some(chunk) = state.batcher.take_if_current(&timer) {
                                if let Some(projection) = state.projections.get(&chunk.session_id) {
                                    projection.observe_display(chunk);
                                }
                            }
                        }
                    });
                }
            }
            DisplayInput::Gap | DisplayInput::Reset => {
                state.batcher.discard_session(session);
                if matches!(input, DisplayInput::Reset) {
                    projection.reattach_display();
                } else {
                    projection.observe_display_gap();
                }
            }
            DisplayInput::TurnStart => projection.start_display(),
            DisplayInput::RunStart { rich_ui } => projection.start_run(rich_ui),
            DisplayInput::MessageEnd { mid } => {
                // Words still held for batching go out first, so they are part of the
                // message the end makes final.
                for chunk in state.batcher.drain_session(session) {
                    projection.observe_display(chunk);
                }
                projection.end_message(&mid);
            }
            DisplayInput::TurnEnd => {
                for chunk in state.batcher.drain_session(session) {
                    projection.observe_display(chunk);
                }
                projection.end_display();
            }
            // Words held for batching go out first, so a reader sees text and calls
            // in the order the runner reported them.
            DisplayInput::ToolStarted(start) => {
                for chunk in state.batcher.drain_session(session) {
                    projection.observe_display(chunk);
                }
                projection.start_tool(session, start);
            }
            DisplayInput::ToolFinished(end) => {
                for chunk in state.batcher.drain_session(session) {
                    projection.observe_display(chunk);
                }
                projection.end_tool(end);
            }
        }
    }
}
