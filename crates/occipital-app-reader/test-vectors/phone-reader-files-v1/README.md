# phone-reader-files-v1

Files the person attached to their messages, as an app reads them on a transcript
page. Each attached file is a `type: "file"` part beside the message's text parts, in
stored order:

```json
{"type":"file","filename":"pixel.png","mime":"image/png","byteCount":69,"partKey":"1","partIndex":1}
```

- `filename`, `mime` and `byteCount` are left out when the runner doesn't know them.
  Pi's images, for example, carry no filename.
- `width` and `height`, in pixels, appear when the runner records them. No case here
  carries them.
- The message also lists its files under `files`, the same descriptors without `type`
  and `partKey`.
- An exact read (`messageOrdinal`) is for a message's whole tool output and leaves
  files out, so keep the page's file parts for a message.

## Where the files come from

Occipital's app reader served this page at the commit that adds this folder, through
the same serving path a phone's route takes. The chat is a test OpenCode 2 session
(harness `opencode2`) whose two stored messages are the runner contract's own bodies,
from `display-opencode-v2.json` in cortexkit-role-llm-runner 0.8.0's
`test-vectors/llm-runner-v1`:

- `msg_text_and_file` is case `2.2.3-seq-54`, recorded from OpenCode 2.2.3: text and
  a 69-byte PNG.
- `msg_file_alone` is case `synthetic-existing-image-ref`, which the contract wrote by
  hand: the same PNG, with no text.

The runner contract gives a stored file's type and size only in its media reference,
and the reader takes them from there. `01-files-newest-page.json` is the whole response
body, `{"result": …}`, and `01-files-newest-page.request.json` is the request's
`{method, params}`. The test `app_reader::broca::tests::files_samples` in
`occipital-module` serves the request again and fails if the reply's bytes change, so
a reader change that alters them needs a new sample set, never an edit to this one.

## Cases

| Files | What it shows |
| --- | --- |
| `01-files-newest-page` | The newest page: a message with text and an image (`partIndex: 1`), then a message holding the image alone (`partIndex: 0`, `text: ""`). |
