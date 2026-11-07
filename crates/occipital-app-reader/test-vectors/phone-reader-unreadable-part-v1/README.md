# phone-reader-unreadable-part-v1

An OpenCode 2 chat whose two messages each hold one part that can't be
shown, served through Occipital's real page path by
`occipital-module/src/app_reader/broca/tests/unreadable_samples.rs`. Each
`NN-name.json` is the reply the app receives for `NN-name.request.json`.
`SHA256SUMS` lists every file but this README.

- `01-newest-page`: the newest page.
  - Ordinal 1, the person's message, reads as usual: its text part and its
    `text`. The photo it carried, whose stored data couldn't be read, is a
    part of its own kind in its place: `{type: "unreadable", partKey,
    partIndex}`, with nothing added to the row's `text`.
  - Ordinal 2, the answer, holds a `read` call whose result also carried such
    a file. The call keeps its result's size (`outputBytes`), and nothing
    marks the file: tool output is shown as text.
  - No row is `unreadable`.
- `02-exact-read`: the exact read of ordinal 2. The call carries its result's
  text.

Draw an `unreadable` part as a quiet marker in its place; an app that
doesn't know the kind skips it. A whole row is still `unreadable` only when
its stored body has no usable shape (see `phone-reader-unreadable-v1`).

**Text:** the chat text is invented.
