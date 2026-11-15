# Coded host parts on OpenCode 2 continuation rows, second recording

This folder replaces the OpenCode 2 files of `../host-rows-v1/` for readers of the
llm-runner contract. The Claude Code files of that folder are unchanged and are not
repeated here.

## Why a second recording

In the first recording the `*-read.json` and `*-tag-read.json` samples named the
continuation row's text by the key OpenCode 2 stores it under, `text`. The llm-runner
contract's projection gives that part the key `0`, and a reader applies a read entry's
`host_parts` fact to the projected part with the same key
(`ReadMessage::check_host_parts`). A reader therefore found no part for the fact and
showed the row as host text without its code. The runner now names the part `0` in
`session.read` entries and in the entry inside a numbered-part answer. The display rows
were already right.

The compaction contract names the same part by its stored key, so the compaction files
here (`opencode2-entries*.json`, `*-notice.json`) still say `text`. That is correct and
is what a compaction provider matches against the entry's own `parts`.

## Files

Recorded from the runner's serializers on Linux with a real OpenCode 2.2.3, isolated
host homes and a loopback stand-in model, by the test
`runner::tests_opencode2_real::host_rows::real_opencode2_continuation_host_rows_and_model_request_bytes`.
No personal prompt, live account or installed host was used. The recorder replaces its
scratch and repository paths, the environment's user name in paths, and the
environment's host name before writing, and the folder is scanned for personal paths,
user names, email addresses and machine names.

For each of the three codes (`continued_after_interruption`,
`continued_after_output_limit`, `continued_after_restart`):

| File | What it is | Key of the row's text |
| --- | --- | --- |
| `<code>-read.json` | The row's `session.read` entry, read with display facts | `host_parts[0].part` is `0` |
| `<code>-display.json` | The row's display row | `parts[0].part` is `0` |
| `<code>-tag-read.json` | The answer to a read of the row's numbered part | `entry.host_parts[0].part` is `0`; the answer's own `part` is the index `0` |
| `<code>-notice.json` | The `compaction.notice` params that carried the row to a provider declaring `compaction-host-parts/v1` | `host_parts[0].part` is `text` |

`opencode2-entries.json` is the `session.entries` page for the three rows, served to a
declaring provider, and `opencode2-entries-legacy.json` is the same page as an
undeclaring provider receives it: identical except for the absent host facts.

How each row was produced is as in the first recording: the interruption and
output-limit rows were written by the real host, and the restart row was written
through the host's own `synthetic` route by the test, because no observed restart has
produced one on a runner session.

## Checks

`runner::compaction::host_rows_tests::published_host_row_samples_decode_with_the_pinned_contracts_and_agree`
decodes every file with the pinned contract types, projects each read entry's body
with the llm-runner contract's own projection and requires the entry's fact to pass
`check_host_parts` against the projected keys.

`SHA256SUMS` covers every file above and this README, excluding itself. Verify it with
`sha256sum -c SHA256SUMS` or `shasum -a 256 -c SHA256SUMS`. This folder is immutable
once published; later recordings belong in a new versioned folder.
