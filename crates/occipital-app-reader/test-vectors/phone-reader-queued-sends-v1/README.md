# phone-reader-queued-sends-v1

`queuedSends`: app sends that Thalamus accepted but whose message the agent hasn't
received yet, and sends that ended without being delivered.

Files `00` to `09` were recorded by the phone team on 2026-10-28 against harness module
`87d50e616` (placed at 18:46Z), on the Canary Head (harness `opencode`, session
`ses_e82d035a7ffeFAtBg5sNRs6C7C`). Two sends went in 2 s apart, the second while the
first's turn was running, so for a moment both were waiting. File `10` is synthetic and
explained below. `SHA256SUMS` covers every file except this README.

## The list

`queuedSends` is an array of entries, oldest first, at most 20. When more exist, the
reply also carries `queuedSendsTruncated: true`. The member is absent when the list is
empty.

| Member | Meaning |
|---|---|
| `clientMessageId` | The id the app sent with the message. |
| `effectId` | The send's receipt id, as `session.enqueue_user_message` answered it. |
| `acceptedAtMs` | When Thalamus accepted the send. |
| `state` | `queued`: accepted, not yet stored as a message. `unsent`: ended undelivered. |
| `reason` | On `unsent` only: why it ended. |
| `endedAtMs` | On `unsent` only: when it ended. |

An `unsent` entry stays listed for 48 hours after it ended, so a failed send is never
silent. A delivered send leaves the list. Thalamus checks delivery every 5 s, so a send
can still be listed `queued` for up to about 5 s after its row appears in the
transcript. Match entries to rows by `clientMessageId` (see notice 3 below).

Known `reason` values: `superseded`, `expired`, `template_part_gone`, `session_ended`,
`hold_expired` (still waiting after 8 days), `steer_expired` (a worker steer whose
window ended), or the runner's own reason for ending the send's run. `not_sent` means
the runner gave none. The member is an open string: show unknown values as "not sent".

## Where it appears

- Every `session.transcript_page` reply, whatever its verdict, including `matched`.
- `session.details`.
- Transcript watch notices that carry a page or a claim answer.
- A bare notice, `{"queuedSends": [...]}`, sent when only the list changed. `[]` means
  it emptied. Only watches whose request carries `acceptQueuedSends: true` get these,
  because phone builds since 298 close a watch on a notice kind they don't know.

## The recording

| File | What it shows |
|---|---|
| `00-warmup-details` | Details before any send: no `queuedSends`. |
| `01-newest-page` | The page before the sends. |
| `02-transcript-watch-opted-in` | A watch with `acceptQueuedSends: true`. Notice 1 is the claim answer, with no list yet. Notice 2 is a bare notice listing the first send. Notice 3 is a claim answer whose new turn is the first send's stored row (echoing `22d44c9a`), while that send is still listed `queued`: the delivery lag described above. Notice 4 is bare, listing both sends. Notice 5 is bare, listing the second only. Notices 6 and 7 are claim answers carrying the second send. Notice 8 is bare, `[]`. |
| `03-transcript-watch-plain` | A watch without the opt-in: no bare notices, but its claim answers carry the list. |
| `04-send-first`, `05-send-second` | The two `session.enqueue_user_message` calls and their receipts. |
| `06-newest-page-while-queued`, `07-details-while-queued` | Both sends listed `queued`. |
| `08-newest-page-after-delivery`, `09-details-after-delivery` | No `queuedSends`. Both user rows carry their `clientMessageId`. |
| `10-unsent-entries.synthetic.json` | Synthetic, as a bare notice: two `unsent` entries (`hold_expired`, `not_sent`) and one `queued`. Nothing undelivered happened naturally in the recording, so this file shows the `unsent` shape. A Thalamus test checks that the module's own list function produces exactly this value from the same three sends. |

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
