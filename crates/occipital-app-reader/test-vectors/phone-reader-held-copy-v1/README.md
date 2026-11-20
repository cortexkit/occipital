# phone-reader-held-copy-v1

Claims that name the copy they hold, on `session.transcript_page` and
`session.transcript_watch`.

## The rule

- **Asking:** a read with `acceptHeldCopy: true` is served `heldCopy`, an
  opaque string of at most 64 printable ASCII bytes. Without it nothing
  changes, so every earlier set stays as it was.
- **Which replies carry it:** a newest-page read, and every claim answer
  (`matched`, `diverged`, `cursor_expired`, `outside_window`). Each names the
  copy of the chat Occipital kept for that reply. A page back (`beforeOrdinal`
  or `beforeCursor`) records no copy and carries none.
- **What the app keeps:** the `heldCopy` from the reply that last set its
  newest end, kept while it pages back, and saved with its copy of the chat.
  The Full and Simple views keep their own; never carry one to the other.
- **Claims:** send it back as `claim.heldCopy`. While Occipital still keeps
  that copy at `heldVersion`, the claim is checked against exactly that copy.
  A name it no longer keeps (evicted, or the module restarted and hasn't seen
  that page again) is checked by the rules for a claim that names none.
- **Names:** a copy's name is worked out from its version and rows. A new
  version or a changed row gets a new name, and two copies holding the same
  rows at the same version share one.

## Files

The chat is the one in `phone-reader-simple-v2`, read in the Simple view: the
person's messages, the agent's `say` messages and asks among 1,300 messages
of the agent's prose. `m1307` holds a `say` whose prefrontal record (the
message it answers) arrives late. That changes `m1307`'s row in place, under
the same version.

- `01-newest-page`: the phone reads the newest three (`m1307`, `m1309`,
  `m1311`) and keeps its `heldCopy`.
- `02-page-back`: the phone pages back to `m1304`, `m1306`. No `heldCopy`.
- `03-wider-page`: after the record lands, the desktop reads a wider newest
  page, `m1` to `m1311`, at the same version, with another `heldCopy`. Its
  copy also ends at `m1311` and holds `m1304`, so a claim that names no copy
  would be checked against it and answered `matched`, and the phone would
  keep `m1307` without its `replyTo`.
- `04-claim-named`: the phone's claim (`m1304` to `m1311`) names its copy and
  is answered `diverged` from `m1307`, with the new row. Its `heldCopy` is the
  desktop's from `03`, since the phone now holds the same rows.
- `05-claim-caught-up`: the claim naming that copy is `matched`.
- `06-claim-unknown-copy`: a name Occipital doesn't keep, answered by the
  rules for a claim that names none.
- `07-full-newest-page`: the full view's newest page has a name of its own.

These replies come from Occipital's real handlers over a fake Broca route.
Each `.json` file is the response body and each `.request.json` file is the
request. `SHA256SUMS` covers every file but this README.
