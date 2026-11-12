# Reply-cleaning samples

One synthetic store session on the current compaction contract, built and read through the
real handlers by `dump_reply_cleaning_samples` (`.cortexkit/scripts/reply-samples/dump.rs`,
appended to `crates/broca-module/src/store_actor/entries_comparison.rs` and run by
`.cortexkit/scripts/reply-samples/run.sh` on the build server). The session runs three runs
under a plan naming the compaction provider `ck-mc` (no `render_tags`), whose replies are,
as the model wrote them:

1. `§2§ Done.`
2. `let me look` (with a tool call)
3. `§5§ §6§ It says file contents.`
4. `As §3§ shows, the file §4§ holds it.`

`base/` is the base commit (accc2a346); `fix/` is the fix. Each holds:

- `session-read-{inline,blocks,media_refs,descriptors}.json`: `session.read` from ordinal 1 in
  each media mode.
- `session-history-raw.json`, `session-history-display.json`: `session.history` from ordinal 1.
- `run-result.json`: `run.result` of `r1`, `r2`, `r3`, in order.
- `session-head.json`: `session.head`.
- `session-entries.json`: the compaction route's `session.entries`, `after`, full detail.
- `SHA256SUMS`: that folder's files.

What differs between the two, by design:

- Assistant text: raw (tags) in `base/`, cleaned in `fix/`, on every read, history and
  descriptor; `descriptors` digests are the cleaned bodies'. `run.result` was already cleaned
  in `base/` and is unchanged.
- `history_version`: `0` in `base/` (the stored version), `1` in `fix/` (stored + 1), in reads,
  history pages, history cursors and `session.head`.
- Output-reference set ids in display history keep the stored version (`h1.<…>.0/…`) in both.
- Message ids, incarnations and wall-clock times are minted per run, so they differ between the
  two folders anyway.
- `session.entries`: identical apart from the minted ids and times; every entry and part digest
  is identical between `base/` and `fix/` (checked by `run.sh`).

`without-fix.log`: the fix with only `session.read`'s cleaning block removed from
`store_read::page`, running
`every_exposure_serves_a_cleaned_reply_and_the_store_keeps_the_written_one`: it fails at the
`session.read Inline` assertion with the raw text.

`SHA256SUMS` (this folder) covers both folders' files and `without-fix.log`.
