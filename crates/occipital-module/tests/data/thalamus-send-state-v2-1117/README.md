# Send-state bus notices, version 2

Contract: llm-runner 0.8.60, implemented from the Subconscious contract maintainer's wire description
(the crate was not yet published when recorded). Wire schema remains v1.

Recorded from commit `f4e374693d7420a592a3bf248c16c991e8c65190` by
`send_state_delivery_reconciliation_and_not_delivered_are_recorded`.
The test commits send/run changes through the runner store, then records the raw
JSON bodies from the durable fact outbox. No wrapper or trailing newline is added.
The examples use separate fixture stores with the same deterministic session
reference; they are independent projection examples, not one chronological feed.

Readers: Occipital (`m_occipital`) and AFT (`m_aft_sendstate`). Subjects are
`ck.{account}.event.thalamus-harness.send_state.v1.{sender}`; the sender is
the filename prefix. All keyed reserved senders publish, even without a reader;
a plan's `person: true` does not change the reserved sender's eligibility.
Direct, unverified, native and seeded sends have no notices.

Bodies use the JSON Canonicalization Scheme (JCS), with exactly `send_id`, `seq`, `session_ref`, `state`, `v`
in sorted order, no whitespace or trailing newline, and fewer than 512 bytes.
Oversized bodies are refused and counted, not truncated. The eight samples cover
every state this runner produces: pending, active, finished, superseded, expired,
retracted and not_delivered. It never projects paused. Both active and delivered
show active: delivery is a receipt, not a state. A retracted
send has a notice-only state: send_status refuses to return its status with
`send_retracted`. Nats-Msg-Id is `{session_ref}:{send_id}:{seq}`, using the
caller's wire send id. Headers also carry `kind: send_state`, `session_ref`,
`send_id` and decimal `seq` (plus the existing `Ck-Content-Digest`).

Sequence numbers strictly increase per session, across all its sends and receipt
changes, but need not be dense and are not comparable between sessions. A move
carries the high-water to the new reference. Retirement erases the positions
and high-water; the lineage tombstone prevents any later notice from reusing it.
The durable outbox and source changes commit together before at-least-once
publication; an acknowledgement lost after delivery reuses the message id.

Re-record into a new sample version with `THALAMUS_RECORD_DIR` and
`THALAMUS_RECORD_COMMIT` set when running the named test. SHA256SUMS covers
the eight JSON bodies and this README.
