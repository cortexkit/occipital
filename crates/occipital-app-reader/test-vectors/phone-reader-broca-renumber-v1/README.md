# phone-reader-broca-renumber-v1

One Broca chat as the apps read it before Broca's runner moved to runner contract 0.8,
and after. A phone that cached the chat before the move holds the before page. These
samples show what it gets when it reads or claims the chat afterwards.

What changes across the move:

- **Ordinals:** each message's `ordinal` is one higher, 0–3 before and 1–4 after,
  because runners now number entries from 1.
- **Message ids:** they stay the same, `m0`–`m3`.
- **Version:** the page `version` changes from `history:0:incarnation:…` to
  `history:<counter>:incarnation:…`, because the runner now keeps a history counter
  for the log. The log's identity (`incarnation`) is the same.

A different version is a history rewrite, so a phone drops its copy and takes the new
page. It never merges the two by ordinal. A claim built from the before page answers
`cursor_expired` with the whole page under the new version (case 03).

## Where the replies come from

Broca recorded one chat: a user message, an assistant tool call, its result and a
final reply. Broca 0.3.306 (runner contract 0.7) wrote it and answered `session.head`
and `session.read` with display facts. Broca 0.3.307 (runner contract 0.8) then
answered the same two reads from the same store, with no new send. Those four replies
are in `occipital-module/tests/data/broca-renumber-1103/`, with Broca's README and
checksums. Project, harness and session are made up.

Occipital served each page through the same serving path a phone's route takes:

- **Case 01:** served once by Occipital `8745905`, the last build that read both
  runner contracts, from the 0.3.306 replies. Later builds read only contract 0.8 and
  can't serve it again, so it is kept as those recorded bytes.
- **Cases 02 and 03:** served from the 0.3.307 replies by the commit that adds this
  folder. The test `contract_goldens::the_renumbering_samples_are_what_the_reader_serves`
  in `occipital-module` serves them again and fails if a reply's bytes change. So a
  change that alters them needs a new sample set, never an edit to this one.

`<name>.json` is the whole response body, `{"result": …}`. `<name>.request.json` is
the request's `{method, params}`.

## Cases

| Files | What it shows |
| --- | --- |
| `01-before-newest-page` | The newest page before the move: ordinals 0–3, version `history:0:…`. |
| `02-after-newest-page` | The same request after the move: ordinals 1–4 under the same message ids, and a new version. |
| `03-after-claim-held-before` | A claim built from case 01 (`heldFloorMessageId: m0`, `sinceMessageId: m3`, its version), sent after the move: `cursor_expired`, with the whole page in `resync` under the new version. |
