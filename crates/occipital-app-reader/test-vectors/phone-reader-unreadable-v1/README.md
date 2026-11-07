# phone-reader-unreadable-v1

An OpenCode 2 chat of three messages whose middle one can't be read, served
through Occipital's real page path by
`occipital-module/src/app_reader/broca/tests/unreadable_samples.rs`. Each
`NN-name.json` is the reply the app receives for `NN-name.request.json`.
`SHA256SUMS` lists every file but this README.

- `01-newest-page`: the newest page. Ordinals 1 and 3 are ordinary rows.
  Ordinal 2 is the unreadable placeholder: `unreadable: true`, role
  `unknown`, its `messageId` and `ordinal` kept, and one text part with no
  `partKey` or `partIndex`, since it stands for no stored part.
- `02-exact-read`: the exact read of ordinal 2, which is the same placeholder.

A row is unreadable when the runner contract can't project the stored body,
here a compaction checkpoint, which is never served. Draw a quiet marker from
the `unreadable` flag and never show the placeholder's text, which is English
and not meant for display. An unreadable row is cached like any other; a later
reader that can read the message changes a cached copy only once the chat's
page version changes.

**Text:** the chat text is invented.
