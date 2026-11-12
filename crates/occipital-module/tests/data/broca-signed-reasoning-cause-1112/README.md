# Signed reasoning rejection reader samples

Generated on 12 November 2026 from broca's real serializers in
`wire::signed_reasoning::signed_reasoning_cause_survives_wal_projection_and_real_run_surfaces`
and `signed_reasoning_cause_survives_new_storage_and_legacy_rebuild`, using synthetic
session `s1`, run `sample`, and synthetic input. No live store or provider was accessed.

The synthetic provider body uses the exact pinned Anthropic refusal text with request path
`messages.89.content.10`. It is classified as HTTP 400 / `invalid_request_error` by the real
Anthropic classifier. `after.*` retains its broca-assigned `cause`;
`before.*` clears only that optional field before recording the otherwise identical error,
so the real serializers emit the pre-field shape. These are not hand-written response JSON.

- `before.run.status.json`, `after.run.status.json`: raw Response frame bodies from `run.status`.
- `before.run.result.json`, `after.run.result.json`: raw Response frame bodies from `run.result`.
- `before.subscribe-error.json`, `after.subscribe-error.json`: raw StreamData frame bodies from
  `session.subscribe {"from":"start"}`, containing `kind: control`, `unit.type: error`.
  These are run error events, not transport Error frames. Cursors are opaque per-session values.
- `before.wal-run-finished.json`, `after.wal-run-finished.json`: real serialized terminal WAL records.
- `after.store-run-end.bin`: new storage's `RunEnd.encode_body()` bytes; decode with the RunEnd
  record kind, not as a complete framed log record. Its `error` is an opaque JSON member.

The after error carries `class: permanent`, `provider_code: request_invalid`, and
`cause: signed_reasoning_rejected`. The provider message remains diagnostic text; callers
route on the structured fields, never on the message or `provider_subcode`.

Reproduce on Linux with `BROCA_SIGNED_REASONING_SAMPLE_DIR` naming an output directory:

```sh
CARGO_BUILD_JOBS=4 nice -n 10 env CARGO_INCREMENTAL=0 cargo nextest run --locked \
  -p broca-module-serve --test it -E 'test(wire::signed_reasoning)'
```

`SHA256SUMS` covers every payload and this README. Verify with `shasum -a 256 -c SHA256SUMS`.
