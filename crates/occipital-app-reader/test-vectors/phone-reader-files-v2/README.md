# phone-reader-files-v2

Files the person attached to their messages, as an app reads them, and the
images' bytes through `session.media`. It replaces `phone-reader-files-v1`: image
file parts now carry the image's digest and size.

A file part on a transcript page:

```json
{"type":"file","filename":"photo.jpg","mime":"image/jpeg","byteCount":677,"sha256":"d23fb9ff…","width":4,"height":8,"partKey":"1","partIndex":1}
```

- `sha256` is the file's digest, from the runner's media reference. An app asks for
  the image's bytes by it.
- `width` and `height` are the picture's size in pixels as it is drawn, after its
  EXIF orientation. They appear on images whose size Occipital knows. Occipital
  reads each image's first bytes once and remembers the size, and a page waits at
  most one second for those reads. An image not read in time comes without a size
  on that read and gets one on a later read. The page's `version` stays the same
  across both reads, so an app that keeps the earlier copy doesn't reload it.
- A file that isn't an image, such as a PDF, has a digest but no size.
- The message also lists its files under `files`, the same descriptors without
  `type` and `partKey`.
- An exact read (`messageOrdinal`) is for a message's whole tool output and leaves
  files out, so keep the page's file parts for a message.

`session.media {session, harness?, sha256, size}` reads an image the chat
references:

- `size: "thumb"` answers `{mime, width, height, data}`: a thumbnail at most 1024
  pixels on its longest side, never larger than the original, upright after its
  EXIF orientation. It is JPEG, or PNG when any pixel is transparent. `data` is
  standard base64.
- `size: "full"` answers `{mime, size, offset, data}`: the original's bytes from
  `offset` (default 0), at most `length` bytes (default and largest 1,048,576).
  The bytes are the original's, unchanged, so a viewer applies their EXIF
  orientation itself. To read on, ask again with `offset` moved past the bytes
  decoded, until it reaches `size`.
- Refusals: `unknown_media` when the chat doesn't reference the digest,
  `media_not_image` for a file that isn't an image or doesn't decode,
  `media_too_large_to_thumb` above 32 MiB or 50 megapixels, and `unsupported`
  when the session's runner serves no media.

## Where the replies come from

Occipital's app reader served each request at the commit that adds this folder,
through the same serving path a phone's route takes. The chat is a test OpenCode
2 session (harness `opencode2`) on a runner that declares the runner contract's
`attachments` group and serves `session.media_read`. Its messages:

- `msg_text_and_file`: the runner contract's case `2.2.3-seq-54`, recorded from
  OpenCode 2.2.3, with text and a 69-byte, one-pixel PNG. The contract's own body,
  from `display-opencode-v2.json` in cortexkit-role-llm-runner 0.8.0's
  `test-vectors/llm-runner-v1`.
- `msg_file_alone`: the contract's case `synthetic-existing-image-ref`, the same PNG
  with no text.
- `msg_photo`: written for this set. An 8 by 4 pixel JPEG whose EXIF orientation
  (6) turns it a quarter, as a phone held upright writes a photo. It is drawn 4
  wide and 8 tall.
- `msg_pdf`: written for this set. A short PDF.

`<name>.json` is the whole response body: `{"result": …}`, or the refusal
`{code, message}`. `<name>.request.json` is the request's `{method, params}`. The
test `app_reader::broca::tests::files_samples` in `occipital-module` serves every
request again and fails if a reply's bytes change, so a change that alters them
needs a new sample set, never an edit to this one. The refusals' `message` is the
test runner's wording; read the `code`.

## Cases

| Files | What it shows |
| --- | --- |
| `01-files-newest-page` | The newest page: each image part with `sha256`, `width` and `height`; the photo drawn 4 by 8; the PDF with `sha256` and no size. |
| `02-thumb` | The photo's thumbnail: JPEG, 4 by 8, upright. |
| `03-full` | The PNG's original bytes, in one piece: `offset` 0, `size` 69. |
| `refusal-media_not_image` | A thumbnail of the PDF. |
| `refusal-unknown_media` | A thumbnail of a digest the chat doesn't reference. |
