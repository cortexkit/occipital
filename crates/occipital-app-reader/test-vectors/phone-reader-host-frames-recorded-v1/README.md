# phone-reader-host-frames-recorded-v1

Messages the person sent while a Claude Code turn was running, recorded by
Agent Runner from real Claude Code and served through Occipital's real request
handling by `occipital-module`'s `broca::tests::contract_097_samples`, so the
reply is byte for byte what a phone receives. `01-newest-page.request.json` is
the request the app sends and `01-newest-page.json` the reply body.
`SHA256SUMS` lists every file but this README. The read entries Occipital was
given are in `occipital-module/tests/data/thalamus-host-frames-v1/`, copied
byte for byte from Agent Runner.

`phone-reader-host-frames-v1` says how to draw a row with `hostFrames`. This
set shows the same rows from real recordings:

| Row | Recorded with | What it shows | `hostFrames` |
| --- | --- | --- | --- |
| 1 (ordinal 6) | Claude Code 2.1.332 | two messages (`MID_TURN_FIRST`, `MID_TURN_SECOND`) sent during one tool call, stored beside its result: head, words `1`, tail, head, words `2`, tail | `1`, `2` |
| 2 (ordinal 7) | Claude Code 2.1.331 | one message (`please also check the logs`) stored as its own `system` message: head, words `0`, tail | `0` |

Row 1's tool result answers a call made before this page, so the page shows
no part for it. Row 1 has no `clientMessageId`, since it holds two of the
person's messages. Row 2 keeps the one its message was sent with.
