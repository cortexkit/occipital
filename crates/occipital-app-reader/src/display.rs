//! Memory-only display batching and the app's display polling frames.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayPartType {
    Text,
    Reasoning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayDeltaIngress {
    pub session_id: String,
    /// Taken from the runner event's `mid` field, not the provider-assigned message id.
    pub message_id: String,
    pub delta_text: String,
    pub part_type: DisplayPartType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayChunk {
    pub session_id: String,
    pub message_id: String,
    pub chunk: String,
    pub part_type: DisplayPartType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DisplayStreamFrame {
    Display {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "messageID")]
        message_id: String,
        chunk: String,
        #[serde(rename = "partType")]
        part_type: DisplayPartType,
        /// The rich UI stamp of the run this text streams in, in the shape page rows
        /// carry it (`{dialect, catalogDigest}`), so the phone can draw the reply the
        /// same way live as it will once stored. Absent when the run carries none.
        #[serde(rename = "richUi", skip_serializing_if = "Option::is_none")]
        rich_ui: Option<Value>,
    },
    DisplayGap,
    DisplayEnd,
    /// A tool call starting or ending, sent only to readers that opted in with
    /// `acceptToolFrames`.
    Tool(Box<ToolFrame>),
}

/// Where a tool call is, on its live frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPhase {
    Running,
    Answered,
    Unanswered,
}

/// A tool call's live state. Each member follows the rule of the stored tool
/// part with the same name, so a running row and its stored row carry the same
/// `callId`, `callKey`, `schemaPin` and `input`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolFrame {
    #[serde(rename = "sessionID")]
    pub session_id: String,
    /// The runner's `mid` for the call's message, as text frames carry it.
    #[serde(rename = "messageID")]
    pub message_id: String,
    #[serde(rename = "callId")]
    pub call_id: String,
    pub tool: String,
    pub phase: ToolPhase,
    #[serde(rename = "callKey", skip_serializing_if = "Option::is_none")]
    pub call_key: Option<String>,
    #[serde(rename = "schemaPin", skip_serializing_if = "Option::is_none")]
    pub schema_pin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(
        rename = "inputTruncatedFields",
        skip_serializing_if = "Option::is_none"
    )]
    pub input_truncated_fields: Option<Vec<String>>,
    #[serde(rename = "startedAtMs", skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
    #[serde(rename = "finishedAtMs", skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<u64>,
    #[serde(rename = "isError", skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
    /// Why the call ended without an answer, as the stored part's `unanswered`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unanswered: Option<String>,
    /// The answer's size in bytes, on an end frame.
    #[serde(rename = "outputBytes", skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<u64>,
    /// The computer's time when the reply or notice carrying this frame was
    /// built, so an app can count a running call's time without trusting its own
    /// clock to match the computer's. Set as the frame is served, never stored.
    #[serde(rename = "asOfMs", skip_serializing_if = "Option::is_none")]
    pub as_of_ms: Option<u64>,
}

impl ToolFrame {
    /// The running frame for a started call of `session_id`. The row's `input` is
    /// projected only for a call that names its tool version, as a stored part's is.
    pub fn running(session_id: &str, start: crate::display_events::ToolStart) -> Self {
        let projection = start
            .schema_pin
            .as_ref()
            .and(start.arguments.as_ref())
            .and_then(crate::tool_input::project);
        let (input, input_truncated_fields) = match projection {
            Some(projection) => (Some(projection.input), Some(projection.truncated_fields)),
            None => (None, None),
        };
        Self {
            session_id: session_id.to_owned(),
            message_id: start.mid,
            call_id: start.call_id,
            tool: start.tool,
            phase: ToolPhase::Running,
            call_key: start.call_key,
            schema_pin: start.schema_pin,
            input,
            input_truncated_fields,
            started_at_ms: start.started_at,
            finished_at_ms: None,
            is_error: false,
            unanswered: None,
            output_bytes: None,
            as_of_ms: None,
        }
    }

    /// This running call's end frame. `ok` and `error` are answers; any other
    /// outcome is the reason the call went unanswered, kept as the runner wrote it.
    fn ended(&self, end: &crate::display_events::ToolEnd) -> Self {
        let mut frame = self.clone();
        frame.finished_at_ms = end.finished_at;
        frame.output_bytes = end.output_bytes;
        match end.outcome.as_str() {
            "ok" => frame.phase = ToolPhase::Answered,
            "error" => {
                frame.phase = ToolPhase::Answered;
                frame.is_error = true;
            }
            reason => {
                frame.phase = ToolPhase::Unanswered;
                frame.unanswered = Some(reason.to_owned());
            }
        }
        frame
    }
}

impl From<DisplayChunk> for DisplayStreamFrame {
    fn from(value: DisplayChunk) -> Self {
        Self::Display {
            session_id: value.session_id,
            message_id: value.message_id,
            chunk: value.chunk,
            part_type: value.part_type,
            rich_ui: None,
        }
    }
}

/// A display frame stamped by the memory-only polling ring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SequencedDisplayFrame {
    pub seq: u64,
    pub frame: DisplayStreamFrame,
}

/// Why a display read cannot offer a cursor or current display state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayReadAbsent {
    NoRingAttached,
    RingQuiet,
}

/// The oldest retained display sequence after a polling cursor falls behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DisplayWindowExpired {
    #[serde(rename = "oldestSeq")]
    pub oldest_seq: u64,
}

/// Read-only reply from the display polling ring.
///
/// `frame` is a cold-read baseline with the current accumulated display text.
/// Incremental reads carry the original coalesced frames, while an expired cursor
/// deliberately omits `frames` so a consumer cannot mistake a partial window for
/// a contiguous stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum DisplayReadReply {
    Absent {
        absent: DisplayReadAbsent,
    },
    Baseline {
        seq: u64,
        frame: DisplayStreamFrame,
        /// For a reader that accepts tool frames: a running frame for each call
        /// still in flight, in the order they started.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tools: Vec<ToolFrame>,
    },
    Incremental {
        seq: u64,
        frames: Vec<SequencedDisplayFrame>,
    },
    Expired {
        seq: u64,
        #[serde(rename = "windowExpired")]
        window_expired: DisplayWindowExpired,
    },
}

/// A bounded, non-consuming display-only view over a session's live events.
///
/// Phone polling owns its cursor and must be repeatable. Keeping these frames
/// separate means one reader never consumes another reader's display state.
#[derive(Debug)]
pub struct DisplayRing {
    capacity: usize,
    frames: VecDeque<SequencedDisplayFrame>,
    current: Option<DisplayStreamFrame>,
    next_seq: u64,
}

impl DisplayRing {
    pub fn new(capacity: usize) -> Self {
        debug_assert!(capacity > 0);
        Self {
            capacity,
            frames: VecDeque::with_capacity(capacity),
            current: None,
            next_seq: 0,
        }
    }

    /// Store and number `frame`. Returns false for a gap that wasn't stored.
    pub fn push(&mut self, frame: DisplayStreamFrame) -> bool {
        // A gap tells a reader to drop the live text and calls it shows. A second gap
        // with nothing shown since tells it nothing more, so it isn't stored or
        // numbered: a run's start, which resets the display several times before any
        // text, reaches the reader as one gap.
        if matches!(frame, DisplayStreamFrame::DisplayGap)
            && matches!(
                self.frames.back().map(|last| &last.frame),
                Some(DisplayStreamFrame::DisplayGap)
            )
        {
            return false;
        }
        // A baseline's `frame` is the live text; tool frames reach a baseline as its
        // running calls instead.
        if !matches!(frame, DisplayStreamFrame::Tool(_)) {
            self.update_current(&frame);
        }
        self.next_seq = self
            .next_seq
            .checked_add(1)
            .expect("display polling sequence exhausted");
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(SequencedDisplayFrame {
            seq: self.next_seq,
            frame,
        });
        true
    }

    fn update_current(&mut self, frame: &DisplayStreamFrame) {
        self.current = Some(match (&mut self.current, frame) {
            (
                Some(DisplayStreamFrame::Display {
                    session_id,
                    message_id,
                    chunk,
                    part_type,
                    ..
                }),
                DisplayStreamFrame::Display {
                    session_id: next_session_id,
                    message_id: next_message_id,
                    chunk: next_chunk,
                    part_type: next_part_type,
                    rich_ui: next_rich_ui,
                },
            ) if session_id == next_session_id
                && message_id == next_message_id
                && part_type == next_part_type =>
            {
                chunk.push_str(next_chunk);
                DisplayStreamFrame::Display {
                    session_id: session_id.clone(),
                    message_id: message_id.clone(),
                    chunk: chunk.clone(),
                    part_type: *part_type,
                    rich_ui: next_rich_ui.clone(),
                }
            }
            _ => frame.clone(),
        });
    }

    pub fn read(&self, since_seq: Option<u64>) -> DisplayReadReply {
        if self.frames.is_empty() {
            return DisplayReadReply::Absent {
                absent: DisplayReadAbsent::RingQuiet,
            };
        }
        let Some(since_seq) = since_seq else {
            // Only tool frames set no text: the baseline then shows none.
            return DisplayReadReply::Baseline {
                seq: self.next_seq,
                frame: self
                    .current
                    .clone()
                    .unwrap_or(DisplayStreamFrame::DisplayGap),
                tools: Vec::new(),
            };
        };
        let oldest_seq = self
            .frames
            .front()
            .map(|frame| frame.seq)
            .expect("current display frame is retained");
        if since_seq > self.next_seq || since_seq.saturating_add(1) < oldest_seq {
            return DisplayReadReply::Expired {
                seq: self.next_seq,
                window_expired: DisplayWindowExpired { oldest_seq },
            };
        }
        DisplayReadReply::Incremental {
            seq: self.next_seq,
            frames: self
                .frames
                .iter()
                .filter(|frame| frame.seq > since_seq)
                .cloned()
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayTimer {
    pub session_id: String,
    pub message_id: String,
    generation: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DisplayPush {
    pub flushed: Vec<DisplayChunk>,
    pub timer: Option<DisplayTimer>,
}

#[derive(Debug)]
struct PendingDisplay {
    chunk: String,
    part_type: DisplayPartType,
    generation: u64,
    ordinal: u64,
}

#[derive(Debug, Default)]
struct DisplayBatchState {
    pending: HashMap<(String, String), PendingDisplay>,
    next_generation: u64,
    next_ordinal: u64,
}

/// Coalesces display text without observing clocks or touching durable state. The caller owns
/// the 300 ms tick and uses its generation token to prevent an old timer from flushing a
/// newer batch after a completion-triggered drain.
#[derive(Debug, Default)]
pub struct DisplayBatcher {
    state: Mutex<DisplayBatchState>,
}

impl DisplayBatcher {
    pub fn push(&self, delta: DisplayDeltaIngress) -> DisplayPush {
        if delta.delta_text.is_empty() {
            return DisplayPush::default();
        }
        let key = (delta.session_id.clone(), delta.message_id.clone());
        let mut state = self.state.lock().expect("display batcher poisoned");
        if let Some(pending) = state.pending.get_mut(&key) {
            if pending.part_type == delta.part_type {
                pending.chunk.push_str(&delta.delta_text);
                return DisplayPush::default();
            }
        }

        let flushed = state
            .pending
            .remove(&key)
            .map(|pending| chunk_from_pending(&key, pending))
            .into_iter()
            .collect();
        state.next_generation = state.next_generation.saturating_add(1);
        state.next_ordinal = state.next_ordinal.saturating_add(1);
        let generation = state.next_generation;
        let ordinal = state.next_ordinal;
        state.pending.insert(
            key,
            PendingDisplay {
                chunk: delta.delta_text,
                part_type: delta.part_type,
                generation,
                ordinal,
            },
        );
        DisplayPush {
            flushed,
            timer: Some(DisplayTimer {
                session_id: delta.session_id,
                message_id: delta.message_id,
                generation,
            }),
        }
    }

    pub fn take_if_current(&self, timer: &DisplayTimer) -> Option<DisplayChunk> {
        let key = (timer.session_id.clone(), timer.message_id.clone());
        let mut state = self.state.lock().expect("display batcher poisoned");
        if state.pending.get(&key)?.generation != timer.generation {
            return None;
        }
        state
            .pending
            .remove(&key)
            .map(|pending| chunk_from_pending(&key, pending))
    }

    pub fn drain_session(&self, session_id: &str) -> Vec<DisplayChunk> {
        let mut state = self.state.lock().expect("display batcher poisoned");
        let keys = state
            .pending
            .keys()
            .filter(|(pending_session, _)| pending_session == session_id)
            .cloned()
            .collect::<Vec<_>>();
        let mut pending = keys
            .into_iter()
            .filter_map(|key| state.pending.remove(&key).map(|value| (key, value)))
            .collect::<Vec<_>>();
        pending.sort_by_key(|(_, value)| value.ordinal);
        pending
            .into_iter()
            .map(|(key, value)| chunk_from_pending(&key, value))
            .collect()
    }

    pub fn discard_session(&self, session_id: &str) {
        self.state
            .lock()
            .expect("display batcher poisoned")
            .pending
            .retain(|(pending_session, _), _| pending_session != session_id);
    }

    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.state
            .lock()
            .expect("display batcher poisoned")
            .pending
            .len()
    }
}

fn chunk_from_pending(key: &(String, String), pending: PendingDisplay) -> DisplayChunk {
    DisplayChunk {
        session_id: key.0.clone(),
        message_id: key.1.clone(),
        chunk: pending.chunk,
        part_type: pending.part_type,
    }
}

struct DisplayProjectionState {
    display_ring: DisplayRing,
    /// The same frames as `display_ring` with each tool call's start and end among
    /// them, for readers that accept tool frames. A separate ring keeps the plain
    /// ring's sequence, and so every reply to a reader that didn't opt in, exactly as
    /// it was before tool frames existed.
    tool_ring: DisplayRing,
    /// The calls started and not yet ended, in the order they started.
    running: Vec<ToolFrame>,
    display_turn_open: bool,
    display_end_emitted: bool,
    /// The running run's rich UI stamp, set when the run starts and cleared when it
    /// ends, so a later run without one never inherits it.
    rich_ui: Option<Value>,
    /// The message whose words are the live footer: the last one text was shown for
    /// and not yet ended.
    live: Option<LiveMessage>,
}

/// The message whose words are the live footer.
struct LiveMessage {
    /// The key its frames carry as `messageID`: the runner's `mid`, or the
    /// `live_id` a runner gives a message its host names only when storing it.
    key: String,
    /// The `mid` the runner stored a `live_id`-keyed message under. Its frames
    /// keep the `live_id` until its end, because an app hands live words over
    /// to the stored copy by matching the words under one unchanging key.
    stored_as: Option<String>,
}

impl LiveMessage {
    /// Whether `mid` names this message, by its key or the `mid` it was stored as.
    fn is(&self, mid: &str) -> bool {
        self.key == mid || self.stored_as.as_deref() == Some(mid)
    }
}

/// Per-session display state, independent of the runner's durable event stream.
pub struct DisplayProjection {
    state: Mutex<DisplayProjectionState>,
    /// A generation wakes every reader without letting readers consume each other's frames.
    display_changed: watch::Sender<u64>,
}

impl DisplayProjection {
    pub fn new(ring_entries: usize) -> Result<Self, String> {
        std::num::NonZeroUsize::new(ring_entries)
            .map(Self::with_ring)
            .ok_or_else(|| "display ring_entries must be positive".to_string())
    }

    /// A projection keeping `ring_entries` frames, which a non-zero size can't refuse.
    pub fn with_ring(ring_entries: std::num::NonZeroUsize) -> Self {
        Self {
            state: Mutex::new(DisplayProjectionState {
                display_ring: DisplayRing::new(ring_entries.get()),
                tool_ring: DisplayRing::new(ring_entries.get()),
                running: Vec::new(),
                display_turn_open: false,
                display_end_emitted: false,
                rich_ui: None,
                live: None,
            }),
            display_changed: watch::channel(0).0,
        }
    }

    /// Subscribe before reading so a frame arriving between a read and a wait is not lost.
    pub fn subscribe_display(&self) -> watch::Receiver<u64> {
        self.display_changed.subscribe()
    }

    /// Called after releasing the state lock; retain the generation even without receivers.
    fn signal_display_changed(&self) {
        self.display_changed
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }

    pub fn start_display(&self) {
        let mut state = self.state.lock().expect("display projection poisoned");
        state.display_turn_open = true;
        state.display_end_emitted = false;
    }

    /// A run started: open the display and stamp its text frames with the run's stamp.
    pub fn start_run(&self, rich_ui: Option<Value>) {
        let mut state = self.state.lock().expect("display projection poisoned");
        state.display_turn_open = true;
        state.display_end_emitted = false;
        state.rich_ui = rich_ui;
    }

    pub fn observe_display(&self, chunk: DisplayChunk) {
        let mut state = self.state.lock().expect("display projection poisoned");
        if state.display_end_emitted {
            return;
        }
        state.display_turn_open = true;
        if state
            .live
            .as_ref()
            .is_none_or(|live| live.key != chunk.message_id)
        {
            state.live = Some(LiveMessage {
                key: chunk.message_id.clone(),
                stored_as: None,
            });
        }
        let mut frame = DisplayStreamFrame::from(chunk);
        if let DisplayStreamFrame::Display { rich_ui, .. } = &mut frame {
            rich_ui.clone_from(&state.rich_ui);
        }
        state.push_both(frame);
        drop(state);
        self.signal_display_changed();
    }

    /// Lost display text. Calls are reported on the runner's control lane, which a
    /// display gap doesn't touch, so each call still in flight is sent again after the
    /// gap that told readers to drop it.
    pub fn observe_display_gap(&self) {
        let mut state = self.state.lock().expect("display projection poisoned");
        if state.display_end_emitted {
            return;
        }
        state.display_turn_open = true;
        state.live = None;
        state.display_ring.push(DisplayStreamFrame::DisplayGap);
        if state.tool_ring.push(DisplayStreamFrame::DisplayGap) {
            for call in state.running.clone() {
                state
                    .tool_ring
                    .push(DisplayStreamFrame::Tool(Box::new(call)));
            }
        }
        drop(state);
        self.signal_display_changed();
    }

    /// A call of `session_id` went to its tool. A repeated start of one call
    /// replaces the first.
    pub fn start_tool(&self, session_id: &str, start: crate::display_events::ToolStart) {
        let frame = ToolFrame::running(session_id, start);
        let mut state = self.state.lock().expect("display projection poisoned");
        state.running.retain(|call| call.call_id != frame.call_id);
        state.running.push(frame.clone());
        state
            .tool_ring
            .push(DisplayStreamFrame::Tool(Box::new(frame)));
        drop(state);
        self.signal_display_changed();
    }

    /// A call ended. A call this projection never saw start, such as one already
    /// running when the session was first followed, has no live row to end: the
    /// stored page shows it.
    pub fn end_tool(&self, end: crate::display_events::ToolEnd) {
        let mut state = self.state.lock().expect("display projection poisoned");
        let Some(index) = state
            .running
            .iter()
            .position(|call| call.call_id == end.call_id)
        else {
            return;
        };
        let frame = state.running.remove(index).ended(&end);
        state
            .tool_ring
            .push(DisplayStreamFrame::Tool(Box::new(frame)));
        drop(state);
        self.signal_display_changed();
    }

    /// The runner stored `mid`. When its words are the live footer, an end tells the
    /// phone they are final: it keeps them until the stored copy lands, where a gap
    /// would clear them first. The turn stays open for the run's next message. When a
    /// later message's words already replaced it, nothing is sent: the phone ends a
    /// message when the next one's text starts, and an end now would end the new one.
    pub fn end_message(&self, mid: &str) {
        let mut state = self.state.lock().expect("display projection poisoned");
        if state.display_end_emitted || !state.live.as_ref().is_some_and(|live| live.is(mid)) {
            return;
        }
        state.live = None;
        state.push_both(DisplayStreamFrame::DisplayEnd);
        drop(state);
        self.signal_display_changed();
    }

    /// The runner stored the message it streamed under `live_id` as `mid`. When
    /// that message's words are the live footer, `mid` becomes another name for
    /// it: [`DisplayProjection::message_key`] turns later words under `mid` back
    /// into the `live_id`, and [`DisplayProjection::end_message`] ends it by
    /// either. Otherwise, as for a replayed store of a message never shown, it
    /// does nothing.
    pub fn store_message(&self, mid: &str, live_id: &str) {
        let mut state = self.state.lock().expect("display projection poisoned");
        if state.display_end_emitted {
            return;
        }
        if let Some(live) = state.live.as_mut().filter(|live| live.key == live_id) {
            live.stored_as = Some(mid.to_owned());
        }
    }

    /// The key words under `mid` are shown with: the live message's `live_id`
    /// when it was stored as `mid`, so its frames keep one key to its end;
    /// otherwise `mid` itself.
    pub fn message_key(&self, mid: &str) -> String {
        let state = self.state.lock().expect("display projection poisoned");
        match &state.live {
            Some(live) if live.stored_as.as_deref() == Some(mid) => live.key.clone(),
            _ => mid.to_owned(),
        }
    }

    /// The run ended or paused. A call still running gets an `unanswered`
    /// (`interrupted`) end first, so no reader shows it running for good.
    pub fn end_display(&self) {
        let mut state = self.state.lock().expect("display projection poisoned");
        state.rich_ui = None;
        let interrupted = crate::display_events::ToolEnd {
            call_id: String::new(),
            outcome: "interrupted".to_owned(),
            output_bytes: None,
            finished_at: None,
        };
        let closed: Vec<ToolFrame> = std::mem::take(&mut state.running)
            .iter()
            .map(|call| call.ended(&interrupted))
            .collect();
        let any_closed = !closed.is_empty();
        for frame in closed {
            state
                .tool_ring
                .push(DisplayStreamFrame::Tool(Box::new(frame)));
        }
        if !state.display_turn_open || state.display_end_emitted {
            drop(state);
            if any_closed {
                self.signal_display_changed();
            }
            return;
        }
        state.display_turn_open = false;
        state.display_end_emitted = true;
        state.live = None;
        state.push_both(DisplayStreamFrame::DisplayEnd);
        drop(state);
        self.signal_display_changed();
    }

    /// The live state restarts: the runner reset it, or the session is followed
    /// afresh. Which calls still run is no longer known, so none is kept; the stored
    /// page shows each call's phase.
    pub fn reattach_display(&self) {
        let mut state = self.state.lock().expect("display projection poisoned");
        state.display_turn_open = true;
        state.display_end_emitted = false;
        state.live = None;
        state.running.clear();
        state.push_both(DisplayStreamFrame::DisplayGap);
        drop(state);
        self.signal_display_changed();
    }

    /// Reads do not advance a server cursor or consume another subscriber's frames.
    pub fn display_read(&self, since_seq: Option<u64>) -> DisplayReadReply {
        self.state
            .lock()
            .expect("display projection poisoned")
            .display_ring
            .read(since_seq)
    }

    /// [`Self::display_read`] for a reader that accepts tool frames: the ring with
    /// each call's start and end, each tool frame stamped with `now_ms`.
    pub fn display_read_with_tools(&self, since_seq: Option<u64>, now_ms: u64) -> DisplayReadReply {
        let state = self.state.lock().expect("display projection poisoned");
        match state.tool_ring.read(since_seq) {
            DisplayReadReply::Baseline { seq, frame, .. } => DisplayReadReply::Baseline {
                seq,
                frame,
                tools: state
                    .running
                    .iter()
                    .map(|call| ToolFrame {
                        as_of_ms: Some(now_ms),
                        ..call.clone()
                    })
                    .collect(),
            },
            DisplayReadReply::Incremental { seq, mut frames } => {
                for sequenced in &mut frames {
                    if let DisplayStreamFrame::Tool(frame) = &mut sequenced.frame {
                        frame.as_of_ms = Some(now_ms);
                    }
                }
                DisplayReadReply::Incremental { seq, frames }
            }
            other => other,
        }
    }
}

impl DisplayProjectionState {
    /// A text, gap or end frame goes to both rings.
    fn push_both(&mut self, frame: DisplayStreamFrame) {
        self.display_ring.push(frame.clone());
        self.tool_ring.push(frame);
    }
}

/// The computer's time in Unix milliseconds, as tool frames are stamped with it.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayReadError {
    pub code: &'static str,
    pub message: String,
}

impl DisplayReadError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_request",
            message: message.into(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct DisplayReadRequest<'a> {
    pub session: &'a str,
    pub since_seq: Option<u64>,
    /// `acceptToolFrames`: the reader draws tool frames. Without it a reader never
    /// sees one, since shipped phone builds end a watch on a frame kind they don't
    /// know.
    pub accept_tool_frames: bool,
}

impl<'a> DisplayReadRequest<'a> {
    pub fn parse(params: &'a serde_json::Value) -> Result<Self, DisplayReadError> {
        let object = params.as_object().ok_or_else(|| {
            DisplayReadError::invalid("session.display_read params must be an object")
        })?;
        const AMBIENT_CALLER_KEYS: [&str; 3] = ["harness", "sessionId", "callerDirectory"];
        for key in object.keys() {
            if key != "session"
                && key != "sinceSeq"
                && key != "acceptToolFrames"
                && !AMBIENT_CALLER_KEYS.contains(&key.as_str())
            {
                return Err(DisplayReadError::invalid(format!(
                    "session.display_read has unknown parameter '{key}'"
                )));
            }
        }
        let session = object
            .get("session")
            .and_then(serde_json::Value::as_str)
            .filter(|session| !session.is_empty())
            .ok_or_else(|| {
                DisplayReadError::invalid(
                    "session.display_read requires non-empty string 'session'",
                )
            })?;
        let since_seq = match object.get("sinceSeq") {
            None => None,
            Some(value) => Some(value.as_u64().ok_or_else(|| {
                DisplayReadError::invalid(
                    "session.display_read sinceSeq must be an unsigned integer when present, not null",
                )
            })?),
        };
        let accept_tool_frames = match object.get("acceptToolFrames") {
            None => false,
            Some(value) => value.as_bool().ok_or_else(|| {
                DisplayReadError::invalid(
                    "session.display_read acceptToolFrames must be a boolean when present",
                )
            })?,
        };
        Ok(Self {
            session,
            since_seq,
            accept_tool_frames,
        })
    }

    /// The host supplies a lookup result, never a newly created projection.
    pub fn read(&self, projection: Option<&DisplayProjection>) -> DisplayReadReply {
        self.read_at(projection, now_ms())
    }

    /// [`Self::read`], stamping tool frames with `now_ms`.
    pub fn read_at(&self, projection: Option<&DisplayProjection>, now_ms: u64) -> DisplayReadReply {
        match projection {
            Some(projection) if self.accept_tool_frames => {
                projection.display_read_with_tools(self.since_seq, now_ms)
            }
            Some(projection) => projection.display_read(self.since_seq),
            None => DisplayReadReply::Absent {
                absent: DisplayReadAbsent::NoRingAttached,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(text: &str, part_type: DisplayPartType) -> DisplayDeltaIngress {
        DisplayDeltaIngress {
            session_id: "ses".into(),
            message_id: "msg".into(),
            delta_text: text.into(),
            part_type,
        }
    }

    /// Gaps in a row with nothing shown between them reach the reader as one; a gap after
    /// shown text is still stored.
    #[test]
    fn consecutive_gaps_are_stored_once() {
        let mut ring = DisplayRing::new(16);
        for _ in 0..9 {
            ring.push(DisplayStreamFrame::DisplayGap);
        }
        let DisplayReadReply::Incremental { seq, frames } = ring.read(Some(0)) else {
            panic!("not incremental");
        };
        assert_eq!(seq, 1);
        assert_eq!(frames.len(), 1);
        ring.push(DisplayStreamFrame::Display {
            session_id: "ses".into(),
            message_id: "msg".into(),
            chunk: "text".into(),
            part_type: DisplayPartType::Text,
            rich_ui: None,
        });
        ring.push(DisplayStreamFrame::DisplayGap);
        ring.push(DisplayStreamFrame::DisplayGap);
        let DisplayReadReply::Incremental { seq, frames } = ring.read(Some(1)) else {
            panic!("not incremental");
        };
        assert_eq!(seq, 3);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1].frame, DisplayStreamFrame::DisplayGap);
    }

    #[test]
    fn batcher_concatenates_and_stale_timer_cannot_flush_a_completion_successor() {
        let batcher = DisplayBatcher::default();
        let first_timer = batcher
            .push(delta("hel", DisplayPartType::Text))
            .timer
            .unwrap();
        assert_eq!(
            batcher.push(delta("lo", DisplayPartType::Text)),
            DisplayPush::default()
        );
        assert_eq!(batcher.drain_session("ses")[0].chunk, "hello");

        let next_timer = batcher
            .push(delta("new", DisplayPartType::Text))
            .timer
            .unwrap();
        assert!(batcher.take_if_current(&first_timer).is_none());
        assert_eq!(batcher.take_if_current(&next_timer).unwrap().chunk, "new");
    }

    #[test]
    fn part_type_change_flushes_before_starting_the_next_chunk() {
        let batcher = DisplayBatcher::default();
        batcher.push(delta("why", DisplayPartType::Reasoning));
        let pushed = batcher.push(delta("answer", DisplayPartType::Text));
        assert_eq!(pushed.flushed[0].chunk, "why");
        assert_eq!(pushed.flushed[0].part_type, DisplayPartType::Reasoning);
        assert_eq!(batcher.pending_count(), 1);
    }

    fn chunk(text: &str) -> DisplayChunk {
        DisplayChunk {
            session_id: "ses_1".into(),
            message_id: "msg_1".into(),
            chunk: text.into(),
            part_type: DisplayPartType::Text,
        }
    }

    #[tokio::test]
    async fn two_display_waiters_both_wake_on_one_observe_display() {
        let projection = DisplayProjection::new(16).unwrap();
        let waiters: Vec<_> = (0..2)
            .map(|_| {
                let mut receiver = projection.subscribe_display();
                tokio::spawn(async move {
                    receiver.changed().await.expect("display sender alive");
                    *receiver.borrow_and_update()
                })
            })
            .collect();
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(waiters.iter().all(|waiter| !waiter.is_finished()));
        projection.observe_display(chunk("hello"));
        for waiter in waiters {
            let generation = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
                .await
                .expect("display waiter woke")
                .expect("display waiter task");
            assert_eq!(generation, 1);
        }
    }

    fn assert_signalled(receiver: &mut watch::Receiver<u64>, what: &str) {
        assert!(
            receiver.has_changed().unwrap(),
            "{what} did not signal display subscribers"
        );
        receiver.borrow_and_update();
    }

    #[test]
    fn every_display_ring_change_advances_the_display_signal() {
        let projection = DisplayProjection::new(16).unwrap();
        let mut receiver = projection.subscribe_display();
        projection.observe_display(chunk("a"));
        assert_signalled(&mut receiver, "observe_display");
        projection.observe_display_gap();
        assert_signalled(&mut receiver, "observe_display_gap");
        projection.end_display();
        assert_signalled(&mut receiver, "end_display");
        projection.reattach_display();
        assert_signalled(&mut receiver, "reattach_display");
        projection.start_display();
        receiver.borrow_and_update();
        projection.end_display();
        assert_signalled(&mut receiver, "run end closing the display turn");
    }

    #[test]
    fn display_calls_that_push_no_frame_do_not_signal() {
        let projection = DisplayProjection::new(16).unwrap();
        let receiver = projection.subscribe_display();
        projection.end_display();
        assert!(!receiver.has_changed().unwrap());
        projection.observe_display(chunk("a"));
        projection.end_display();
        let mut receiver = projection.subscribe_display();
        // Late chunks and gaps cannot reopen a finished turn without a start or reset.
        projection.observe_display(chunk("late"));
        projection.observe_display_gap();
        projection.end_display();
        assert!(!receiver.has_changed().unwrap());
        assert_eq!(*receiver.borrow_and_update(), 2);
    }

    #[test]
    fn display_read_contract_vectors_are_producer_real_from_a_driven_ring() {
        use crate::display_events::test_support::FakeSession;
        use serde_json::{json, Value};

        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/harness_display/contract_vectors.json"
        ))
        .unwrap();
        assert!(fixture["_provenance"]
            .as_str()
            .unwrap()
            .contains("Producer-real"));
        let vectors = fixture["vectors"].as_array().unwrap();
        assert_eq!(
            vectors.len(),
            6,
            "display-read vector census is contractual"
        );
        for field in [
            "absent",
            "seq",
            "frame",
            "frames",
            "windowExpired",
            "oldestSeq",
            "kind",
            "sessionID",
            "messageID",
            "chunk",
            "partType",
        ] {
            assert!(fixture["_delivery_notes"]["consumer_ping_fields"]
                .as_array()
                .unwrap()
                .contains(&json!(field)));
        }
        for vector in vectors {
            let session = vector.get("drive").map(|drive| {
                let session = FakeSession::new(drive["session"].as_str().unwrap(), drive["ringEntries"].as_u64().unwrap() as usize);
                for frame in drive["frames"].as_array().unwrap() {
                    let event_type = if frame["partType"] == "reasoning" { "reasoning_delta" } else { "text_delta" };
                    session.ingest(json!({"kind":"display","event":{"type":event_type,"mid":frame["messageID"],"delta":frame["chunk"]}}));
                    // A deterministic drain produces each fixture frame without waiting for a tick.
                    session.flush();
                }
                session
            });
            let request = DisplayReadRequest::parse(&vector["request"]).unwrap();
            let actual = request.read(session.as_ref().map(|session| session.projection.as_ref()));
            assert_eq!(
                crate::display_watch::display_body(&actual),
                serde_json::to_vec(&vector["reply"]).unwrap(),
                "producer-real reply bytes for {}",
                vector["name"]
            );
        }
    }

    #[test]
    fn display_read_is_repeatable_and_read_only_for_the_same_since_seq() {
        let projection = DisplayProjection::new(2).unwrap();
        projection.observe_display(chunk("paint"));
        let params = serde_json::json!({"session":"ses_1","sinceSeq":0});
        let request = DisplayReadRequest::parse(&params).unwrap();
        let first = request.read(Some(&projection));
        let second = request.read(Some(&projection));
        assert_eq!(first, second, "the phone, not the server, owns the cursor");
        assert!(
            matches!(first, DisplayReadReply::Incremental { seq: 1, frames } if frames.len() == 1)
        );
    }

    #[test]
    fn display_read_validates_only_the_read_keys_and_never_creates_a_ring() {
        use serde_json::json;
        for invalid in [
            json!(null),
            json!({}),
            json!({"session":""}),
            json!({"session":1}),
            json!({"session":"s","sinceSeq":null}),
            json!({"session":"s","sinceSeq":-1}),
            json!({"session":"s","sinceSeq":0.5}),
            json!({"session":"s","mystery":true}),
        ] {
            assert_eq!(
                DisplayReadRequest::parse(&invalid).unwrap_err().code,
                "invalid_request",
                "{invalid}"
            );
        }
        let params = json!({"session":"s","sinceSeq":0,"harness":"app","sessionId":"ambient","callerDirectory":"/w"});
        let request = DisplayReadRequest::parse(&params).unwrap();
        assert_eq!(
            request.read(None),
            DisplayReadReply::Absent {
                absent: DisplayReadAbsent::NoRingAttached
            }
        );
        assert!(DisplayProjection::new(0).is_err());
    }
}
