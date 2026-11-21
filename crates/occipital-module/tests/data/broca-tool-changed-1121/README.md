# tool-changed-1121: broca's `tool_changed` frames (llm-runner 0.9 `tool_versions`)

Captured from broca's real serve loop (`contract_samples::tool_changed_contract_samples`, a
synthetic session). A call to the versioned tool `work`, dispatched under its `tp2` pin, is
refused `tool_major_changed` by its provider.

- `tool_finished.tp2.frame.json`: the refused call's lifecycle frame. Its `schema_pin` is the
  `tp2` pin the call carried. The unit is the call's error result, as for any refused call.
- `tool_changed.frame.json`: the next frame. `event` is `tool_changed {provider, tool,
  expected, current}`, with both pins copied from the refusal and `provider` the plan's
  provider id. It rides a `message_boundary` unit naming the message that made the call, as
  `tool_started` and `tool_finished` already do. It is projected from the call's stored
  outcome, so a replay from a cursor serves it again at the same cursor.

broca sends `tool_changed` only once it declares `tool_versions` in `role.describe`, and it
will not declare before every reader below passes these samples. A reader must not treat
`tool_changed` as a display reset: it leaves the live words of the step in flight alone.

Readers asked: Occipital, the desktop app, the phone app, prefrontal, Magic Context, the
terminal client.
