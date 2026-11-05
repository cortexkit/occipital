# phone-reader-run-endings-v2

How each way a run can end reads to apps, from Thalamus's OpenCode 2 runner (module
`thalamus-harness`, bind harness `opencode2`, llm-runner 0.8.6). The real files were
recorded from a real OpenCode 2.2.3 server, at build `3d70587c3` (live since
2026-11-05 11:26 UTC), by
`runner::tests_opencode2_real::real_opencode2_records_run_endings`, with a loopback
stand-in model. Each ending ran in its own session, and was read once its run had
ended. `phone-reader-run-endings-v1` stays as published.

## What changed from v1

- **Display facts:** each ending also has `session.read` with display facts
  (`-session-read-display-facts.json`), the read apps use for tool rows.
- **Unanswered calls:** a tool call no tool answered carries `unanswered` in its
  display facts. It's `not_executed` for the call the step limit left unrun (`02`),
  and `denied` for the call to a tool the plan doesn't offer (`05`). The error text
  stays as the model saw it. A tool's own failure carries no mark.
- **Model selection:** `session.head.model` is the session's current model selection.

## Recorded files

Each ending has four files: `session.head` (`-session-head.json`), `run.status` for
its run (`-run-status.json`), `session.read {from_ordinal: 1, limit: 50}`
(`-session-read.json`), and the same read with display facts
(`-session-read-display-facts.json`).

| Prefix | Ending | `last_run_state` | `last_run_error` |
|---|---|---|---|
| `01-completed` | the model answered | `completed` | — |
| `02-max-steps` | the step limit (`stop_when.max_steps: 1`) ended the run; its `bash` call is left unrun, `unanswered: not_executed` | `max_steps` | — |
| `03-error-provider` | the provider refused the request (HTTP 400) | `error` | `{class: permanent}` |
| `04-error-compaction-unavailable` | no compaction view could be had, so the turn never reached the model | `error` | `{class: compaction}` |
| `05-completed-tool-not-offered` | the model called a tool the plan doesn't offer; the call failed `tool_not_selected`, `unanswered: denied`, and the run went on | `completed` | — |
| `06-cancelled` | the owner stopped the run while its step was in flight | `cancelled` | — |
| `07-interrupted` | the module restarted while the step was in flight | `interrupted`, `reason: runner_crash` | — |

On `run.status`, an error also names the code callers branch on: `provider_code`
`provider.invalid-request` (with `status: 400` and the provider's message) for
`03`, and `compaction_unavailable` for `04`.

## Synthetic files

Endings this runner doesn't reach on OpenCode 2 today, or that need a provider's own
failure, are built from a recorded head with only `last_run_state` and
`last_run_error` changed. They are named `*.synthetic.json`; each decodes as a valid
llm-runner head.

| File | Built from | Ending |
|---|---|---|
| `S1-active-compaction-wait-session-head.synthetic.json` | `01` | `active`, `reason: compaction_wait`: held for a compaction view; ends on its own |
| `S2-active-context-ceiling-wait-session-head.synthetic.json` | `01` | `active`, `reason: context_ceiling_wait`: held at the context ceiling; ends on its own |
| `S3-paused-auth-required-session-head.synthetic.json` | `01` | `paused`, `reason: auth_required`: waits for a sign-in |
| `S4-paused-restart-session-head.synthetic.json` | `01` | `paused`, `reason: restart`: waits for a caller after a restart |
| `S5-context-ceiling-session-head.synthetic.json` | `01` | `context_ceiling` |
| `S6-error-transient-session-head.synthetic.json` | `03` | `error`, `{class: transient, retry_after_secs: 2.5}` |
| `S7-error-auth-required-session-head.synthetic.json` | `03` | `error`, `{class: auth_required}` |
| `S8-error-provider-unavailable-session-head.synthetic.json` | `03` | `error`, `{class: provider_unavailable, resets_at_ms}` |
| `S9-error-context-overflow-session-head.synthetic.json` | `03` | `error`, `{class: context_overflow}` |

## Classes on this runner

`last_run_error.class` is an open string. On OpenCode 2 this runner sets `transient`
(rate limit, network, provider internal error, timeout), `auth_required` (a sign-in
is needed), `provider_unavailable` (a usage or spend limit, lifting at its reset),
`permanent` (fails the same way again), `compaction` (no compaction view) and
`runner` (the runner refused the model step). `SHA256SUMS` lists every other file.
