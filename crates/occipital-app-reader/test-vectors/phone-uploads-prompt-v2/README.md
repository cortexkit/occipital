# phone-uploads-prompt-v2

Sending a photo or screenshot into the model's prompt, as a phone does it. Each case
is one request on a paired phone's route and the reply the phone receives.

This set replaces `phone-uploads-prompt-v1`. Cases 01 to 08 and the four refusals are
the same flow. The differences:

- A prompt begin's reply also carries `media_only`, and the model's image limits when
  the runner reports them (case 01).
- A photo above the model's limit is refused `media_too_large` (case 09).
- A runner that stores a message of files alone takes a photo with no text (cases 10
  and 11).

The upload ids in the last two refusals' requests differ from v1, because the new cases
take ids before them.

The flow:

1. `upload.begin` with `prompt: true`. The reply adds `destination`: `prompt` when the
   session's runner takes uploads and its model isn't known to refuse the file's type,
   `folder` otherwise. Without `prompt: true` the reply has no `destination` and the
   file always goes to the folder, as in `phone-upload-v1`. A `prompt` reply also
   carries:
   - `media_only`: `true` when the session's runner stores a message of files with no
     text (the runner contract's `media_only_send` group), `false` otherwise.
   - `max_image_bytes`: the largest image file the model takes, counting the file's
     bytes as uploaded. Present only when the runner reports it.
   - `max_image_dimension`: the longest side, in pixels, of one image in a request.
     Present only when the runner reports it.

   A phone can encode a photo to fit these limits before its pieces are sent.
2. The pieces, by `upload.chunk`, exactly as in `phone-upload-v1`. They aren't
   repeated here.
3. `upload.finish`. A prompt upload answers `{attachment_id, size}` instead of a path.
   Occipital keeps the bytes for a day, waiting for a send to name them.
4. `session.enqueue_user_message` with the message's text and a `type: "file"` part
   naming the `attachment_id`. Occipital copies the file to the session's runner and
   sends it with the text. A repeat under the same `clientMessageId` answers
   `duplicate: true` from Occipital's receipt, and matches files by their bytes, so a
   repeat that uploaded the same file again under a new id is the same message.
5. When a send is refused `media_unsupported` or `media_too_large`, nothing was sent.
   `media_too_large` names the exceeded limit in `detail`. The phone either calls
   `upload.place`, then sends the text with an `Uploaded file: <path>` line under a new
   `clientMessageId`, or encodes the photo smaller and uploads it again. `upload.place`
   puts the same file in the session's project and answers `{path, size}`.

A message of files with no text goes through only when the begin reply said
`media_only: true`; otherwise it is refused `invalid_request`, and Occipital makes up no
text for it. Only the device that uploaded a file can send or place it.

## Where the replies come from

Occipital's app reader served each request at the commit that adds this folder,
through the same serving path a phone's route takes. The runner is a stand-in that
declares the runner contract's `attachments` group, keeps what it receives, and refuses
the photo as the contract's `media_unsupported` in case 06 and `media_too_large` in case
09. It also declares `media_only_send` from case 10 on. The session's model takes
images and nothing else, so the screen recording in case 03 goes to the folder. It
reports a 5,242,880-byte limit and an 8,000-pixel longest side. The project's folder
is written as `/path/to/project`, and the dates as the test clock gives them
(1 January 2026).

`<name>.json` is the whole response body: `{"result": …}`, or the refusal
`{code, message, detail?}`. `<name>.request.json` is the request's `{method, params}`.
The test `app_reader::send::tests::prompt::samples` in `occipital-module` serves every
request again and fails if a reply's bytes change, so a change that alters them needs
a new sample set, never an edit to this one.

## Cases

| Files | What it shows |
| --- | --- |
| `01-begin-photo` | A screenshot's begin: `destination: "prompt"`, `media_only: false` and both image limits. |
| `02-finish-photo` | Its finish: `{attachment_id, size}`. |
| `03-begin-video` | A screen recording's begin with `prompt: true`: `destination: "folder"`, with no `media_only` or limits, because the model can't take video. |
| `04-send` | The message naming the screenshot: accepted, `duplicate: false`. |
| `05-send-repeat` | The same request again, as after a lost reply: `duplicate: true`, same `effectId`. |
| `06-send-media_unsupported` | A second photo the runner refuses for its type: `media_unsupported` with the runner's `detail`. Nothing was sent. |
| `07-place` | That photo placed in the project: `{path, size}`. |
| `08-resend-with-path` | The resend under a new `clientMessageId`, its text ending with the `Uploaded file:` line. |
| `09-send-media_too_large` | A photo the runner refuses as above the model's limit: `media_too_large`, with the limit in `detail`. Nothing was sent. |
| `10-begin-photo-alone` | A photo's begin once the runner stores messages of files alone: `media_only: true`. |
| `11-send-photo-alone` | That photo sent with empty text: accepted. |
| `refusal-invalid_request-files-alone` | A file with empty text, before the runner stored messages of files alone. |
| `refusal-upload_sent` | `upload.place` on the screenshot, which case 04 already sent. |
| `refusal-upload_not_finished` | A send naming an upload whose finish hasn't been called. |
| `refusal-upload_expired` | A send naming a photo finished a day earlier. |
