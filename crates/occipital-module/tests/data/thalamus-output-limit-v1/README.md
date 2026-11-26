# Output limits and tool loops — reader samples v1

These are reply envelopes from the module's production `run.tool_calls`, `session.read`
and `run.result` serializers using llm-runner 0.10.1. The Linux endpoint tests
`output_limit_uncapped_guard_stops_only_three_consecutive_cut_steps` and
`tool_loop_refuses_the_twentieth_call_before_provider_dispatch` recorded them with
`THALAMUS_RECORD_OUTPUT_LIMIT` set to this directory. The stand-in host and tool
provider supply synthetic messages and results; no live sessions or credentials
are used. Verify the files with `sha256sum -c SHA256SUMS` from this directory.

- `run-tool-calls.json`: the cut calls are `not_executed` with no `dispatched_to`.
  The calls on a recoverable cut step name `reason: "arguments_incomplete"`. The
  last call was never dispatched because the run hit the third-cut hard stop; it
  has no recoverable-cut reason. A completed call keeps its actual receipt; the
  runner never marks an already-dispatched sibling as not executed.
- `session-read.json`: the `include_display_facts` reply carries the same reason in
  `tool_times[]`, with `unanswered: "not_executed"`. The native `message` keeps its
  original error and arguments unchanged. Readers that do not request display facts
  should not expect the reason alongside the read entry.
- `run-result-output-limit.json`: terminal `error` with `provider_code: "output_limit"`,
  `class: "permanent"`, and the last model step's factual finish `length`.
- `run-result-tool-loop.json`: terminal `error` with `provider_code: "tool_loop"` and
  `class: "permanent"`. The runner refused the 20th call before dispatch; no model
  step was stored in this fixture, so the reply does not invent a `finish`.
