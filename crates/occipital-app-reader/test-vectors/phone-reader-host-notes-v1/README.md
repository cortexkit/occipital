# phone-reader-host-notes-v1

Text a host wrote into a person's message, as the apps receive it. The page is
the runner contract's own host-part cases (`display-host-codes.json` in
`cortexkit-role-llm-runner` 0.8.33), numbered from ordinal 1 as runners number
them and read on a Claude Code chat through Occipital's real request handling,
so the reply is byte for byte what a phone receives.

A runner names such parts in the read's `host_parts` fact. Occipital serves
each as an `injected` part with `host: true`, and `hostCode` when the runner
matched a known marker, and leaves its text out of the row's `text`.

`01-page-host-notes.request.json` is the request the app sends, and
`01-page-host-notes.json` is the reply body it receives. Its four rows:

| Row | What it shows |
| --- | --- |
| 1 | A message holding only Claude Code's `[Request interrupted by user]`: a `user` row with empty `text` and one host part, `hostCode: "interrupted"`. |
| 2 | The same for `[Request interrupted by user for tool use]`, `hostCode: "interrupted_for_tool_use"`. |
| 3 | A host part with a code no app knows yet (`future_notice_7`). |
| 4 | A host part with no code, then the person's own words, which alone make up `text`. |

A `user` row whose only parts are host parts is not something the person sent.
Draw it as a note written from its code, or draw nothing; never as the
person's message, and never as a "sent a file" placeholder. OpenCode 2's
host-written `synthetic` messages will reach the apps in this same shape.

`SHA256SUMS` covers every file except this README. A test in
`occipital-module` (`broca::tests::claude_code_samples`) serves the set again
and fails if any byte changes.
