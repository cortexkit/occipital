# phone-reader-host-frames-v2

This set supersedes `phone-reader-host-frames-v1`, which stays frozen. Only the Simple rule's number in `02-simple-page.json` changes, from 2 to 3. Every file is byte-identical to v1 once `:simple:2` reads `:simple:3`.

Messages the person sent while a Claude Code turn was running, as the apps
receive them from runner contract 0.9.7 on, whose reads name each wrapper in a
`host_frames` list. Served through Occipital's
real request handling by `occipital-module`'s
`broca::tests::contract_097_samples`, so each reply is byte for byte what a
phone receives. `NN-name.request.json` is the request the app sends and
`NN-name.json` the reply body. `SHA256SUMS` lists every file but this README.
This set replaces `phone-reader-host-frame-v1`, whose rows predate `hostFrames`.

Claude Code wraps each such message in two fixed sentences of its own. The
runner names each wrapper in the read, and Occipital trusts it only when the
stored text is exactly those sentences around the words. Claude Code 2.1.331
stores one message as a `system` message. From 2.1.332 it stores the message
as a text block beside the tool result it arrived with, and several messages
sent during one tool call are several blocks in one stored message, so one
row can hold several of the person's messages.

## How to draw a row with `hostFrames`

`hostFrames` lists the person's messages in the row, in stored order. Each
entry is `{partKey, code}`: `partKey` names the part holding that message's
words, and `code` names the host that wrapped it (`claude_code_mid_turn`).

- Draw one message from the person per entry, in list order, from the part
  its `partKey` names. Never split a row by reading part keys.
- The wrapper's two sentences are `injected` parts with `host: true` and
  `hostCode`, keyed `<partKey>#host_frame:head` and `…:tail`. They are
  Claude Code's text, not the person's. Leave them out of the message; an
  app may show them on request, as the host's own text.
- Every other part keeps its usual meaning. A tool result that arrived in the
  same stored message is not one of these parts: it is drawn with the call it
  answers, in the reply's row.
- The row's `text` joins its text parts, for search and copy.
- Every entry names a part the row holds. If an app ever meets one that
  doesn't, it draws the row's parts as it would without `hostFrames`.

A row with no `hostFrames` has no wrapped message, as before.

## `01-newest-page.json`: the full view

| Row | `messageId` | What it shows | `hostFrames` |
| --- | --- | --- | --- |
| 1 to 6 | `steer-1` … `steer-6` | Claude Code 2.1.331's six recorded messages, each a `user` row from the runner's single `host_frame` fact: head, words (part `0`), tail. Row 4's words copy Claude Code's opening line and row 5's its closing paragraph, and both still split only at the frame's own bytes. Row 6's words start with `/`. | one entry, `0` |
| 7 | `reply-dial` | the reply whose tool call rows 8 to 12 answer; the result is drawn with this call | none |
| 8 | `two-frames-read` | two messages sent during one tool call: head, words `1`, tail, head, words `2`, tail | `1`, `2` |
| 9 | `one-frame-read` | one message sent during one tool call | `1` |
| 10 | `bad-tail-read` | two messages where the second wrapper is one byte off: the first splits, the second stays a plain text part holding the whole stored text | `1` |
| 11 | `refused-read` | a list the contract refuses (two entries naming one part): both parts stay as stored | none |
| 12 | `woken-and-sent` | two messages in one tool call where the first is a module's wake notice (`injected`, no `host`) and the second the person's words | `1`, `2` |
| 13 | `steer-1-without-the-fact` | row 1's message from an older runner, with no fact: a `system` row holding the whole stored text | none |

Rows 8 to 12 use the runner contract's own invented examples
(`steer-frame-claude-code-v2`) and an invented notice. Every message names
this module as its sender, as one typed in the apps does.

## `02-simple-page.json`: the Simple view

The same chat with `view: "simple"`. The view drops host parts and a notice's
words, and `hostFrames` drops the entries whose words part it dropped: row 12
keeps only its entry for part `2`.
