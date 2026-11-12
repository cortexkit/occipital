# Temperature discovery reader samples

These are compact `role.describe` discovery objects, **without** the production
`{"result": ...}` envelope. Both were emitted by the real
`broca_wire::broca_role_describe_with_writers` serializer with the fixed fixture
implementation version `0.0.0-writer-golden`, all writers enabled, and service
tiers disabled. No JSON was edited to synthesize the capability change.

- `role-describe-before.json`: captured before the temperature declaration. Its
  SHA-256 also matches the writer discovery golden on base commit
  `3d2e097ce70f8585fe0fb3b3acca7d90a0b805bd`.
- `role-describe-after.json`: captured after adding the independent `temperature`
  capability. It matches the regenerated writer discovery golden exactly.
- `SHA256SUMS`: checksums of those two files, suitable for `shasum -a 256 -c`.

The only decoded difference is the addition of `temperature` to `capabilities`.
There is no new discovery metadata and no durable-record field. The effective
sampling setting is stored in the existing generation member; older runs retain
their recorded generation bytes without re-admission or re-clamping.

Capture uses the test `temperature_describe_serializer_sample` with
`BROCA_TEMPERATURE_SAMPLES` set to this directory and
`BROCA_TEMPERATURE_SAMPLE_PHASE=before` or `after`. The before capture ran while
the declaration was still absent. The archived contract crates were downloaded
from crates.io and verified against their registry-index checksums in the sibling
`contract-ref` directory.

Before deploy, the integrating agent must run native conformance cases
`temperature_reaches_model`, `temperature_invalid_refused`, and
`temperature_with_output_cap`. The current kit has no public per-case selector;
its daemon suite is deliberately not run by this worker. The rig now declares
`Capability::Temperature`, supplies a dedicated scripted catalog row accepting
`[0, 2]`, routes `cf-temperature-*` sessions to it, and observes the actual request
temperature. `temperature_with_output_cap` remains inapplicable to discovery
because Broca intentionally does not declare `max_output_tokens` across ChatGPT
routes; the in-tree admission/render regression covers the sibling setting.
