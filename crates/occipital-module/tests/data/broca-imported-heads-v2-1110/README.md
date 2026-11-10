# Imported head reader responses, v2

Product commit: `1eb63ae64754c534c0ebfced90fd5b6ad78804b4`.

## Source samples

Thalamus's regenerated samples are in
`.cortexkit/internal-docs/export-calls-1110-v2/`. The SHA-256 of that source
folder's `SHA256SUMS` is
`ef2669f71fd91cacc6111e4f78d00c175f61ab9ae73dc91d424fc4fbfbb060c1`.
All four source files verified against the manifest before copying, unchanged,
into the committed fixture directory:
`crates/ck-session-import/tests/fixtures/thalamus-export-calls-v1/`.

- OpenCode 2: `opencode2/export.json` and `opencode2/export.calls.json`.
- Pi: `pi/export.jsonl` and `pi/export.calls.json`.

Each sample contains two calls: one pinned, one unpinned. The side file records
each call's opaque key, original message/call locators, and individual start and
finish times. The effective carrier of all four sample calls is
`thalamus-harness`.

## Captured files

Each host directory contains three exact serialized Response frame bodies:

- `read-default-1.json`: `session.read` with `{}`.
- `read-display-facts-1.json`: `session.read` with
  `{"include_display_facts":true}`.
- `history-display-1.json`: `session.history` with `{"form":"display"}`.

These are the complete response JSON bodies, not extracted result objects. No
JSON re-encoding, normalization, timestamp replacement, or identity masking was
performed. Response correlations, storage incarnations and cursors are the ones
from this capture run and are not reusable production addresses.

## Capture method and verification

The real import/serve integration test
`head_import_serve::thalamus_call_side_file_keeps_pins_times_and_unknown_facts_through_serve`
imports the committed v2 fixtures into fresh new-storage sessions through
`session.import`. A temporary recorder in that test used its existing wire
request helper and wrote `Frame::body` bytes directly after each read above.
Only the two unmodified source-sample cases were recorded; the extra synthetic
cancelled-call case in the test did not overwrite these files. The recorder was
removed after capture, restoring the committed test exactly.

Executed on Linux with 8 vCPUs and `CARGO_BUILD_JOBS=8`:

```sh
export BROCA_IMPORTED_READ_RECORD_DIR="$PWD/.cortexkit/internal-docs/imported-head-reads-v2"
nice -n 10 env CARGO_INCREMENTAL=0 cargo nextest run --locked \
  -p broca-module-serve --features test-clock --test it \
  -E 'test(thalamus_call_side_file_keeps_pins_times_and_unknown_facts_through_serve)'
```

Run `b0696e26-082f-4e94-be3c-6d66ca5c7bf2`: 1 passed, 0 failed, 467 skipped.
All six files were returned byte-for-byte from the Linux job. Their JSON was
independently checked against the source call facts: both calls keep their
opaque keys/carriers, the unpinned call stays pinless, and `tool_times` and
display tool-call parts keep each call's own recorded times. Default reads omit
`tool_times`. The adjacent `SHA256SUMS` covers these six bodies and this README.

No additional product commit was made. The tracked worktree was restored clean
at the product commit named above.
