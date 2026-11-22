# phone-reader-capacity-wait-v1

What the apps get when routing has no free capacity for an agent's model right
now. The runner contract (llm-runner 0.9.0) names it `concurrency_full` and
counts it as a wait, like the account waits in `phone-reader-account-wait-v2`:
work waiting on it must never be shown as failed. Routing names when capacity
frees up (`resets_at_ms`), and the wait ends then.

It reaches the apps in four shapes.

## 1. A run waiting for capacity (files 01 and 02)

The run is paused and resumes by itself on the same account once capacity
frees up. The person and the app do nothing. Session facts (`session.details`,
the newest page, claims and watches) carry it as `lastRun`:

```json
{"lastRun":{"runId":"r1","state":"paused","reason":"concurrency_full",
 "awaits":{"party":"runner","reason":"concurrency_full",
  "sinceMs":1795342184183,"nextMs":1795342190183}}}
```

- `awaits.party` is `runner`: the runner ends the wait itself.
- `nextMs` is when the runner tries again. Once that time has passed the runner
  drops it, and `nextMs` is absent (file 02).
- The `awaits` body is Agent Runner's recorded `run.status` (OpenCode 2, through
  routing's real refusal), carried on the head's last-run state.

## 2. A turn that ended while waiting (file 03)

The turn ended in an error with the wait's code, so no answer comes from it:

```json
{"lastRun":{"runId":"r1","state":"error","reason":"error",
 "error":{"class":"transient","providerCode":"concurrency_full","resetsAtMs":1795342190183}}}
```

`resetsAtMs` is when capacity frees up. Show it as a wait, never as a failure.

## 3. A send queued behind a running turn (file 04)

The runner held the send for the next turn. When that turn came, routing had no
capacity, so the message never reached the chat. Its entry in `queuedSends`
turns `unsent` with `reason: "error"` and names the wait:

```json
{"clientMessageId":"q1","state":"unsent","reason":"error","endedAtMs":1767225605000,
 "runnerCode":"concurrency_full","retryAfterMs":60000}
```

The words may go again at `endedAtMs + retryAfterMs`, as a new message under a
new `clientMessageId`, as for the account waits.

## 4. A send refused at once (file 05)

`session.enqueue_user_message` answers `runner_refused`, transient, with the wait
before sending again:

```json
{"kind":"moduleError","code":"runner_refused","message":"the runner answered concurrency_full: …",
 "detail":{"class":"transient","runnerCode":"concurrency_full","retryAfterMs":30000}}
```

The runner stored nothing, so a send again is a new send. `05-send-refused.runner.json`
is the runner's refusal as the contract maps it (routing's `no_route` with reason
`concurrency_full`, outer code `concurrency_full`); it is written from that
mapping, not recorded from a runner.

## Files

- `01-paused.json`: the paused run, with `nextMs`.
- `02-paused-past-reset.json`: the same run after its retry time has passed.
- `03-run-end.json`: a turn that ended while waiting.
- `04-queued-unsent.json`: the queued send, unsent with the wait.
- `05-send-refused.json`: the direct refusal the app gets, beside
  `05-send-refused.runner.json`, the runner's refusal it maps.
