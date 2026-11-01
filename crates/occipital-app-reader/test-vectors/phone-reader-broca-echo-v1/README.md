# Phone reader samples with the send id echo, Broca, version 1

Recorded by the iOS app's simulator on 2026-10-25 between 22:48 and 22:50 UTC against the production harness module `21e68a5bc`, through Callosum, with two Broca workers. Each reply file `NN-*.json` is the body the phone received, with its chat text replaced (see **Text** below). Its `NN-*.request.json` is the request the phone sent, including the members the app adds to every request (`sessionId` and `callerDirectory`, which name the app's install).

Besides the text, the machine's user name inside home paths was replaced by `user-redact`, which has the same length, so every byte count is unchanged. The files were renamed to number both sessions in one sequence. `verify_published_set` in `tests/phone_reader_samples.rs` checks each file against `SHA256SUMS` and that no other home path remains. `SHA256SUMS` hashes the exact JSON bytes; this README is not part of it.

| Files | What they show |
|---|---|
| `01` | `session.details` for `alfonso:bg_0e46af454901d03e`, a worker that ran on 2026-10-23. |
| `02`, `03` | That worker's newest page (messages 4 to 9) and older page (messages 0 to 3). Some tool rows there have no `createdAtMs`, because Broca recorded no time for tool results before 2026-10-24. So the reader makes no display-history call for those pages, and user row `m1` carries no `clientMessageId`. |
| `04`, `05` | The newest page (messages 1 to 6) and older page (message 0) of `alfonso:bg_e6b76fa02f687e7d`, a worker started on 2026-10-25, before the send. |
| `06` | `session.enqueue_user_message` into that worker's running turn: accepted, `clientMessageId: 3363cebb-3fd4-41de-88de-ed9f9ab1a258`, `duplicate: false`. |
| `07` | The newest page after the steer landed, as user row `m8` (message 8), carrying the same `clientMessageId` as the receipt in `06`. |
| `08` | That worker's `session.details` afterwards. |

The reader takes `clientMessageId` from Broca's display history over exactly the page's range, and only when every message on the page has a write time. No other row in this set carries one: user row `m1` in `04` was the worker's launch prompt, which came with no phone id.

**Order rows by `ordinal`, not by time.** In `07`, user row `m8` has an earlier `createdAtMs` than tool row `m7`. The steer was recorded when the phone sent it, while the tool was still running, and it reached the conversation after that tool's result. `ordinal` is the order the model saw. `createdAtMs` is when each message was written, and steers can be written before the message they follow.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
