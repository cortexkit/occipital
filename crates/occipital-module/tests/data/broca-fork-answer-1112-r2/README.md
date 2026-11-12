# fork_answer reader samples (Broca, 2026-11-12)

Samples for the readers of Broca's `fork_answer` (llm-runner 0.8.42 §9.20) before Broca emits
any of it. The contract is pinned at runner 0.8.43 and kit 0.17.53.

## role-describe/

`role.describe` as Broca serves it, before and after the change, both from the real serializer
(`broca_wire::broca_role_describe_with_writers`, pinned by its golden tests). The before files are
the goldens on master; the after files are the goldens the change regenerated
(`crates/broca-wire/tests/golden/role_describe*.json`). The `-service-tier-` pair is the same
reply with `service_tier.enabled` on. After the change:

- top-level `fork_idle_s: 600`;
- `majors[0].ops` gains `session.fork_prepare`, `session.fork_ask` and `session.fork_end`, after
  `session.send_status`;
- `capabilities` gains `fork_answer`, after `retention` and before `service_tier` when that is on.

Nothing else changes.

## replies/

Wire bodies of the three new operations, captured from Broca's real serve path
(`crates/broca-module-serve/tests/it/fork_answer_serve.rs`, `BROCA_FORK_ANSWER_SAMPLES`). The
sessions are synthetic and the model calls scripted. There is no "before" for these: a build
without the change doesn't declare or serve the operations.

- `fork_prepare.reply.json`, `fork_ask.reply.json`, `fork_end.reply.json`: successful replies.
- `fork_ask.reply.json` (`end_turn`), `fork_ask.max_tokens.json`, `fork_ask.model_error.json`,
  `fork_ask.tool_call_refused.json`: one answer for each `finish` broca produces.
  `fork_ask.replay.max_tokens.json` is the replay of the `max_tokens` ask, byte-identical to it.
- Refusals: `fork_prepare.scope_owner_mismatch.json`, `fork_ask.unknown_fork.json`,
  `fork_ask.invalid_params.json` (`detail.field`), `fork_ask.ask_id_reuse.json`,
  `fork_ask.fork_busy.json`, `fork_ask.fork_ended.json`,
  `fork_ask.refused.{0,1}.fork_account_held.json` (`detail.routing_code` or `detail.mismatch`),
  `fork_ask.refused.2.fork_account_blocked.json` (`detail.routing_code`),
  `fork_ask.refused.3.fork_base_unavailable.json`.

## Step events

There is no step-event sample yet. This build emits no step event for a fork attempt, and no
`fork_id` anywhere on the attempt feed. Before/after step-event samples come with the change that
adds `fork_id`, which waits until Astrocyte confirms its decoder.

## Refreshed after the rebase onto master 2436e9640

The `role-describe/` files were regenerated on the rebased branch. The before files are master's
goldens at 2436e9640. Master's `role_describe_service_tier.json` golden lacks `app_readers`, which
`role_describe.json` has. The after file for the service-tier pair is the serializer's output with
the switch on, so it gains `app_readers` as well as the fork members.
