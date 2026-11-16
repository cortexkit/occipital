# Host-unavailable reply bodies, version 1

Recorded from commit `5d51f613a98be48613343e252548c9a00e90f35f` by `host_unavailable_reply_bodies_recorded`.
The test drives real Runner dispatch, its durable sweep, and session subscription,
with an in-memory host and synthetic session, send, message and run identifiers.
No live store, account, project or person's words were used.

The send was accepted, never handed over, then retired after its stored five-minute
deadline while its host was unreachable. Reattaching does not deliver it.
The run ends once, error class `host_unavailable`, provider code `provider_error`,
cause `host_unreachable`. `session.send_status` and a same-id replay are final
`not_delivered` with reason `host_unavailable`, without a delivery receipt.
The subscriber receives the ordinary `run_finished` event for this ending.
These new bodies require reader acceptance before a producer card ships.

Files are unmodified dispatch bodies (no trailing newline). SHA256SUMS includes
this README and every JSON body. Regenerate on Linux from this commit with
`THALAMUS_HOST_UNAVAILABLE_SAMPLES=<output-dir> THALAMUS_SAMPLE_COMMIT=5d51f613a98be48613343e252548c9a00e90f35f cargo test --locked -p thalamus-harness-module --features test-support --lib runner::tests::held_hosts::host_unavailable::host_unavailable_reply_bodies_recorded -- --exact`.
