# phone-reader-notice-records-aft-v2

This set replaces `phone-reader-notice-records-aft-v1`. The tool catalog changed in place when
replies stopped holding `Table`, so its digest is now
`sha256:f144092fdccf4813f84f84f034356532bc6770e7ae63dce3bf0fc2f7600f20d3`,
and v1's (`sha256:6e323928…`) is no longer accepted. The templates name
the new digest, which changes the template digests the notice views give (`04`) and the templates `05` to `10` read.
Every other file is v1's bytes.

AFT recorded its views against the old catalog. The test reads each one
with its catalog digest moved to the current one, the only change AFT's
republished views carry.

AFT's real notice records and notice views, as the apps receive them from
Occipital. AFT is the module that gives agents their file and shell tools; its
notices say when a background command finishes or needs a look. AFT exported
its writes, their `display_record_ready.v1` bodies and its views bucket values
from the AFT commit that added these notices (`01289469c7`). Occipital's
test data holds them byte for byte, in
`occipital-module/tests/data/aft-notice-1120/`. Each view reaches Occipital as
AFT's bucket delivers it, and each record as AFT announces it, read back with
`display.read`. Served through Occipital's real paths by
`occipital-module/src/app_reader/broca/tests/aft_notice_1120.rs`. Each
`NN-name.json` is the reply the app receives for `NN-name.request.json`.
`SHA256SUMS` lists every file but this README.

The chat is an OpenCode 2 chat on Agent Runner, the runner module that hosts
OpenCode 2 sessions. Its session id,
`ses_aft_notice_samples`, is the one AFT's records name, so each record joins
its own row. It holds AFT's seven notices about background commands:

| Ordinal | Message | AFT's case | View |
| --- | --- | --- | --- |
| 1 | an answer whose `read` call carried AFT's reminder on its result | still running after 20 min | `nv1:aft:bash_still_running:1` |
| 2 | AFT's notice | finished, exit 0 after 22 min | `nv1:aft:bash_finished:1` |
| 3 | AFT's notice | failed, exit 1, preview cut | `nv1:aft:bash_finished:1` |
| 4 | AFT's notice | stopped after 3 min | `nv1:aft:bash_finished:1` |
| 5 | AFT's notice | timed out after 30 min | `nv1:aft:bash_finished:1` |
| 6 | AFT's notice | a watch matched | `nv1:aft:bash_watch:1` |
| 7 | AFT's notice | a watched command exited | `nv1:aft:bash_watch:1` |

The reminder on message 1 and the notice on message 2 share the fold key
`aft:bash-task:bash-0123456789abcdef`, so an app shows only message 2's card
once it has arrived. The watch notices carry no key and never fold.

- `01-page`: the newest page. AFT's notices are notice rows, with no record.
- `02-wake-records-with-notices`: `session.wake_records {messageIds,
  acceptNoticeRecords: true}` for messages 2 to 7. Each row's record is under
  `notices`, as `{sender, view, key?, data}`.
- `03-notices-riding`: `session.notices {callKeys, acceptNoticeRecords:
  true}`. The reminder that rode the call carries its record as
  `noticeRecord`.
- `04-notice-views`: `display.views` for AFT's three notice view keys. All
  three name the same row and page templates, so their digests match. One
  template draws every kind of bash notice from `data.kind` and
  `data.status`.
- `05` to `10`, `NN-template-<view>-<row|page>`: `display.template` for each
  view's row and page. They are the two templates each view names, read once
  per view.

A template evaluates with the record's `data` as `input` and the fixed view
of a notice: `{call: {phase: "answered", startedAtMs: null, finishedAtMs:
null, isError: null, outputBytes: null, unanswered: null}, record: null,
completion: null}`, plus `detail: null` and `output: null` on a page.
