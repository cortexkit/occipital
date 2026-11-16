# phone-reader-in-place-v1

A row that changes in place while the chat's page version stays the same, and the
claim answers that carry it to every app.

Broca stores a step before its tool calls run, and each call's result as a later
entry. The call's row gains its result (`outputBytes`, and its finish time when the
runner serves one) only when that entry lands. The runner's history counter, and so
the page `version`, doesn't move for an append. So an app that holds the row from
before the result has an out-of-date copy under the same version. A claim from it
is answered `diverged` from that row, with the row's new copy first in the resync.
Before this set, it was answered `matched`, with only the rows after its
`sinceMessageId`, and the app kept the call unfinished until a full reload.

Each reply was served through Occipital's real request handling, over a fake Broca
session, so it is byte for byte what a phone receives. Each `NN-*.request.json` is
the request, and `NN-*.json` beside it is the reply.

| Files | What they show |
| --- | --- |
| `01-page-small` | One app's newest page, one row: `m41`, the step with its `bash` call running (no `outputBytes`). |
| `02-page-large` | Another app's page of four rows: `m40` and `m41`, the same. |
| `03-page-after-result` | The result (`m42`) and the answer (`m43`) have landed. `m41`'s call now has `outputBytes: 14`; `version` is unchanged. |
| `04-claim-small`, `05-claim-large` | Each app's claim from the page it holds. Both answer `diverged` with `validThroughMessageId: "m40"` and a resync from `m41`: the call's new row, then `m42` and `m43`. |
| `06-claim-caught-up` | A claim from the new page: `matched` with no `newTurns`. |

Apply `diverged` as for any chat: keep the held rows through `validThroughMessageId`
and replace everything after it with the resync. The reply's `diagnostics.deltaRead`
is `native`, as for a `matched` answer on this chat.

`SHA256SUMS` covers every file except this README. The test
`broca::tests::in_place_rows` in `occipital-module` serves the set again and fails if
any byte changes.
