# phone-reader-account-pause-v1

A run Broca paused because the account serving it is at its usage limit, as
the apps read it. Broca pauses a run instead of ending it when the account's
reset is near: it resumes the same run itself, on the same account, at the
time it names. Nobody needs to send anything, and Stop isn't needed to free
the chat.

Each `NN-*.request.json` is the app's request and `NN-*.json` the reply bytes it
receives, served through Occipital's real handlers. The run's state is the
`last_run_state` Broca's real serializer wrote for runner contract 0.8.42, on a
synthetic session.

## What the apps read

`lastRun`, at the top of `session.details` replies, newest pages and claims:

```json
{"reason":"account_limit","resumeNotBeforeMs":1790000600000,"runId":"run-1","state":"paused"}
```

- `state: "paused"` with `reason: "account_limit"`: the run waits for its
  account. It hasn't failed.
- `resumeNotBeforeMs`: the earliest time the runner resumes the run, in Unix
  milliseconds. It's present only while the run is paused for an account
  limit, and only when the runner names it. A paused run without it still
  waits; the app then can't say until when.

Once the run resumes, `lastRun` reads as any other run: `active` while it runs,
then its ending.

## The files

| Files | Shows |
|---|---|
| `01-details-paused` | `session.details` for the chat while its run is paused. |
| `02-newest-page-paused` | The newest page, with the same `lastRun`. |
| `03-claim-paused` | A claim on that page (`matched`), with the same `lastRun`. |
| `04-details-resumed-and-finished` | `session.details` once the run has resumed and finished: `completed`, no resume time. |

`SHA256SUMS` lists every file except this README.
