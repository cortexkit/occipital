# phone-reader-opencode-v1

The phone reading and sending to an OpenCode session that Thalamus's own runner hosts.
CKIOS recorded it on 2026-10-28 from an iOS simulator against the production harness
module (`thalamus-harness` `50bf8b9f7`), on the throwaway "Canary Head", harness
`opencode`, session `ses_e82d035a7ffeFAtBg5sNRs6C7C`. Every request names
`harness: "opencode"`. The files are CKIOS's recording, apart from their chat text (see **Text**
below).

Each case has three files:

- `NN-<case>.request.json`: the bare params, as the phone sent them, apart from their text (see **Text** below).
- `NN-<case>.json`: the raw reply body.
- `NN-<case>.timing.txt`: when it was sent and how long it took.

The two watches have `.open.txt` and `.end.txt`, plus one `.notice-N.json` and
`.timing.txt` per notice.

## Cases

| Files | What it shows |
|---|---|
| `00-warmup-details` | `session.details` before anything else. |
| `01-newest-page` | The newest 6 rows, ordinals 14–19. Rows have the same shape as Broca's, plus `model` on assistant rows. |
| `02-older-page` | Paging back with `beforeOrdinal`: ordinals 8–13, `nextBeforeOrdinal` 8. |
| `03-claim-current` | A claim holding the newest message: `matched` with empty `newTurns`. |
| `04-claim-behind` | A claim one turn behind: `matched` with that one turn. |
| `05-exact-read` | One message with `fullContent`. |
| `06-session-details` | `lastRun {runId, state: completed}` and `senderUnknown: true`. This runner doesn't report the phone's send grant on the session head yet, so the phone gets unknown, not a refusal. |
| `07-display-watch` | The reply's live text as it streams: one reasoning and three text frames for the reply's message, then `display_gap` and `display_end`. The watch stays open until the phone unsubscribes. |
| `08-send` | `session.enqueue_user_message` with a `clientMessageId`. |
| `09-newest-page-after-send` | User row 20 carries the sent `clientMessageId`. Row 18, sent by the phone in an earlier recording, carries its own. |
| `10-transcript-watch` | A claim from the page after the send. Its one notice has no new turns. |
| `11-session-details-after-send` | `lastRun` names the send's completed run. |

## Notes

- **`display_gap` frames before the first text.** Notice 2 of the display watch carries
  ten `display_gap` frames in one batch, sent as the run starts, before any text. Their
  source in the runner's event stream isn't traced yet. A `display_gap` drops live text
  the phone hasn't placed, so repeats before the first text change nothing on screen.
- **Ordering.** Show rows by `ordinal`, never by time, as for Broca.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
