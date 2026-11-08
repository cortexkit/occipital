# phone-reader-text-file-v1

A text file the person shared into an OpenCode 2 chat, as the apps receive it.
The chat's runner takes `text/plain` uploads into the model's prompt. It
stores the file as its own part, and the model reads its numbered text only in
the request, so the file never shows as the person's words.

The chat is a real OpenCode 2.2.3 session recorded by Thalamus's runner, served
through Occipital's own frame loop. The person typed "Here is a note." and
attached a 43-byte note with no file name. The note's text is invented.

`NN-*.request.json` is the request the app sends, and `NN-*.json` is the reply body
it receives.

| Files | What they show |
| --- | --- |
| `01-newest-page` | The newest page. Message 3 is the person's: its text is only what they typed, and it has one file part, `text/plain`, 43 bytes, with the file's sha256. The file is listed under `files` too. It has no `filename`, because the upload had none. |
| `02-full` | `session.media` with `size: "full"`: the note's bytes in one piece, base64. |
| `03-refusal-media_not_image` | `session.media` with `size: "thumb"`: refused, since a text file isn't an image. |

Draw a file part that isn't an image as a chip with its type and size. Open it
with a `full` read, never a `thumb` read.

`SHA256SUMS` covers every file except this README. A test in `occipital-module`
(`broca::tests::text_upload_1108`) serves the set again and fails if any byte
changes.
