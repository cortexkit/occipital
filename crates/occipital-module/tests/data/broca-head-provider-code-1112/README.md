# session.head provider-code reader samples

Synthetic data only. No live stores, accounts, credentials or provider services were used.

This bundle contains 16 **unmodified response bodies** from the real session-bound
`session.head` dispatch path: `before/` and `after/`, with four cases for each of
`legacy` and `store`. Each JSON file includes the served `{"result": ...}` wrapper.
The capture helper writes the response frame's body bytes directly, without
pretty-printing or normalizing them. `SHA256SUMS` hashes the raw files, this README,
the verification logs and the sample-validation output.

## Expected reader-visible differences

| Case | Before: last_run_error | After: last_run_error |
| --- | --- | --- |
| billing_exhausted | `{"class":"permanent"}` | `{"class":"permanent","provider_code":"billing_exhausted"}` |
| account_limit_exhausted | `{"class":"transient","retry_after_secs":120.5,"resets_at_ms":1800000120500}` | Same fields plus `"provider_code":"account_limit_exhausted"` |
| permanent_no_code | `{"class":"permanent"}` | Unchanged; no provider_code member |
| completed | No last_run_error member | Unchanged; no last_run_error member |

File names are `<storage>.<case>.json`. For example, compare
`before/store.account_limit_exhausted.json` with
`after/store.account_limit_exhausted.json`.

Runs in separate captures allocate different run ids, incarnations, history
versions, cursor digests and update times. These values were deliberately left in
the raw response bodies. The semantic comparison recorded in
`sample-validation.txt` masks only those runtime-generated fields and verifies
that adding provider_code on the two coded errors is the sole remaining change
within each storage. Storage-specific cursor differences are not normalized
between legacy and store: the files show each storage's actual reply.

## Provenance and reproduction

Base commit: `6c3c057ef65fdfa032257966a9bcd415bfe5ad04`.
Fix commit: `cb2f48480ce3e5bc3a240b92a7000d930a47bcc6`.

Tests in `crates/broca-module-serve/tests/head_read_additions.rs`:

- `legacy_head_run_error_carries_provider_code_from_status`
- `store_head_run_error_carries_provider_code_from_status`

The delivery rig serves framed management requests over its in-process transport
through the production serve dispatcher and registry. The synthetic provider
emits real normalized error events, and each storage's run loop writes the run
ending. Each test first completes a bootstrap run using a real frozen Anthropic
render configuration. For the new-storage test only, the operator route then
moves that session with `session.move_to_store`; all four captured runs execute
after the move through the new storage's loop. The legacy test never moves its
session. Both tests read named `run.status` replies as well as the default
`session.head` reply, and compare the head's error summary with literal expected
fields and the reported run-status code. The final completed run checks that no
stale error summary remains.

The baseline capture removed only the two provider_code assignments in
`serve.rs` and `store_head.rs` locally before copying the checkout to Linux.
Both tests then failed on the missing billing_exhausted code, after capturing
all eight baseline bodies: **0 passed, 2 failed, 20 unselected** (exit 100), in
`before-tests.log`. No baseline production fix remained during that run.
After restoring the assignments, the same tests passed: **2 passed, 0 failed,
20 unselected** (exit 0), in `after-tests.log`.

Run on Linux with 4 vCPUs and 12 GiB RAM. For each capture set `phase` to
`before` or `after` and use:

```sh
mkdir -p .cortexkit/internal-docs/research/head-provider-code-1112/"$phase"
nice -n 10 env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 \
  BROCA_LLM_RUNNER_SAMPLE_DIR=.cortexkit/internal-docs/research/head-provider-code-1112/"$phase" \
  cargo nextest run --locked -p broca-module-serve \
  -F broca-provider/live -F broca-provider/test-harness \
  -F broca-catalog/live -F broca-module-serve/test-clock \
  --test head_read_additions \
  -E 'test(head_run_error_carries_provider_code_from_status)'
```

`quickcheck.log` records the final staged-tree Linux commit attestation for
`broca-module` and `broca-module-serve`: fmt, all-target compilation/clippy,
**2400 tests passed, 0 failed, 835 unselected**, and two empty doctest targets.
The attested tree is `24c76d79063d0c40ec7ee752e3e40aa40a8321d2`. No native
kill-conformance, real-daemon or crash-harness release lane was run or is needed
specifically for this read-only field projection.
`gated-commit.log` confirms the exact tree matched the attestation and passed the
required local formatting and layout checks before committing.

## Reader handoff

This worktree-local report is the sandbox-safe handoff copy. Copy the complete
directory to:

`~/Work/Projects/CortexKit-scratch/shared-broca-readers/head-provider-code-1112/`

It is intentionally uncommitted. Reader compatibility runs against this bundle
must precede shipping; these captures and broca's regressions do not themselves
claim that downstream readers have been run.
