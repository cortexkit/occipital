# Converting-session refusals, before and after (broca 0.4.42)

Each file is the error body broca's real serializer writes for a refused
operation. `before/` is what 0.4.41 sends; `after/` is what 0.4.42 will send.

| File | When it is sent |
|---|---|
| `background-converting.json` | the background pass holds this session for the seconds it takes to convert it (live since 0.4.36) |
| `open-while-converting.json` | the same hold, met while broca opens the session |
| `startup-window-converting.json` | the startup conversion window, which holds every session until it ends |
| `startup-window-unconverted.json` | a session that could not be converted (unchanged) |

The change: a converting session's refusal goes out with the contract's
retryable outer code `transient`, `detail.class` `transient`, and
`detail.cause` `session_converting`. Before, the outer code was
`session_converting`, which `errors::is_retryable` does not list, so a caller
deciding by code alone treated it as final. `detail.retryable` and
`detail.refusal_class` go from these bodies; the message says which hold it is.
`session_unconverted` (permanent) does not change.

`concurrency-full-0441.json` is unrelated to this change: the body 0.4.41
already sends when routing answers `no_route` with `detail.reason`
`concurrency_full` (outer `account_selection_refused`, an account wait under
`errors::is_account_wait`, timed by `retry_after_secs` and `resets_at_ms`).
