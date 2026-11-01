# Phone session facts, version 2

This set supersedes `phone-session-facts-v1` only in its opaque `activityToken` values. The published version 1 set remains unchanged. Every other JSON value and every request/response shape is identical; `contract_valid_head_samples_change_only_the_opaque_activity_token` checks that boundary.

The module's fake Broca route has `first_ordinal: 20`, so the llm-runner 0.6.4 contract requires a `cut` in its head. Including that previously missing metadata changes the activity-token hash. These samples are recorded through the same module handlers as version 1, not from a live session. See version 1's README for the individual model, usage, sender, incarnation and network cases.

Verify the handler output:

```sh
cargo test --locked -p thalamus-harness-module --lib the_session_fact_samples_are_what_the_handlers_answer
cargo test --locked -p thalamus-app-reader --test phone_reader_samples
```

To record a candidate set, set `THALAMUS_RECORD_SESSION_SAMPLES` to a new scratch directory when running the handler test. Never overwrite a published set. `SHA256SUMS` hashes the exact JSON bytes; the README is not part of the manifest.
