# phone-reader-wake-notice-record-v2

This set holds everything `phone-reader-wake-notice-record-v1` holds, plus one
case: a wake that holds a second notice beside the one it points at
(message 7). That wake keeps prefrontal's own card, because AFT's card in its
place would hide the second notice. The tool-display contract says so from
commons `00269430`: a writer names `notice_record` only on a wake whose single
item is that notice, with no `omittedItems`, and the join serves the pointed
card only for such a wake.

What the apps receive when prefrontal's wake tells an agent that a
background command ended and AFT's own card draws it (tool-display 0.1.15,
§8.4). AFT gives agents their file and shell tools. When one of its background
commands ends, AFT writes its notice record under a record id it never sends
to the agent. Prefrontal's wake then reports the end to the agent and points
at that record in its wake record as `notice_record: {writer: "aft", id}`.

Occipital gives the wake's row AFT's card only when all of these hold:
- the wake holds that one notice and nothing else: one item, no
  `omittedItems`;
- the record is kept for the wake's own session;
- AFT wrote it (the writer the daemon attested at its write);
- AFT serves its view;
- its `data` fits that view's data schema.

Otherwise the row keeps prefrontal's own wake card. An app shows one card per
row, never both. AFT's record never makes a row of its own.

AFT's records and its three notice views are AFT's real ones, exported from
the AFT commit that added these notices (`01289469c7`). They are the same
records and views as `phone-reader-notice-records-aft-v2`, which shows each of
them joined to a notice AFT sent itself and lists the templates each view
names. Each view reaches Occipital as AFT publishes it: AFT keeps its views in
a store on the local message bus that Occipital watches, so no view is fetched.

A notice record is kept under the id its writer gave it, the write's
`send_id`. For AFT's records that id is a record key AFT never sends, and each
pointer names it: `notice_record.id` is the `send_id` of the AFT record the
wake points at. Prefrontal's wake bodies are shaped like the contract's own
example of such a wake (tool-display's `wakes.json`, the case tagged
`awaited_work_notice_record`): one `notice` item and the pointer. Each title
and pointer is changed to fit its case.

`occipital-module/src/app_reader/broca/tests/wake_notice_record_1126.rs`
serves the chat through Occipital's real request handlers and checks every
reply against this set, so a reader change that alters a reply fails it. Each
`NN-name.json` is the reply the app receives for `NN-name.request.json`.
`SHA256SUMS` lists every file but this README.

The chat is an OpenCode 2 chat on Thalamus, the runner for third-party
harnesses. Its session id, `ses_aft_notice_samples`, is the one AFT's records
name, so each pointed record is kept for the wake's own session. It holds
seven wakes:

| Ordinal | Message | Prefrontal's wake points at | What AFT wrote | The row draws |
| --- | --- | --- | --- | --- |
| 1 | an answer whose `read` call carried the wake on its result | a watched command exited | the record | AFT's `bash_watch` card |
| 2 | prefrontal's wake | finished, exit 0 | the record | AFT's `bash_finished` card |
| 3 | prefrontal's wake | failed | nothing | prefrontal's card |
| 4 | prefrontal's wake | timed out | the record without `command`, which the view requires | prefrontal's card |
| 5 | prefrontal's wake | a watch matched, sent as progress | the record | AFT's `bash_watch` card |
| 6 | prefrontal's wake for a browser task | no pointer | nothing | prefrontal's card |
| 7 | prefrontal's wake with a second notice | stopped | the record | prefrontal's card |

- `01-page`: the newest page. The seven rows are prefrontal's, and none is
  AFT's.
- `02-wake-records`: `session.wake_records {messageIds, acceptNoticeRecords:
  true}` for messages 2 to 7. `messages` holds every wake record as
  prefrontal wrote it, the pointer included. `notices` holds AFT's card for
  messages 2 and 5 only, as `{sender, view, key?, data}`.
- `03-wake-records-without-opt-in`: the same read without
  `acceptNoticeRecords`. It holds `messages` alone, as today's apps decode it.
- `04-notices-riding`: `session.notices {callKeys, acceptNoticeRecords:
  true}`. The wake that rode the call carries its wake record as `wake` and
  AFT's card as `noticeRecord`.

The two templates AFT's views name, and each view's reply from
`display.views`, are files `04` to `10` of
`phone-reader-notice-records-aft-v2`. They are unchanged, so this set doesn't
repeat them.
