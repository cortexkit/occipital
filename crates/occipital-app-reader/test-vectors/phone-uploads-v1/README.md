# phone-uploads-v1

The phone's project uploads, recorded by CKIOS on 2026-10-26 from an iOS simulator
against the production harness module (`thalamus-harness` `c306be0f2`, daemon 0.37.10),
uploading into a throwaway Broca worker, `alfonso:bg_906640f571fdda6b`.

Each case has three files:

- `NN-<case>.request.json`: the bare params, as the phone sent them, apart from their text (see **Text** below).
- `NN-<case>.json`: the raw reply body.
- `NN-<case>.timing.txt`: when it was sent and how long it took.

Each `upload.chunk` case also has `.payload-sha256.txt`, giving the digest and size of
the piece sent as the request's raw framed payload. The bytes themselves aren't kept.

## How the module answers

All five operations answer the same way.

- **Success:** a response frame whose body is `{"result": …}`.
- **Refusal:** an error frame whose body is `{code, message}`, plus `detail` when the
  refusal has one.

Case 22 has no raw body, only the phone client's typed failure, in
`22-aborted-status-after-abort.refusal.json` and `.refusal.txt`. Its refusal,
`upload_unknown`, carries no `detail`. The other refusals here do carry one, and the
phone's recorder captured those as raw bodies. That difference comes from a layer after
the module, not from the module.

## Cases

| Files | What it shows |
|---|---|
| 00 | A warm-up `session.details` read. |
| 01–05 | A 69-byte text file. `upload.begin` gives `chunk_bytes` 1,048,576, then one chunk and `upload.finish` with the file's path. A second `upload.finish` gives the same answer, and `upload.abort` after finishing answers `aborted: false`. |
| 06, 24 | A steer into the running worker naming that path. The newest page (24) shows it as user row 11, carrying the receipt's `clientMessageId`. |
| 07–14 | 3,145,851 bytes in 1 MiB pieces. Status partway reads 1,048,576, a repeat of chunk 0 is accepted without changing `committed`, and finish places the file. |
| 15–17 | The name `../CON.txt` is placed as `_CON.txt`. Names are cleaned rather than refused. |
| 18–22 | A piece at offset 10 with nothing held is refused `offset_gap` (`detail.committed` 0), and finishing early is refused `checksum_mismatch` (0 of 49). Abort answers `aborted: true`, and status afterwards is refused `upload_unknown`. |
| 23 | `upload.begin` with a size of 2 GiB + 1 is refused `upload_too_large` (`detail.max_bytes`). |

## Redaction

The recording machine's user name is replaced in every path with the same-length
`user-redact`, so every byte length is unchanged. No other personal data was found.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
