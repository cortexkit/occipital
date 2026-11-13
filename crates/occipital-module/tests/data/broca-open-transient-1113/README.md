# broca: retryable open refusals answer code `transient` (open-transient-1113)

Bodies written by broca's real serializer (`admission_refusal` + `serde_json::to_vec`, the exact
ERROR frame body; `queued_admission_error` for a queued send's stored run error), before and after.

What changes: an open refusal broca already classes `transient` stops using code `open_failed`
and uses the llm-runner contract's retryable code `transient`, adding `detail.cause`
(`key_unavailable` while the new storage's key loads in the first ~12-30 s after a start,
`open_failed` otherwise). Message and `detail.class` are unchanged. A permanent open refusal is
unchanged (`send-permanent-unchanged.json`, identical before and after).

Where it appears: `session.send` refusals, `session.read`/other reads that fail to open the
session, `session.import`, and a queued send's stored run error (`run.status` `error`, as
`provider_code`, no detail).

Why: callers decide retries by code alone. Today a worker or sidekick launched in the first
seconds after a broca restart is refused `open_failed` and fails for good, though it would be
admitted seconds later.

Files: before/*.json, after/*.json (same names).
