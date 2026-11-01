# phone-reader-no-runner-v1

The phone naming OpenCode sessions Thalamus doesn't host: one no session has, and one
of the operator's own heads (long-lived OpenCode agent sessions), which Entorhinal
records with no runner. The phone team (CKIOS) recorded this set on 2026-10-28 from an
iOS simulator against the production harness module (`thalamus-harness` `95d75e70f`).
Every request names `harness: "opencode"`; the head is the phone team's own, session
`ses_06be916fbffezpvuoIO3ac4yMZ`.

The files are the recording apart from their chat text (see **Text** below) and the
user name in one absolute path in `07-head-upload-finish.json`, replaced by
`user-redact`, which has the same length. `ckios-residence-row.json` is the head's Entorhinal residence at the time.

## Cases

| Files | What it shows |
|---|---|
| `00-warmup-details` | `session.details` for an unknown session: `session_unknown`, `detail: {runner: null}`. |
| `01-unknown-page` | A newest page for an unknown session: the same refusal. |
| `02-head-page` | A newest page for the head: `session_unknown` naming no runner, since no runner hosts it. |
| `03-head-display-watch` | A display watch on the head ends at once with that refusal. |
| `04-head-send` | `session.enqueue_user_message` to the head: `not_found`, `detail: {runner: null}`. |
| `05`–`07` `head-upload-*` | Begin, one chunk and finish into the head: the file lands under its project's `.cortexkit/uploads/<date>/`. |
| `08-unknown-upload-begin` | `upload.begin` for an unknown session: `session_unknown`, `detail: {runner: null}`. |

## What the refusals mean

`session_unknown` with `detail.runner` tells the phone which runner Entorhinal records
for the session, so it can read through that runner's owner instead: `null` means no
runner hosts it (or Entorhinal doesn't know it). A send to such a session is refused
`not_found` with the same `detail.runner`.

Uploads don't need a runner: Thalamus finds the head's project folder through
Entorhinal's `agent.session_for_harness`, writes the file there, and the phone then
names the path in its message.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
