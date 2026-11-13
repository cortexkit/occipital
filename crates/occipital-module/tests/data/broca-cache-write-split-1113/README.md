# Cache-write lifetime samples for billing readers

Synthetic data only. No production session or store was opened.

## Emission switch

`broca_core::step_usage::EMIT_CACHE_WRITE_SPLIT` is committed as `false`.
`before/` captures that default-off behavior; `after/` captures the same code with
only that constant temporarily set to `true` for testing. Do not enable emission
until BASAL accepts these samples. No WAL record, session-store record or durable
usage-checkpoint format changes are included.

## Samples

Each phase contains both `legacy/` (WAL-backed) and `store/` (new session storage),
with `complete/` and `incomplete/` synthetic sessions. Each session has:

- `run.status.json`
- `run.result.json`
- `session.head.json`

These are unmodified `serde_json::to_vec` bytes from the actual registry serve
responses, not hand-built usage objects. They are response bodies, without a bus
or JSON-RPC envelope. Incarnation IDs, paths and times are synthetic test metadata
and differ between captures; compare usage members rather than entire replies.

The fixture uses a scripted provider with Anthropic-shaped `usage.iterations`:

- Complete: two counted serving iterations of `claude-sonnet`, with cache-write
  totals 30 and 70, five-minute writes 10 and 20, and one-hour writes 20 and 50.
  After enabling the switch, both run replies carry `(30, 70)` at the run level
  and in the single `by_model` entry; their existing total remains 100.
- Incomplete: the first and third iterations each report 30 writes split `(10,
  20)`; the middle reports 70 writes with no lifetime split. Both members remain
  absent at both levels even though later iterations are complete. The existing
  total remains 130.

The tests decode every run usage as llm-runner 0.8.49 `RunUsage` and assert
`consistent()`. Session usage is decoded as `SessionUsage` and checked separately.
Additional serve-path cases cover mixed models (only the fully measured model
keeps its pair), absent totals versus explicitly reported zero, malformed and
partial per-step splits, and aggregate overflow.

## Surface inventory and deliberate session-usage deferral

- `run.status`: both storages use `RunUsageSteps`; every known run's aggregate
  and each model's aggregate get the guarded fill.
- `run.result`: both storages use the same reconciled run usage, including its
  `by_model` entries.
- `session.head`: neither storage serves last-run `RunUsage`. Its `usage` is
  session-wide `SessionUsage`, whose totals flatten `RunUsage`. Session totals
  deliberately keep the pair absent on both storages: durable totals currently
  do not retain lifetime completeness, and scanning 150k-message sessions on
  every head is not acceptable. This deferral was explicitly approved by the
  parent; completeness can be persisted after the storage cutover. Existing
  session totals and absence rules remain unchanged.
- Subscribe `run_finished`: the control carries `broca_protocol::Usage`, not
  `RunUsage` or `ModelUsage`; its existing lifetime members already serialize.
  The live lifecycle event carries no usage. Neither is changed by the new
  aggregate-emission switch. Stored terminal usage and historical subscriber
  usage units are not rewritten.

A counted step with no `cache_write_tokens` invalidates the aggregate pair;
absence is not measured zero. `Some(0)` with an explicit `(Some(0), Some(0))`
pair is complete. A missing, one-sided, non-summing or overflowing split makes
that aggregate unknown. Global and per-model completeness are independent.

## Reproduction

The tests are in
`crates/broca-module/src/store_actor/cache_write_split_tests.rs`, nested under
`store_actor::status_tests::cache_write_split`. They run both real storage serve
paths, not a separate arithmetic-only implementation.

Use the Linux build server with 4 vCPUs and 12 GiB, `CARGO_BUILD_JOBS=4` and
`CARGO_INCREMENTAL=0`. The targeted command is:

```sh
nice -n 10 env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 \
  cargo nextest run --locked -p broca-module \
  -E 'test(store_actor::status_tests::cache_write_split::)'
```

Set `BROCA_CACHE_WRITE_SAMPLE_DIR` to an output directory to capture the complete
and incomplete cases. Run once with the switch enabled for `after/`, and once
with its committed false value for `before/`. Tracked files on the runner are
read-only, so change the constant locally between jobs, restoring the committed
file exactly afterwards. `SHA256SUMS` hashes the 24 raw sample files.

## Verification evidence

- Requested lock-only updates installed exactly runner 0.8.49 and conformance
  0.17.59 in `Cargo.lock`; no unrelated lock edges changed.
- `attestation.log`: Linux quickcheck for `broca-core broca-module broca-subc`
  passed fmt, workspace all-target clippy, 3,142 tests (zero failures, 225
  skipped), and two doctests (zero failures). The local gated commit reused the
  exact-tree attestation and passed local fmt/layout checks.
- `before-fix.log`: restored only the pre-fill `step_usage.rs` implementation
  while retaining the new test and a true emission switch. Exactly the selected
  `complete_lifetimes_fill_both_run_replies_on_both_storages` test failed: the
  served five-minute counter was null/absent, expected 30. Zero tests passed,
  one failed; nextest exit 100. This is the expected regression evidence.
- `enabled.log`: with the committed implementation and only its switch set
  true, all six filtered tests passed (zero failures; 1,504 filtered/skipped).
- A local data-only check verified 24 raw samples, 12 before/after usage
  comparisons, and all 24 SHA-256 checksums. Removing the two new members from
  complete run usage reproduces the old usage exactly; incomplete run usage
  and session usage remain equal without modification.

The implementation was committed through `scripts/gated-commit.sh` as
`2c2ff8a930219eca0d87003aa363b90ea0c6703d` before temporary regression edits, as
requested by the parent. The file and index were restored exactly and checked
clean before enabling the switch for the green run, and restored again to the
committed false value afterwards. The runner refuses writes to tracked files,
so the parent approved separate remote jobs with local temporary edits instead
of one job that rewrites the source.

An earlier attestation attempt passed all the same checks but refused to issue
an attestation because a relative sample-output path created unignored files
under the crate directory; `attestation-tree-change.log` retains that evidence.
Using an absolute path rooted in this ignored report directory resolved it.
Earlier setup attempts (an ignored script not uploaded and a refused tracked
file write) ran no tests. Fresh IDE inspection was unavailable (`INTERRUPTED`);
the authoritative all-target clippy/typecheck and test gates passed instead.

No native kill-conformance, real-daemon or crash-harness release lanes were run.
This projection-only change requires no additional native-specific test; the
integrating agent should include llm-runner 0.17.59 conformance in its usual
merged-batch lane before enabling emission. Session totals remain intentionally
unmeasured by lifetime, with no new scan and no durable state-format change.
