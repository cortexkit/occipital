# Account-wait refusal samples (llm-runner 0.8.39 / 0.8.41)

These are the exact bytes broca's real serve path answers with. They were captured on synthetic
sessions over a real `serve_connection`, registry and session actor, with the harness in
`crates/broca-module-serve/tests/it/resolution_refusal_class.rs`. The account labels (acct1,
acct2) are synthetic.

The routing stand-in answers `route.account_for` with the error reply shown for each case. broca
reads that reply through its own conversion,
`broca_module_serve::account_routing::classify_account_for_reply`, and the run's executor refuses
admission with the result. That is how the module's executor refuses when routing declines to
choose an account.

Cases 4, 5 and 7 come from the committed tests `account_selection_refusals_say_by_their_code_whether_to_wait`
and `a_queued_send_refused_by_routing_ends_with_the_refusal_code`, run with
`BROCA_ACCOUNT_WAIT_SAMPLES` set. Cases 1-3 come from an earlier, uncommitted generator using the
same harness.

Sends carry `session_class: "consult"`, `model: {provider: "google", model:
"antigravity-gemini-4-carbon", variant: "high"}`, `send_id: "turn-1"` and a head scope stamp owned
by `prefrontal-core`. Direct-send cases are named `<case>.<storage>.<plain|plan>.json`. Storage
`todays` is today's WAL storage; `new` is the new session storage. `plan` means the send carries a
fetch plan. Core sends `new.plan`. Within a case, all four files are byte-identical, except case
2's reset time.

## Outer codes (llm-runner 0.8.41)

The outer `code` says whether to wait:

- `account_limit_exhausted`: the limit set (pool exhausted or reserve-limited, hold, unknown
  quota, no eligible account). Transient.
- `account_selection_refused`: any other transient routing code (`account_selection_unavailable`,
  `pin_generation_conflict`, `store_error`, `internal_error`).
- `account_selection_final`: a permanent routing code (`account_not_authorized`,
  `invalid_request`, `admission_conflict`, `unknown_decision`, `unscoped_caller`,
  `account_selection_unsupported`) and **every routing code broca does not know**. Waiting can't
  change it. Its `detail` is the same shape as `account_selection_refused`'s, with no
  `provider_code`.

## Cases

1. `1_account_limit_exhausted_retry_and_reset.*` — routing replies
   `{"code":"account_pool_exhausted","detail":{"retry_after_s":120,"reset_at_ms":4102444800000,"accounts":["acct1","acct2"]}}`.
   Code `account_limit_exhausted`, with `retry_after_s` 120, `retry_after_secs` 120.0 and
   `resets_at_ms`.
2. `2_account_limit_exhausted_reset_only.*` — routing replies `account_pool_exhausted` with only
   `reset_at_ms` (90 s after the request) and no wait. broca **derives** `retry_after_s` and
   `retry_after_secs` from the reset (rounded up to the next second, here 90). Whenever routing
   gives a reset, broca never sends `resets_at_ms` without them. `resets_at_ms` is wall-clock
   time, so it differs between files and between runs.
3. `3_account_selection_refused_retry.*` — routing replies `account_selection_unavailable` with
   `retry_after_s: 30`. Code `account_selection_refused`, class `transient`, with
   `retry_after_s` and `retry_after_secs`.
4. `4_account_selection_final_no_hint.*` — routing replies `account_not_authorized` with detail
   `{"account":"acct2"}` and no wait or reset. Code `account_selection_final`, class `permanent`,
   no hints. Before 0.8.41 this body's code was `account_selection_refused`, the same bytes
   otherwise (the earlier file `4_account_selection_refused_no_hint.*`).
5. `5_queued_<provider_code>.<storage>.*` — a first send is admitted and held running. A second
   send (`send_id: "turn-2"`, with a fetch plan) is queued behind it (`session_send`: its
   `pending` reply). When the first run ends, the queued send is refused at its own admission by
   routing. The run's `error.provider_code` follows the same mapping as the direct send's outer
   code:
   - `5_queued_account_limit_exhausted.*` — routing replies case 1's reply. `error`: class
     `transient`, `retry_after_secs` 120.0, `provider_code: "account_limit_exhausted"`,
     `resets_at_ms`.
   - `5_queued_account_selection_refused.*` — routing replies `account_selection_unavailable`
     with `retry_after_s: 30`. `error`: class `transient`, `retry_after_secs` 30.0,
     `provider_code: "account_selection_refused"`.
   - `5_queued_account_selection_final.*` — routing replies case 4's reply. `error`: class
     `permanent`, `provider_code: "account_selection_final"`, no timing.

   Each has `session_send.json`, `session_send_status.json` (`session.send_status` for `turn-2`:
   `finished`, reason `error`) and `run_status.json` (`run.status` for the queued run). Both
   storages answer byte for byte the same (`cmp` reports the `todays` and `new` files equal), so
   the new storage has no different queued answer. These replace the earlier `5_queued.*` files,
   which held the account-limit case only. The run ids are random per run. The queued `error`
   carries the outer code as `provider_code`, `class`, `message` and the timing
   (`retry_after_secs`, `resets_at_ms`); it has no `retry_after_s`, `routing_code` or
   `routing_detail`.
7. `7_unknown_code_final.*` — routing replies a code broca does not know,
   `route_state_unavailable`, with `retry_after_s: 60`. Code `account_selection_final`, class
   `permanent`. The wait stays in `detail` but doesn't make it a wait.

Check the files with `shasum -a 256 -c SHA256SUMS` from this folder.
