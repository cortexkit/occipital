# Recorded model-fallback reply facts

## Samples

`before/` and `after/` contain the exact pretty-JSON bytes produced by the real
`broca_wire::SessionReadResponse` serializer and `history::read` display serializer
in the actor-backed regression
`store_actor::entries_comparison::model_fallback::refused_reply_exposes_recorded_model_fallback_on_read_and_history`.
`SHA256SUMS` covers the six sample files, relative to this directory.

Each file contains the whole four-entry page, including both replies:

| Ordinal | Speaker | Outcome |
| --- | --- | --- |
| 1 | user | ordinary run input |
| 2 | assistant | requested `stub/m` answers; mentions the fallback model in text only |
| 3 | user | next run input |
| 4 | assistant | requested `stub/m` refuses; its discarded answer is replaced by `stub/claude-opus-4-8` |

The two runs use the real registry, store runtime, session actor, session loop and
encrypted scratch store, with a scripted provider. No live session was opened.

| File in each directory | Surface | Expected change |
| --- | --- | --- |
| `session-read.json` | `session.read`, default options | No fallback fact on either reply |
| `session-read-with-display-facts.json` | `session.read`, `include_display_facts: true` | Only ordinal 4 gains `model_fallback` |
| `session-history.json` | `session.history`, `form: display` | Only ordinal 4's `row` gains the same fact |

The before run used the new contract pins and regression test, with no production
projection changes. It failed specifically because ordinal 4's read fact was null
instead of `{requested: "stub/m", answered: "stub/claude-opus-4-8"}`. The after run
passed. These are independent scratch sessions, so incarnation, generation-based
message ids, cursor identity/digests, output-reference set ids and acceptance times
differ between runs; those differences are not caused by the new fact. All four
`session.read` message bodies compare equal as JSON values across before/after,
with and without display facts. Both samples retain `history_version: 1`.

`before-test.log` records the expected regression failure (0 passed, 1 failed).
`filtered-tests.log` records the initial green actor and serializer checks (4 passed,
0 failed). `zero-io-tests.log` records the final implementation's 63 passing checks:
the complete `store_actor::reads_tests` group, store-actor read-named tests, the
four original actor/serializer checks, and three map provenance/bound tests. The
after samples were refreshed by that final run. The requested-model probe after
the ten scheduled fallback replies has no fact.

The first attestation exposed an implementation regression: a run-scoped ledger
lookup made ordinary opt-in reads fall back to storage. Its log is retained as
`initial-attestation.log` (1259 passed, 2 failed, 864 skipped). The failing tests were
`store_actor::reads_tests::every_read_equals_a_fresh_views_at_every_step` and
`store_actor::reads_tests::in_memory_reads_refuse_only_where_they_did_before`.
Both assertions were kept unchanged; the transient fold-built map removed those
lookups, and both tests pass in the final 63-test run.

## Exact durable evidence

The session's append, open-log load/view and recovery folds pass the records they
already read to a bounded, non-encoded map keyed by assistant message id. The read
projection looks up that map without any ledger or storage read. It uses:

1. `RunStart.run_id` and `RunStart.model`: the originally requested model.
2. `StepStart.run_id`, `StepStart.step_id`, and `StepStart.mid`: the exact link from
   the completed step to the read entry's `mid`.
3. Matching `StepEnd.run_id` and `StepEnd.step_id`, and `StepEnd.fallback` decoded
   as `broca_wal::StepFallback`: an explicit `route.model` proves the runner
   substituted the refusal-fallback model; optional `served_model` overrides the
   answering id when the provider itself handed that routed request to another
   model. A schedule without a route is insufficient.
4. The assistant message's stored `origin`: the qualified answered id must match
   the model already served on its display row.

The refusal itself is recorded and made durable as `StepRefused` before resend.
The fact does not come from that response's usage, attempt usage, a model name in
text, or from the current refusal schedule. Attempt reservations name physical
sends and response envelopes can name serving models, but neither substitutes for
the committed route and requested model used here. Missing/invalid ids, absent
routes, mismatched entry origins and equal requested/answered ids produce no fact.

The map retains only fallback replies and is capped at 256 KiB, evicting the oldest
proofs rather than fetching them again. Its strings, tree and current-run/step
fold context are counted in `SessionHeld.rest`, and therefore in actor and cached
view memory reports. No map or fold context is encoded.

The deliberate omission path is sealed/checkpoint-only history: a sealed entry is
rebuilt from a transcript block, which contains no ledger `StepStart`/`StepEnd`
route evidence. A fresh load/view does not fetch those older ledger segments for
this fact; if its existing open-log traversal did not see the evidence, the fact
is absent. A live session may retain a previously proved fact until eviction.
Absence remains no claim, never a claim that the requested model answered.

`display::ModelFallback` validates the qualified ids and
`ReadMessage::check_model_fallback` validates the known assistant role. The new
storage's read path alone attaches the fact; `history::project` copies it to the
display row. Legacy reads keep `None`, which the serializer omits. No storage
record, stored field, model-bound body, content identity, tag or compaction byte
was added or changed.

## Subscribe and rollout

Broca's subscribe display lane carries text/reasoning/tool-input deltas, run status,
gaps and resets (`broca-protocol/src/display.rs`), not `DisplayRow` values or a
row-level `model`. Its authoritative assistant control unit carries an
`AssistantMessage`, also without a row-level `model`. There is no subscribe row
projection serving a model to annotate; neither payload was changed. Readers
fetch the annotated reply through `session.read` or display history.

No deployment or live emission was performed. The integrating agent must retain
the published readers-first rollout order; reader readiness is not asserted by
these tests. No native kill-conformance, real-daemon or crash-harness lane was run.

## Reproduction

Run on Linux with 4 CPUs / 12 GiB, `CARGO_BUILD_JOBS=4`,
`CARGO_INCREMENTAL=0`, and a 60-minute timeout:

```sh
BROCA_MODEL_FALLBACK_SAMPLES="$PWD/.cortexkit/internal-docs/research/model-fallback-1113/after" \
  nice -n 10 cargo nextest run --locked -p broca-module -p broca-wire \
  -E 'test(entries_comparison::model_fallback::) | test(stateful_caller_message_serializer_uses_borrowed_content) | test(messages_without_provider_state_are_unchanged)'
```

The sample environment variable is optional; absent it, the tests write no sample
files. The default-read serializer retains absent facts as absent, not `null`.

## Final gates

- Commit: `9064eda09db270a19cc583a0b2da1206c53d370b`
  (`mason: expose recorded refusal fallback on replies`).
- Final filtered command used packages `broca-module`, `broca-wire` and
  `broca-session-store`, and filter
  `test(store_actor::reads_tests::) | (test(store_actor::) & test(read)) | test(model_fallback::) | test(stateful_caller_message_serializer_uses_borrowed_content) | test(messages_without_provider_state_are_unchanged)`:
  63 passed, 0 failed, 2114 filtered/skipped.
- `scripts/linux-quickcheck-attest.sh --output .cortexkit/linux-attestation.txt
  broca-module broca-module-serve broca-session-store broca-subc broca-wire`:
  passed workspace all-target Clippy/typechecking, selected formatting, selected
  crate tests and guards; 3673 passed, 0 failed, 867 skipped. All five selected
  crates' doctest targets were empty (0 tests). Evidence: `attestation.log`.
- `scripts/gated-commit.sh -m "mason: expose recorded refusal fallback on replies"
  broca-module broca-module-serve broca-session-store broca-subc broca-wire`:
  reused the exact passing Linux attestation; local formatting and layout checks
  passed. Evidence: `commit-gate.log`.
- Sidekick comment review of the final implementation and public documentation:
  no findings. Earlier ambiguous storage/opt-in wording was clarified.
- Sample validation: all six SHA-256 entries generated; both read modes retain
  equal JSON message bodies across before/after and `history_version: 1`.

Native kill-conformance, real-daemon, crash-harness release lanes and ignored
read-cost benchmark bars were not run. The change adds no storage format or
model-bound behavior, so no additional native lane is requested specifically for
this projection; the integrating agent owns the normal merged-batch lanes.
