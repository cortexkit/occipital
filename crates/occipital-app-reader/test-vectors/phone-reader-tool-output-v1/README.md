# phone-reader-tool-output-v1

How a Broca chat's tool results reach the apps: as the text the model was given. Broca
stores each result in its own envelope (`{"kind": "text", "text": …}`, `json`, `denied`,
`error`, or `parts` holding text and media). Occipital reads it through the runner
contract's own conversion of that envelope (`owned_broca::tool_output`), which Broca's
model requests use too, so a step log shows what the model read and never the envelope.

Each request is served through Occipital's own handlers against a test runner holding one
Broca chat. The chat is invented, except case 05's tool result, which is Broca's recording
of a tool that returned a text part, a PNG and a PDF
(`occipital-module/tests/data/broca-tool-image-reads-1117/`). `NN-*.request.json` is the
request, and `NN-*.json` the whole reply body, `{"result": …}`.

Cases 01 to 05 are exact reads (`session.transcript_page {messageOrdinal}`) of one call
each. The call's `content` is the model's text and `isError` says whether the result was
an error. `outputBytes` counts that text in UTF-8 bytes.

| Files | What it shows |
| --- | --- |
| `01-text` | A text result, as is. |
| `02-json` | A JSON result, as compact JSON text. |
| `03-denied` | A call the person or a hook refused: `denied: <reason>`, with `isError: true`. |
| `04-error` | A provider outage: the error's message, then one line per member (`module: aft`, `code: module_reloading`, `nothing was sent`), the block the model reads. |
| `05-picture-and-document` | A result with text and two files. `content` joins the text parts with a line break and leaves the files out. `media` lists each file in order, as the file descriptor a photo part has (`type: "file"`, `mime`, `sha256`, `byteCount`, and `width` and `height` for a picture), without a part index. An app fetches a thumbnail or the bytes with `session.media` by `sha256`, as for a photo the person sent. |
| `06-newest-page` | The newest page. Each call's row carries `outputBytes` for the model's text, and neither `content` nor `media`. |

Only a runner declaring `tool_media_refs` (runner contract 0.8.61) serves a tool's files as
references, and Occipital asks it to (`tool_media: "refs"`) on every read. Elsewhere a
tool's file stays a `[media not shown: …]` line in the text, and `media` is absent.

`SHA256SUMS` covers every file except this README. The test
`app_reader::broca::tests::tool_output_samples` in `occipital-module` serves the set again
and fails if any byte changes, so a change that alters a reply needs a new set, never an
edit to this one.
