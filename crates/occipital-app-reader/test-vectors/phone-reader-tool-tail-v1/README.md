# phone-reader-tool-tail-v1

A command whose row shows its last output lines (tool-display 0.1.12,
`text_lines`). The lines are a completion-only field: the view declares them
in its record schema's `completion_fields`, a record never carries them, and
the completion carries them once the command has answered.

## The field

- `display.views` serves the view's `recordSchema` with
  `completion_fields: {tail: "text_lines"}`, beside the ordinary `fields`.
- `session.tool_records` serves the completion with `tail` as the provider
  wrote it: zero to three lines, each `{text, tone?}`. `text` is 1 to 200
  characters with no control characters, and `tone` is `muted` or `danger`.
  A line without a tone uses the default tone.

A reader that checks a completion against its view's schema must know
`completion_fields`. If it doesn't, it refuses the completion for its
undeclared field, and the row stays without its completion.

## Files

- `01-page`: the newest page. The call carries its key and tool version, which
  joins the completion to it.
- `02-views`: the view, with the completion-only field in `recordSchema`.
- `03-records`: the completion, with three lines: one in the default tone,
  one `muted` and one `danger`.

The command, its output and the pin are invented.
