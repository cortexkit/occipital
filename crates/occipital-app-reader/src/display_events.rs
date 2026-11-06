//! Map the llm-runner subscription lanes without depending on a runner or transport crate.

use serde_json::Value;

use crate::display::DisplayPartType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayInput {
    Delta {
        /// The message's `mid`, or its `live_id` while its host hasn't stored it
        /// (`live_ids`). A runner never gives a `live_id` that equals a `mid`.
        mid: String,
        text: String,
        part_type: DisplayPartType,
    },
    Gap,
    Reset,
    TurnStart,
    /// The runner stored the message it streamed under `live_id` as `mid`
    /// (`message_started` carrying both).
    MessageStored {
        mid: String,
        live_id: String,
    },
    /// A run started, with its rich UI stamp as the apps read it
    /// (`{dialect, catalogDigest}`, built by `broca_entries::rich_ui`) when it carries
    /// one, and its `run_id` when the event names it.
    RunStart {
        rich_ui: Option<Value>,
        run_id: Option<String>,
    },
    /// The runner stored this message: its live words are final, and the stored copy
    /// replaces them once the transcript is read.
    MessageEnd {
        mid: String,
    },
    /// A run finished or paused, naming its `run_id` when the event does.
    TurnEnd {
        run_id: Option<String>,
    },
    /// A call of the message went to its tool.
    ToolStarted(ToolStart),
    /// A call of the message ended.
    ToolFinished(ToolEnd),
}

/// A runner's `tool_started`, as the display lane's tool frames need it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolStart {
    pub mid: String,
    /// The model's id for the call, as the stored part's `callId` carries it.
    pub call_id: String,
    pub tool: String,
    /// The call's arguments, when the runner sent them whole and they parse.
    pub arguments: Option<Value>,
    pub started_at: Option<u64>,
    pub call_key: Option<String>,
    pub schema_pin: Option<String>,
}

/// A runner's `tool_finished`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolEnd {
    pub call_id: String,
    /// `ok`, `error`, `cancelled`, `interrupted`, `denied`, or a value a newer
    /// runner added.
    pub outcome: String,
    pub output_bytes: Option<u64>,
    pub finished_at: Option<u64>,
}

#[derive(Clone, Copy)]
enum Effect {
    Text,
    Reasoning,
    Ignore,
    Gap,
    Reset,
    Start,
    RunStart,
    MessageEnd,
    End,
    ToolStarted,
    ToolFinished,
}

// Keep the serialized lane and event.type names together so changes can be checked mechanically.
// Tool argument deltas have no text/reasoning frame in the app's display protocol.
const EVENT_NAMES: &[(&str, &str, Effect)] = &[
    ("display", "text_delta", Effect::Text),
    ("display", "reasoning_delta", Effect::Reasoning),
    ("display", "tool_input_delta", Effect::Ignore),
    ("display", "display_gap", Effect::Gap),
    ("display", "display_reset", Effect::Reset),
    ("control", "message_started", Effect::Start),
    ("control", "message_finished", Effect::MessageEnd),
    ("control", "tool_started", Effect::ToolStarted),
    ("control", "tool_finished", Effect::ToolFinished),
    ("control", "run_started", Effect::RunStart),
    ("control", "run_finished", Effect::End),
    ("control", "run_paused", Effect::End),
];

/// The run an event names, when it names one.
fn run_id(event: &Value) -> Option<String> {
    event
        .get("run_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// Accept a subscription envelope with `kind` and `event` fields.
/// An event type this reader doesn't know, on either lane, resets the live display.
pub fn map(envelope: &Value) -> Option<DisplayInput> {
    let lane = envelope.get("kind").and_then(Value::as_str)?;
    // Delivery metadata has no display effect. Only an unknown typed live event
    // invalidates the live rows; a runner's own durable hints do not.
    if envelope.get("event").is_none() && envelope.get("runner_event").is_some_and(Value::is_string)
    {
        return None;
    }
    let event = &envelope["event"];
    let name = event.get("type").and_then(Value::as_str);
    let effect = EVENT_NAMES
        .iter()
        .find(|(kind, event_type, _)| *kind == lane && Some(*event_type) == name)
        .map(|(_, _, effect)| *effect);
    let Some(effect) = effect else {
        // The runner contract makes a reader decode a type it doesn't know, on the display
        // lane or the control lane, as `display_reset`. Newer runners may add types, and
        // dropping the live state of the step in flight is the one reaction that can never
        // leave the phone showing stale text.
        return Some(DisplayInput::Reset);
    };
    match effect {
        Effect::Text | Effect::Reasoning => {
            let text = event.get("delta").and_then(Value::as_str)?;
            if text.is_empty() {
                return None;
            }
            // A message its host hasn't stored yet streams under a `live_id`.
            let mid = event
                .get("mid")
                .or_else(|| event.get("live_id"))
                .and_then(Value::as_str)?;
            Some(DisplayInput::Delta {
                mid: mid.to_string(),
                text: text.to_string(),
                part_type: if matches!(effect, Effect::Reasoning) {
                    DisplayPartType::Reasoning
                } else {
                    DisplayPartType::Text
                },
            })
        }
        Effect::Ignore => None,
        Effect::Gap => Some(DisplayInput::Gap),
        Effect::Reset => Some(DisplayInput::Reset),
        Effect::Start => {
            let text = |name: &str| event.get(name).and_then(Value::as_str);
            match (text("mid"), text("live_id")) {
                (Some(mid), Some(live_id)) => Some(DisplayInput::MessageStored {
                    mid: mid.to_owned(),
                    live_id: live_id.to_owned(),
                }),
                _ => Some(DisplayInput::TurnStart),
            }
        }
        // A malformed stamp is dropped, so the text streams unstamped, as a malformed
        // stamp on a stored row is.
        Effect::RunStart => Some(DisplayInput::RunStart {
            rich_ui: event.get("rich_ui").and_then(crate::broca_entries::rich_ui),
            run_id: run_id(event),
        }),
        Effect::MessageEnd => {
            let mid = event.get("mid").and_then(Value::as_str)?;
            Some(DisplayInput::MessageEnd {
                mid: mid.to_string(),
            })
        }
        Effect::End => Some(DisplayInput::TurnEnd {
            run_id: run_id(event),
        }),
        // A tool event missing a member it requires is dropped, as a text delta
        // without its `mid` is: the stored page still shows the call.
        Effect::ToolStarted => {
            let text = |name: &str| event.get(name).and_then(Value::as_str).map(str::to_owned);
            let arguments = match event.get("arguments_truncated").and_then(Value::as_bool) {
                Some(true) => None,
                _ => event
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|arguments| serde_json::from_str(arguments).ok()),
            };
            Some(DisplayInput::ToolStarted(ToolStart {
                mid: text("mid")?,
                call_id: text("tool_call_id")?,
                tool: text("name")?,
                arguments,
                started_at: event.get("started_at").and_then(Value::as_u64),
                call_key: text("call_key").filter(|key| !key.is_empty()),
                schema_pin: text("schema_pin").filter(|pin| !pin.is_empty()),
            }))
        }
        Effect::ToolFinished => Some(DisplayInput::ToolFinished(ToolEnd {
            call_id: event
                .get("tool_call_id")
                .and_then(Value::as_str)?
                .to_owned(),
            outcome: event.get("outcome").and_then(Value::as_str)?.to_owned(),
            output_bytes: event.get("output_bytes").and_then(Value::as_u64),
            finished_at: event.get("finished_at").and_then(Value::as_u64),
        })),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;

    use super::*;
    use crate::display::{DisplayBatcher, DisplayDeltaIngress, DisplayProjection, DisplayTimer};

    pub struct FakeSession {
        pub session: String,
        pub batcher: DisplayBatcher,
        pub projection: Arc<DisplayProjection>,
    }

    impl FakeSession {
        pub fn new(session: &str, capacity: usize) -> Self {
            Self {
                session: session.into(),
                batcher: DisplayBatcher::default(),
                projection: Arc::new(DisplayProjection::new(capacity).unwrap()),
            }
        }

        pub fn ingest(&self, envelope: Value) -> Option<DisplayTimer> {
            match map(&envelope)? {
                DisplayInput::Delta {
                    mid,
                    text,
                    part_type,
                } => {
                    let pushed = self.batcher.push(DisplayDeltaIngress {
                        session_id: self.session.clone(),
                        message_id: self.projection.message_key(&mid),
                        delta_text: text,
                        part_type,
                    });
                    for chunk in pushed.flushed {
                        self.projection.observe_display(chunk);
                    }
                    pushed.timer
                }
                DisplayInput::Gap => {
                    self.batcher.discard_session(&self.session);
                    self.projection.observe_display_gap();
                    None
                }
                DisplayInput::Reset => {
                    self.batcher.discard_session(&self.session);
                    self.projection.reattach_display();
                    None
                }
                DisplayInput::TurnStart => {
                    self.projection.start_display();
                    None
                }
                DisplayInput::MessageStored { mid, live_id } => {
                    self.flush();
                    if !self.projection.store_message(&mid, &live_id) {
                        self.projection.start_display();
                    }
                    None
                }
                DisplayInput::RunStart { rich_ui, run_id } => {
                    self.projection.start_run(rich_ui, run_id);
                    None
                }
                DisplayInput::MessageEnd { mid } => {
                    self.flush();
                    self.projection.end_message(&mid);
                    None
                }
                DisplayInput::TurnEnd { run_id } => {
                    self.flush();
                    self.projection.end_run(run_id.as_deref());
                    None
                }
                DisplayInput::ToolStarted(start) => {
                    self.flush();
                    self.projection.start_tool(&self.session, start);
                    None
                }
                DisplayInput::ToolFinished(end) => {
                    self.flush();
                    self.projection.end_tool(end);
                    None
                }
            }
        }

        pub fn flush(&self) {
            for chunk in self.batcher.drain_session(&self.session) {
                self.projection.observe_display(chunk);
            }
        }

        pub fn fire(&self, timer: &DisplayTimer) {
            if let Some(chunk) = self.batcher.take_if_current(timer) {
                self.projection.observe_display(chunk);
            }
        }

        /// `session.display_read`'s request and reply for this session at `now`,
        /// as a served read or a display watch notice makes them.
        pub fn read(&self, since: Option<u64>, tools: bool, now: u64) -> (Value, Value) {
            let mut params = serde_json::json!({"session": self.session});
            if let Some(since) = since {
                params["sinceSeq"] = serde_json::json!(since);
            }
            if tools {
                params["acceptToolFrames"] = serde_json::json!(true);
            }
            let request = crate::display::DisplayReadRequest::parse(&params).unwrap();
            let reply = request.read_at(Some(&self.projection), now);
            (
                serde_json::json!({"method": "session.display_read", "params": params}),
                serde_json::json!({"result": reply}),
            )
        }

        pub fn ingest_timed(self: &Arc<Self>, envelope: Value) {
            let Some(timer) = self.ingest(envelope) else {
                return;
            };
            let session = Arc::clone(self);
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                session.fire(&timer);
            });
        }
    }
}

/// A published sample set: each file's bytes and a `SHA256SUMS` over them. With
/// `OCCIPITAL_WRITE_SAMPLES` set to a folder the set is written there; otherwise
/// each committed file must match byte for byte, so a change to what the reader
/// serves fails until the set is regenerated and given a new tag.
#[cfg(test)]
pub(crate) fn keep_sample_set(set: &str, files: &[(String, Value)]) {
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;

    let mut sums = String::new();
    let mut bytes_of = Vec::new();
    for (name, value) in files {
        let bytes = serde_json::to_vec(value).unwrap();
        sums.push_str(&format!("{:x}  {name}\n", Sha256::digest(&bytes)));
        bytes_of.push((name.clone(), bytes));
    }
    if let Ok(folder) = std::env::var("OCCIPITAL_WRITE_SAMPLES") {
        let root = PathBuf::from(folder).join(set);
        std::fs::create_dir_all(&root).unwrap();
        for (name, bytes) in &bytes_of {
            std::fs::write(root.join(name), bytes).unwrap();
        }
        std::fs::write(root.join("SHA256SUMS"), &sums).unwrap();
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors")
        .join(set);
    for (name, bytes) in &bytes_of {
        let committed = std::fs::read(root.join(name)).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&committed),
            String::from_utf8_lossy(bytes),
            "{name}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.join("SHA256SUMS")).unwrap(),
        sums
    );
}

#[cfg(test)]
mod live_ids;

#[cfg(test)]
mod tool_frames;

#[cfg(test)]
mod tests {
    use super::test_support::FakeSession;
    use super::*;
    use crate::display::{DisplayReadReply, DisplayStreamFrame};
    use serde_json::json;

    fn delta(mid: &str, text: &str) -> Value {
        json!({"kind":"display", "event":{"type":"text_delta", "mid":mid,
            "message_id":"", "delta":text}})
    }

    /// Text streamed in a stamped run carries the run's stamp, in a cold read and in the
    /// incremental frames, in the phone's shape. A later run without a stamp streams
    /// unstamped.
    #[test]
    fn a_stamped_runs_text_frames_carry_its_stamp_until_the_run_ends() {
        let session = FakeSession::new("ses", 16);
        let raw = json!({"dialect":"1.0", "catalog_digest":"a".repeat(64)});
        let phone = json!({"dialect":"1.0", "catalogDigest":"a".repeat(64)});
        session.ingest(
            json!({"kind":"control", "event":{"type":"run_started", "run_id":"r1", "rich_ui":raw}}),
        );
        session.ingest(delta("m1", "hello"));
        session.flush();
        let cold = serde_json::to_value(session.projection.display_read(None)).unwrap();
        assert_eq!(cold["frame"]["richUi"], phone, "{cold}");
        let frames = serde_json::to_value(session.projection.display_read(Some(0))).unwrap();
        assert_eq!(frames["frames"][0]["frame"]["richUi"], phone, "{frames}");
        session.ingest(json!({"kind":"control", "event":{"type":"run_finished", "run_id":"r1"}}));
        session.ingest(json!({"kind":"control", "event":{"type":"run_started", "run_id":"r2"}}));
        session.ingest(delta("m2", "plain"));
        session.flush();
        let cold = serde_json::to_value(session.projection.display_read(None)).unwrap();
        assert_eq!(cold["frame"]["chunk"], "plain", "{cold}");
        assert!(cold["frame"].get("richUi").is_none(), "{cold}");
    }

    #[test]
    fn an_unknown_display_event_type_resets_the_display() {
        let session = FakeSession::new("ses", 16);
        let stale = session.ingest(delta("m1", "hel")).unwrap();
        session.ingest(json!({"kind":"display", "event":{"type":"display_rewind"}}));
        assert_eq!(session.batcher.pending_count(), 0);
        assert_eq!(
            session.projection.display_read(Some(0)),
            DisplayReadReply::Incremental {
                seq: 1,
                frames: vec![crate::display::SequencedDisplayFrame {
                    seq: 1,
                    frame: DisplayStreamFrame::DisplayGap
                }],
            }
        );
        session.fire(&stale);
        let next = session.ingest(delta("m1", "new")).unwrap();
        session.fire(&next);
        assert_eq!(
            serde_json::to_value(session.projection.display_read(None)).unwrap(),
            json!({"seq":2,"frame":{"kind":"display","sessionID":"ses","messageID":"m1","chunk":"new","partType":"text"}})
        );
        session.ingest(json!({"kind":"display", "event":{}}));
        assert_eq!(map(&json!({"kind":"display"})), Some(DisplayInput::Reset));
        assert_eq!(
            serde_json::to_value(session.projection.display_read(None)).unwrap(),
            json!({"seq":3,"frame":{"kind":"display_gap"}})
        );
    }

    #[test]
    fn an_unknown_control_event_type_also_resets_the_display() {
        assert_eq!(
            map(&json!({"kind":"control", "event":{"type":"run_rewound"}})),
            Some(DisplayInput::Reset)
        );
        let session = FakeSession::new("ses", 16);
        session.ingest(delta("m1", "hel"));
        session.ingest(json!({"kind":"control", "event":{"type":"run_rewound"}}));
        assert_eq!(session.batcher.pending_count(), 0);
    }

    #[test]
    fn runner_delivery_metadata_does_not_reset_live_rows() {
        assert_eq!(
            map(
                &serde_json::json!({"kind":"control","cursor":{"seq":1},"runner_event":"delivered"})
            ),
            None
        );
        assert_eq!(
            map(
                &serde_json::json!({"kind":"control","runner_event":"run_started","event":{"type":"run_started","run_id":"r"}})
            ),
            Some(DisplayInput::RunStart {
                rich_ui: None,
                run_id: Some("r".into())
            })
        );
    }

    #[test]
    fn display_deltas_are_keyed_by_mid_even_when_the_provider_message_id_is_empty() {
        let session = FakeSession::new("ses", 16);
        session.ingest(delta("m1", "hi"));
        session.ingest(delta("m2", "yo"));
        session.flush();
        let reply = serde_json::to_value(session.projection.display_read(Some(0))).unwrap();
        assert_eq!(reply["frames"].as_array().unwrap().len(), 2);
        for (index, mid, text) in [(0, "m1", "hi"), (1, "m2", "yo")] {
            assert_eq!(reply["frames"][index]["frame"]["messageID"], mid);
            assert_eq!(reply["frames"][index]["frame"]["chunk"], text);
        }
    }

    #[test]
    fn a_display_delta_with_an_empty_mid_is_still_delivered() {
        let session = FakeSession::new("ses", 16);
        session.ingest(delta("", "x"));
        session.flush();
        assert_eq!(
            serde_json::to_value(session.projection.display_read(None)).unwrap(),
            json!({"seq":1,"frame":{"kind":"display","sessionID":"ses","messageID":"","chunk":"x","partType":"text"}})
        );
    }

    #[test]
    fn contract_lanes_control_turns_and_ignore_tool_arguments() {
        let session = FakeSession::new("ses", 16);
        session.ingest(delta("m", "hi"));
        session.ingest(json!({"kind":"control","event":{"type":"run_finished"}}));
        session.ingest(delta("m", "late"));
        session.flush();
        assert_eq!(
            serde_json::to_value(session.projection.display_read(None)).unwrap()["frame"]["kind"],
            "display_end"
        );
        for start in ["run_started", "message_started"] {
            session.ingest(json!({"kind":"control","event":{"type":start}}));
            session.ingest(delta("m", "next"));
            session.flush();
            session.ingest(json!({"kind":"control","event":{"type":"run_paused"}}));
        }
        for name in [
            "tool_input_delta",
            "message_finished",
            "tool_started",
            "tool_finished",
        ] {
            let lane = if name == "tool_input_delta" {
                "display"
            } else {
                "control"
            };
            assert_eq!(map(&json!({"kind":lane,"event":{"type":name}})), None);
        }
        assert_eq!(
            map(&json!({"kind":"control","event":{"type":"future"}})),
            Some(DisplayInput::Reset)
        );
        assert_eq!(
            map(&json!({"kind":"display","event":{"type":"display_gap"}})),
            Some(DisplayInput::Gap)
        );
        assert_eq!(
            map(&json!({"kind":"display","event":{"type":"display_reset"}})),
            Some(DisplayInput::Reset)
        );
        assert_eq!(
            map(
                &json!({"kind":"display","event":{"type":"reasoning_delta","mid":"m","delta":"why"}})
            ),
            Some(DisplayInput::Delta {
                mid: "m".into(),
                text: "why".into(),
                part_type: DisplayPartType::Reasoning
            })
        );
    }

    /// A stored message ends its live words with `display_end`, never a gap, and the
    /// run's next message streams on. An end for a message whose words a later message
    /// already replaced sends nothing, since it would end the new one.
    #[test]
    fn a_finished_message_ends_its_words_and_the_next_message_streams_on() {
        let session = FakeSession::new("ses", 32);
        let kinds = |session: &FakeSession| -> Vec<String> {
            serde_json::to_value(session.projection.display_read(Some(0))).unwrap()["frames"]
                .as_array()
                .unwrap()
                .iter()
                .map(|frame| frame["frame"]["kind"].as_str().unwrap().to_owned())
                .collect()
        };
        session.ingest(json!({"kind":"control","event":{"type":"run_started"}}));
        session.ingest(delta("step-one", "first answer"));
        session
            .ingest(json!({"kind":"control","event":{"type":"message_finished","mid":"step-one"}}));
        // Step two starts in the same notice as step one's end.
        session.ingest(delta("step-two", "second"));
        session
            .ingest(json!({"kind":"control","event":{"type":"message_finished","mid":"step-one"}}));
        session.flush();
        assert_eq!(kinds(&session), ["display", "display_end", "display"]);
        session
            .ingest(json!({"kind":"control","event":{"type":"message_finished","mid":"step-two"}}));
        session.ingest(json!({"kind":"control","event":{"type":"run_finished"}}));
        assert_eq!(
            kinds(&session),
            [
                "display",
                "display_end",
                "display",
                "display_end",
                "display_end"
            ]
        );
        // A message that showed no words on the live display ends nothing.
        let quiet = FakeSession::new("quiet", 8);
        quiet.ingest(json!({"kind":"control","event":{"type":"run_started"}}));
        quiet.ingest(json!({"kind":"control","event":{"type":"message_finished","mid":"user"}}));
        let read = serde_json::to_value(quiet.projection.display_read(Some(0))).unwrap();
        assert!(
            read["frames"].as_array().is_none_or(Vec::is_empty) && read.get("frame").is_none(),
            "a message with no live words sent a frame: {read}"
        );
    }

    fn live_delta(live_id: &str, text: &str) -> Value {
        json!({"kind":"display", "event":{"type":"text_delta", "live_id":live_id, "delta":text}})
    }

    /// A runner can report the next run's start before the previous run's end,
    /// as Thalamus's runner did when a held send started a run in the same pass
    /// that saw the last one finish. The late end names the earlier run, so the
    /// running one keeps its stamp, its running calls and its live words, and its
    /// own end still ends it. An end naming no run, or a run with no id, ends it.
    #[test]
    fn a_late_end_of_an_earlier_run_leaves_the_running_one_going() {
        let session = FakeSession::new("ses", 32);
        let raw = json!({"dialect":"1.0", "catalog_digest":"a".repeat(64)});
        session.ingest(
            json!({"kind":"control", "event":{"type":"run_started", "run_id":"new", "rich_ui":raw}}),
        );
        session.ingest(json!({"kind":"control", "event":{"type":"run_finished",
            "run_id":"old", "reason":"completed"}}));
        session.ingest(
            json!({"kind":"control", "event":{"type":"tool_started", "mid":"m1",
            "tool_call_id":"c1", "name":"bash", "arguments":"{}"}}),
        );
        session.ingest(delta("m1", "still going"));
        session.flush();
        let cold = serde_json::to_value(session.projection.display_read(None)).unwrap();
        assert_eq!(cold["frame"]["chunk"], "still going", "{cold}");
        assert!(cold["frame"].get("richUi").is_some(), "{cold}");
        let kinds = |session: &FakeSession| -> Vec<String> {
            frames(session).into_iter().map(|(kind, _)| kind).collect()
        };
        assert_eq!(kinds(&session), ["display"]);
        session.ingest(json!({"kind":"control", "event":{"type":"run_finished",
            "run_id":"new", "reason":"completed"}}));
        assert_eq!(kinds(&session), ["display", "display_end"]);

        for end in [
            json!({"type":"run_finished"}),
            json!({"type":"run_paused", "run_id":"other"}),
        ] {
            let session = FakeSession::new("ses", 32);
            let start = if end.get("run_id").is_some() {
                json!({"type":"run_started"})
            } else {
                json!({"type":"run_started", "run_id":"r1"})
            };
            session.ingest(json!({"kind":"control", "event":start}));
            session.ingest(delta("m1", "words"));
            session.flush();
            session.ingest(json!({"kind":"control", "event":end}));
            assert_eq!(kinds(&session), ["display", "display_end"], "{end}");
        }
    }

    fn frames(session: &FakeSession) -> Vec<(String, String)> {
        serde_json::to_value(session.projection.display_read(Some(0))).unwrap()["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|frame| {
                let frame = &frame["frame"];
                (
                    frame["kind"].as_str().unwrap().to_owned(),
                    frame["messageID"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn a_live_id_keys_deltas_and_a_store_naming_both_is_its_own_input() {
        assert_eq!(
            map(&live_delta("live-1", "hi")),
            Some(DisplayInput::Delta {
                mid: "live-1".into(),
                text: "hi".into(),
                part_type: DisplayPartType::Text
            })
        );
        let both = json!({"kind":"display", "event":{"type":"text_delta", "mid":"m1",
            "live_id":"live-1", "delta":"hi"}});
        assert!(matches!(map(&both), Some(DisplayInput::Delta { mid, .. }) if mid == "m1"));
        assert_eq!(
            map(
                &json!({"kind":"control", "event":{"type":"message_started", "mid":"m1",
                "live_id":"live-1"}})
            ),
            Some(DisplayInput::MessageStored {
                mid: "m1".into(),
                live_id: "live-1".into()
            })
        );
        assert_eq!(
            map(&json!({"kind":"control", "event":{"type":"message_started", "mid":"m1"}})),
            Some(DisplayInput::TurnStart)
        );
    }

    /// A message streamed under a `live_id` keeps that key to its end: words
    /// the runner sends under its `mid` after storing it, as an attach or reset
    /// replay does, show under the `live_id`, and its `message_finished` ends it.
    /// Apps hand live words over to the stored copy under one unchanging key.
    #[test]
    fn a_live_id_message_keeps_its_key_until_its_end() {
        let session = FakeSession::new("ses", 32);
        session.ingest(json!({"kind":"control","event":{"type":"run_started"}}));
        session.ingest(live_delta("live-1", "Pi "));
        session.ingest(live_delta("live-1", "answers"));
        session.ingest(json!({"kind":"control","event":{"type":"message_started",
            "mid":"m1", "live_id":"live-1"}}));
        session.ingest(delta("m1", " late"));
        session.ingest(json!({"kind":"control","event":{"type":"message_finished","mid":"m1"}}));
        session.flush();
        assert_eq!(
            frames(&session),
            [
                ("display".to_owned(), "live-1".to_owned()),
                ("display".to_owned(), "live-1".to_owned()),
                ("display_end".to_owned(), String::new()),
            ]
        );
        // The next message, named at creation, streams under its own `mid`.
        session.ingest(delta("m2", "next"));
        session.flush();
        assert_eq!(frames(&session)[3], ("display".to_owned(), "m2".to_owned()));
    }

    /// A store naming a `live_id` whose words weren't shown renames nothing. It is
    /// the message's start, as a reader that didn't ask for `live_id`s gets it
    /// before the words under the `mid`: after a run ended, those words show.
    #[test]
    fn a_store_for_a_live_id_never_shown_is_the_messages_start() {
        let session = FakeSession::new("ses", 32);
        session.ingest(json!({"kind":"control","event":{"type":"run_started"}}));
        session.ingest(delta("m0", "done"));
        session.ingest(json!({"kind":"control","event":{"type":"run_finished"}}));
        assert_eq!(frames(&session).len(), 2);
        session.ingest(json!({"kind":"control","event":{"type":"message_started",
            "mid":"m9", "live_id":"live-9"}}));
        session.ingest(delta("m9", "words"));
        session.ingest(json!({"kind":"control","event":{"type":"message_finished","mid":"m9"}}));
        assert_eq!(
            frames(&session)[2..],
            [
                ("display".to_owned(), "m9".to_owned()),
                ("display_end".to_owned(), String::new()),
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn contract_display_deltas_coalesce_through_the_batch_timer() {
        let session = std::sync::Arc::new(FakeSession::new("ses_display", 16));
        for text in ["read ", "at ", "cadence"] {
            session.ingest_timed(delta("msg_display", text));
        }
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(299)).await;
        assert!(matches!(
            session.projection.display_read(None),
            DisplayReadReply::Absent { .. }
        ));
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            serde_json::to_value(session.projection.display_read(Some(0))).unwrap(),
            json!({"seq":1,"frames":[{"seq":1,"frame":{"kind":"display","sessionID":"ses_display","messageID":"msg_display","chunk":"read at cadence","partType":"text"}}]})
        );
    }
}
