# phone-reader-runner-refused-v1

When a send reaches the agent's runner and the runner refuses it, `session.enqueue_user_message`
answers `runner_refused`, and the refusal's `detail` says whether the same send may succeed
later. Nothing is stored for a refused send, so resending it under the same `clientMessageId`
runs it at most once.

Files `00` to `05` were recorded by the phone team on 2026-10-31 at 03:32Z on the Canary Head
(harness `opencode`, session `ses_e82d035a7ffeFAtBg5sNRs6C7C`): a send, then the same
`clientMessageId` sent again 31 s later. Files `06` to `08` are synthetic, because neither
refusal class can be produced on demand from the phone. A module test proves each synthetic
file equals what the production mapping answers for that runner refusal, byte for byte after
parsing. `SHA256SUMS` covers every file except this README.

## The refusal

```json
{"kind":"moduleError","code":"runner_refused","message":"the runner answered <code>: <text>",
 "detail":{"class":"transient"|"final","runnerCode":"<code>","retryAfterMs":1500}}
```

| Member | Meaning |
|---|---|
| `class` | `transient`: the runner contract says the same request may succeed later. `final`: it won't, whatever the timing. |
| `runnerCode` | The runner's own refusal code. Show it in details, never parse it. |
| `retryAfterMs` | Present only when the runner named a wait. |

The other send refusals (`not_found`, `unsupported`, `enqueue_not_authorized`, `run_paused`,
`context_ceiling`, `conflict`) keep their own codes and carry no `class`.

## The files

| File | Shows |
|---|---|
| `00-details-before` | The head's details before the send. |
| `01-send` | The send: `ok`, `duplicate: false`, its `effectId`. |
| `02-details-after` | Details after the send. |
| `03-send-again-same-id` | The same `clientMessageId` 31 s later: the same `effectId`, `duplicate: true`. No second run. |
| `04-details-after-again` | Details after the resend: still one run. |
| `05-newest-page` | The page with the one message the two sends stored. |
| `06-runner-refused-transient.synthetic` | `fetch_unavailable`, a contract-retryable code, without a wait. |
| `07-runner-refused-transient-wait.synthetic` | The same refusal where the runner named a 1.5 s wait: `retryAfterMs: 1500`. |
| `08-runner-refused-final.synthetic` | `session_ended` with reason `plan_stale`: the session's runner ended it, so no resend can land. |

## What the app does

Resend a `transient` refusal under the same `clientMessageId` after `retryAfterMs`, or after its
own back-off when there is none. Hold a `final` refusal for the person to edit or delete. A
resend that lands answers like `03`.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
