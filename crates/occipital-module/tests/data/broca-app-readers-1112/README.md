# Broca plan-named reader samples

These samples use synthetic sessions, synthetic principals and a scripted model only. No live session or personal store was opened. The contract reference is `cortexkit-role-llm-runner =0.8.42`, sections 9.19 and 10.1–10.2.

## Files

- `role-describe-before.json`: the real `broca_role_describe_with_writers` serializer at the task's base, before the implementation was installed. The ordinary pre-implementation regression run produced it.
- `role-describe-after.json`: the same real serializer with the reader policy implementation. Its only JSON difference is the added `app_readers` capability. Both discovery samples use the fixed synthetic implementation version `0.0.0-writer-golden`, with the optional writer switches enabled, matching Broca's pinned describe fixture. These two files are role payloads, without a transport response envelope.
- `session-read-unlisted-refusal.json`: the real serve layer's `scope_owner_mismatch` error for `synthetic-stranger`, which is not in `readers` and holds no implicit grant.
- `plan-readers-invalid-refusal.json`: the real serve layer's `invalid_params` error with `detail.field` equal to `plan.readers`, for a duplicate reader list.
- `session-read-owner.json`: the real `session.read` response frame body for the synthetic session owner, `prefrontal-core`.
- `session-read-listed.json`: the same page served to the plan-named `synthetic-reader`. Its bytes are identical to the owner's page. These page files include the normal transport response envelope.
- `SHA256SUMS`: SHA-256 hashes for this README and the six JSON samples.

## Evidence and reproduction

The serve samples are produced by `store_ops_serve::app_readers` in the `broca-module-serve` integration target `it`, over the real `serve_connection` route, registry, new-storage actor and stored-view paths. Set `BROCA_APP_READERS_SAMPLES` to the output folder when running those filtered tests. Set `BROCA_APP_READERS_SAMPLE_PHASE=before` only when capturing discovery from the pre-implementation tree; otherwise the discovery sample is named `after`.

The cold/live admission matrix also checks that refusal does not dispatch a management operation, enter the new-storage page handler or open an actor. Authorization itself may obtain a durable stored view through the same existing view cache that serves an admitted read. The existing management-operation census has no individual `session.plan_view` entry, so the matrix compares the complete counter map as well as page-handler and actor-open counts.

The ordinary pre-implementation run restored only production source files to the base revision and left the new regressions in place. Seven behavior/discovery tests failed as expected; the pre-existing storage byte representation test stayed green. The subscription test failed its shared 60-second event guard because the old implementation never ended a removed reader's stream. No guard mutations or planted-break proofs were used.

An older Broca build can decode a stored plan carrying `readers` while silently reverting to its old read admission. Downgrade after accepting such a plan is therefore unsupported; no rollback machinery is added.
