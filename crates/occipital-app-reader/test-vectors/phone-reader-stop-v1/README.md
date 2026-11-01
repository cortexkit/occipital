# phone-reader-stop-v1

The phone stopping a running turn with `session.stop`. CKIOS recorded it on 2026-10-29
from an iOS simulator against the production harness module (`thalamus-harness`
`b310d1655`), on the Canary Head (harness `opencode`, session
`ses_e82d035a7ffeFAtBg5sNRs6C7C`), with one Broca worker for the refusal.

The files are the recording apart from their chat text (see **Text** below); none carries a user name.

## Cases

| Files | What it shows |
|---|---|
| `50-stop-details` | `session.details` before: the phone's grant has `interrupt: true`, so Stop is offered. |
| `51-stop-send` | A send that starts a long plain-text turn, with no tool calls to move it to the background. |
| `52-stop-details-running` | Details while the turn streams: `lastRun` is that run, `state: "active"`. |
| `53-stop` | `session.stop` naming that run: `{state: "stopped", runId}`. |
| `54-stop-repeat` | The same request again, same `clientRequestId`: the same answer. |
| `55-stop-unknown-run` | A stop naming a run that isn't running: `{state: "not_active", runId}`. |
| `56-stop-broca` | A stop on a Broca worker: refused `unsupported`, `detail.field: "session.stop"`, because Broca doesn't let a sender cancel a run. |
| `57-stop-newest-page-after` | The newest page after: `lastRun` `cancelled`, and the partial reply kept at ordinal 45 with `completedAtMs`. |
| `58-stop-details-after` | Details after: `lastRun` `cancelled`. |

## Reading Stop

- Offer Stop only while the grant in `session.details` has `interrupt: true` and
  `lastRun.state` is `"active"`. `lastRun` rides details, newest pages and transcript
  watch notices.
- Name the run to stop with `runId` from that `lastRun`, so a late press never stops the
  next turn.
- `stopped` means the named run has ended cancelled; `not_active` means it isn't
  running. Either way, read `lastRun` for what happened.
- A repeat with the same `clientRequestId` gets the first answer.
- The text the model wrote before the stop stays in the transcript as a finished reply.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
