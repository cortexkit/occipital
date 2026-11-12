# phone-reader-account-wait-v1

A send the runner refuses because no account can serve it right now: the account
that would serve it is at its usage limit, or routing can't choose one yet. The
runner contract (llm-runner 0.8.39) calls this an account wait. The send stored
nothing, it is safe to send again under the same `clientMessageId`, and it may
succeed later, but not at once. The message must never be shown as failed.

`session.enqueue_user_message` answers such a send `runner_refused` with
`class: "transient"`, like any refusal that may succeed later, and `retryAfterMs`,
the wait the runner named. `runnerCode` says it is an account wait, so the app can
say why the message waits.

Each `NN-*.runner.json` is the refusal Broca's real serve path answered, recorded
on 2026-11-12 over a synthetic session with a routing stand-in, copied unchanged.
Each `NN-*.json` beside it is the refusal the app receives for it. A module test
proves the app files are exactly what Occipital's send answers for those runner
refusals. The account labels are synthetic.

## The refusal

```json
{"kind":"moduleError","code":"runner_refused","message":"the runner answered <code>: <text>",
 "detail":{"class":"transient","runnerCode":"account_limit_exhausted","retryAfterMs":120000}}
```

| `runnerCode` | Means |
|---|---|
| `account_limit_exhausted` | The account that would serve the send is at its usage limit. |
| `account_selection_refused` | Routing can't choose an account for the send right now. |

`retryAfterMs` is the runner's relative wait when it named one, else the time
left until the limit resets. Without either, use your own back-off, never zero.

## The files

| Files | Shows |
|---|---|
| `01-limit-retry-and-reset` | The account is at its limit; the runner names a 120 s wait and a reset time. The wait wins: `retryAfterMs: 120000`. |
| `02-limit-reset-only` | Routing gave only a reset time 90 s away; Broca turns it into a 90 s wait: `retryAfterMs: 90000`. |
| `03-selection-retry` | Routing can't choose an account yet and names a 30 s wait: `retryAfterMs: 30000`. |

## What the app does

Keep the message, its files and its `clientMessageId`. Send it again no sooner
than `retryAfterMs` after the refusal arrived. Never show it as failed for this
reason.

A refusal waiting can never cure comes under another `runnerCode`, which reads
`final` like any refusal Occipital doesn't know as transient.

`SHA256SUMS` lists every file except this README.
