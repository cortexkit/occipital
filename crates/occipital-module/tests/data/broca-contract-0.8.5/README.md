# llm-runner 0.8.4 / 0.8.5 serve-path reply samples

These are synthetic **raw reply bodies** captured through broca's real framed
`serve_connection` path, using its production registry, WAL writer, and scripted
provider fixtures. No live project, session, credentials, or model account was used.
`.jsonl` files contain one reply body followed by a newline; `.json` files contain
the body without an added newline. Framing headers are not included. No body was
normalized or edited after capture; dynamic run IDs, incarnations, versions, and
timestamps naturally differ between runs.

`before/` means before the corresponding behavior was enabled, not one simultaneous
snapshot of all methods. The pin-only step first passed the unchanged wire/describe
goldens. Image-refusal baselines were recaptured with the image-size fix disabled,
using the same valid PNG inputs as the after run (including ancillary padding for
byte-limit cases); the other adopted features do not change those refusal bodies.

## Pairs to decode

| Reply body, present in both directories | What changed |
|---|---|
| `runner-role.describe.jsonl` | `media_only_send` is declared alongside `attachments`. |
| `runner-session.head.known-limits.jsonl` | Active Anthropic-profile input reports `max_image_bytes: 7500000` (10 MB padded-base64 bound converted to file bytes) and `max_image_dimension: 8000` for one image. |
| `runner-session.head.unknown-limits.jsonl` | The active OpenAI Chat profile has no documented per-image byte/dimension bounds; both fields remain absent, not zero. |
| `session.read.display_facts.json` | An opted-in descriptor read adds `completed_at: 505` and one readable Responses summary at stored part index 1. |
| `session.read.created_at.json` | The 0.8.5 pair adds recorded creation times: accepted input at 250 and the assistant's producing request sent at 600. Its completion remains 505: host clocks can step, so readers clamp negative displayed durations to zero rather than changing either fact. |
| `runner-photo-only.send.jsonl` | Previously refused for missing text; now an ordered media-only send is accepted. |
| `runner-photo-only.stored-message.jsonl` | Range beginning at the photo's ordinal: empty before refusal, one media-only user message after admission. No empty text part is stored. |
| `runner-media_too_large.dimension.jsonl` | A valid 9000×1 PNG exceeds only the reported 8000-pixel bound. |
| `runner-media_too_large.bytes.jsonl` | A valid PNG with an ancillary padding chunk exceeds only the reported 7500000-file-byte bound. |
| `runner-media_too_large.both.jsonl` | A valid padded 9000×1 PNG exceeds both bounds. New refusal details contain only those bounds, never a generic delivery field. |
| `plan.senders.send.json` | A frozen flow's widening plan previously met `plan_drift`; it now meets `invalid_params {field: "plan.senders"}` before any fetch/write. |
| `plan.senders.refresh.json` | The owner-authorized widening refresh was previously accepted; it is now refused with the same `plan.senders` field and no write. |

The Responses record deliberately contains synthetic opaque/encrypted sentinels in
its existing stored `redacted_reasoning` data. The new `reasoning_summaries` envelope
extracts **only** `summary_text`; the OpenRouter opaque part is not a summary. No
stored part was added or rewritten. Tests separately pin pre-change default blocks,
raw history, and descriptor reply bytes, including whole-message and part digests.

Creation and completion come from recorded host facts and shared projectors, not
another clock. Cached values are scalar columns, never copied from other timestamp
columns. Their metadata-only migration is the `store.schema` boot step
`message_display_fact_times`, with no default, backfill, or index. Previously cached
rows without those facts remain unknown. Reads that do not request display facts
omit creation, completion, and summary envelope members.

## Capture sources

- `crates/broca-module-serve/tests/session_change.rs`: frozen-flow send/refresh tests.
- `crates/broca-module-serve/tests/head_read_additions.rs`:
  `responses_summary_and_completion_are_display_only_with_stable_bytes_and_digests`
  and `created_at_uses_recorded_acceptance_and_request_send_time_only_when_opted_in`.
- `crates/broca-module-serve/tests/media_runtime.rs`: the three `runner_*` media tests.

The tests use `BROCA_LLM_RUNNER_SAMPLE_DIR` or `BROCA_MEDIA_SAMPLE_DIR` to write these
bodies. The size-refusal test also checks a file beyond the store ceiling, without
publishing its bytes; that extra boundary is a regression test, not another sample.

`SHA256SUMS` covers the reply files and this README, excluding the checksum file
itself. Verify from this directory with `shasum -a 256 -c SHA256SUMS`.
