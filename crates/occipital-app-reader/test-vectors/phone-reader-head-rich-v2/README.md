# phone-reader-head-rich-v2

A rich UI reply stamped with catalog 1.0's current digest
(`4e256f56bf6a6f69d85f0bd9a101fd53637784fd76f7e8c53447069fd1dd77f5`), read through
Occipital on 14 November 2026. `phone-reader-head-rich-v1` stays frozen: its reply
carries the earlier digest `beb0904e…`, which the library no longer holds, so an app
draws it as its readable text. This set is the one to test native drawing against.

The reply came from a scratch head on Broca (a head is an agent a person works
with), asked for an invented status card, so the chat text is as recorded: nothing in
it is private. Each step is a request file (`*.request.json`, the params the app sent)
and the reply (`*.json`, `{"result": …}`), recorded with the `compare_readers` example
in `crates/occipital-module` with `COMPARE_RAW=1`. Nothing was edited after recording.

| Files | What they show |
| --- | --- |
| `01-details` | Session details after the turn. |
| `02-newest-page` | The newest page. User row 1 asks for the card and carries no stamp. Assistant row 2 carries `richUi {dialect: "1.0", catalogDigest: "4e256f56…"}` and its text holds one `Badge`, one `Callout` and a three-row `Table`. |
| `03-exact-read` | An exact read of row 2, with the same `richUi`. |

Draw a row rich only when it carries `richUi` and its `catalogDigest` is one the app's
library holds; otherwise draw the revision's readable text.

## Verify

```sh
(cd crates/occipital-app-reader/test-vectors/phone-reader-head-rich-v2 && shasum -a 256 -c SHA256SUMS)
```
