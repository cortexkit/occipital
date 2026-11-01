# phone-reader-tool-error-v1

A failed tool call as the phone sees it, for both harnesses Thalamus serves, plus two
calls that look like failures but aren't. CKIOS recorded it from an iOS simulator against
the production harness module: the Broca files on 2026-10-28 (`thalamus-harness`
`95d75e70f`, throwaway worker `alfonso:bg_9921ba0f4815a3bb`), the OpenCode head files on
2026-10-29 (`4d8ce0fc4`, the Canary Head, harness `opencode`, session
`ses_e82d035a7ffeFAtBg5sNRs6C7C`).

The files are the recording apart from their chat text (see **Text** below) and the user name in absolute paths in
`04-head-newest-page.json`, `06-broca-exact-ord2.json`, `10-broca-newest-page.json` and
`11-broca-exact-read.json`, replaced by `user-redact`, which has the same length.

## Cases

| Files | What it shows |
|---|---|
| `03-head-send` | The send that asks the head to call `read` with no path. |
| `04-head-newest-page` | The head's newest page: the `read` at ordinal 39 has `status: "error"`, `isError: true` and `error`. |
| `05-head-exact-read` | The exact read of ordinal 39: the same part, with the whole `error`. |
| `06-broca-exact-ord2` | Not a failure: a Broca `bash` that exits non-zero has `isError: false`, with the exit code in its output. |
| `08-head-exact-ord25` | Not a failure: a head `read` of a missing file completed, with a not-found error as its output. |
| `10-broca-newest-page` | The Broca worker's newest page: its `read` of a missing file at ordinal 6 has `isError: true`. |
| `11-broca-exact-read` | The exact read of ordinal 6: the same part. |

## Reading a tool part

- `isError: true` marks a failed call, for both harnesses. On Broca list pages the member
  appears only when true; exact reads always carry it.
- `error` is display text: never parse it. On OpenCode it holds the first line of the
  failure on list pages and the whole text on exact reads. For a module tool call on the
  OpenCode runner it is currently the provider's outcome serialized as JSON, as in
  `04`/`05`, which is the text the model saw; that will change to the contract's rendered
  outage text.
- A tool that runs and reports a problem in its output (a non-zero exit, a missing file)
  has not failed: its output is the answer.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
