# Phone Broca part-key samples, version 3

This set supersedes `phone-reader-broca-v2-keys` only in its opaque `activityToken`. The published version 2 set remains unchanged. Every other JSON value and all request, row, claim and watch shapes are identical; `contract_valid_head_samples_change_only_the_opaque_activity_token` checks that boundary.

The fake Broca head's positive `first_ordinal` requires a `cut` under llm-runner 0.6.4. Adding that previously missing test metadata changes the newest page's activity-token hash, not its durable identities or parts. The samples still come from the module's device-route handlers over a fake Broca route. See version 2's README for provenance, case descriptions and the part-key contract.

Verify the handler output and checksums:

```sh
cargo test --locked -p thalamus-harness-module --lib the_part_key_samples_are_what_the_handlers_answer
cargo test --locked -p thalamus-app-reader --test phone_reader_samples
```

To record a candidate set, set `THALAMUS_RECORD_KEY_SAMPLES` to a new scratch directory when running the handler test. Never overwrite a published set. `SHA256SUMS` hashes the exact request and reply bytes; the README is not part of the manifest.
