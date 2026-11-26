# phone-reader-run-stopped-v1

What the apps receive for a run that Thalamus, the runner for third-party
harnesses, stopped under its runaway-reply rules (runner contract 0.10.1,
§14.7). There are two such stops:
- `output_limit`: three model steps in a row reached the output limit while
  writing a tool call's arguments, so the run stopped;
- `tool_loop`: the run stopped before repeating the same tool call, with the
  same result, a twentieth time.

Both are the runner's own decision, not the model provider's, and both come
with `class: "permanent"`. Occipital reads runner contract 0.10.2, which names
both codes, so it passes each to the apps.

Each run error is the one Thalamus recorded for such a run, copied unchanged
into `occipital-module/tests/data/thalamus-output-limit-v1/`. Its message
stays out of what the apps receive.

| File | What the app receives |
| --- | --- |
| `01-last-run-output-limit.json` | `lastRun` of a chat whose last run stopped with `output_limit` |
| `02-last-run-tool-loop.json` | `lastRun` of a chat whose last run stopped with `tool_loop` |
| `03-unsent-output-limit.json` | `queuedSends` with a message that waited behind that run: unsent, `reason: "error"`, `runnerCode: "output_limit"` |
| `04-unsent-tool-loop.json` | the same for `tool_loop` |

The `lastRun` files are served on every read that carries session facts
(newest page, claim, details and transcript watch), checked by
`occipital-module/src/app_reader/broca/tests/output_limit_1127.rs`. The
`queuedSends` files are what the waiting list holds once the runner says the
message's turn ended in that error, checked by
`occipital-module/src/app_reader/send/tests/run_stopped_1127.rs`. A stopped
run is final, so neither unsent message names a wait (`retryAfterMs`) before
the words may go again as a new message. `SHA256SUMS` lists every file but
this README.

## Every provider code the contract names

Occipital passes `lastRun.error.providerCode`, and `runnerCode` on an unsent
message, only when the code is one the runner contract it reads names
(`errors::provider_codes::CODES`). It drops any other code, and never passes a
provider's own code: that travels as `provider_detail_code`, which Occipital
doesn't serve. So an unknown `providerCode` is always a contract code an app
has no words for yet. At 0.10.2 the contract names these, with its own
one-line meaning:

| Code | Meaning (runner contract 0.10.2) |
| --- | --- |
| `provider_error` | the provider refused or failed and no code the contract names applies; decide only by `class` |
| `host_policy_refused` | the host's managed policy, an administrator's setting, refused the request; final |
| `routing_decision_refused` | routing refused the caller's decision, an expired one included; no model call was made |
| `pre_user_unavailable` | a hook that must check each user turn was unavailable, so the turn was refused |
| `context_overflow` | the provider refused the request as too long for the model's context window |
| `output_ceiling` | the provider's output-token ceiling prevents a complete response |
| `output_limit` | three model steps in a row ended at the output limit with an unfinished tool call; `class: "permanent"` |
| `tool_loop` | the runner stopped a loop of identical calls with identical results; `class: "permanent"` |
| `response_deadline` | the provider's response deadline ended the request |
| `account_limit_exhausted` | the account's usage limit is exhausted |
| `account_selection_refused` | routing refused the account choice for now; an account wait |
| `account_selection_final` | routing refused the account choice and waiting can't change it; final |
| `concurrency_full` | routing's in-flight capacity is full; wait, never fail the work |
| `billing_exhausted` | the account's billing or spending allowance is exhausted |
| `account_changed` | the provider served an account other than the run's own |
| `cyber_policy` | a provider content-policy refusal |
| `bio_policy` | a provider content-policy refusal |
| `misalignment_policy_violation` | a provider content-policy refusal |
| `content_filter` | the provider's safety finish, treated as a provider policy refusal |
| `access_terminated_error` | the provider ended the account's access; the agent can't run |
| `insufficient_quota` | an older name for `billing_exhausted`, in errors recorded before the rename |

## What `class` means

The contract fixes a class only for `output_limit` and `tool_loop`, both
`permanent`. For every other code the runner sets the class. `class` is an
open string. The contract gives only examples (`transient`, `permanent`,
`auth_required`, `context_overflow`, `provider_unavailable`; older records may
say `auth`), says readers keep any value, and gives no one-line meaning for
each. It doesn't say that `permanent` means the provider refused. It also says
an error with no class reported means "not reported", never `permanent`.
