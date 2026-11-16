# Not-delivered step 2: reader bodies and real-host proof

Recorded from production code at **a26166458c7d19e38636e9dbd04b997de1a1f6f2**,
compared with live build **d7942f06c**. No production change is delivered. The
recorder temporarily attached a test-only module to `session_events/live.rs`,
ran on the Linux builder, and was then removed. The retained recorder is
`.cortexkit/scripts/not-delivered-step2-recording.rs`.

This is a reader-handoff sample pack, **not a compatibility approval**. Occipital
(which serves the apps), prefrontal, and Magic Context must decode the applicable
exact files with their live builds. No reader repository or live reader was run
here. Reader names below identify the intended handoff audience,
not a claim that all readers currently call every operation.

## Scope and changed bodies

The comparison range also contains later length-refusal/compaction, Pi turn-end
steer and request-rendering changes. This pack covers the no-forward/permanent
handover work from `fd31f4fec` through `73e08da35` (including its corrective
commits), not those independent later changes. The `not_delivered` send shape and
`nothing_written` error already existed at d7942f06c; step 2 adds proof-aware
routes into them and preserves additional permanent-refusal information.

| Surface | Step-2 difference from live | Readers |
| --- | --- | --- |
| `session.send` / `session.send_status` | A typed, understood no-forward proof permits one retry: pending first, then `not_delivered` / reason `handover_failed` if the second attempt also proves no forwarding. The final reply has `run_id`, no `delivered` member; same-id send repeats the stored answer. No new outer fields. Known causes are listed below. A permanent refusal does **not** become `not_delivered`; its send retains `delivered.as: unknown` while its run is `error`. | Occipital, prefrontal |
| `run.status` / `run.result` | Two proven failures now end `error` with class `handover_failed`, provider code `provider_error`, cause equal to the proof. Permanent adapter refusals now end `error`, rather than the previous interrupted/not-sent outcome, preserving the mapping below. Error shapes are existing contract types with new reachable values. | Occipital, prefrontal, Magic Context |
| `session.head` | `last_run_state` reports the error ending; its sibling `last_run_error` carries the class/provider-code summary, not the full message or cause. | Occipital, prefrontal, Magic Context |
| `session.read` | No new RunStatus/error member. It returns host-history entries with run attribution; a prompt never forwarded contributes no user entry. Empty pages for these synthetic failures are included to prevent inventing an error member here. | Occipital, prefrontal, Magic Context |
| `session.subscribe` | Stored `run_ended` projects to `run_finished`, outer state `error` and event reason `error` (not the no-forward cause). The permanent branch's stored null reason and the no-forward branch's absent reason both project this way. No new fields. | Occipital, prefrontal, Magic Context |
| Lifecycle `turn_ended` bus notice | The error-ended run maps to outcome `failed` rather than the previous permanent refusal's interrupted/stopped outcome. The live bus path sends headers and an **empty payload**. The digest/key identify the canonical body served by `events_get`. | prefrontal, Magic Context; Occipital if subscribed |
| `events_get` | Same result wrapper (`body` string, digest, key, name, publication time). Canonical `turn_ended` body has outcome `failed`, `error_class: unknown`, no detail: the runner-to-turn mapper does not copy the RunError. The management transport wraps this unchanged result in its ordinary result envelope. | prefrontal, Magic Context; Occipital if authorized |
| Delivery receipt | Optional `not_forwarded: {cause, blocked_by?}`, optional permanent `run_error`, and `blocker_resolved: true` after exact holder acknowledgement; absent/false fields are omitted. State remains ordinary `not_sent`/adapter refusal so older builds can ignore additions. Unknown proof causes do not authorize retry; mere landing of an earlier operation does not resolve a blocker. | Internal thalamus runner/adapter/ledger; not exposed as a new Occipital/prefrontal/Magic Context API body |

There is **no bus `send_state` publisher or body** at this commit. This pack
records `turn_ended` plus `events_get` instead. No `send_state` body has been
synthesized.

Only `run.status` serializes `RunStatus`, and only `run.result` serializes
`RunResult`; there is no public `run.wait`. The no-`run_id` form of `run.status`
returns only a session-level state, not the ended run's error, so it has no new
step-2 ending body. No public management operation or session-event notification
exposes the raw ledger receipt; the receipt files are internal proof evidence.

Source anchors: `runner/events.rs:48-110,2035-2099,2226-2242`,
`runner/send.rs:615-680`, `runner/read.rs:781-803,2111-2141,2275-2448`,
`runner/subscribe.rs:491-515`, `runner/turn_events.rs:37-76`,
`session_events/live.rs:330-350`, `session_events/store.rs:124-153`, and
`crates/thalamus-harness/src/ledger.rs:176-246,866-929`.

## Ending cases

Each prefix has all twelve bodies/envelopes in the file index. `blocked` also has
a thirteenth receipt after acknowledgement. All ids, prompts, account labels,
scope references and model labels are synthetic fixture values; no account,
installed login, personal store, or real model service was used.

| Prefix | Trigger / proof | Full RunError after ending |
| --- | --- | --- |
| `host_unreachable` | Both handovers positively fail to connect before forwarding | `handover_failed` / `provider_error` / cause `host_unreachable` |
| `not_forwarded` | Both handovers have a typed pre-forward refusal | `handover_failed` / `provider_error` / cause `not_forwarded` |
| `nothing_written` | Both handovers have a zero-write proof | `handover_failed` / `provider_error` / cause `nothing_written` |
| `no_stdin` | Both checked writes find no stdin before writing | `handover_failed` / `provider_error` / cause `no_stdin` |
| `blocked` | Both exact blocked-operation proofs are acknowledged resolved before retry/ending; the recorder invokes the ledger's acknowledgement seam, not a timeout or guessed outcome | `handover_failed` / `provider_error` / cause `blocked` |
| `attachment_unreadable` | Permanent receipt from `refuse_for_good` | `handover_failed` / `provider_error` / cause `attachment_unreadable` |
| `host_configuration_invalid` | Permanent receipt from `refuse_for_good` | `host_relaunch_failed` / `provider_error` / cause `host_configuration_invalid` |
| `host_version_unsupported` | Permanent receipt from `refuse_for_good` | `host_relaunch_failed` / `provider_error` / cause `host_version_unsupported` |
| `host_policy_refused` | Permanent receipt from `refuse_for_good` | class `runner`, provider code and error reason `host_policy_refused`; no cause |
| `future-permanent` | Representative unknown/open permanent code, using the real fallback branch | class `provider`, provider code `future-permanent`; no cause |

The recorder uses the committed stand-in host's `not_forwarded` and
`not_sent_error` test seams. Each goes through its actual ledger refusal write,
runner observation, dispatch replies, subscription projection, transactional
lifecycle publisher, `Publisher::flush_one`, live `wire`, and authorized
`Publisher::get`. Dispatch reply strings are preserved byte-for-byte, with no
added newline. Receipts, subscription frames, get result objects and bus
envelopes are serialized from actual returned values. `turn-ended-body` preserves
the canonical body string exactly; `events-get.body` is the same string. Bus
envelope files are recording containers (`subject`, actual wire `headers` as
lists, actual `payload`), not JSON payloads published on NATS. No NATS broker or
reader process was needed or claimed. Publication timestamps are builder clock
values; all semantic ids and prompts are synthetic.

The body recorder uses the shared runner's OpenCode 2 stand-in fixture. It does
not relabel that fixture's canonical lifecycle body as Pi or Claude Code:
`harness: opencode2` in these samples is genuine. The real-host cases below
independently prove the Pi and Claude no-forward paths feed the same runner
ending logic; their host-specific session bytes are not included in this pack.

## Real-host and boundary test results

All builds/tests ran on `runon: linux,8c,16gb,net`. Nextest used the committed
default profile: `slow-timeout = { period = "60s", terminate-after = 15 }`, no
retries. Every filtered job completed in less than 15 minutes. No real-host test
failed, and no production fix was made.

Preparation commands (passed):

```sh
bash probes/opencode-linux/fetch.sh /tmp/oc2
npm ci --prefix pi-extension/thalamus
bash probes/claude-code-linux/fetch.sh /tmp/cc
cargo build --locked -p thalamus-harness-module --bin ck-thalamus-harness --features test-support
```

Pinned hosts: OpenCode 2.2.3, Pi 1.5.0, Claude Code 2.1.318. Fetch scripts
verified archive/binary SHA-256 values. `npm ci` installed 125 packages with zero
audit vulnerabilities; npm warned of unapproved dependency lifecycle scripts.
Cargo warned about seven pre-existing unused workspace dependencies.

Real-host command (3 passed, 0 failed, 2717 skipped):

```sh
THALAMUS_OPENCODE2_BIN=/tmp/oc2/opencode \
PI_BIN=$PWD/pi-extension/thalamus/node_modules/.bin/pi \
THALAMUS_CLAUDE_CODE_BIN=/tmp/cc/claude \
THALAMUS_TEST_HOLDER_IMAGE=$PWD/target/debug/ck-thalamus-harness \
cargo nextest run --locked -p thalamus-harness-module --features test-support \
  --lib --run-ignored only -E 'test(not_delivered) | test(not_forwarded)'
```

| Fully-qualified test | Result |
| --- | --- |
| `runner::tests_claude_code_real::not_delivered::real_claude_code_no_stdin_twice_reads_not_delivered_and_never_writes_the_prompt` | PASS, 1.889s |
| `runner::tests_opencode2_real::not_delivered::real_opencode2_unreachable_server_retries_once_then_never_receives_the_prompt` | PASS, 18.144s |
| `pi_hosted::tests::real::real_hosted_pi_with_closed_stdin_retries_once_then_reads_not_delivered` | PASS, 30.856s |

The fixture tests in
`crates/thalamus-harness-module/src/pi_head/open/tests/not_delivered.rs` and
`crates/thalamus-harness-module/src/opencode2/unresolved/tests/not_forwarded.rs`
are **non-ignored tests**, not additional real Pi/OpenCode binary tests. They use
FakePi / a stand-in OpenCode server and the actual holder. They were also run:

```sh
THALAMUS_TEST_HOLDER_IMAGE=$PWD/target/debug/ck-thalamus-harness \
cargo nextest run --locked -p thalamus-harness-module --features test-support \
  --lib -E 'test(opencode2::unresolved::tests::not_forwarded::) | test(pi_head::open::tests::not_delivered::)'
# 4 passed, 0 failed, 2716 skipped; the first alternative matched nothing
# because its actual module name ends in not_forwarded_tests.

cargo build --locked -p thalamus-harness-module --bin ck-thalamus-harness --features test-support
THALAMUS_TEST_HOLDER_IMAGE=$PWD/target/debug/ck-thalamus-harness \
cargo nextest run --locked -p thalamus-harness-module --features test-support \
  --lib -E 'test(opencode2::unresolved::tests::not_forwarded_tests::)'
# 4 passed, 0 failed, 2716 skipped.
```

| Fully-qualified test | Result |
| --- | --- |
| `pi_head::open::tests::not_delivered::p07_two_checked_closed_rpcs_read_not_delivered_and_never_reach_pi` | PASS, 1.417s |
| `pi_head::open::tests::not_delivered::p08_a_holders_zero_write_is_handed_over_once_more_and_lands_once` | PASS, 1.419s |
| `pi_head::open::tests::not_delivered::p08_two_holder_zero_writes_read_not_delivered_and_never_reach_pi` | PASS, 1.423s |
| `pi_head::open::tests::not_delivered::p07_a_closed_rpc_is_handed_over_once_more_and_lands_once` | PASS, 1.428s |
| `opencode2::unresolved::tests::not_forwarded_tests::a_holder_that_cannot_reach_its_server_records_a_typed_no_forward_proof` | PASS, 0.935s |
| `opencode2::unresolved::tests::not_forwarded_tests::a_holder_pre_forward_refusal_unknown_to_the_decoder_never_posts_the_prompt` | PASS, 0.935s |
| `opencode2::unresolved::tests::not_forwarded_tests::a_checked_holder_resolution_releases_the_blocked_input_not_the_unknown_earlier_one` | PASS, 0.935s |
| `opencode2::unresolved::tests::not_forwarded_tests::an_unreadable_attachment_is_recorded_not_sent_with_its_permanent_error` | PASS, 0.988s |

## Recording command and reproducibility

To reproduce, copy the retained Rust recorder to the worktree root as
`step2-recording.rs`, temporarily add this **test-only** declaration before
`fn wire` in `crates/thalamus-harness-module/src/session_events/live.rs`, and remove
the declaration and root copy after the remote job ends:

```rust
#[cfg(test)]
#[path = "../../../../step2-recording.rs"]
mod step2_recording;
```

```sh
STEP2_RECORDING_OUTPUT=$PWD/step2-recorded-bodies.json \
cargo nextest run --locked -p thalamus-harness-module --features test-support \
  --lib -E 'test(record_step2_reader_bodies)' --success-output immediate
```

Return `step2-recorded-bodies.json` from that job. The final run passed:
`session_events::live::step2_recording::record_step2_reader_bodies`, 1.256s,
**1 passed, 0 failed, 2720 skipped; 121 bodies/envelopes across 10 endings**.
The first recorder compile attempt failed with E0308 (NativeSessionId used where
SessionId was required). Two recorder-only test attempts then failed at fixture
setup: invalid account label `synthetic-account`, and `event_not_visible` without
an owner-authorized scope stamp. These were corrected only in the temporary
recorder (account `synthetic_account`, authorized matching stamp); no committed
real-host or production test failed.

Materialize returned strings without normalizing or redacting their bytes:

```sh
python3 .cortexkit/scripts/not-delivered-step2-materialize.py \
  step2-recorded-bodies.json \
  .cortexkit/internal-docs/samples/not-delivered-step2-v1
```

`SHA256SUMS` lists every JSON file and this README, in filename order, and excludes
itself. The generated index below names every case, surface and reader handoff.
No manifests or lockfiles changed. Crate clippy/typecheck/full-suite/mutation
gates are not applicable to this documentation-only delivery; the temporary
recorder was compiled by its filtered nextest run and then restored away.

The retained `not-delivered-step2-verify.py` validates the materialized files
against the original returned capture strings, explicit expected ending values,
the notice/get/body digest links, all manifest entries, the README file index,
and scans JSON for personal home paths/names, email addresses and credential
patterns. To run it on the builder, copy this sample folder, the returned capture
JSON and the verifier into an untracked root folder `sample-verification/`
(ignored `.cortexkit` files are not transferred to builders), then use:

```sh
python3 sample-verification/verify.py sample-verification/samples sample-verification/capture.json
```

The Linux verifier ran with Python 3.13.5 and passed: **121 exact captured
bodies/envelopes, 10 explicit ending contracts, 10 bus/get digest links, 122
SHA-256 entries and README index entries; 121 JSON files scanned with zero
personal-data/credential pattern hits**. This automated scan supplements the
synthetic-fixture provenance; it is not a claim to detect every possible form of
personal data. The final manifest is regenerated after prose edits.

<!-- GENERATED FILE INDEX -->

## Recorded file index

| File | Case | Body / surface | Reader handoff |
| --- | --- | --- | --- |
| `attachment_unreadable--events-get.json` | `attachment_unreadable` | events_get result object, including exact canonical body string | prefrontal, Magic Context; Occipital if authorized |
| `attachment_unreadable--receipt.json` | `attachment_unreadable` | Delivery receipt before terminal retirement | thalamus runner/adapter/ledger only; not the three external readers |
| `attachment_unreadable--run-finished.json` | `attachment_unreadable` | session.subscribe run_finished frame | Occipital, prefrontal, Magic Context |
| `attachment_unreadable--run.result.json` | `attachment_unreadable` | run.result, full RunResult | Occipital, prefrontal, Magic Context |
| `attachment_unreadable--run.status.json` | `attachment_unreadable` | run.status, full RunStatus | Occipital, prefrontal, Magic Context |
| `attachment_unreadable--send-first.json` | `attachment_unreadable` | session.send, initial attempt | Occipital, prefrontal |
| `attachment_unreadable--send-repeat.json` | `attachment_unreadable` | session.send, same send_id after ending | Occipital, prefrontal |
| `attachment_unreadable--send-status.json` | `attachment_unreadable` | session.send_status after ending | Occipital, prefrontal |
| `attachment_unreadable--session.head.json` | `attachment_unreadable` | session.head, last_run_state and last_run_error summary | Occipital, prefrontal, Magic Context |
| `attachment_unreadable--session.read.json` | `attachment_unreadable` | session.read, history page (no RunStatus member) | Occipital, prefrontal, Magic Context |
| `attachment_unreadable--turn-ended-body.json` | `attachment_unreadable` | Canonical turn_ended body retrieved through events_get | prefrontal, Magic Context; Occipital if authorized |
| `attachment_unreadable--turn-ended-notice.json` | `attachment_unreadable` | Lifecycle bus notice: subject, live wire headers and payload | prefrontal, Magic Context; Occipital if subscribed |
| `blocked--events-get.json` | `blocked` | events_get result object, including exact canonical body string | prefrontal, Magic Context; Occipital if authorized |
| `blocked--receipt-blocker-resolved.json` | `blocked` | Delivery receipt after exact blocker acknowledgement | thalamus runner/adapter/ledger only; not the three external readers |
| `blocked--receipt.json` | `blocked` | Delivery receipt before terminal retirement | thalamus runner/adapter/ledger only; not the three external readers |
| `blocked--run-finished.json` | `blocked` | session.subscribe run_finished frame | Occipital, prefrontal, Magic Context |
| `blocked--run.result.json` | `blocked` | run.result, full RunResult | Occipital, prefrontal, Magic Context |
| `blocked--run.status.json` | `blocked` | run.status, full RunStatus | Occipital, prefrontal, Magic Context |
| `blocked--send-first.json` | `blocked` | session.send, initial attempt | Occipital, prefrontal |
| `blocked--send-repeat.json` | `blocked` | session.send, same send_id after ending | Occipital, prefrontal |
| `blocked--send-status.json` | `blocked` | session.send_status after ending | Occipital, prefrontal |
| `blocked--session.head.json` | `blocked` | session.head, last_run_state and last_run_error summary | Occipital, prefrontal, Magic Context |
| `blocked--session.read.json` | `blocked` | session.read, history page (no RunStatus member) | Occipital, prefrontal, Magic Context |
| `blocked--turn-ended-body.json` | `blocked` | Canonical turn_ended body retrieved through events_get | prefrontal, Magic Context; Occipital if authorized |
| `blocked--turn-ended-notice.json` | `blocked` | Lifecycle bus notice: subject, live wire headers and payload | prefrontal, Magic Context; Occipital if subscribed |
| `future-permanent--events-get.json` | `future-permanent` | events_get result object, including exact canonical body string | prefrontal, Magic Context; Occipital if authorized |
| `future-permanent--receipt.json` | `future-permanent` | Delivery receipt before terminal retirement | thalamus runner/adapter/ledger only; not the three external readers |
| `future-permanent--run-finished.json` | `future-permanent` | session.subscribe run_finished frame | Occipital, prefrontal, Magic Context |
| `future-permanent--run.result.json` | `future-permanent` | run.result, full RunResult | Occipital, prefrontal, Magic Context |
| `future-permanent--run.status.json` | `future-permanent` | run.status, full RunStatus | Occipital, prefrontal, Magic Context |
| `future-permanent--send-first.json` | `future-permanent` | session.send, initial attempt | Occipital, prefrontal |
| `future-permanent--send-repeat.json` | `future-permanent` | session.send, same send_id after ending | Occipital, prefrontal |
| `future-permanent--send-status.json` | `future-permanent` | session.send_status after ending | Occipital, prefrontal |
| `future-permanent--session.head.json` | `future-permanent` | session.head, last_run_state and last_run_error summary | Occipital, prefrontal, Magic Context |
| `future-permanent--session.read.json` | `future-permanent` | session.read, history page (no RunStatus member) | Occipital, prefrontal, Magic Context |
| `future-permanent--turn-ended-body.json` | `future-permanent` | Canonical turn_ended body retrieved through events_get | prefrontal, Magic Context; Occipital if authorized |
| `future-permanent--turn-ended-notice.json` | `future-permanent` | Lifecycle bus notice: subject, live wire headers and payload | prefrontal, Magic Context; Occipital if subscribed |
| `host_configuration_invalid--events-get.json` | `host_configuration_invalid` | events_get result object, including exact canonical body string | prefrontal, Magic Context; Occipital if authorized |
| `host_configuration_invalid--receipt.json` | `host_configuration_invalid` | Delivery receipt before terminal retirement | thalamus runner/adapter/ledger only; not the three external readers |
| `host_configuration_invalid--run-finished.json` | `host_configuration_invalid` | session.subscribe run_finished frame | Occipital, prefrontal, Magic Context |
| `host_configuration_invalid--run.result.json` | `host_configuration_invalid` | run.result, full RunResult | Occipital, prefrontal, Magic Context |
| `host_configuration_invalid--run.status.json` | `host_configuration_invalid` | run.status, full RunStatus | Occipital, prefrontal, Magic Context |
| `host_configuration_invalid--send-first.json` | `host_configuration_invalid` | session.send, initial attempt | Occipital, prefrontal |
| `host_configuration_invalid--send-repeat.json` | `host_configuration_invalid` | session.send, same send_id after ending | Occipital, prefrontal |
| `host_configuration_invalid--send-status.json` | `host_configuration_invalid` | session.send_status after ending | Occipital, prefrontal |
| `host_configuration_invalid--session.head.json` | `host_configuration_invalid` | session.head, last_run_state and last_run_error summary | Occipital, prefrontal, Magic Context |
| `host_configuration_invalid--session.read.json` | `host_configuration_invalid` | session.read, history page (no RunStatus member) | Occipital, prefrontal, Magic Context |
| `host_configuration_invalid--turn-ended-body.json` | `host_configuration_invalid` | Canonical turn_ended body retrieved through events_get | prefrontal, Magic Context; Occipital if authorized |
| `host_configuration_invalid--turn-ended-notice.json` | `host_configuration_invalid` | Lifecycle bus notice: subject, live wire headers and payload | prefrontal, Magic Context; Occipital if subscribed |
| `host_policy_refused--events-get.json` | `host_policy_refused` | events_get result object, including exact canonical body string | prefrontal, Magic Context; Occipital if authorized |
| `host_policy_refused--receipt.json` | `host_policy_refused` | Delivery receipt before terminal retirement | thalamus runner/adapter/ledger only; not the three external readers |
| `host_policy_refused--run-finished.json` | `host_policy_refused` | session.subscribe run_finished frame | Occipital, prefrontal, Magic Context |
| `host_policy_refused--run.result.json` | `host_policy_refused` | run.result, full RunResult | Occipital, prefrontal, Magic Context |
| `host_policy_refused--run.status.json` | `host_policy_refused` | run.status, full RunStatus | Occipital, prefrontal, Magic Context |
| `host_policy_refused--send-first.json` | `host_policy_refused` | session.send, initial attempt | Occipital, prefrontal |
| `host_policy_refused--send-repeat.json` | `host_policy_refused` | session.send, same send_id after ending | Occipital, prefrontal |
| `host_policy_refused--send-status.json` | `host_policy_refused` | session.send_status after ending | Occipital, prefrontal |
| `host_policy_refused--session.head.json` | `host_policy_refused` | session.head, last_run_state and last_run_error summary | Occipital, prefrontal, Magic Context |
| `host_policy_refused--session.read.json` | `host_policy_refused` | session.read, history page (no RunStatus member) | Occipital, prefrontal, Magic Context |
| `host_policy_refused--turn-ended-body.json` | `host_policy_refused` | Canonical turn_ended body retrieved through events_get | prefrontal, Magic Context; Occipital if authorized |
| `host_policy_refused--turn-ended-notice.json` | `host_policy_refused` | Lifecycle bus notice: subject, live wire headers and payload | prefrontal, Magic Context; Occipital if subscribed |
| `host_unreachable--events-get.json` | `host_unreachable` | events_get result object, including exact canonical body string | prefrontal, Magic Context; Occipital if authorized |
| `host_unreachable--receipt.json` | `host_unreachable` | Delivery receipt before terminal retirement | thalamus runner/adapter/ledger only; not the three external readers |
| `host_unreachable--run-finished.json` | `host_unreachable` | session.subscribe run_finished frame | Occipital, prefrontal, Magic Context |
| `host_unreachable--run.result.json` | `host_unreachable` | run.result, full RunResult | Occipital, prefrontal, Magic Context |
| `host_unreachable--run.status.json` | `host_unreachable` | run.status, full RunStatus | Occipital, prefrontal, Magic Context |
| `host_unreachable--send-first.json` | `host_unreachable` | session.send, initial attempt | Occipital, prefrontal |
| `host_unreachable--send-repeat.json` | `host_unreachable` | session.send, same send_id after ending | Occipital, prefrontal |
| `host_unreachable--send-status.json` | `host_unreachable` | session.send_status after ending | Occipital, prefrontal |
| `host_unreachable--session.head.json` | `host_unreachable` | session.head, last_run_state and last_run_error summary | Occipital, prefrontal, Magic Context |
| `host_unreachable--session.read.json` | `host_unreachable` | session.read, history page (no RunStatus member) | Occipital, prefrontal, Magic Context |
| `host_unreachable--turn-ended-body.json` | `host_unreachable` | Canonical turn_ended body retrieved through events_get | prefrontal, Magic Context; Occipital if authorized |
| `host_unreachable--turn-ended-notice.json` | `host_unreachable` | Lifecycle bus notice: subject, live wire headers and payload | prefrontal, Magic Context; Occipital if subscribed |
| `host_version_unsupported--events-get.json` | `host_version_unsupported` | events_get result object, including exact canonical body string | prefrontal, Magic Context; Occipital if authorized |
| `host_version_unsupported--receipt.json` | `host_version_unsupported` | Delivery receipt before terminal retirement | thalamus runner/adapter/ledger only; not the three external readers |
| `host_version_unsupported--run-finished.json` | `host_version_unsupported` | session.subscribe run_finished frame | Occipital, prefrontal, Magic Context |
| `host_version_unsupported--run.result.json` | `host_version_unsupported` | run.result, full RunResult | Occipital, prefrontal, Magic Context |
| `host_version_unsupported--run.status.json` | `host_version_unsupported` | run.status, full RunStatus | Occipital, prefrontal, Magic Context |
| `host_version_unsupported--send-first.json` | `host_version_unsupported` | session.send, initial attempt | Occipital, prefrontal |
| `host_version_unsupported--send-repeat.json` | `host_version_unsupported` | session.send, same send_id after ending | Occipital, prefrontal |
| `host_version_unsupported--send-status.json` | `host_version_unsupported` | session.send_status after ending | Occipital, prefrontal |
| `host_version_unsupported--session.head.json` | `host_version_unsupported` | session.head, last_run_state and last_run_error summary | Occipital, prefrontal, Magic Context |
| `host_version_unsupported--session.read.json` | `host_version_unsupported` | session.read, history page (no RunStatus member) | Occipital, prefrontal, Magic Context |
| `host_version_unsupported--turn-ended-body.json` | `host_version_unsupported` | Canonical turn_ended body retrieved through events_get | prefrontal, Magic Context; Occipital if authorized |
| `host_version_unsupported--turn-ended-notice.json` | `host_version_unsupported` | Lifecycle bus notice: subject, live wire headers and payload | prefrontal, Magic Context; Occipital if subscribed |
| `no_stdin--events-get.json` | `no_stdin` | events_get result object, including exact canonical body string | prefrontal, Magic Context; Occipital if authorized |
| `no_stdin--receipt.json` | `no_stdin` | Delivery receipt before terminal retirement | thalamus runner/adapter/ledger only; not the three external readers |
| `no_stdin--run-finished.json` | `no_stdin` | session.subscribe run_finished frame | Occipital, prefrontal, Magic Context |
| `no_stdin--run.result.json` | `no_stdin` | run.result, full RunResult | Occipital, prefrontal, Magic Context |
| `no_stdin--run.status.json` | `no_stdin` | run.status, full RunStatus | Occipital, prefrontal, Magic Context |
| `no_stdin--send-first.json` | `no_stdin` | session.send, initial attempt | Occipital, prefrontal |
| `no_stdin--send-repeat.json` | `no_stdin` | session.send, same send_id after ending | Occipital, prefrontal |
| `no_stdin--send-status.json` | `no_stdin` | session.send_status after ending | Occipital, prefrontal |
| `no_stdin--session.head.json` | `no_stdin` | session.head, last_run_state and last_run_error summary | Occipital, prefrontal, Magic Context |
| `no_stdin--session.read.json` | `no_stdin` | session.read, history page (no RunStatus member) | Occipital, prefrontal, Magic Context |
| `no_stdin--turn-ended-body.json` | `no_stdin` | Canonical turn_ended body retrieved through events_get | prefrontal, Magic Context; Occipital if authorized |
| `no_stdin--turn-ended-notice.json` | `no_stdin` | Lifecycle bus notice: subject, live wire headers and payload | prefrontal, Magic Context; Occipital if subscribed |
| `not_forwarded--events-get.json` | `not_forwarded` | events_get result object, including exact canonical body string | prefrontal, Magic Context; Occipital if authorized |
| `not_forwarded--receipt.json` | `not_forwarded` | Delivery receipt before terminal retirement | thalamus runner/adapter/ledger only; not the three external readers |
| `not_forwarded--run-finished.json` | `not_forwarded` | session.subscribe run_finished frame | Occipital, prefrontal, Magic Context |
| `not_forwarded--run.result.json` | `not_forwarded` | run.result, full RunResult | Occipital, prefrontal, Magic Context |
| `not_forwarded--run.status.json` | `not_forwarded` | run.status, full RunStatus | Occipital, prefrontal, Magic Context |
| `not_forwarded--send-first.json` | `not_forwarded` | session.send, initial attempt | Occipital, prefrontal |
| `not_forwarded--send-repeat.json` | `not_forwarded` | session.send, same send_id after ending | Occipital, prefrontal |
| `not_forwarded--send-status.json` | `not_forwarded` | session.send_status after ending | Occipital, prefrontal |
| `not_forwarded--session.head.json` | `not_forwarded` | session.head, last_run_state and last_run_error summary | Occipital, prefrontal, Magic Context |
| `not_forwarded--session.read.json` | `not_forwarded` | session.read, history page (no RunStatus member) | Occipital, prefrontal, Magic Context |
| `not_forwarded--turn-ended-body.json` | `not_forwarded` | Canonical turn_ended body retrieved through events_get | prefrontal, Magic Context; Occipital if authorized |
| `not_forwarded--turn-ended-notice.json` | `not_forwarded` | Lifecycle bus notice: subject, live wire headers and payload | prefrontal, Magic Context; Occipital if subscribed |
| `nothing_written--events-get.json` | `nothing_written` | events_get result object, including exact canonical body string | prefrontal, Magic Context; Occipital if authorized |
| `nothing_written--receipt.json` | `nothing_written` | Delivery receipt before terminal retirement | thalamus runner/adapter/ledger only; not the three external readers |
| `nothing_written--run-finished.json` | `nothing_written` | session.subscribe run_finished frame | Occipital, prefrontal, Magic Context |
| `nothing_written--run.result.json` | `nothing_written` | run.result, full RunResult | Occipital, prefrontal, Magic Context |
| `nothing_written--run.status.json` | `nothing_written` | run.status, full RunStatus | Occipital, prefrontal, Magic Context |
| `nothing_written--send-first.json` | `nothing_written` | session.send, initial attempt | Occipital, prefrontal |
| `nothing_written--send-repeat.json` | `nothing_written` | session.send, same send_id after ending | Occipital, prefrontal |
| `nothing_written--send-status.json` | `nothing_written` | session.send_status after ending | Occipital, prefrontal |
| `nothing_written--session.head.json` | `nothing_written` | session.head, last_run_state and last_run_error summary | Occipital, prefrontal, Magic Context |
| `nothing_written--session.read.json` | `nothing_written` | session.read, history page (no RunStatus member) | Occipital, prefrontal, Magic Context |
| `nothing_written--turn-ended-body.json` | `nothing_written` | Canonical turn_ended body retrieved through events_get | prefrontal, Magic Context; Occipital if authorized |
| `nothing_written--turn-ended-notice.json` | `nothing_written` | Lifecycle bus notice: subject, live wire headers and payload | prefrontal, Magic Context; Occipital if subscribed |
