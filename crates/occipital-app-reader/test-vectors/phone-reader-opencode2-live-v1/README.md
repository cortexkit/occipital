# phone-reader-opencode2-live-v1

One OpenCode 2 reply streamed live, as an app's `session.display_watch` receives it,
with the chat's `session.details` while the run is active and after it ends.

The app opens the watch on an idle chat, as the phone does while the chat is on
screen, before the run starts. Then the runner starts a run with a rich UI stamp,
streams one reply in five pieces, stores it and ends the run.

- **The first notice is one frame:** `{"frame": {…}, "seq": N}`, the same shape as a
  `session.display_read` reply. On an idle chat it is `display_gap`, which tells the
  app to drop any live text it holds.
- **Every later notice is a batch:** `{"frames": [{"frame": {…}, "seq": N}, …], "seq": N}`,
  where the outer `seq` is the last frame's.
- **A text frame:**
  `{"kind": "display", "messageID", "partType": "text", "chunk", "richUi", "sessionID"}`.
  `chunk` adds to the words already shown for `messageID`. Occipital joins the pieces
  that arrive within about 300 ms into one chunk, so the five pieces arrive as two
  frames here. `richUi` is the run's stamp: dialect `1.0` and the original catalog
  1.0 digest, which every app build accepts.
- **The ends:** after the reply is stored, two `display_end` frames follow. The first
  ends the message, and the second ends the run.
- **The words match:** the chunks joined equal the stored reply's text, so an app
  can hand the live words over to the stored row.

## Where the replies come from

Occipital's app reader served each request at the commit that adds this folder,
through the same serving path a phone's route takes. The chat is a test OpenCode 2
session (harness `opencode2`) on a test runner, and the text is invented. The test
runner has no send path, so the details carry the model and the run but no `sender`.
`<name>.json` is the whole response body. `<name>.request.json` is the request's
`{method, params}`. A watch notice has no request of its own: they all answer
`01-session.display_watch.request.json`. The test
`app_reader::broca::tests::opencode2_live_samples` in `occipital-module` serves every
request again and fails if a byte changes, so a change that alters them needs a new
sample set, never an edit to this one.

## Cases

| Files | What it shows |
| --- | --- |
| `01-session.display_watch.request.json` | The watch request, with `acceptToolFrames`. |
| `02-watch-notice` | The first notice on the idle chat: `display_gap`. |
| `03-watch-notice` | The run's first words: the heading and first sentence, with the stamp. |
| `04-session.details-active` | The details while the run is active: `lastRun.state` `active`. |
| `05-watch-notice` | The rest of the words. |
| `06-watch-notice` | The reply is stored and the run ends: two `display_end` frames. |
| `07-session.details-after` | The details after the run: `lastRun.state` `completed`. |
