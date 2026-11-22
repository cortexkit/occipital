# notice-keep-1122: broca's kept notices (llm-runner `notice_keep`)

Captured from broca's real serve loop and compaction entry projection, on synthetic sessions
(`plan_admission::message_marks` serve tests and
`compaction_v2::metadata_tests::compaction_v2_recorded_kept_notice_carries_keep_and_an_unkept_one_is_unchanged`).
The session's scope owner sends a notice with `keep: true`: the person's own words, which no
compaction provider may drop on its own.

- `keep-send.json`: the `session.send` reply for the kept notice. Unchanged in shape.
- `keep-read.json`: `session.read` with `include_display_facts: true`. The kept message carries
  `notice: true` and `keep: true` beside its other display facts. An unkept message has no
  `keep` member at all (never `keep: false`).
- `keep-default-read.json`: a default `session.read` of the same session. Marks never appear
  there, so neither does `keep`; the message body and `history_version` equal the opted-in
  read's.
- `keep-history-raw.json`, `keep-history-display.json`: `session.history` in both forms. The
  raw entry, and the display row's `row`, carry `notice: true` and `keep: true`.
- `entry-kept-notice.json`: the notice's compaction entry, as `session.entries` and
  `compaction.notice` carry it. Its `notice` origin is `{"sender": …, "keep": true}`; the
  entry digest covers `keep`.
- `entry-notice.json`: the same notice not kept. Its `notice` has no `keep` member, so it is
  byte-identical to what broca sent before kept notices.
- `keep-refused-keep.json`: `keep: true` without `notice: true` refuses `invalid_params`,
  `detail.field: "keep"`, before any write. The same refusal answers `keep` from any sender
  that isn't the session's scope owner.
- `keep-refused-delivery.json`: a kept notice whose delivery order allows `tool_result`
  refuses `invalid_params`, `detail.field: "delivery"`: a kept notice never rides a tool
  result.
- `keep-reuse.json`: a `send_id` retry that drops `keep` refuses `send_id_reuse`,
  `detail.field: "keep"`.
- `role-describe.json`: `role.describe` with `notice_keep` declared right after
  `notice_with_speaks_for`.

broca's next release (0.4.96) only reads kept notices; it still refuses `keep` on a send and
does not declare `notice_keep`. The writer that admits `keep` and declares `notice_keep` ships
after every reader below passes these samples.

Readers asked: Magic Context (entries and reads), Occipital, the desktop app, the phone app,
the terminal client, prefrontal.
