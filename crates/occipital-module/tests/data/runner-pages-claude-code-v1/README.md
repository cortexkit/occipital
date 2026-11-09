# runner-pages-claude-code-v1

What an app reader (Occipital) is served of real Claude Code 2.1.318 sessions by
Thalamus's runner (module `thalamus-harness`, harness `claude-code`, llm-runner 0.8.31).
Recorded in one run of
`runner::tests_claude_code_real::pages::real_claude_code_records_the_pages_apps_read`
(an ignored test: Claude Code from `probes/claude-code-linux/fetch.sh` in
`THALAMUS_CLAUDE_CODE_BIN`, `python3`, and `THALAMUS_RECORD_DIR` naming a new folder),
on Thalamus `ae9ab3d00` plus the recorder and four runner fixes it relies on:
descriptors reads of Claude Code sessions, `unanswered` marks on Claude Code calls,
`cancelled` as OpenCode 2's live outcome for a call a caller stopped, and
`unanswered: cancelled` on a call a caller's stop cut off.

The model is a stand-in. Every case but 6 runs through the module's own gateway, as a
module start wires it, with the gateway tests' stand-in Anthropic provider behind it,
answering each model request with the answer the case scripts (message ids such as
`msg_text_reasoned` are the script's; usage counts are the stand-in's). Answers stream one
event every 40 ms, as a model writing them does. Case 6 runs on the spike's stand-in in
the gateway's place, as the upgrade's own test does, so its prompts name the stand-in's
scripts (`[[script:cc-echo]]`). The plan's one tool is `echo`, served by a stand-in tool
provider through the module's tool lane.

Each test folder's path is written as `/scratch`. The recorder refuses to write a set
holding an email address, or a home or temporary folder of the machine that recorded it
(a macOS or Linux home folder, or macOS's per-user temporary folder). `SHA256SUMS` lists
every file but itself (`sha256sum -c SHA256SUMS`).

## In every folder

Each folder is one Claude Code session. Every request is the session owner's
(`prefrontal-core`) unless a file name says `as-<module>`. Each file is the reply's body
as the wire carries it; a refused request's file would hold the body of its error frame.

| File | Request |
|---|---|
| `session-read-inline.json` | `session.read {from_ordinal: 1, limit: 100, include_display_facts: true}` (default media mode, inline) |
| `session-read-descriptors.json` | the same, `media: "descriptors"` |
| `session-read-media-refs.json` | the same, `media: "media_refs"` |
| `session-history-inline.json` | `session.history {limit: 100}` |
| `session-history-descriptors.json` | the same, `media: "descriptors"` |
| `session-history-media-refs.json` | the same, `media: "media_refs"` |
| `session-history-display.json` | `session.history {limit: 100, form: "display"}`. Display history takes no `media` (the contract: display rows describe media and never inline it), and no `include_display_facts`: its rows carry those facts themselves. |
| `session-head.json` | `session.head`, at the case's end |
| `subscribe-frames.json` | every frame of the `session.subscribe {}` (live) opened before the case's first send |

`subscribe-frames.json` is `{"subscriptions": [{"params", "frames", "ended"}]}`. Each
frame is a STREAM_DATA body as the wire sends it: `kind` and `seq`, then either a typed
`event` or a runner record (`runner_event`: `admitted`, `send_accepted`, `delivered`,
...), and a `cursor` on durable frames. When the runner ends a stream (the session's
history moved to a new line under it: a `display_gap` frame, then the end), `ended` holds
the body of the error frame the wire ends it with,
`{"error": {"code": "transient", "message"}}`, and the next entry is the subscription a
reader opens again, live, which starts with a `display_reset` when a reply is in
flight. `{"closed": ...}` means the module stopped and its connection closed.

The module stores a Claude Code answer only once Claude Code has journalled the model
request after it, which carries the answer and its calls' results (ARCHITECTURE.md,
Claude Code). So an answer's `message_started`, its calls' `tool_started` and its
`message_finished` go out together when it is stored, after its calls ran and after
the text and reasoning deltas streamed live; a run's `run_finished` goes out once its
last rows are stored.

## Cases

### 01-text-and-reasoning

Two runs. `msg_text_reasoned` thinks aloud, then answers. `msg_text_unshown` came back
with a thinking block of no text (only its signature), as Anthropic sends thinking it
doesn't show: Claude Code stores it as `{"type": "thinking", "thinking": ""}`, the reads
keep it, and its display row keeps a `reasoning` part whose `text` is `""`. The frames
show the first answer's `reasoning_delta`s and both answers' `text_delta`s.

### 02-tool-outcomes

One session with a step limit of two (`stop_when.max_steps: 2`), one run per outcome:

| Run (`send_id`) | Call | `run.tool_calls` | Display row / tool time | Live `tool_finished` |
|---|---|---|---|---|
| `tools-succeeded` | `echo`, answered | `succeeded` | no mark | `ok` |
| `tools-failed` | `Bash`, a tool the session doesn't offer: Claude Code answers it with an error | `failed` | no mark | `error` |
| `tools-step-limit` | `echo` twice: the second call, on the run's last allowed step, is refused unrun at the module's gate (its result reads `not_executed_at_step_limit`) and the run ends `max_steps` | `succeeded`, `not_executed` | the cut call: `unanswered: "not_executed"` | `ok`, `denied` |
| `tools-stopped` | `echo`, held by its provider, then `run.cancel` (`run-cancel.json`) | `failed` | `unanswered: "cancelled"` | `cancelled` |

`run.tool_calls` has no `cancelled` outcome: a dispatched call whose result is an error is
`failed`. `session-read-after-stop.json` is the read right after the stop: the answer that
made the stopped call is stored (its tool time already `unanswered: "cancelled"`), its
result not yet. A further run (`tools-after-stop`) sends the result Claude Code wrote for
the stopped call ("The user doesn't want to proceed with this tool use..."), which the
final pages hold. Also `run-tool-calls-<run>.json` and `run-status-<run>.json` for each of
the four runs. The session's history moved to a new line once, between the step-limit run
and the stopped run, so the first subscription ends with a `display_gap` and the one
opened again starts with a `display_reset`.

### 03-photo-and-file

A PNG photo and a PDF file, each uploaded as Occipital uploads (`session.upload_begin`,
`_append`, `_commit`) and sent with a prompt (`content: [{type: "upload"}]`).
`session-media-read-photo.json` is `session.media_read {sha256, offset: 0, length}` of the
photo: its bytes, base64, read back from Claude Code's session file. The inline reads and
the display rows put a text marker `[media not shown: kind=... mime=... bytes=... sha256=...
reason=not_requested]` in each file's place; the `media_refs` and `descriptors` reads keep
the `image` and `document` blocks with a `media_ref` (`kind`, `mime`, `byte_size`,
`sha256`, and the photo's `width` and `height`) in place of the base64 data. Claude Code
adds a text block of its own after the photo, `[Image: source: <path>]`, naming where it
saved the image; the path is under the test folder (written `/scratch`), and its project
folder name, which Claude Code makes from the project's path, is written `-scratch-...`.

### 04-plan-and-turn-notes

The plan names `magic-context` as a sender with a note template. After a first run the
owner changes the plan's system text (`session-refresh.json`: `session.refresh` with
`policy: "notes_until_rebuild"` and a `system_note`), so the change waits for its reconcile
point and the next run's first model request carries a plan note. While that run's `echo`
call runs, `magic-context` sends the template's note listing `turn_system`
(`session-send-turn-note.json`), which goes out as a system message after the call's
result. The turn note is served as a notice (`notices`, `delivered_as: "turn_system"`) on
the message holding the result, and announced live (`notice_delivered`). A plan note is
a runner record that only the model's requests carry: no read, row or frame shows it, so
readers never see one.

### 05-model-and-effort-switch

Three runs: `claude-haiku-5-5` at the default effort, the same model at `variant: "high"`,
then `claude-opus-4-5` at `variant: "low"`. `run-status-<n>.json` names each run's model
in its usage; `session-head-after-run-<n>.json` follows the head. The session's history
moved to a new line once, when run 3 switched the model (`history_version` 0 after runs 1
and 2, 1 after run 3); the effort switch alone didn't move it. The first subscription ends
with a `display_gap` there, and a reader subscribes again.

### 06-legacy-id-upgrade

A session stored as the build before provider message ids stored it (each answer under
its content hash), as the first test head (CCTEST) was: an `echo` run and a plain run.
Then the module is deployed (restarted) with ids and takes the session up again, which
moves its history to provider ids in one write; a second restart changes nothing.

| File | When |
|---|---|
| `session-head-before-upgrade.json`, `session-read-inline-before-upgrade.json`, `session-history-display-before-upgrade.json` | before the deploy |
| `session-head-after-upgrade.json` | after the deploy and the upgrade |
| `session-head-after-second-restart.json` | after a further restart |

`history_version` moves exactly once, from 0 to 1, and stays at 1 across the second
restart. `incarnation` doesn't move: the upgrade is a history rewrite on the same log, not
a new one. The head's `head` cursor moves on with each event, as always. The entries keep
their bodies and numbers; the answers' ids change from content hashes to the provider's
`msg_...` ids, and the user and tool-result messages, which have no provider id, keep
their content-hash ids. The frames, in four subscriptions: the first ends when the module
stops; the one opened on the restarted module is sent the upgrade's `display_gap` and
ends; the one opened again ends when the module stops a second time; the last, opened on
the module started again, is sent nothing.

### 07-describe-and-head

`role-describe.json` is `role.describe`. The plan names `occipital` as a sender
(`idle_start`) and `occipital` and `hippocampus` as readers: `session-head.json` is the
owner's head, `session-head-as-occipital.json` the sender's (with `sender`: the delivery
modes it may use), and `session-head-as-hippocampus.json` a reader's.

### 08-notices-and-marks

While a run's `echo` call runs, the plan's sender `magic-context` sends a note listing
`tool_result` (`session-send-ride.json`): it rides the call's result, is served as a
notice (`notices`, `delivered_as: "tool_result"`) on the message holding that result, and
is announced live as `notice_delivered`. Then two sends of the owner's carrying message
marks, which reads and rows show: one `notice: true`, one
`speaks_for: {agent_id: "agent-a"}`. The contract refuses both marks on one send, so there
are two.

### 09-client-and-send-ids

The plan names `occipital` as a sender. A module's send (the owner's, `send_id:
"core-send-1"`) and an Occipital-style send (as `occipital`, `send_id: "phone-send-1"`,
`client_message_id: "phone-message-1"`, `delivery: ["steer", "idle_start"]`). The read's
display facts show each user message's `sender` and `send_id`, and the phone's
`client_message_id`; the display rows show `sender` and `client_message_id` (a display
row has no `send_id`). `session-send-status-core.json` and
`session-send-status-occipital.json` are each sender's `session.send_status`, and
`session-read-inline-as-occipital.json` the read as Occipital.

### 10-long-call

An `echo` call its provider holds while the session is read:
`session-read-mid-call.json`, `session-history-display-mid-call.json`,
`session-head-mid-call.json` and `run-status-mid-call.json` (the run `active`) are taken
mid-call, and `subscribe-frames-opened-mid-call.json` is a `session.subscribe {from:
"live"}` opened then; its `frames_while_call_ran` counts the frames it was sent before
the call was let finish. The module stores the answer that made the call, and the call's
start with it, only once the call has run (see above), so mid-call the reads hold only
the prompt, and the attach is sent a `display_reset` and the answer's streamed words but
no `tool_started`: it is sent the call's `tool_started` together with the answer once the
call returns. A `tool_started` replayed to a subscription opened while the call runs (as
OpenCode 2 sends one) can't be produced on Claude Code.

### 11-long-payloads

Two runs. The first calls `echo` with arguments over 2,048 bytes, and the tool answers
about 100 KB. The module's tool lane takes back at most 64 KiB of a result from a tool that
doesn't answer in `tool-result/v1`, on every host, so that call's result is the lane's
error `tool_result_too_large` (`session-read-exact-refused-result.json`, with the output's
`bytes` and the `limit`, 65536); the tool's 100 KB never reach Claude Code. The second
run's tool answers about 60 KB, stored whole (`session-read-exact-result.json`).
`session-history-display-app-limits.json` is the display history with the app limits
(`limits: {arguments_bytes: 2048, result_bytes: 16384, error_result_bytes: 65536}`): the
long arguments come back `arguments_truncated` and the 60 KB result `truncated`.
`session-read-exact-call.json` reads the long call's message alone, whole.

### 12-run-endings

One session with a step limit of two, one run per ending a reader sees as the session's
last run, each followed by `<ending>-run-status.json` (`run.status`) and
`<ending>-session-head.json` (`session.head`, with `last_run_state`):

| Ending | How | `state` |
|---|---|---|
| `completed` | a plain answer | `completed` |
| `failed` | the provider refuses the request with a 429 (`rate_limit_error`) | `error`, class `transient` |
| `stopped` | `run.cancel` while a call runs | `cancelled` |
| `step-limit` | the model keeps calling `echo` | `max_steps` |

The session's history moved to a new line once, between the failed run and the stopped
run (`history_version` 0 after the failed run, 1 after the stopped one), so the first
subscription ends with a `display_gap` and a reader subscribes again.

### 13-stop-all

`session-stop-all.json` is `session.stop_all {request_id, run_id}` while a run's `echo`
call runs: the turn ends `cancelled` (`run-status.json`), the call is stopped in the
module's tool lane, and the plan's tool provider, asked to stop the session's work,
answers with what it stopped.
