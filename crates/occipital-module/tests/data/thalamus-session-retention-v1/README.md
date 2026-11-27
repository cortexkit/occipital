# Session retention reader samples, version 1

Contract: `cortexkit-role-llm-runner` 0.10.0, §9.14. OpenCode 2 and Claude Code declare `session_retention` alongside the existing seconds-based `retention`; this does not declare it on Pi.

These are the request and bare result or error bodies from the runner's actual dispatch and serializers, with a private stand-in host, owner and wall clock. The daemon wraps successful results in `{"result": ...}`. The samples include a probe's first send with `keep: {"days": 7}`, the policy reported in its head, and the same head after a durable release while its last run is active. `pending` does not mean the transcript is gone. `deleted` means that call committed the runner's removal; a repeat after completion answers `absent`.

Recorded on Linux by setting `THALAMUS_RECORD_SESSION_RETENTION` to an empty output directory and running `cargo test --locked -p thalamus-harness-module --features test-support --lib session_retention_release_waits_for_run_and_reader_and_survives_a_restart`. Verify the published files with `sha256sum -c SHA256SUMS` from this directory. The fixtures are pinned; future changes must publish a new version.
