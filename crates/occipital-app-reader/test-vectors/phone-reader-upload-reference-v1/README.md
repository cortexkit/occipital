# phone-reader-upload-reference-v1

An app send that refers to an uploaded file by its path in the message text, on a Broca
worker and on an OpenCode head that Thalamus hosts.

Recorded by the phone team on 2026-10-28 against harness module `a3b0e86c9`. Each half
uploads a 70-byte PNG, sends a message whose text carries the uploaded path, then reads
the newest page about 25 s later. `SHA256SUMS` covers every file except this README.

## The text form

Thalamus does not accept `parts` on `session.enqueue_user_message` yet: a non-empty
`parts` array is refused `unsupported` and nothing is sent. A send refers to an upload by
putting the absolute path `upload.finish` returned into its text. The phone uses the
typed text, a blank line, then one line per file in attachment order:

```
Uploaded file: <absolute path from upload.finish>
```

The agent reads the file with its own tools. Nothing is inlined into the model's input.

## What each half shows

| Files | Shows |
|---|---|
| `00`-`04` | A Broca worker (`alfonso:bg_bf62751b4de7829e`). `upload.begin` carries `harness: "broca"`, the file lands under the worker's project in `.cortexkit/uploads/2026-10-28/probe-image.png`, the send's text carries that path, and on the page the user row `m8` echoes the send's `clientMessageId` and the worker's reply `m9` names the file. |
| `10`-`14` | The Canary Head (`ses_e82d035a7ffeFAtBg5sNRs6C7C`), hosted by Thalamus's OpenCode runner. `upload.begin` carries `harness: "opencode"`, the file lands under the head's project, the stored user row echoes the send's `clientMessageId`, and the head's reply names the file. The two rows before it on that page are from an earlier recording. |

`upload.begin` resolves the session the way reads and sends do: without `harness`, or
with `"broca"`, it is a Broca session; any other harness is routed by the runner
Entorhinal records. `upload.chunk` and `upload.finish` work from the upload id.

The macOS user name in paths is replaced with `user-redact`, the same length, in files
`02`, `03`, `04`, `12`, `13` and `14`.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
