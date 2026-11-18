# Pi and Claude Code tool call timing controls, v1

`pi.jsonl` and `claude-code.jsonl` each contain one `tool_started` control and
its corresponding `tool_finished` control. The timestamps are Unix epoch
milliseconds from the module's dispatch and answer records, not timestamps on
the host transcript. Readers should use the two integers as supplied; neither
member is present when the corresponding time was not recorded.

The samples come from real Pi 1.5.2 and Claude Code 2.1.327 processes driving a
stand-in model and tool provider on Linux. The provider is held at a gate;
both tests verify that the start is recorded before the provider runs and
that the finish is recorded after the gate is released. The timestamps,
outcomes, output byte counts and test schema pins are unchanged. Session,
message and call IDs are replaced consistently, arguments are replaced with
`{}`, and delivery metadata such as subscription cursors is not included.
There are no live accounts, personal paths or user data.

Run the existing real-host rigs with `THALAMUS_RECORD_CALL_TIMES=1` and
`cargo nextest run --success-output immediate` to get `CALL_TIMES_SAMPLE`
records. The tests are `real_pi_head_live_attach_mid_call_replays_the_steps_controls`
and `real_claude_code_a_running_calls_start_reaches_subscribers_while_it_runs`.
Each record's JSON array is written one control per line. Check the published
files with `sha256sum -c SHA256SUMS` from this directory.
