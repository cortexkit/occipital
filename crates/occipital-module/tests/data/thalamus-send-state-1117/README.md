# Send-state bus notices, version 1

Recorded from commit `5dbc3e77f3a6c1fe72bcb67a7bb763bbbd8b6004` by
`send_state_delivery_reconciliation_and_not_delivered_are_recorded`.
The test commits send/run changes through the runner store, then records the raw
JSON bodies from the persistent queue of notices awaiting bus acknowledgement.
No wrapper or trailing newline is added.

Readers: Occipital (`m_occipital`) and AFT (`m_aft_sendstate`). Subjects are
`ck.{account}.event.thalamus-harness.send_state.v1.{sender}`; the sender is
the filename prefix. Each sample has seq 2 after its initial notice. The delivered
send still has an active run: delivery is a receipt, not a state. A retracted
send has a notice-only state: send_status refuses to return its status with
`send_retracted`. Nats-Msg-Id is `{session_ref}:{send_id}:{seq}`, using the
caller's wire send id.

Re-record into a new sample version with `THALAMUS_RECORD_DIR` and
`THALAMUS_RECORD_COMMIT` set when running the named test. SHA256SUMS covers
the three JSON bodies and this README.
