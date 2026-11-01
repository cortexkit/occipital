//! Map the llm-runner subscription lanes without depending on a runner or transport crate.

use serde_json::Value;

use crate::display::DisplayPartType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayInput {
    Delta {
        mid: String,
        text: String,
        part_type: DisplayPartType,
    },
    Gap,
    Reset,
    TurnStart,
    /// A run started, with its rich UI stamp in the phone's shape when it carries one.
    RunStart {
        rich_ui: Option<Value>,
    },
    /// The runner stored this message: its live words are final, and the stored copy
    /// replaces them once the transcript is read.
    MessageEnd {
        mid: String,
    },
    TurnEnd,
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
            let mid = event.get("mid").and_then(Value::as_str)?;
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
        Effect::Start => Some(DisplayInput::TurnStart),
        // A malformed stamp is dropped, so the text streams unstamped, as a malformed
        // stamp on a stored row is.
        Effect::RunStart => Some(DisplayInput::RunStart {
            rich_ui: event.get("rich_ui").and_then(crate::broca_entries::rich_ui),
        }),
        Effect::MessageEnd => {
            let mid = event.get("mid").and_then(Value::as_str)?;
            Some(DisplayInput::MessageEnd {
                mid: mid.to_string(),
            })
        }
        Effect::End => Some(DisplayInput::TurnEnd),
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
                        message_id: mid,
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
                DisplayInput::RunStart { rich_ui } => {
                    self.projection.start_run(rich_ui);
                    None
                }
                DisplayInput::MessageEnd { mid } => {
                    self.flush();
                    self.projection.end_message(&mid);
                    None
                }
                DisplayInput::TurnEnd => {
                    self.flush();
                    self.projection.end_display();
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
            Some(DisplayInput::RunStart { rich_ui: None })
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
