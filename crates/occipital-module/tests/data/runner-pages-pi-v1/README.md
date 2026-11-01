# runner-pages-pi-v1

A Pi head's runner replies, recorded from real Pi 1.4.0 (npm
`@earendil-works/pi-coding-agent@1.4.0`) by the harness module's real-Pi test
(`real_pi_head_attaches_runs_plan_tools_and_takes_a_delivery`) on 2026-11-01. The
head runs on the `thalamus-pi` module with `harness: "pi"`. The model is a local
stand-in provider, so every reply's text is `PROBE_DONE`.

The session has three turns:
- **`TOOL_TURN`:** the model calls the plan's `bash` tool, gets `PROBE_TOOL` back, and answers.
- **`DELIVERED_TURN`:** a message the module delivers.
- **`CORE_TURN`:** a turn core sends with `session.send`.

| File | Request | What it shows |
| --- | --- | --- |
| `01-session-read.json` | `session.read {}` | Every entry, ordinals 0 to 8. Ordinal 0 is the system entry Pi writes when a session starts; ordinals 2 and 3 are the tool call and its result as separate Pi messages. `message` is Pi's journal row as served (`served_body`). |
| `02-session-read-display-facts.json` | `session.read {include_display_facts: true}` | The same entries, each with its display facts beside it. Pi stores no tool times, so its tool calls carry only their ids. |
| `03-session-head.json` | `session.head {}` | The head: ordinals 0 to 8, the session's model, usage and the sender grant. |
| `04-session-history.json` | `session.history {limit: 200}` | The same entries as `session.read`, as raw rows. |
| `05-session-history-display.json` | `session.history {limit: 200, form: "display"}` | The same entries as display rows, built by the contract's `display::pi`. |
| `06-session-history-before.json` | `session.history {before_mid: <newest>, limit: 1}` | One page back from the newest entry. |
| `07-subscribe-live-ids.json` | `session.subscribe {live_ids: true}`, opened before `CORE_TURN` | The reply's words under `live-…` before Pi stores it, then `message_started` naming both the stored `mid` and that `live_id`, then the run's end. |
| `08-subscribe-plain.json` | `session.subscribe {}`, opened at the same point | The same turn for a reader that didn't ask: `message_started` names both, and the words follow under the stored `mid`. |

Each `session.*` file is the reply's result, without the daemon's `{"result": …}`
envelope. The two subscribe files are the frames the runner hands to the daemon,
each one `STREAM_DATA` frame's body, in order, as one JSON array.

Not in this set: a shell run (`!`) and an entry edited by a Pi context edit. This
test doesn't drive either. Their rows follow the same contract rules
(`served_body` and `display::pi`).

The system entry at ordinal 0 holds Pi's own default prompt sections, with the
build server's temporary install path. The model was given the plan's system text
instead. Nothing else was edited, and the files are byte-identical to what the
test wrote.
