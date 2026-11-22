# Tool versions: `tool_changed` and `refetch`

Wire samples for `llm-runner/v1` (`cortexkit-role-llm-runner` 0.9.0), recorded in one
Linux build by `real_opencode2_tool_versions` (OpenCode 2.3.2),
`real_pi_head_tool_versions` (Pi 1.6.0), and `real_claude_code_tool_versions`
(Claude Code 2.1.330). The tests run the real hosts against a loopback model and a
scripted tool provider, and turn on tool-version support through a test-only runner hook.

Each host directory contains:

| File | Wire body |
|---|---|
| `tool-changed.json` | The `session.subscribe` control frame after a provider refused a call with `tool_major_changed`. Its cursor is opaque and specific to the recording. The test also reads the same frame on replay. |
| `refetch-request.json` | The owner's `session.refresh` request, authorizing a fresh read of the named provider with the same plan and an immediate policy. |
| `refetch-reply.json` | The runner's reply accepting that request. The test then checks that the next model request offers the new definition and its tool call dispatches with the new major's pin. |

OpenCode 2 and Pi install the newer compatible minor at a run-start reconcile point.
Claude Code keeps the old definition at that point: the gateway receives an
already-built model request and has no automatic run-start installer. It can still
recover through explicit owner refetch, but cannot declare `tool_versions`.
These samples do not declare the capability on any host.

To record into a **new** directory, stage those pinned hosts and set their binary paths.
Set `THALAMUS_TEST_HOLDER_IMAGE` to the test-support build of `ck-thalamus-harness`,
whose holder process keeps the OpenCode 2 server running, and run:

```sh
THALAMUS_TOOL_VERSION_SAMPLES=/absolute/path/to/new-directory \
  cargo nextest run --locked -p thalamus-harness-module --features test-support \
  --lib --run-ignored only --no-fail-fast -E 'test(/::tool_versions::real_.*_tool_versions$/)'
```

`SHA256SUMS` covers the nine JSON wire bodies.
