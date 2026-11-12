# phone-reader-run-awaits-v2

What a run waits on, as the apps read it. From runner contract 0.8.45, a
waiting or continuing run says who ends the wait and with which facts, so an
app can word any wait from fields rather than one code per case.

Each `NN-*.request.json` is the app's request and `NN-*.json` the reply bytes it
receives, served through Occipital's real handlers. Each run's state is the
contract's own sample head (`run-awaits.json` in `cortexkit-role-llm-runner`
0.8.45's test vectors), on a synthetic session.

## What the apps read

`lastRun.awaits`, beside `lastRun`'s other members, on `session.details`, newest
pages, claims and watches:

```json
{"awaits":{"attempt":2,"deadlineMs":1790000060000,"nextMs":1790000002000,"party":"runner","reason":"provider_retry","sinceMs":1790000000000},"reason":"provider_retry","runId":"r2","state":"active"}
```

- `party`: who ends the wait. `runner` (it moves by itself, such as a retry
  or an account reset), `caller` (whoever sent the turn decides, such as
  sending again after an account wait), `person` (the person must act; see
  `person`), or `nobody` (it won't move; the run's ending is final). A party
  the app doesn't know is no information: word the wait from the other
  members, and never show it as a failure.
- `reason`: the runner's reason code, such as `account_limit` or
  `provider_retry`. Word codes the app knows and fall back to a generic line.
- `sinceMs`: when the wait began, in Unix milliseconds.
- `nextMs`: when it is expected to move by itself, if known.
- `deadlineMs`: when the runner gives up, if it will.
- `attempt`: which try this is, from 1, when the wait is a retry.
- `person`: `{action, on?}` when the person must act: `sign_in`, `approve`
  or `answer`, and `on` a display label such as an account's name. An action
  the app doesn't know passes too; word it generically.
- `message`: `{text, from}`, quoted words about the wait, and `from` says
  whose words they are: `runner`, `plugin` (a sign-in plugin), `host` (the
  program the agent runs in) or `provider` (the model's provider). Never draw
  it as the wait's own line: open it on tap, quoted under a label from `from`.
  Words from a source the contract doesn't name are left out, since no label
  for them would be true.

`awaits` is absent when the runner says nothing about a wait: that is no
information, not "not waiting". `state`, `reason` and `resumeNotBeforeMs` stay
beside it as before.

## The files

| Files | Shows |
|---|---|
| `01-details-account-limit-floor-passed` | A paused account wait whose resume time has passed, so `awaits` has no `nextMs`; the runner resumes it. |
| `02-details-account-limit` | A paused account wait, with `nextMs` its resume time. |
| `03-details-host-retry` | An active run whose host retries a failed request, with its attempt, next try and deadline, and the host's own retry text (`from: host`). |
| `04-details-held-tool` | An active run whose tool call is held (`tool_held`); the runner moves it on. |
| `05-details-auth-required` | A paused run waiting for the person to sign in again. |
| `06-details-terminal-account-wait` | A run that ended on an account wait: the caller may send again at `nextMs`. |
| `07-details-terminal-retryable` | A run that ended on an error the caller may retry. |
| `08-details-terminal-final` | A run that ended on a final error (`party: nobody`). |
| `09-details-completed` | A completed run: no wait. |
| `10-details-unknown-party-action` | A party and an action the app doesn't know. |

`SHA256SUMS` lists every file except this README.

The only change from `phone-reader-run-awaits-v1` is `message`: v1 passed it
only from the runner or a plugin, so its host-retry file had none.
