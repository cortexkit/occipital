# Session fact samples

These are served reply bodies of the apps' `session.transcript_page` and
`session.details` operations. Thalamus's own handlers produced them over a fake
Broca route. Each file is compact JSON with no trailing newline, written the way
`serde_json::to_vec` writes it. `SHA256SUMS` lists every file.

## How they were produced

The test
`app_reader::broca::tests::session_facts::the_session_fact_samples_are_what_the_handlers_answer`
in `crates/thalamus-harness-module` builds each session on a fake Broca route.
It serves the reply through the module's `AppReader`, the handler behind the
operations, with a fake Entorhinal lookup in front. It then compares every reply
byte for byte with its file here, and compares `SHA256SUMS` with the digests of
those bytes. To record them again, point
`THALAMUS_RECORD_SESSION_SAMPLES` at a directory and copy what it writes here:

```sh
THALAMUS_RECORD_SESSION_SAMPLES=/tmp/session-facts \
  cargo test --locked -p thalamus-harness-module --lib the_session_fact_samples_are_what_the_handlers_answer
```

Verify:

```sh
(cd crates/thalamus-app-reader/test-vectors/phone-session-facts-v1 && shasum -a 256 -c SHA256SUMS)
```

The activity tokens and times come from the fake route's fixed head. They're
opaque test values, not tokens a live session would accept.

| File | Scenario | Send grant |
| --- | --- | --- |
| `newest-page-model.json` | Newest page whose head names the next send's `model`. | `sender` |
| `matched-claim-model.json` | Claim on that page through `m22` after `m23` was appended. It's matched, carries `model`, and its `newTurns` hold `m23`. | `sender` |
| `details-usage.json` | `session.details` with `model`, `usage` and `incarnation`. | `sender` |
| `details-no-usage.json` | `session.details` whose head has no `usage` and no `sender`, from a runner that doesn't declare `head_sender`. | `senderUnknown: true` |
| `details-no-grant.json` | The same head from a runner that declares `head_sender`. | neither |
| `details-sender-refused.json` | `session.details` for a session Thalamus has no send path to, while the head still names a grant. | neither |
| `newest-page-network.json` | Newest page with each per-message `network` value. | `senderUnknown: true` |

## Reading the members

- **`model`**, `{provider, model, variant?}`: what a send naming no model would
  use next. The newest page carries it, and so does every claim reply that carries
  turns (`matched`, `diverged`, `cursor_expired`). An `unchanged: true` reply and an
  older history page don't. Absent means unknown.
- **Send grant**, one of three forms. `sender` is never `null`, and it never
  appears beside `senderUnknown`.
  - `sender: {modes, interrupt, owner?}`: the runner grants Thalamus, the sender
    the app's messages go through, these delivery modes, so the app may send.
  - Neither member: the app may not send. Either the runner declares the
    `head_sender` describe group (llm-runner 0.6.4) and reports no grant for
    Thalamus, or Thalamus itself refuses sends to the session.
  - `senderUnknown: true`: the runner's head has no `sender`, and the runner
    doesn't declare `head_sender`, or its `role.describe` couldn't be read. An
    older runner leaves `sender` out because it doesn't know the member, so its
    silence can't be read as "no grant".
- **`usage`** (`session.details` only): the runner's durable session spend in
  llm-runner's own `session_usage` shape, snake_case members included. It has a
  required `since` run id, and it never decreases. A count above 2^53 − 1 is left
  out as unknown. Absent means unknown.
- **`incarnation`** (`session.details` only): an opaque identity of the session's
  log. It stays the same across compaction cuts, reverts, restarts, restores and
  moves, and changes only when the log is created again. Compare it only for
  equality. Absent means unknown. When the head carries it, claim versions and
  history cursors bind it instead of the lineage, so a compaction cut no longer
  expires a claim, while a recreated log still does.
- **`createdAtMs` and `network`** on each message depend on the runner serving the
  entry's `written_at`. Broca's live reads don't carry it yet, so on a live
  session today these rows have no `createdAtMs` and their `network` is
  `unknown`. In `newest-page-network.json` the network policy is yes from 700 and
  no from 710. `m20`, written at 705, reads `yes`. `m21`, written at 715, reads
  `no`. `m22` has no write time, so it reads `unknown` and has no `createdAtMs`.
