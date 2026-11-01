# phone-session-facts-opencode-v1

The phone reading the session facts of an OpenCode head that Thalamus's own runner
hosts: the head's model, its spend, and the phone's send grant. CKIOS recorded it on
2026-10-28 from an iOS simulator against the production harness module
(`thalamus-harness` `95d75e70f`), on the throwaway "Canary Head", harness `opencode`,
session `ses_e82d035a7ffeFAtBg5sNRs6C7C`. Every request names `harness: "opencode"`.
The files are CKIOS's recording apart from their chat text (see **Text** below), and contain no personal data.

Each case has the same three files as `phone-reader-opencode-v1`: the request's bare
params, the raw reply body, and its timing. The two watches have `.open.txt`,
`.end.txt` and one file pair per notice.

## Cases

| Files | What it shows |
|---|---|
| `00-warmup-details` | `session.details` before anything else. |
| `01-newest-page` | The newest page, with the head's `model`. |
| `02-older-page` | Paging back with `beforeOrdinal`. |
| `03-claim-current`, `04-claim-behind` | Claims answer `matched`, with `model`. |
| `06-session-details` | Before this recording's turn: `model`, the send grant in `sender`, `lastRun`, and no `usage`. |
| `07-display-watch` | The reply's live text as it streams. |
| `08-send` | `session.enqueue_user_message` with a `clientMessageId`. |
| `09-newest-page-after-send` | The sent user row carries its `clientMessageId`. |
| `10-transcript-watch` | A claim from the page after the send. |
| `11-session-details-after-send` | After the turn: `usage` counts that run, and `since` is its run id. |

## What the facts mean

- **`model`** is the provider, model and variant the head's newest user message names,
  otherwise the model the head started with.
- **`usage`** is the head's spend on runs Thalamus's runner served, counted once per
  finished model step as the runner stores it, and never taken back by a revert. It is
  absent until the first such step: that means nothing has been counted yet, not zero
  spend. History from before Thalamus hosted the head isn't in it. `since` names the
  first counted run.
- **`sender`** is the phone's own send grant on the head. Thalamus is a sender the head's
  plan names, so it may steer, start a turn when the head is idle, and interrupt.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
