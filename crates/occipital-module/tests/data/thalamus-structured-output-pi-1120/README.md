# structured-output-pi-v1

`run.status` and `run.result` response bodies from a send-started Pi 1.6.0 on Linux
with a loopback chat-completions stand-in. Each file contains the two bodies under
`run_status` and `run_result`. The inputs, tools and outputs are synthetic; no live
account or provider is used.

| File | Outcome |
| --- | --- |
| `submit-passed.json` | A schema-invalid attempt did not end its turn; a later accepted submit supplied the value. |
| `submit-failed.json` | An invalid attempt was recorded as failed, including its schema error and skipped sibling. |
| `submit-absent.json` | The turn completed without a submit. |
| `submit-skipped.json` | One plan tool ran; both later calls were skipped, including one that Pi refused before execute. |
| `final-text-passed.json` | The final assistant message parsed and matched the schema. |
| `final-text-failed.json` | The final assistant message was not JSON. |

All structured outcomes are `best_effort` with reason `host_unsupported`. Pi's
capability declaration stays off until consumers can read these response bodies.

The real tests print the captured JSON with a `PI_VECTOR` prefix when
`THALAMUS_PRINT_PI_STRUCTURED_VECTORS=1`. To capture them again, install the pinned
Pi (`npm ci --prefix pi-extension/thalamus`), build the module binary that hosts Pi
(`cargo build --locked -p thalamus-harness-module --features test-support --bin ck-thalamus-harness`), and run
`cargo nextest run --locked -p thalamus-harness-module --features test-support --lib
--run-ignored only --success-output immediate -E 'test(real_pi_structured_)'` with
`PI_BIN` and `THALAMUS_TEST_HOLDER_IMAGE` set. Format each body as JSON; run IDs and
message IDs are those of the test session.

`SHA256SUMS` covers this README and the six response samples. Publish a new version
rather than updating files that readers have pinned.
