# llm-runner 0.8.11 real serve-path samples

All serve samples are unnormalized JSON frame bodies from `delivery_rig`'s real
in-process SUBC connection, not reconstructed typed replies. Dynamic run IDs,
incarnations, timestamps, history versions and digests naturally differ across runs.
`SHA256SUMS` covers every listed JSON body, including the two separate model pins.
The fixtures contain only synthetic text and synthetic module principals.

## The original 57-body comparison

- `before/` (22 bodies) and `before-denied/` (6 bodies) retain the original
  0.8.7 runner / 0.17.9 kit / 0.1.2 compaction-view serving implementation.
- `after/` (23 bodies) and `after-denied/` (6 bodies) are regenerated from the
  corrected final writer tree. The additional stream body is `notice_delivered`.
- The carrying ride in `before/session.*` and `before/subscribe.*` was recorded
  **before the envelope**, as a legacy `SendDelivered` with no `ride`. Its ordinary
  read/history output remains `worked\n\nsynthetic notice ☃`.
- The carrying ride in `after/session.*` and `after/subscribe.*` was recorded
  **with the writer enabled**, using the `rides-v2/v1` envelope. Its tool output is
  `worked` on every application path. Notice text is separate in opted-in facts
  and the event, and is still joined by the shared renderer for the model.
- The denied-call fixtures contain no riding send: they exercise the denied
  result's call identity in reads and live frames, not a legacy/envelope transition.

`role.describe.json` is discovery; `session.head.json` is the session head;
`session.read.default.json` is the default read; `session.read.facts.json` opts in
with `include_display_facts`; the raw/display history files exercise their named
forms. Numbered `subscribe.N.json` files preserve stream arrival order.
The after discovery also includes master's `routed_accounts` addition (`apikey`).

Legacy records receive neither notice facts nor notice events. Their existing
subscribe ToolResult and compaction entry already served the tool's own output;
those shapes and compaction's separate `delivered` metadata are preserved.
There is no history rewrite or `history_version` increment for the read correction.

## Added Core-reader comparison: nine files on each side

`core-before/` is the submitted 0.8.11 implementation **before the serving
correction**, with master merged (merge commit provenance is recorded below).
Its source is merge commit `d1230cd46cc011b35c059b351a1258ea900fface` plus the test-only capture fixture. It is not mislabeled as 0.8.7: 0.8.7 could not naturally emit `cursor_expired`.
`core-after/` is the corrected final writer. The two turns in this fixture have
no riding sends, so neither turn belongs to the legacy/envelope ride categories.
Every file below exists in both directories:

| File | What the real frame body shows |
| --- | --- |
| `session.send.accepted.json` | An accepted first send, `state: active`, its run ID and admission baseline. |
| `session.send.pending.json` | A second send durably queued while the first model call is held, `state: pending`, with its submission ID. |
| `session.send.refused.json` | An actual ERROR from retrying the pending send ID with a different prompt: `send_id_reuse`, `detail.field: prompt`. No fabricated refusal. |
| `run.status.json` | The first run after completion, including its final finish reason and usage. |
| `run.result.json` | The same completed run, with final message identity and text. |
| `session.read.page.0.json` | A two-entry page returning ordinals 1 and 2 (`m0`, `m1`); its `next_from_ordinal` is 3. |
| `session.read.page.1.json` | The next request follows that exact continuation and returns ordinals 3 and 4, with no further continuation. |
| `session.subscribe.cursor_expired.error.json` | The ERROR frame body for a well-formed position-only cursor, before any event. |
| `session.subscribe.invalid_params.error.json` | The ERROR frame body for a malformed cursor missing `sub_index`, with `detail.field: from`. |

## Model-byte equality pins

`model/frozen-ride-request.json` and `model/physical-append-request.json` pin the
same synthetic model-input body. One uses the shared frozen-ride renderer; the
other physically appends the same text using the legacy helper. Both exact bodies
are asserted independently against a literal and against each other. These two
files are model-input pins, **not serve-response captures**. Model request bytes
were already right and needed no fix; this pin passed from its first execution.

## Capture and rollout

Capture uses `BROCA_CAPTURE_SAMPLES=1 NEXTEST_SUCCESS_OUTPUT=final` and the filtered
`upgrade_wire_samples`, `denied_call_wire_samples`, `core_reader_wire_samples`
and model pin tests. Raw logs and exact commands are retained under
`evidence/serving-correction/`; see its ledger and
`../investigations/llm-runner-0811-serving-correction.md` for red-first failures,
performance measurements, accepted decisions and remaining integration gates.
No body is pretty-printed or normalized after capture.

The historical byte-pin / reader / writer commits are retained. Master was merged
first; the new correction commit is the reader release (writer switch false), and
the following commit flips only that switch. Ship the corrected reader one release
before the writer. The v1 runner alias remains exactly 0.7.48.

Final release boundaries:

- Historical byte pin: `77f3c633544b9cafdf550fde460f2436988808fa` (retained).
- Master merge: `d1230cd46cc011b35c059b351a1258ea900fface`, incorporating master
  `15c4c5474f1cae3a816dbea19270606f619b1132` before the correction.
- Corrected reader, writer disabled: `b6d463f56dadaac21d785ee1f438868f7d48a991`,
  exact attested tree `bc795b7a7a3a886fbb359cff87d37926db619292`.
- Writer-only flip: `1ee1769db5986ecf670fccae0e19ee5255ccecc9`, exact attested and
  captured tree `e357dec096c7b434563e0363d78a49dcf730658e`.

All **77 JSON body hashes** passed: the original comparison's 57 bodies, 18 added
Core before/after bodies, and two separately labeled model pins. The original
before captures are unchanged. The four final capture tests passed without any
normalization; their 40 final writer bodies are in
`evidence/serving-correction/serving-writer-final-captures.log`.
