# Tool-result image reads (broca, synthetic)

Captured on 17 Nov 2026 (all four files from one run) from broca's own serve path (test `media_runtime::media_capable_model_without_config_sends_tool_png_on_next_request`, broca master after 0.4.60). One synthetic session: the person says "look", the model calls a tool, the tool returns a text part and a PNG part, the model answers "seen". Each file holds the reply body exactly as broca sent it on the route.

- `tool-image-session-read-default.jsonl`: `session.read {}`. The tool result reads as `kind: text`, with the image as a `[media not shown: … reason=not_requested]` marker.
- `tool-image-session-read-blocks.jsonl`: `session.read {"media": "blocks"}`. The tool result reads as `kind: parts`: `[{kind: text}, {kind: media, media: {sha256, …}}]`.
- `tool-image-session-read-media-refs.jsonl`: `session.read {"media": "media_refs"}`. Byte-equal to the default read: `media_refs` replaces only media the inline read serves in place, and keeps a tool result's flattened text and markers (llm-runner 0.8.28 changelog; `read::descriptors::media_refs`). The tool's image reads by its digest only through `blocks`.
- `tool-image-session-media-read.jsonl`: `session.media_read {sha256, offset: 0, length}` for that part's sha256: `{data (base64), size, mime}`. The same op serves a photo the person sent.
