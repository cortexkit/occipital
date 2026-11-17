# phone-reader-broca-renumber-v2

The same Broca chat as `phone-reader-broca-renumber-v1`, before and after Broca's runner
moved to runner contract 0.8, served by a later Occipital. Read that set's README for what
changes across the move and where the replies come from.

The only difference from v1 is in cases 02 and 03: the tool call's `outputBytes` is now 6,
the length of the text the model read (`worked`), where v1 has 31, the length of Broca's
stored envelope (`{"kind":"text","text":"worked"}`). Occipital now reads a Broca tool
result through the runner contract's own conversion of that envelope, so an app's step
log shows what the model was given. Case 01 is v1's recorded bytes, unchanged, so it
still says 31.

The test `contract_goldens::the_renumbering_samples_are_what_the_reader_serves` in
`occipital-module` serves cases 02 and 03 again and fails if a reply's bytes change.
`<name>.json` is the whole response body, `{"result": …}`. `<name>.request.json` is the
request's `{method, params}`.

| Files | What it shows |
| --- | --- |
| `01-before-newest-page` | The newest page before the move, as v1 recorded it: ordinals 0–3, version `history:0:…`. |
| `02-after-newest-page` | The same request after the move: ordinals 1–4 under the same message ids, and a new version. |
| `03-after-claim-held-before` | A claim built from case 01, sent after the move: `cursor_expired`, with the whole page in `resync` under the new version. |
