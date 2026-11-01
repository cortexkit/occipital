# phone-reader-head-rich-v1

The first rich UI reply from a head on Thalamus's OpenCode runner, recorded from the
phone's reader on 2026-10-30 at about 10:09 UTC. The head's plan carried Occipital's
rich UI teaching, so its replies are stamped with Occipital's dialect and catalog.

The files are as recorded, apart from their chat text (see **Text** below). `SHA256SUMS` lists every one.

| Files | What they show |
| --- | --- |
| `60-warmup-details.*` | Session details before the send. |
| `61-display-watch.*` | A display watch opened before the send. One `display_gap` starts the run. The reasoning frame and the text frame each carry `richUi {dialect, catalogDigest}`. A `display_gap` follows when the finished reply moves to the stored transcript, then `display_end`. The short reply arrived in one text chunk. |
| `62-send.*` | The send, with its `clientMessageId`. |
| `63-newest-page.*` | The newest page. User row 98 echoes the send's `clientMessageId`. Assistant row 99 carries the same `richUi` as the live frames, and `completedAtMs`. |
| `64-exact-read.*` | An exact read of row 99, with the same `richUi`. |
| `65-details.*` | Session details after the turn. |

A stamped row's text uses Occipital's catalog: one Badge, one Callout and a three-row
Table. Draw a row rich only when it carries `richUi`, and match its `catalogDigest` to
the catalog the app holds.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
