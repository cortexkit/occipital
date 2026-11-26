# phone-reader-wake-notice-record-v1

What the apps receive when prefrontal's wake tells an agent that a
background command ended and AFT's own card draws it (tool-display 0.1.15,
§8.4). AFT gives agents their file and shell tools. When one of its background
commands ends, AFT writes its notice record under a record id it never sends
to the agent. Prefrontal's wake then reports the end to the agent and points
at that record in its wake record as `notice_record: {writer: "aft", id}`.

Occipital gives the wake's row AFT's card only when the record qualifies:
- it is kept for the wake's own session;
- AFT wrote it (the writer the daemon attested at its write);
- AFT serves its view;
- its `data` fits that view's data schema.

Otherwise the row keeps prefrontal's own wake card. An app shows one card per
row, never both. AFT's record never makes a row of its own.

AFT's records and its three notice views are AFT's real ones, the same files
as `phone-reader-notice-records-aft-v2` (AFT commit `01289469c7`). Each view
reaches Occipital as AFT's views bucket delivers it. Each pointer names the
record's `send_id`, the id the contract has a wake point at. Prefrontal's wake
bodies take the contract's example for an awaited-work wake (tool-display's
`wakes.json`, `awaited_work_notice_record`), with each title and pointer
changed to fit its case. Served through Occipital's real paths by
`occipital-module/src/app_reader/broca/tests/wake_notice_record_1126.rs`. Each
`NN-name.json` is the reply the app receives for `NN-name.request.json`.
`SHA256SUMS` lists every file but this README.

The chat is an OpenCode 2 chat on Thalamus, the runner for third-party
harnesses. Its session id, `ses_aft_notice_samples`, is the one AFT's records
name, so each pointed record is kept for the wake's own session. It holds six
wakes:

| Ordinal | Message | Prefrontal's wake points at | What AFT wrote | The row draws |
| --- | --- | --- | --- | --- |
| 1 | an answer whose `read` call carried the wake on its result | a watched command exited | the record | AFT's `bash_watch` card |
| 2 | prefrontal's wake | finished, exit 0 | the record | AFT's `bash_finished` card |
| 3 | prefrontal's wake | failed | nothing | prefrontal's card |
| 4 | prefrontal's wake | timed out | the record without `command`, which the view requires | prefrontal's card |
| 5 | prefrontal's wake | a watch matched, sent as progress | the record | AFT's `bash_watch` card |
| 6 | prefrontal's wake for a browser task | no pointer | nothing | prefrontal's card |

- `01-page`: the newest page. The six rows are prefrontal's, and none is
  AFT's.
- `02-wake-records`: `session.wake_records {messageIds, acceptNoticeRecords:
  true}` for messages 2 to 6. `messages` holds every wake record as
  prefrontal wrote it, the pointer included. `notices` holds AFT's card for
  messages 2 and 5 only, as `{sender, view, key?, data}`.
- `03-wake-records-without-opt-in`: the same read without
  `acceptNoticeRecords`. It holds `messages` alone, as today's apps decode it.
- `04-notices-riding`: `session.notices {callKeys, acceptNoticeRecords:
  true}`. The wake that rode the call carries its wake record as `wake` and
  AFT's card as `noticeRecord`.

AFT's views and templates are read as in `phone-reader-notice-records-aft-v2`
(`04` to `10` there); this set doesn't repeat them.
