# phone-reader-search-v1

The phone opening a transcript around a message it found by search, then paging older
and newer from there. CKIOS recorded it on 2026-10-29 from an iOS simulator against the
production harness module (`thalamus-harness` `4d8ce0fc4`), on the Canary Head (harness
`opencode`, session `ses_e82d035a7ffeFAtBg5sNRs6C7C`), whose newest message was ordinal
40.

The files are the recording apart from their chat text (see **Text** below) and the user name in one absolute path
in `35-anchor-newer-page-02.json`, replaced by `user-redact`, which has the same length.

## Cases

| Files | What it shows |
|---|---|
| `30-search-warmup-details` | `session.details` before the search reads. |
| `31-search-newest-page` | The newest page, for comparison. |
| `32-search-middle-exact` | An exact read of the message the search found (ordinal 20). |
| `33-anchor-page` | `aroundMessageId` with `limit: 3`: ordinals 17–23, `anchorOrdinal: 20`, `nextBeforeOrdinal: 17`, `nextAfterOrdinal: 23`. |
| `34-anchor-older-page` | `beforeOrdinal: 17`, `limit: 3`: ordinals 14–16. |
| `35-anchor-newer-page-01` | `afterOrdinal: 23`, `limit: 10`: ordinals 24–33, `nextAfterOrdinal: 33`. |
| `35-anchor-newer-page-02` | `afterOrdinal: 33`, `limit: 10`: ordinals 34–40, `reachedNewest: true`. |
| `36-anchor-message-not-found` | An unknown message id: `message_not_found`, `detail.version` naming the version checked. |
| `37-anchor-form-mixing` | `aroundMessageId` together with `beforeOrdinal`: `invalid_request`. |

## Reading the pages

- `limit` on an `aroundMessageId` read counts per side: up to `limit` messages before the
  anchor and up to `limit` after it, plus the anchor itself.
- Older pages continue as on any page: `nextBeforeOrdinal`, or `nextBeforeCursor` at the
  first numbered message when older unnumbered history remains.
- Newer pages continue with `nextAfterOrdinal` until a page ends at the newest message,
  which says `reachedNewest: true` instead.
- Every page here carries the same `version`. Join pages on it, as with older pages; a
  page with a different `version` means the history changed (a cut or a revert), so the
  phone reads again from the newest page.
- `message_not_found` means the message isn't in this version of the transcript: never
  written, or below a cut on this history line.
- A runner that can't find messages by id refuses `aroundMessageId` with `unsupported`,
  `detail.field` naming it. Both runners Thalamus serves today can, so this set has no
  recording of it; a unit test pins it.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
