# phone-reader-account-wait-v2

What the apps get when no account can serve an agent's model right now: the
account that would serve it is at its usage limit, or routing can't choose one
yet. The runner contract (llm-runner 0.8.41) calls this an account wait. Work
waiting on it must never be shown as failed. It reaches the apps in three shapes.
This set adds the second and third shapes, and the final code, to
`phone-reader-account-wait-v1`, which stays frozen. Files 01 to 03 are v1's cases,
recorded again on 0.8.41, and the app files are byte for byte v1's.

## 1. A send refused at once (files 01 to 04, 08)

`session.enqueue_user_message` answers `runner_refused`. The runner stored
nothing, so a send again under the same `clientMessageId` is a new send.

```json
{"kind":"moduleError","code":"runner_refused","message":"the runner answered <code>: <text>",
 "detail":{"class":"transient","runnerCode":"account_limit_exhausted","retryAfterMs":120000}}
```

| `runnerCode` | `class` | Means |
|---|---|---|
| `account_limit_exhausted` | `transient` | The account that would serve the send is at its usage limit. |
| `account_selection_refused` | `transient` | Routing can't choose an account for the send right now. |
| `account_selection_final` | `final` | Routing refused the account, and waiting won't help. |

`retryAfterMs` is the wait before sending again: the runner's relative wait when
it named one, else the time left until the limit resets. Without either, use your
own back-off, never zero. A `final` refusal never carries `retryAfterMs`, even
when the runner's text names a wait (file 08).

## 2. A send queued behind a running turn (files 05 to 07)

The runner accepted the send and held it for the next turn. When that turn came,
the account was refused, so the message never reached the chat. The send's entry
in `queuedSends` (on pages, claims and watches that opted in with
`acceptQueuedSends`) turns `unsent` with `reason: "error"`, and now also names why:

```json
{"clientMessageId":"q1","effectId":"app-…","acceptedAtMs":1767225600000,"state":"unsent",
 "reason":"error","endedAtMs":1767225605000,"runnerCode":"account_limit_exhausted",
 "retryAfterMs":120000}
```

- `runnerCode` is the code the turn ended with, when the runner contract names it.
  It has the same meanings as in shape 1.
- `retryAfterMs`, only on the two waits, counts from `endedAtMs`. The words may go
  again at `endedAtMs + retryAfterMs`.
- **Send them again as a new message, under a new `clientMessageId`.** The old
  send is finished for good. Sent again under its old id, it is answered with the
  same finished record, and nothing is delivered.
- An entry with no `runnerCode` (an older runner, or one that didn't answer) is
  unsent as before.

## 3. A turn that ended on the account (files 09 to 12)

The person's message is in the chat, but the turn that should answer it ended in
an error, so no answer comes. Session facts (`session.details`, the newest page,
claims) carry `lastRun.error.providerCode` beside the timing the runner named:

```json
{"lastRun":{"runId":"r-wait","state":"error","reason":"provider_error",
 "error":{"class":"transient","providerCode":"account_limit_exhausted",
  "retryAfterSecs":120.0,"resetsAtMs":4102444800000}}}
```

The agent's own side continues the turn after the wait; the app sends nothing for
it. `providerCode` appears only for codes the runner contract names.
`account_selection_final` (file 12) is not a wait.

## The files

| Files | Shows |
|---|---|
| `01-limit-retry-and-reset` | Refused at once: at its limit, a 120 s wait and a reset time. The wait wins. |
| `02-limit-reset-only` | Refused at once: only a reset 90 s away; the runner turns it into a 90 s wait. |
| `03-selection-retry` | Refused at once: routing can't choose yet and names 30 s. |
| `04-selection-final` | Refused at once, final: routing refused the account, no wait. |
| `05-queued-limit` | Queued, then refused at its turn: at its limit, 120 s. |
| `06-queued-selection` | Queued, then refused at its turn: routing can't choose, 30 s. |
| `07-queued-final` | Queued, then refused at its turn, final: no `retryAfterMs`. |
| `08-unknown-routing-code-final` | Refused at once with a routing code the runner doesn't know: final. Its text names 60 s, and `retryAfterMs` is still absent. |
| `09-run-end-limit` | A turn ended at the account's limit, with a wait and a reset. |
| `10-run-end-selection` | A turn ended because routing couldn't choose, with a wait. |
| `11-run-end-no-timing` | A turn ended at the account's limit, with no time named. |
| `12-run-end-final` | A turn ended on a final refusal. |

Each `NN-*.runner.json` is the refusal Broca's real serve path answered, recorded
on 2026-11-12 over a synthetic session with a routing stand-in (synthetic account
labels), copied unchanged. Files 05 to 07 are what Occipital lists after Broca's
recorded `session.send_status` and `run.status` for the queued send. Files 09 to
12 come from synthetic `session.head` replies (the runner's session summary, which
names the last run and its error), because no runner served that error's
`provider_code` when the set was cut. Each carries the run error of Broca's recorded queued case for
that code, and files 10 to 12 trim its timing. Module tests prove every app file
is what Occipital answers.

`SHA256SUMS` lists every file except this README.
