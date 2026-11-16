# Control frames without a spurious display_reset (broca, 16 Nov)

One run of a test session, as `session.subscribe` sends its control lane: a step that calls a
tool (`read`), the tool's result, then a text step. Each file is one frame's exact bytes from
broca's own serializer (`broca_wire::encode_live_subscribe_control_with_cursor`), numbered in
stream order.

- `before/`: today's build. Control units with no lifecycle event of their own carry
  `"event":{"type":"display_reset"}`: each `tool_call` unit (05) and each `step_finished`
  unit (06, 13), sent right after the step's `assistant_message` (`message_finished`).
- `after/`: the fix. The same units carry no `event` member, as `status` control frames
  already don't. Every other frame keeps its event.

Cursors' `incarnation` and `digest` differ between the folders because each is a fresh test
session; nothing else differs except the dropped `event` members.

A real word drop (a refused step, a retry, a rewritten answer) still sends `display_reset`,
on the display lane, from the run loop; that is unchanged.
