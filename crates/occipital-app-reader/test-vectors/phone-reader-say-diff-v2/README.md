# phone-reader-say-diff-v2

This set supersedes `phone-reader-say-diff-v1`, which remains frozen. The `activityToken` now includes the joined message material; this changes how an app checks for a new record, not the shape of the diff media.

`say` messages whose records carry `diff` media. It's the chat of
`phone-reader-say-v2`, with different records. What `phone-reader-say-v1` and
`phone-reader-say-v2` show still holds: an answered `say` is a `message` part
in the call's place, and the record adds its `replyTo`, `media` and alert
settings.

## The `diff` item

Prefrontal writes a message's media in its message record (tool-display
contract §9). Occipital passes each item to the `message` part's `media`
exactly as prefrontal wrote it, with no change of name or shape:

`{kind: "diff", artifact, repo: {host?, owner?, name}, pr | commit | range, files, added, removed}`

- **The changes:** exactly one of `pr` (`{number, head?}`, `head` a full SHA),
  `commit` (a full SHA) or `range` (`{base, head}`, both full SHAs).
- **The repository:** with `host` and `owner`, the apps can link it. A local
  checkout names only `name` and draws no link.
- **`files`:** at most 200 entries, each `{path, added, removed, from?}` or
  `{path, binary: true, from?}`. `from` is a renamed file's previous path. A
  binary file has no line counts.
- **`added`, `removed`:** the totals over every file.
- **`artifact`:** the unified diff itself (`text/x-diff`). Draw the card from
  the members above, and fetch the artifact only when the person opens it.
  Artifacts are prefrontal's: the apps read them from prefrontal, as for an
  image or a file in a message, never from Occipital.

## Files

- `01-newest-page`: the newest page.
  - `m2`'s message answers `m1` with a verified quote and carries two diffs.
    The first is pull request 512 with its head SHA, holding a changed file,
    a renamed file (`from`) and a binary file. The second is one commit,
    holding a changed file and a renamed binary file.
  - `m8`'s message carries a range in a local checkout (`repo: {name}` only),
    so it draws no repository link.
- `02-exact-read`: the exact read of `m2`, with the same message and media.

The SHAs, artifact ids and paths are invented.
