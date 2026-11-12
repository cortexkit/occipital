# ChatGPT usage-limit errors with a window: before and after

What changes for readers when broca reads the usage window of a ChatGPT/Codex usage-limit
refusal from the response's quota headers.

Every body here was generated through broca's own code, not written by hand:

- `*.run.status.json`, `*.run.result.json`: a new-storage session's `run.status` and `run.result`
  replies (the store actor's registry, `run_status_reply` and `run_result`) for a run whose only
  step failed with the error the provider classifier builds
  (`broca_provider::transport::classify_account_limit_exhausted`, ChatGPT family, HTTP 429).
- `*.account_observe.json`: the `route.account_observe` `account_limit_exhausted` body broca
  sends routing for that attempt (`account_routing::attempt_body_on`), with a synthetic
  credential and account.

The 429 body is the production refusal the provider tests pin (2026-09-16). The quota headers
are the first entry of `crates/broca-module-serve/tests/fixtures/usage-observe/
codex-quota-capture-2026-11-02.json`: a real 429 for an account past its weekly allowance,
`x-codex-primary-used-percent: 100`, `x-codex-primary-window-minutes: 10080`,
`x-codex-secondary-used-percent: 0`, `x-codex-secondary-window-minutes: 0`.

| Sample | Headers | `limit_window` |
|---|---|---|
| `before.*` | none (what every ChatGPT limit error carried before: the window was never read) | absent |
| `after.*` | the captured quota headers | `"seven_day"` |

The only difference between a `before` and an `after` body is the added `limit_window` member:
inside `error` for `run.status` and `run.result`, at the top level of the observation. The rule
(`chatgpt_spent_window` in `crates/broca-provider/src/transport.rs`): exactly one of the primary
and secondary windows is used at or above 100 percent, and its length is 300 minutes
(`five_hour`) or 10080 minutes (`seven_day`). Otherwise no window is set, as before.

A spent window named `five_hour` or `seven_day` is what lets a run ask routing to continue on
another account at a ChatGPT usage wall.
