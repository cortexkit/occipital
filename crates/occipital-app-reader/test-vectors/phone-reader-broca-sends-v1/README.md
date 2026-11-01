# Phone reader samples with sends, Broca, version 1

Recorded by the iOS app's simulator on 2026-10-25 against the production harness module `b5ec0cd09`, through Callosum, with throwaway Broca workers. Each reply file `NN-*.json` is the body the phone received, with its chat text replaced (see **Text** below). Its `NN-*.request.json` is the request the phone sent, including the members the app adds to every request (`sessionId` and `callerDirectory`, which name the app's install).

Besides the text, the machine's user name inside home paths was replaced by `user-redact`, which has the same length, so every byte count is unchanged. `verify_published_set` in `tests/phone_reader_samples.rs` checks each file against `SHA256SUMS` and that no other home path remains. `SHA256SUMS` hashes the exact JSON bytes; this README is not part of it.

| Files | What they show |
|---|---|
| `01`, `06`, `09` | `session.details` for the fresh worker `alfonso:bg_08e60782bd7163be`: its model, usage, incarnation, and the send grant Broca reports to Thalamus (`modes: [tool_result, steer]`, `interrupt: false`). |
| `02` | The newest page, `limit: 6`: messages 4 to 9, `nextBeforeOrdinal: 4`. |
| `04` | `session.enqueue_user_message` into the worker's running turn: accepted as a steer, `{ok: true, effectId, clientMessageId, duplicate: false}`. |
| `05` | The newest page after that steer landed, as message 12, role `user`. |
| `07` | A send to the ended worker `alfonso:bg_88c6332b40a22d3d`: refused as `task_paused`, with nothing sent. |
| `08` | That ended worker's details. |
| `10` | A later newest page: messages 14 to 19. |
| `11` | The older page, `beforeOrdinal: 8`: messages 8 to 13. |

Every row carries `createdAtMs` and `network: "yes"`. A `no` or `unknown` row needs the agent's network to have been off or unrecorded at that moment, so this set has none; unit tests cover both.

**Not in this set: `clientMessageId` on the stored row.** Message 12 in `05` and `11` carries no `clientMessageId`, although the send passed it to Broca. Under llm-runner 0.7, a runner serves a message's `client_message_id` only on display rows from `session.history`, never in the default `session.read` entry, and this reader builds pages from `session.read`. The echo arrives once the reader takes it from Broca's display rows, in a later set.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
