# host_refused replies (Agent Runner, card 9a548b160)

Recorded from commit 9a548b160 by `runner::tests::not_delivered_tests::host_refused_ends_a_proven_unsent_send_and_survives_restarts`
with THALAMUS_HOST_REFUSED_SAMPLES set. Synthetic session; no personal data.

For each permanent host refusal cause (attachment_unreadable, host_configuration_invalid,
host_version_unsupported, host_policy_refused), four bodies:

- `<cause>-send-reply.json`: the `session.send` answer, given once the send is saved: `pending` with its `run_id`.
- `<cause>-send-status.json`: `session.send_status` after the host refused it: `not_delivered`, reason `host_refused`, the same `run_id`. The cause is on the run's error, not here. A repeat `session.send` with the same send id answers this same body.
- `<cause>-run-status.json`, `<cause>-run-result.json`: the send's run: `error`, class `host_refused`, the cause, provider_code `host_policy_refused` for a policy refusal and `provider_error` otherwise. The run never started.

A reader passes when it decodes every body and takes `not_delivered` / `host_refused` as final: no resend, no retry under a new send id.
