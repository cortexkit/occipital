# New-storage run waits: reader samples

These JSON bodies are emitted by broca's serializers, not hand-reconstructed envelopes.
The actor fixtures in `broca-module` drive the real session store actor and read its
`run.status`, `session.head`, and `run.result`. Subscribe files use the captured control
or display payload and `broca_wire::encode_live_subscribe_control_with_cursor` or
`encode_live_subscribe_display`, the same frame serializers used by the serving adapter.
The fixture providers and account labels are synthetic; no live account or store was read.
The supplied JSON files contain the serializer's JSON bytes without an added file delimiter.
The capture helpers now write those bytes directly; the earlier dump's trailing LF was removed
losslessly, without parsing or reserializing the JSON.

## Before and after

- `*.before.*`: the legacy-storage fixture's actual serialized bodies for the same scenario.
- `*.after.*`: the new-storage fixture's actual serialized bodies.
- `retry.before.*`: the same real new-store actor before its in-memory retry wait is published;
  legacy storage has no retry-wait event to capture. Its subscribe file is the captured initial
  display-reset frame. The after files include the current wait and its clear-before-retry frame.
- `auth-default-email.after.*`: an additional cache-only email fallback, with no named credential
  label. `auth-required.*` instead uses the explicit frozen account label `synthetic`.
- `subscribe.unknown-party-action.json` and `subscribe.display.unknown-party-action.json`:
  directly constructed open-vocabulary reader cases, not emitted by an actor's wait mapping.

Only the held-call state changes existing lifecycle bytes: the old head calls the held run
`paused`, while the new head, run status, and subscribe status all call it `active`, with
`reason: tool_held`. Other existing reason, deadline, and resume-floor fields retain their
previous meanings. Terminal head reasons and the `run_finished` terminal discriminator are
not normalized to `run.status`'s encoding.

## Where to read `awaits`

| File suffix / frame | Location |
| --- | --- |
| `run.status.json` | `awaits` at the response body root |
| `session.head.json` | `last_run_state.awaits` |
| `run.result.json` | root `awaits`, only for terminal errors |
| `subscribe.status.json` | root `awaits`; `unit` identifies `{type: status, run_id}` |
| `subscribe.paused.json` | `unit.awaits` beside the existing pause reason and resume floor |
| `subscribe.error.json` | `unit.awaits` beside the error |
| `subscribe.run_finished.json` | `unit.awaits` beside the terminal discriminator |
| `subscribe.display.run_status.json` | `event.awaits`; `event.type` is `run_status` |
| `subscribe.display.cleared.json` | `event.type: run_status`, with `awaits` absent |

Durable controls retain broca's verified `cursor: {wal_seq, sub_index, incarnation, digest}`.
Display events have a stream `seq` and **no durable cursor**. Retry waits use this existing
lossy, non-durable lane, not a new control-envelope variant. They are emitted only after an
attempt has been abandoned and before the next attempt starts: older display readers may
safely treat the unfamiliar event as `DisplayReset`. A subscriber joining during the wait
gets its current snapshot from display replay. `old-decoder.log` records a check using the
base revision's actual broca decoder against the new display frame.
Retry replay rebuilds that snapshot from the same actor-held slot status/head use, not a
captured display frame. `retry.past-floor.*` exercises a mid-wait reconnect after `next_ms`
has passed: every surface keeps the current wait but omits that past time.
The coherent base source revision was `755f2d10ea328c5a0ecf3ac91b193d2b354f1496`, with
the two newly published dependency pins retained. Its exact tested input is saved separately
as `../reader-proof-input.json` (SHA-256
`0aee15b66ceec445136842a842671840c0723c8e872d2ae48182162a2d013d8b`); that evidence copy
retains the dump's terminal file LF. The decoder read it as `Display(DisplayReset)`.

## Mapping coverage

- `account-limit`: paused, runner-owned, with `next_ms` equal to the unchanged future
  `resume_not_before_ms`. Past floors are separately covered by the pinned contract case
  and a deterministic projection test; a past floor is never changed or guessed.
- `auth-required`, `auth-default-email`: paused, person-owned `sign_in`; only a known account
  label or cache-served email is shown, never a vault/account ID.
- `restart`: paused, runner-owned; it resumes at startup without a person acting.
- `held`: active, runner-owned `tool_held`.
- `retry`: active, runner-owned `provider_retry`, in actor-held memory, with attempt and next
  time. Transport tests additionally exercise 429 -> `rate_limited`, 503/529 -> `server_busy`,
  and other failures -> `provider_retry`, visibility thresholds, and cancellation cleanup.
- `terminal-account`, `terminal-retryable`, `terminal-final`, `terminal-sign-in`: terminal
  party mapping comes from the contract's `errors::ending_awaits`, using recorded end time.
  Status, head, error control, finished control, and result carry the same wait.
- `terminal-unknown-code`: an existing error with no provider code stays readable on every
  surface with unchanged error bytes, but omits `awaits` rather than guessing retry/finality.
- `message-runner`: a source-grounded, exact broca-authored sentence, marked `from: runner`.
- `terminal-final`: recognized HTTP-wrapper JSON `error.message`, cleaned and marked
  `from: provider`.
- `message-withheld`: deliberately omitted WebSocket text stays omitted. Unknown message
  provenance is also omitted rather than relabelled.

The pinned ten-case commons fixture is copied verbatim into
`crates/broca-wire/tests/golden/run-awaits.contract.json`; tests decode its applicable status,
head, result and paused bodies, and encode/decode broca's actual status/control frames.
Unknown parties/actions survive decoding. Zero attempts and oversized message/label fields
are rejected. A legacy status/head byte pin remains unchanged.

Broca intentionally does **not** declare `run_awaits` yet. Legacy sessions omit the field;
new-storage readers can use it without the declaration. No durable record or feature was added.

`SHA256SUMS` covers the JSON samples and this README. Verification logs live one directory
above the samples; they are not build artifacts.
