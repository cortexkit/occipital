# phone-reader-notice-records-v1

Notice records: the data a module writes for one of its own notices, so the
apps draw the notice as a card from that module's notice view instead of the
folded notice text (tool-display §10). Served through Occipital's real paths
by `occipital-module/src/app_reader/broca/tests/notice_records_samples.rs`.
Each `NN-name.json` is the reply the app receives for `NN-name.request.json`.
`SHA256SUMS` lists every file but this README.

The chat is an OpenCode 2 session (`ses_head`) on Thalamus's runner. AFT sent
each notice and wrote its record first:

| Ordinal | Message | Record |
| --- | --- | --- |
| 1 | AFT's notice: a command finished | `finished:1`, key `task-bg_1` |
| 2 | an answer whose `read` call carried AFT's notice on its result | `still_running:1`, key `task-bg_2` |
| 3 | AFT's notice: a command failed | `finished:1` with `status: "failed"`, key `task-bg_3` |
| 4 | AFT's notice: a watch matched | `watch:1`, no key |
| 5 | AFT's notice: a command still running | `still_running:1`, key `task-bg_5` |
| 6 | AFT's notice: the same command finished | `finished:1`, key `task-bg_5` |
| 7 | AFT's notice, sent to `ses_before_move` before its history moved here | `finished:1`, key `task-bg_7` |
| 8 | AFT's notice | `paused:1`, a view AFT doesn't serve |
| 9 | AFT's notice | none: the write missed AFT's deadline |

AFT serves the views `finished`, `still_running` and `watch`, and answers
`unknown` for `paused`.

- `01-page`: the newest page. AFT's notices are notice rows, as before. No
  record is on the page.
- `02-wake-records-with-notices`: `session.wake_records {session, harness,
  messageIds, acceptNoticeRecords: true}` for the rows the app draws (at most
  100). Beside the wake records (`messages`, none here), the reply's
  `notices` holds each row's notice record whose view its writer serves, as
  `{sender, view, key?, data}`; `view` is the qualified key
  `nv1:<sender>:<name>:<version>`, which the app passes to `display.views` as
  a pin. Message 7 gets the record sent to its first session, since that is
  the only record with its send. Messages 8 and 9 are left out and draw the
  folded notice. Asked for, `notices` is there even when empty.
- `03-wake-records-without-opt-in`: the same read without
  `acceptNoticeRecords`. The reply is the one older apps decode: no
  `notices` member.
- `04-notices-riding`: `session.notices {callKeys, acceptNoticeRecords:
  true}` for the call. The notice that rode its result carries its send's
  record as `noticeRecord`, beside its text.
- `05-notice-views`: `display.views {pins}` with notice view keys. Each of
  AFT's views answers `{provider, templateRevision, row, page}`, the digests
  of its templates that passed Occipital's checks. `nv1:aft:paused:1` is
  `unknown`. `nv1:cerebellum:finished:1` is `unavailable`: only Cerebellum can
  serve a view its key names, and it hasn't yet.
- `06-notice-template`: `display.template {digest}` for the `finished` row.

Folding is the app's: within a chat it shows only the latest notice whose
sender and key match, at the place it arrived. Messages 5 and 6 share
`task-bg_5`, so message 6's card stands and message 5's is folded away.
A template evaluates with the record's `data` as `input` and the fixed view
of a notice: `{call: {phase: "answered", startedAtMs: null, finishedAtMs:
null, isError: null, outputBytes: null, unanswered: null}, record: null,
completion: null}`, plus `detail: null` and `output: null` on a page.
