# Phone upload samples (v1)

Tagged requests and replies of the harness module's five upload operations
(`upload.begin`, `upload.chunk`, `upload.status`, `upload.finish`, `upload.abort`) and
one sample per refusal code the phone's retry policy reads. The phone pins them by
tag and checksum.

## How they were produced

`app_reader::uploads::tests::samples::the_published_samples_are_what_the_handlers_answer`
(in `crates/thalamus-harness-module/src/app_reader/uploads/tests/samples.rs`) runs the
real handlers through the module's own frame loop: it binds a daemon-attested
`reserved:callosum` route with the device identity `fed-phone:<fingerprint>`, sends
each request as the phone does, and records the reply. `upload.chunk` is sent as a
framed request (`FLAG_BINARY`, body `u32 big-endian head length | JSON head |
payload`). The session resolves through the real Entorhinal resolver over a scripted
lookup.

Run with `THALAMUS_PRINT_PHONE_UPLOAD_SAMPLES=1` and `--nocapture`, the test prints
every file as `SAMPLE <name> <hex bytes>`; the files here are those bytes. Without the
variable it records the set again and fails if any file differs, so a change to a
reply shows up as a failing test before it reaches the phone. `SHA256SUMS` was made
with `shasum -a 256` over every file except itself and this README;
`the_upload_samples_are_complete_and_their_checksums_verify` checks it.

To make the recording reproducible:

- the clock is fixed at 2026-01-01T00:00:00Z (`expires_at` is milliseconds since the
  Unix epoch, 24 hours later);
- upload ids come from a counter instead of the random generator. Real ids are 128
  random bits, written as 32 lowercase hex characters, like these;
- free disk space is reported by the test (500 GiB, or just under the floor for
  `disk_floor`);
- the test's temporary project folder is replaced with `/path/to/project`, and paths
  use `/` whatever platform recorded them.

Nothing else is changed.

## Files

Each `.json` file has a `tag`, the operation (`op`), the `request` params, and either
the `reply` (the `result` of the response) or the `refusal` (the error body: `code`,
`message`, and `detail` where there is one). Chunk requests add `payload_hex`, the
payload bytes.

- `01-begin.json` … `05-abort.json`: one of each operation. `02-chunk.request.bin` is
  the exact framed body of the chunk request, head and payload.
- `refusal-<code>.json`: `upload_too_large`, `disk_floor`, `upload_unknown`,
  `upload_expired`, `offset_gap`, `chunk_mismatch`, `checksum_mismatch`,
  `destination_unsafe`, `session_on_other_machine`, `upload_not_yours`.

A payload over the declared 8 MiB never reaches the module: Callosum refuses it
`fed_payload_too_large`, so that code has no sample here.
