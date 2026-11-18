# tool_media_refs samples (broca, 18 Nov)

Real `session.read` and `session.media_read` reply frames from broca's media test rig. The rig's
model asks for one tool call, and the tool returns one PNG. Each `.jsonl` file holds one reply
frame per line. The session, run and media ids are the rig's own synthetic values.

| File | Request |
| --- | --- |
| `tool-image-session-read-default.jsonl` | `session.read {}` |
| `tool-image-session-read-media-refs.jsonl` | `session.read {"media":"media_refs"}` |
| `tool-image-session-read-tool-media-refs.jsonl` | `session.read {"media":"media_refs","tool_media":"refs"}` (new) |
| `tool-image-session-read-blocks.jsonl` | `session.read {"media":"blocks"}` |
| `tool-image-session-media-read.jsonl` | `session.media_read` for the image's reference |

What changes: with `tool_media: "refs"`, a tool result that carries media keeps its parts, and
each image appears as the same reference block `media: "blocks"` serves, never its bytes. In
these samples the reply is byte-identical to the `blocks` read. Without the member, a
`media_refs` read is unchanged, and here equals the default read.

Refusals, each `invalid_params` with `detail.field: "tool_media"`:

- `tool_media` without `media: "media_refs"`, e.g. `{"tool_media":"refs"}` or
  `{"media":"blocks","tool_media":"refs"}`;
- `tool_media: null`;
- any value but `"refs"`, e.g. `"inline"`.

`role.describe` adds the capability `tool_media_refs`. `SHA256SUMS` covers the five files.
