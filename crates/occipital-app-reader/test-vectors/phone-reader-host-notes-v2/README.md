# phone-reader-host-notes-v2

Text a host wrote into a chat, as the apps receive it: in a person's message,
and, new in this set, in a reply. The page is the runner contract's own
host-part cases (`display-host-codes.json` in `cortexkit-role-llm-runner`
0.8.46), numbered from ordinal 1 as runners number them and read on a Claude
Code chat through Occipital's real request handling, so the reply is byte for
byte what a phone receives. `phone-reader-host-notes-v1` stays as it was, with
the first four rows only.

A runner names host-written parts in the read's `host_parts` fact.

- **In a person's message:** Occipital serves each as an `injected` part with
  `host: true`, and `hostCode` when the runner matched a known marker, and
  leaves its text out of the row's `text`.
- **In a reply:** a host part is the host's own bookkeeping, such as the empty
  reasoning blocks that record that another model answered. Occipital leaves
  it out of the row altogether.
- **`modelFallback: {requested, answered}`:** on a reply that a different model
  wrote in place of the one the session asked for. The runner sets it only
  when it knows this exactly: from a boundary marker that parses, or from its
  own record that it replaced the model. Both ids are in the same form as the
  chat's model id. Draw a quiet note under the reply naming both models, with
  their display names from `display.model_names`, or the raw id when there is
  no name. A reply without the member makes no claim either way.

`01-page-host-notes.request.json` is the request the app sends, and
`01-page-host-notes.json` is the reply body it receives. Its seven rows:

| Row | What it shows |
| --- | --- |
| 1 | A message holding only Claude Code's `[Request interrupted by user]`: a `user` row with empty `text` and one host part, `hostCode: "interrupted"`. |
| 2 | The same for `[Request interrupted by user for tool use]`, `hostCode: "interrupted_for_tool_use"`. |
| 3 | A host part with a code no app knows yet (`future_notice_7`). |
| 4 | A host part with no code, then the person's own words, which alone make up `text`. |
| 5 | A reply written by a fallback model. Its two host reasoning blocks are left out, leaving its text part, and `modelFallback` names `claude-fable-5-2` (asked for) and `claude-opus-4-8` (answered). |
| 6 | A reply whose only part was a boundary marker that doesn't parse. The part is left out, so the row has no parts, and there is no `modelFallback`. |
| 7 | A reply the runner itself recorded as replaced, with no host parts: `modelFallback` names `provider/requested` and `provider/answered`. |

A `user` row whose only parts are host parts is not something the person sent.
Draw it as a note written from its code, or draw nothing; never as the
person's message, and never as a "sent a file" placeholder. An `assistant` row
with no parts and empty `text` has nothing to draw.

`SHA256SUMS` covers every file except this README. A test in
`occipital-module` (`broca::tests::claude_code_samples`) serves the set again
and fails if any byte changes.
