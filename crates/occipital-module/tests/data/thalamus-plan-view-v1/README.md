# session-plan-view-v1

What Occipital reads through `session.plan_view` (llm-runner 0.8.40), recorded on
Thalamus `01df11838215067aa176f340a2d9a7bf933edc11`. The real hosts are OpenCode 2.2.3 and Claude Code 2.1.318,
with loopback stand-in models and fixture tool providers; no account is used.

Each case has a `.params.json` file containing the exact request params and a
`.reply.json` file containing the exact wire body: `{"result": ...}` for an answer,
or the refusal object for an ERROR frame. The recorder uses the wire's reply-body
encoder and does not pretty-print, redact or otherwise rewrite those bytes.
The session is route-bound and is not a request parameter; all params here are `{}`.

| Case | Source and reply |
|---|---|
| `01-opencode2-in-force` | Real OpenCode 2 after its first model request: generation 0, joined system text, tools from `aft` and `prefrontal-core` in offered order, schemas and estimated token counts, and `newest_step.at_ms`. |
| `02-opencode2-pending` | The same session after a second request told the model of generation 1: generation 0 stays in force; `pending.notes` lists the system-change text then the `bash` removal sentence, exactly as offered. |
| `03-opencode2-applied` | The same session after `session.flush_prefix` rebuilds the request prefix with the pending plan, followed by another request: generation 1, changed system text, no `bash`, no pending change, and a new `newest_step`. |
| `04-claude-code-in-force` | Real Claude Code through the module's gateway after its first model request: generation 0, the plan's system text, tools from two providers in offered order, schemas, estimates and `newest_step`. |
| `05-no-plan` | Real OpenCode 2 runner, before the test session's first send: `no_plan`, not an empty plan. No host call is needed for this read. |
| `06-awaiting-plan-applied` | Unit-test stand-in host: the fixture starts a plan, then removes `registrations_generation` from its stored session record, simulating a record predating per-generation provider tracking. The real runner dispatch refuses `transient` with `detail.cause: "awaiting_plan_applied"`. The same test applies a change and verifies the read recovers. |

The no-plan case follows contract section 4.9: a session with no applied plan is
refused `no_plan`. It uses the unknown session branch of `Runner::plan_view`
(`readable_snapshot(...).ok_or_else(no_plan)`), before admission creates its record.
The not-ready case comes from
`providers_recorded_for_another_plan_refuse_the_read_until_the_next_change` in
`tests_plan_view.rs`, not a real-host timing race.

Deliberately absent: `system.items`, `applied_at_ms`, and `newest_step.reply_mid`
(the runner does not retain those facts), transport headers, whole model requests,
provider credentials, host-native tools, and Claude Code's own system blocks and
`DeferredToolPlaceholder`. Token counts are estimates (`exact: false`), not billing.
The gateway may rename tools after planning; the reply says `as_planned: true`.
Pi is not a sample: `role.describe` on a `pi` route does not list `session.plan_view`.

Only `newest_step.at_ms` varies between recordings of these cases; no session ids,
machine paths or account data are present. The recorder reuses the pages recorder's
private-content check, adds temporary/runtime paths and personal-name checks, and
refuses the whole set rather than redacting it. `SHA256SUMS` covers every file but
itself. The host-free decoder test uses the pinned crate's `PlanViewRequest` and
`PlanView`; `wire::read_reply::<PlanView>` also checks the success envelope. Refusal bodies
are checked as JSON because the role crate has no public refusal-body decoder.

## Recording again

Run on the Linux build server from the repository root at the commit above. Use a
new relative recording folder (publish a new version, never replace a published
set). Recording intentionally uses one test process, with one thread: the two host
tests and unit fallback share one exclusively new folder. Nothing is written until
the OpenCode 2 test finishes and all six expected cases are present; the final
file check also catches a filtered-out finalizer. Checksums are written once.

```sh
bash probes/opencode-linux/fetch.sh target/scratch/plan-view-hosts/oc2
bash probes/claude-code-linux/fetch.sh target/scratch/plan-view-hosts/cc
THALAMUS_OPENCODE2_BIN="$PWD/target/scratch/plan-view-hosts/oc2/opencode" \
THALAMUS_CLAUDE_CODE_BIN="$PWD/target/scratch/plan-view-hosts/cc/claude" \
THALAMUS_PLAN_VIEW_RECORD_DIR=crates/thalamus-harness-module/test-vectors/session-plan-view-v1 \
cargo test --locked -p thalamus-harness-module --features test-support --lib plan_view -- \
  --include-ignored --skip real_pi_head_refuses_plan_view --test-threads=1 --nocapture
test -f crates/thalamus-harness-module/test-vectors/session-plan-view-v1/SHA256SUMS
```
