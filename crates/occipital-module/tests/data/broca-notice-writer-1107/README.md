# Broca's fixed notice writer, before and after

Replies recorded by Broca from its real serializers for one session in which
three notices rode a single tool call, published for its readers and copied
here unchanged. `SHA256SUMS` lists the 14 reply bodies (this README is
Occipital's own and is not listed).

- **The change:** Broca used to mark the tool-result message that carried
  the notices with an entry-level `delivered_as: {mode: "tool_result",
  call_key}`, which reads as if the notices had made a transcript entry. The
  fix leaves that message unmarked. Ordinals, mids, the stored tool output
  and what the model is sent are unchanged.
- **The bodies, under `before/` and `after/`:** `session.read` (default and
  with display facts), `session.history` (raw and display), `session.entries`,
  the subscribe frames (`0811.subscribe.envelope.json`, one array of frames)
  and the `notice_delivered` frame.
- **What differs:** only the default and display-fact reads and raw history,
  which lose the carrier's mark. Display history, entries and the subscribe
  frames are the same in meaning on both sides.

`src/app_reader/broca/tests/notice_writer_1107.rs` runs every body through
Occipital's reader paths.
