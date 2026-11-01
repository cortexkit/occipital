# phone-reader-empty-claim-v1

A chat with no messages, read and watched by an app that holds nothing yet. The
app sends a claim with `heldVersion` alone: "the newest page I read was empty at
this version".

- **While the page is still empty at that version:** a read answers
  `{unchanged: true}`, and a watch sends nothing.
- **Once the first message lands, or the version changes:** both answer
  `cursor_expired` with the newest page as `resync`. That is the verdict an app
  already applies to a held page that was replaced. The answer never carries
  `validThroughMessageId` or `vouchedFromMessageId`.
- **After that:** the app holds a page, and its next claim names messages as
  usual.

The reader that serves these requests accepts the claim. An app that sends it to
a reader that doesn't gets `invalid_claim`, and should fall back to reading the
newest page.

## Where the files come from

Occipital's app reader served these replies at the commit that adds this
folder. The session was a test Broca session, not a live one, because a live
session can't be held empty on demand. Each `.json` file is the whole reply as an
app receives it:
- for a read, the response body `{"result": …}`;
- for a watch, the notice body.

Each `.request.json` file is the request that produced it. The test
`app_reader::broca::tests::empty_claim_samples` in `occipital-module` serves
every request again and fails if a reply's bytes change, so a reader change
that alters them needs a new sample set, never an edit to this one.

## Cases

| Files | What it shows |
| --- | --- |
| `01-empty-page` | The newest page of a session that has never had a message. It has no rows, `reachedStart: true` and `emptyReason: "empty"`, and its `version` is the one the app then claims. |
| `02-empty-claim-unchanged` | The same request with `claim: {heldVersion}`, while the session is still empty: `unchanged: true` and the same `version`. |
| `03-empty-claim-watch-first-message` | A transcript watch opened with that claim. It stayed silent past two of its 30-second sweeps. Its first notice came after the person's first message (`m0`, "Hello"): `cursor_expired`, with the newest page as `resync`. |
| `04-empty-claim-first-message` | The `02` request read again after that message: the same `cursor_expired` answer as a read. |

`02` and `04` are the same request bytes, sent before and after the first
message.
