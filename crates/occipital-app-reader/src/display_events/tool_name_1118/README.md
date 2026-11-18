# tool_input_delta.tool_name (llm-runner 0.8.65), broca 0.4.70

- `tool_input_delta.fragments.jsonl`: the display events of one `read` call whose input
  streamed as ~2,250 fragments, as broca's run loop sends them (written by
  `a_tool_input_streamed_in_fragments_reaches_the_display_lane_joined`). The provider named
  the tool on every fragment; the first joined chunk carries `tool_name: "read"`, the two
  later chunks carry none.
- `session_subscribe.live_tool.jsonl`, `session_subscribe.live_steer.jsonl`: whole
  `session.subscribe` frames from broca's serve path for a run that calls `work`; the call's
  one chunk carries `tool_name: "work"`.

After a `display_reset` (a provider retry, or a restream of the stored answer), each call's
first chunk carries the name again. The name is the caller's own tool name, never a
provider alias.
