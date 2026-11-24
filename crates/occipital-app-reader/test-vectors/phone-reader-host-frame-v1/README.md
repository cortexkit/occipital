# phone-reader-host-frame-v1

Messages the person sent while a Claude Code turn was running, as the apps
receive them (runner contract 0.9.6, `host_frame`). Served through Occipital's
real request handling by `occipital-module`'s
`broca::tests::contract_096_samples`, so the reply is byte for byte what a phone
receives. `01-newest-page.request.json` is the request the app sends and
`01-newest-page.json` the reply body. `SHA256SUMS` lists every file but this
README.

Claude Code stores such a message as a `system` message: two fixed sentences of
its own around the person's words. The runner names that wrapper in the read's
`host_frame` fact, and Occipital trusts it only when the stored text is exactly
those sentences around the words. The six messages are the runner contract's
own recordings from real Claude Code 2.1.331 (`steer-frame-claude-code-v1`),
with the fact a runner now serves added, since they were recorded before it
existed. They name this module as their sender, as a message typed in the apps
does.

Rows 1 to 6 are `user` rows. Each has `text` holding only the person's words,
its `clientMessageId`, and three parts, all of stored part 0:

| Part | `type` | `partKey` | What it is |
| --- | --- | --- | --- |
| 1 | `injected`, `host: true`, `hostCode: "claude_code_mid_turn"` | `0#host_frame:head` | Claude Code's opening sentence |
| 2 | `text` | `0` | the person's words |
| 3 | `injected`, `host: true`, `hostCode: "claude_code_mid_turn"` | `0#host_frame:tail` | Claude Code's closing sentence |

Draw the row as the person's message from part 2, and the two host parts as a
quiet note worded from the code, or not at all. Row 4's words copy Claude
Code's opening line and row 5's its closing paragraph, and both still split
only at the frame's own bytes. Row 6's words start with `/`.

Row 7 is row 1's message as an older runner serves it, with no fact: a
`system` row holding the whole stored text, as before.
