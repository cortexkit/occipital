# phone-reader-wake-records-v1

Wake records: what each of core's wakes held, so the apps draw it as chips
instead of the prompt text the model received. Served through Occipital's
real paths by
`occipital-module/src/app_reader/broca/tests/wake_records_samples.rs`. Each
`NN-name.json` is the reply the app receives for `NN-name.request.json`.
`SHA256SUMS` lists every file but this README.

The chat is an OpenCode 2 session (`ses_head`) on Thalamus's runner:

| Ordinal | Message | Record |
| --- | --- | --- |
| 1 | core's wake | one item of every kind |
| 2 | an answer whose `bash` call carried a wake on its result | one `work_result` |
| 3 | core's wake, cut by core | two items and `omittedItems: 38` |
| 4 | core's wake | none: the write missed core's deadline |
| 5 | the person's own message | none |
| 6 | core's wake, sent to `ses_before_move` before its history moved here | one `sidekick_answer` |

- `01-page`: the newest page. Core's wakes are notice rows, as before. No
  record is on the page.
- `02-wake-records`: `session.wake_records {session, harness, messageIds}`
  for the rows the app draws (at most 100). The reply's `messages` holds a
  record for each row whose send has one, as core wrote it; every other row
  is left out. Message 6 gets the record sent to its first session, since
  that is the only record with its send.
- `03-notices-riding-wake`: `session.notices` for the call. The notice that
  rode its result carries its send's record as `wake`.

A record is `{items, omittedItems?}`. Each item is `{kind, refs, from?, to?,
at, excerpt}`, in the tool-display contract's snake_case. `excerpt` is plain
text of at most 200 characters, never Markdown. The kinds are
`peer_message`, `room_post` (one item for a fold of a room's posts, from
`first_seq` to `last_seq`), `worker_question`, `work_result`,
`sidekick_answer`, `approval_card` and `notice` (any other member of the wake,
by `family` and `title`). Draw an unknown kind as the plain card. `from` and
`to` are fleet agent ids, with `display_name` for an agent the fleet doesn't
list.

Records never change once written. Fetch them before inserting a row and
cache the answer; a row without a record keeps its plain card, even if a
record arrives later.

**Text:** the chat text, the records and every id are invented.
