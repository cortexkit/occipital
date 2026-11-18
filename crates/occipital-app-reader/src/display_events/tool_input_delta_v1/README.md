# Live tool input samples, version 1

These are unmodified `session.subscribe` frame objects from real heads connected
to loopback stand-in models. The two recordings cover the same sequence with
different live message identifiers:

| File | Head | Tool and live identity |
| --- | --- | --- |
| `pi.json` | Pi 1.5.2 | `bash`, keyed by `live_id` until the assistant message is stored under its `mid` |
| `claude-code.json` | Claude Code 2.1.327 | `echo`, keyed by the provider's `mid` from the first chunk |

- `live` is from a subscription opened before the call. It contains all three
  `tool_input_delta` frames in emission order, followed by the stored assistant
  message's control frames. Only the first chunk has `tool_name`. Joining the
  three `delta` strings gives the `tool_started` event's `arguments` string.
- `attach` is the display snapshot on a separate subscription opened after the
  first two chunks, while the model is held. `display_reset` precedes a single
  `tool_input_delta` with `tool_name` and the entire prefix written so far. The
  prefix is not yet complete JSON.

Frame sequence numbers and control cursors are retained as emitted. Gaps in
`seq` are expected: frames for unrelated messages and run controls are omitted,
and the two subscriptions have independent sequences. IDs, cursors and times
may differ on another recording.

To record into a new directory, set `THALAMUS_TOOL_INPUT_DELTA_SAMPLES` to that
writable directory and run the ignored `real_pi_records_tool_input_delta` and
`real_claude_code_records_tool_input_delta` tests with `test-support` and the
pinned real-host binaries on Linux. `SHA256SUMS` pins these recordings, not the
output of future runs.
