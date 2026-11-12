# phone-reader-out-of-credit-v1

A run that ended because the provider account behind the session's paid key
has no credit left, as the apps read it. The provider refused the request
before any work, so nothing was billed, and sending again won't help until the
account gets credit or the session uses another key.

Each `NN-*.request.json` is the app's request and `NN-*.json` the reply bytes it
receives, served through Occipital's real handlers. The run's state and error
are Broca's recorded `session.head` bodies (runner contract 0.8.43's
`last_run_error.provider_code`), on a synthetic session.

## What the apps read

`lastRun`, at the top of `session.details` replies, newest pages and claims:

```json
{"error":{"class":"permanent","providerCode":"billing_exhausted"},"reason":"error","runId":"…","state":"error"}
```

- `error.providerCode: "billing_exhausted"`: the account is out of credit. Say
  so, in the app's own words.
- `error.class: "permanent"`: sending the same words again fails the same way.
- `providerCode` is present only when the runner names one of the codes the
  runner contract lists. An error without it reads as its class only (`04`).

## The files

| Files | Shows |
|---|---|
| `01-details-out-of-credit` | `session.details` for the chat whose last run ended out of credit. |
| `02-newest-page-out-of-credit` | The newest page, with the same `lastRun`. |
| `03-claim-out-of-credit` | A claim on that page, with the same `lastRun`. |
| `04-details-permanent-no-code` | A permanent error that names no code: class only. |

`SHA256SUMS` lists every file except this README.
