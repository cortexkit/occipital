# Phone reader samples for a Broca session (v1)

These are real requests and replies of the apps' session reader, served by the harness
module `ck-thalamus-harness` at commit `b9eedeffd2784a0ba1366f8a12312595f52040e7`, for one
Broca worker session (`alfonso:bg_0e46af454901d03e`). The phone team (CKIOS) recorded them
on 2026-10-23 from a simulator, through Callosum, against the build placed on the Mac at
14:05:04Z. They pin the shapes the phone decodes: a newest page, the next older page by
its cursor, a claim for exactly the newest page, a claim one turn behind, and the
transcript watch opened with each claim. Each watch delivered one notice and was then
unsubscribed after 20 seconds. `log.txt` is the recording's own log.

Every `NN-*.request.json` is the params the phone sent. Every `.reply.json` or
`.notice-N.json` is what the module answered.

## What changed from the raw recording

The macOS user name inside worktree paths in two replies (`01-page-newest`,
`02-page-older`) was replaced with `user-redact`. The replacement has the same length as
the original, so every recorded byte count (`outputBytes`) still matches its content.
Nothing else was changed. `SHA256SUMS` lists the files as committed here.

## Known gaps in this set

- Rows carry no `createdAtMs`: Broca's `session.read` entries don't carry a write time
  yet. Broca will serve `written_at` once it records one per message.
- Every row's `network` is `unknown` for the same reason, because the per-message network
  value is looked up at the message's write time.
- `model`, `sender` and `senderUnknown` aren't in this set. They arrive with the next
  reader release and a later sample set.

A later version of this directory replaces these files when those fields exist. This one
stays as it is.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
