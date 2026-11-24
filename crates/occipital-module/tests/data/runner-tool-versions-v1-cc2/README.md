# Tool versions: `tool_changed` and `refetch`

Wire samples for `llm-runner/v1` (`cortexkit-role-llm-runner` 0.9.0), recorded on Linux
by `real_opencode2_tool_versions` (OpenCode 2.3.2), `real_pi_head_tool_versions`
(Pi 1.6.0), and `real_claude_code_tool_versions` (Claude Code 2.1.331, in a later build).
The tests run the real hosts against a loopback model and a
scripted tool provider, and turn on tool-version support through a test-only runner hook.

Each host directory contains:

| File | Wire body |
|---|---|
| `tool-changed.json` | The `session.subscribe` control frame after a provider refused a call with `tool_major_changed`. Its cursor is opaque and specific to the recording. The test also reads the same frame on replay. |
| `refetch-request.json` | The owner's `session.refresh` request, authorizing a fresh read of the named provider with the same plan and an immediate policy. |
| `refetch-reply.json` | The runner's reply accepting that request. The test then checks that the next model request offers the new definition and its tool call dispatches with the new major's pin. |

All three hosts install the newer compatible minor at a run-start reconcile point, so
each `tool-changed.json` expects the minor's pin. On Claude Code the version keeps the
tool's name, so the request that reaches the point carries it: the gateway renders the
new definition and the tool lane runs the call on it. Claude Code's samples were
recorded again when it gained that.
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
