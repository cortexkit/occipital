# phone-reader-simple-hand-off-v2

What a voice call hands the agent, as the Simple view and the full view show
it. This set supersedes `phone-reader-simple-hand-off-v1`, which stays frozen.
Only `02-full-page.json` changes: the hand-offs' full rows now carry
`handOff: true` too, so an app drawing the full view tells a hand-off from
any other module's message. Served through
Occipital's real request handling by `occipital-module`'s
`broca::tests::simple_samples`, so each reply is byte for byte what a phone
receives. `NN-name.request.json` is the request the app sends and
`NN-name.json` the reply body. `SHA256SUMS` lists every file but this README.
`phone-reader-simple-v4` holds every other Simple view case.

During a call, the module that runs it (the voice front) sends the agent what
the person said, inside its own framing: a hand-off. Before it sends, it
writes a notice record for the hand-off holding only the person's words from
the call. The agent answers with `say`.

The chat is invented, a Broca head whose plan gives the agent `say`:

| Ordinal | Message | In the Simple view |
| --- | --- | --- |
| 1 | the person types "Morning." | yes |
| 2, 4, 9 | the agent's prose | no |
| 3 | a hand-off, with its record | yes, `handOff: true` |
| 5 | the agent's `say` answer | yes |
| 6 | the `say` call's result | no |
| 7 | a second hand-off, whose record never arrived | yes, `handOff: true` |
| 8 | a message from a module that doesn't run calls | no |

## Files

- `01-simple-page.json`: the Simple view. Each hand-off keeps its text parts
  as served and carries `sentBy` with the voice front, plus `handOff: true`.
- `02-full-page.json`: the full view. Each hand-off keeps every part and
  carries `sentBy` and `handOff: true`. The other module's message carries
  only `sentBy`.
- `03-hand-off-records.json`: `session.wake_records` with
  `acceptNoticeRecords: true` for both hand-offs. The first has its record
  under `notices`. The second has none.
- `04-notice-view.json`: the voice front's notice view,
  `nv1:colliculus:hand_off:1`, whose data is `{said}`.
- `05-template-row.json`, `06-template-page.json`: that view's row and page
  templates, which this set invents. The voice front serves its own.

## How to draw a hand-off

A row with `handOff: true` is a line from the call, never the person's own
message. Its text is the voice front's framing for the agent, so don't draw it.

- When the row has a record, draw the person's words from the record's
  `data.said`, such as "You said on the call: «Is the release ready to
  tag?»".
- Without one, draw a quiet "From your call" line with no words.

A view that shows everything the agent was sent, such as a transcript, may
still show the row's text as the voice front's message.

The agent's `say` answer follows it as an ordinary message.
