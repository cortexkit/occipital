# Phone reader samples with completion times, Broca, version 1

Recorded by the iOS app's simulator on 2026-10-25 at about 21:48 UTC against the production harness module `5a74e10fb`, through Callosum, with two throwaway Broca workers. Each reply file `NN-*.json` is the body the phone received, with its chat text replaced (see **Text** below). Its `NN-*.request.json` is the request the phone sent, including the members the app adds to every request (`sessionId` and `callerDirectory`, which name the app's install).

Besides the text, the machine's user name inside home paths was replaced by `user-redact`, which has the same length, so every byte count is unchanged. The files were also renamed to number both sessions in one sequence. `verify_published_set` in `tests/phone_reader_samples.rs` checks each file against `SHA256SUMS` and that no other home path remains. `SHA256SUMS` hashes the exact JSON bytes; this README is not part of it.

| Files | What they show |
|---|---|
| `01` | `session.details` for `alfonso:bg_0e46af454901d03e`, a worker that ran on 2026-10-23, before Broca 0.3.255. |
| `02` | That worker's newest page, `limit: 6`: messages 4 to 9, `nextBeforeOrdinal: 4`. |
| `03` | Its older page: messages 0 to 3, `reachedStart: true`. |
| `04` | The newest page of `alfonso:bg_08e60782bd7163be`, a worker started on 2026-10-25: messages 17 to 22, `nextBeforeOrdinal: 17`. |
| `05` | Its older page: messages 11 to 16, `nextBeforeOrdinal: 11`. |

Every assistant row carries `completedAtMs`, equal to that row's `createdAtMs`: Broca writes an assistant message only after its step completes, so its write time is when it finished. User and tool rows carry neither `completedAtMs` nor `completion`.

**Not in this set: `completion: "unknown"`.** The reader marks an assistant row that way when Broca reports no write time for it. The older worker ran before Broca recorded write times, but Broca now serves a time for each of its replies, so no row here is unknown. The reader's unit tests cover the unknown case. The second worker's `session.details` was interrupted during recording and is left out.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
