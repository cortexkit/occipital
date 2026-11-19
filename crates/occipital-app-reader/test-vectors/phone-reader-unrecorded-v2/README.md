# phone-reader-unrecorded-v2

This set supersedes `phone-reader-unrecorded-v1`, which remains frozen. `vouchedFromMessageId` on a matched claim is the oldest message compared within the observed window. A held message below that boundary is not certified. The session facts are unchanged.

A Broca session that Entorhinal has no record of, such as one of Magic Context's
dreamer runs (`mc-dream:1.59:0.0`), which no agent owns. Occipital reads it straight
from Broca, which decides whether the session exists and whether Occipital may read
it. Such a session is read only. Each file is the reply Occipital's own handlers
serve against a test runner. The app names the session as it always does
(`session`, `harness: "broca"`), with no new parameter.

- `01-newest-page.json`: the newest page. It has no `sender` and no `senderUnknown`,
  so the app may not send: show no composer and no Stop. Each row's `network` is
  `unknown`, because Occipital has no network facts for the session.
- `02-matched-claim.json`: a claim on that page, `matched` with the new row. The
  same holds.
- `03-details.json`: `session.details` with `commandSubject: true`. There's no
  `sender`, and no `subject` either, so the app lists no commands for it.
- `04-details-once-recorded.json`: the same details once Entorhinal records the
  session. Occipital opens it again through Entorhinal, so `sender` and `subject`
  come back, and sends work.

Sends, Stop, uploads and commands to an unrecorded session are refused
`session_unknown` with `detail.runner: null`, as before. A session Broca doesn't
have, or doesn't let Occipital read, is refused by Broca (`session_unknown` or
`provider_unavailable` naming its code), as any read is.
