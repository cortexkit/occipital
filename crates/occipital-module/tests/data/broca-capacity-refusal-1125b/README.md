# Broca reader samples: routing concurrency full after the change

These are seven exact response frame bodies from `ck-broca` through the real `ck-subc` daemon. The rig scripts routing, the vault and the Anthropic HTTP endpoint; it does not construct or reserialize Broca replies. `SHA256SUMS` covers all seven JSON bodies. IDs, times and retry offsets are sample values, not constants.

## What changed from `capacity-refusal-1125`

| File | What a reader sees |
| --- | --- |
| `continuation-paused.run.status.json` | **New.** A running run that hit its provider quota and then received `no_route` / `concurrency_full` is `paused` with `pause_reason: account_limit`. `result.awaits.party: runner` and `result.awaits.next_ms` identify the live wake, which also appears as `result.resume_not_before_ms`. The wake is the routing reset, capped at 30 minutes from the pause. `result.error.account_continue` retains the uncapped `until_ms` and `reason: concurrency_full`. |
| `continuation.run.status.json`, `continuation.run.result.json` | **Changed.** The test advances the clock to the wake; the spent account returns a second quota 429, and routing refuses again with a fresh, future reset. The run ends `error` instead of pausing a second time in the same step. Both reads retain `account_continue.routing_code`, `reason` and `until_ms` on the provider error. The provider's own `resets_at_ms` and `retry_after_secs` are unchanged. `awaits.party: caller` is a retry hint, not a runner wake. |
| `admission-ordered.run.status.json` | **Changed.** An ordered send refused at admission still ends with `result.state: error` without starting a durable run. It now keeps the router's original object at `result.error.detail.routing_detail`, including `reason: concurrency_full` and `reset_at_ms`. This is not a paused run. |
| `admission-direct.session.send.json`, `admission-ordered.session.send.json`, `admission-ordered.session.send_status.json` | **Same shapes.** A direct refusal still carries `detail.routing_detail` and creates no run or receipt. The ordered send returns a pending receipt, and its send status finishes with `reason: error` and a `run_id` for the run-shaped admission failure. |

Capture: `account_selection_real_daemon::capture_concurrency_full_reader_replies` in `crates/broca-module-serve/tests/it/account_selection_real_daemon.rs`. Run the filtered nextest with `--features test-clock`, `SUBC_CORE_BIN` pointing to the rig's `ck-subc`, `BROCA_REQUIRE_SUBC=1`, and `BROCA_CAPACITY_REFUSAL_SAMPLES` set to the output directory. The daemon starts the `ck-broca` built from the checkout. No live account or upstream service is used.
