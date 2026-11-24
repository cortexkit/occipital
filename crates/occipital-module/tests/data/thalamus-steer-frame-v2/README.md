# Claude Code steer frames v2

Captured with real Claude Code 2.1.331 on Linux and Thalamus build `835e767e9107d083b89a64af034105a642d02fc7`,
using a loopback model and synthetic messages. Each steer landed as a separate
message in the same running turn.

- `steer-N/read-entry.json`: the `session.read` entry with `include_display_facts: true`.
- `steer-N/display-entry.json`: the `session.history` display entry; its row's text is
only the person's words and the wrapper is two separately marked host parts.
- `steer-N/stored-message.json`: the same entry's native `message` as served,
a `role: system` message that retains Claude Code's wrapper.
- `compaction-entries.json`: a full `session.entries` page for all six steers,
served to a provider declaring `compaction-host-frame/v1` at Setup.
- `head.txt`: the 52 UTF-8 bytes before the words, including its newline.
- `tail.txt`: the 232 UTF-8 bytes after the words, starting with two newlines.
- `SHA256SUMS`: SHA-256 digests of all the other files in this directory.

The steers, in order, contain one line, multiple lines with a blank line,
non-ASCII text, a copy of the opening line, a copy of the closing paragraph,
and slash-prefixed words. The private-use character the runner adds to keep
Claude Code from executing a slash command is absent from the served message.
No fields were removed from the read entries.

The compaction contract (W4.6) never numbers text in `system` messages. A steer
is stored as a native `system` message, so `session.tag_read` cannot select it.
A host frame changes no numbering.

To record a fresh set, run the ignored `real_claude_code_records_steer_frame_samples`
test with `THALAMUS_STEER_SAMPLES` pointing at an empty directory and
`THALAMUS_STEER_SAMPLES_BUILD_COMMIT` set to the full build commit.
