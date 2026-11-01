# phone-reader-head-history-v1

Paging back past a moved head's cut on our OpenCode runner. Recorded by the
desktop app on 2026-10-31 at 03:51Z from WERNI's session
(`ses_089301004ffeI4YKU4R01EiTE0`, `harness: "opencode"`), right after WERNI
moved onto the runner with its history cut at message 5,337.

A moved head numbers only the messages from its cut onward. Everything older is
host history: the reader serves it unnumbered, oldest first, on pages reached
with `beforeCursor`.

| File | Request | What it shows |
| --- | --- | --- |
| `01-oldest-numbered-page.json` | `beforeOrdinal: 5400, limit: 500` | Messages 5337 to 5399, numbered. No `nextBeforeOrdinal`, because 5337 is the oldest numbered message. `nextBeforeCursor` anchors on the cut message, `msg_0cd91b217001…`. |
| `02-before-cursor-page-1.json` | that cursor, `limit: 50` | 50 unnumbered messages, oldest first, ending at the head cut `msg_0cd8d2c7e001…`. A further cursor and no `reachedStart`. |
| `03-before-cursor-page-2.json` | page 1's cursor, `limit: 50` | The 50 messages just before page 1, again with a further cursor. |

Each file is the whole reply as the daemon delivered it (`{"result": …}`). The
recording replaced only the values of `text`, `arguments` and `error`. Ids,
ordinals, part keys, cursors, versions and model names are unchanged. The files
are byte-identical to the desktop's recording, and `SHA256SUMS` is its own.
