# phone-reader-say-v2

`say` messages with prefrontal's message record joined. Everything in
`phone-reader-say-v1` still holds; this set adds what the record brings.

## The record

Prefrontal writes one `display.write` with `kind: "message"` for each `say` call
before it answers the call (tool-display contract §9). Only prefrontal may write
one. Occipital keeps it, checked, under the runner and the call's key, and joins
it to the call's `message` part when it builds the row. The message text itself
always comes from the call's stored input.

## What the part gains

`{"type": "message", …, "replyTo"?, "media"?, "level", "key"?, "done"?, "deliver"?}`

- `replyTo`: `{messageId}` for the person's message this answers, by its
  `messageId` on the page. With a quote: `{messageId, quote, verified: true}`.
  Prefrontal writes the quote only when it matched the person's own words, so a
  quote is always verified; draw it only then.
- `media`: the record's items, each with `kind` (`image`, `video`, `file`,
  `link` or `diff`) and the members contract §9 lists. Images and videos carry
  their size, so the row's height is known before the bytes arrive. Files are
  artifacts. Links are never fetched; without a `title`, show the host.
- `level`, `key`, `done`, `deliver`: from the record, which says how the
  message was delivered. They replace what the call's input wrote.

A call whose record isn't there draws from its input alone, as in v1.

## Files

The chat of v1, served by a runner that names each call's key, plus a second
answer by `say` (`m8`, its result in `m9`).

- `01-newest-page`: `m2`'s message answers `m1` with a verified quote, carries
  a picture and a link, and alerted the phone at `attention`. `m8`'s message
  answers `m1` without a quote: its quote didn't match the person's words, so
  prefrontal left it out of the record. `m5`'s failed `say` is still a tool card.
- `02-exact-read`: the exact read of `m2`, with the same message.

These replies come from Occipital's real handlers, fed these runner entries,
the plan, and prefrontal's records written into a display store. Each `.json`
file is the response body, `{"result": …}`, and each `.request.json` file is the
request. `SHA256SUMS` covers every file but this README.
