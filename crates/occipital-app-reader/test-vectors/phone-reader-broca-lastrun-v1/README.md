# phone-reader-broca-lastrun-v1

Recorded on 2026-10-26 from the iOS simulator against the production harness module,
through Callosum, reading real Broca workers. Each `NN-name.request.json` is the request
the phone sent and `NN-name.json` the reply it received. The home path of the recording
machine's user is replaced by `/Users/user-redact`, keeping every byte length.

Files 01 and 02 were recorded on commit `3dddc047e`. Files 03 to 13 were recorded on
`c306be0f2`, which adds the overall read deadline but replies with the same bytes.

| Files | Session | What it shows |
|---|---|---|
| 01-02 | A worker whose run was cancelled mid-turn | `lastRun` on session details and on the newest page: `state` and `reason` are `cancelled`, with no `error`. |
| 03-05 | A finished 151-message worker | The newest page (ordinals 145-150) carries `lastRun`. Older pages (`beforeOrdinal` 145, then 139) don't. The phone holds turns 133-138 after paging back. |
| 06 | Same | A claim whose held turns still exist but lie older than the newest page, sent with `acceptClaims: ["outside_window"]`. It answers `outside_window` with the newest page as its resync and `nextBeforeOrdinal` 145. |
| 07-08 | Same | Paging back from 145 reaches the held anchor `m138`, so the phone fills the gap without treating it as a rewrite. |
| 09 | Same | The same claim without `acceptClaims`. It answers `cursor_expired` with the same resync, which phone builds that don't know `outside_window` reload from. |
| 10-13 | Same | Session details, then newest pages of 100, 50 and 25 rows. These pages come from bounded reads of only the rows they serve. |

Request-to-reply times measured by the phone:
- **Newest page of 6:** 483 ms.
- **Opted-in claim:** 88 ms.
- **Claim without the opt-in:** 19 ms.
- **Newest pages:** 552 ms for 100 rows (126,268 bytes), 146 ms for 50, 24 ms for 25.

`SHA256SUMS` lists every file. `tests/phone_reader_samples.rs` fails if any file changes,
and checks the properties above.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
