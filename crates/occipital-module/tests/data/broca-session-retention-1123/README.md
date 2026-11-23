# session-retention-1123: broca's storage policies and release (llm-runner `session_retention`)

Captured from broca's real serve loop (`head_read_additions::heads_carry_the_sessions_storage_policy`)
and its head serializer (`role::pinned_reply_bytes::a_released_session_head_carries_its_release`),
on synthetic sessions.

- `retention.assigned.head.json`: `session.head` of a session whose starter sent no
  `retention_policy`. Every written head now carries `retention_policy`, here the one broca
  assigns (`keep: "forever"`, not backed up, from the root the session is stored in), and
  `retention_state: {"source": "runner"}`. A head broca stores in its backed-up root reads
  `backup: true`. Nothing else in the head changes.
- `retention.starter.head.json`: `session.head` of a session whose first send carried
  `{"keep": {"days": 7}, "archive_after_days": 0, "backup": false, "request_dumps": true}`.
  The policy is served exactly as sent; `retention_state` is `{"source": "starter",
  "request_dumps_unsupported": true}` because broca saves no request dumps.
- `retention.released.head.json`: a head between its owner's `session.release` and its deletion
  (here a run still holds it). It adds `released: {at_ms, delete_after_ms}`. This sample comes
  from the head serializer with fixed values, since the serve rig can't hold a run open.
- `retention.conflict.send.json`: a later `session.send` carrying a different policy. It refuses
  `retention_conflict` with `detail.frozen` set to the frozen policy, and writes nothing.
- `retention.release.deleted.json`: `session.release` from the owner of an idle session:
  `{"state": "deleted"}`. A release while a run, a pause or a queued send holds the session
  answers `{"state": "pending"}`; a release of a session that isn't there answers
  `{"state": "absent"}`.
- `retention.deleted.head.json`: `session.head` after the deletion: `unknown_session`, not
  `expired`. Reads refuse the same way, `session.media_read` refuses `unknown_media`, and the
  next send starts a new incarnation.

While a session is released and not yet deleted, every send refuses `session_released` with
`detail.delete_after_ms` equal to the head's `released.delete_after_ms`.

broca's live 0.4.103 reads storage-policy records and writes none. The writer (these members,
`session.release`, and `session_retention` declared in `role.describe`) ships only after every
reader below passes these samples and Ufuk rules on two contract points SUBC raised.

Readers asked: the terminal client, the desktop app, the phone app, Occipital, Magic Context,
prefrontal, AFT.
