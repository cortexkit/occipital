# phone-reader-head-rich-v3

A rich UI reply stamped with the reply catalog's current digest
(`a6ea6f5d65a8e71f9794c1a19da76730fe3c83eb22387d3d2aeaa0d9fc440963`, library
0.1.15), as the apps receive it. `phone-reader-head-rich-v1` and `-v2` stay
frozen. Their replies carry earlier digests (`beb0904e…` and `4e256f56…`), which
the library no longer holds, so an app draws them as their readable text. This
set is the one to test native drawing against.

The chat is invented, in the shape Broca serves it, and is served
through Occipital's real request handling by `occipital-module`'s
`broca::tests::rich_ui`, so the reply is byte for byte what a phone receives.
`01-newest-page.request.json` is the request the app sends and
`01-newest-page.json` the reply body. `SHA256SUMS` lists every file but this
README.

| Row | `role` | `richUi` | What it holds |
| --- | --- | --- | --- |
| 1 | `user` | none | the person's question |
| 2 | `assistant` | `{dialect: "1.0", catalogDigest: "a6ea6f5d…"}` | one `Badge` inside a sentence, a `Styled` line, a `Callout` with a title, and a `LinkCard` |

These are the tags a reply may hold when its runner records neither references
nor handles. Draw a row rich only when it carries `richUi` and its
`catalogDigest` is one the app's library holds; otherwise draw the reply's
readable text.

## Verify

```sh
(cd crates/occipital-app-reader/test-vectors/phone-reader-head-rich-v3 && shasum -a 256 -c SHA256SUMS)
```
