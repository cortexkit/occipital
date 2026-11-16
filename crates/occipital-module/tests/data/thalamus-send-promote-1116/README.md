# send-promote-v1

Samples of `session.promote` ("Send now"; the `send_promote` group, §9.24 of the
llm-runner contract, `CONTRACT.md` in the `cortexkit-role-llm-runner` crate) as Thalamus's
OpenCode 2, Pi and Claude Code sessions answer it, for apps that offer to send a queued
message into the running turn. Each `NN-*.json` holding a `request` is one call on the
session's own bound route: the request's params, then either the `reply` body or the
`refusal` (the ERROR frame's `{code, message, detail}`).

- `01-promoted.json`: the caller's message was queued behind the running turn (a
  delivery list `["idle_start"]` the runner still holds) and a run was active. It joins
  that run at its next step boundary as a steer, under the same `send_id`; follow its
  receipt with `session.send_status` as before.
- `02-repeated-request.json`: the same `request_id` again. The first answer, unchanged,
  without acting again (also after a module restart).
- `03-delivered.json`: a send the runner no longer holds as queued. Here the owner's
  legacy `delivery: "queue"` send, which the host was handed when it was accepted;
  equally a send that already started or joined a turn. Its words must not be sent again.
- `04-not-queued.json`: no such send in the caller's own send-id space, or one that ended
  without delivery (withdrawn, replaced by a newer send under its `dedupe_key`, expired,
  or with a receipt that confirms nothing). It confirms no delivery: follow the send's
  own receipt.
- `05-not-promoted-no-active-run.json`: still queued, but no run is active. It stays
  queued and starts the next turn.
- `06-refused-request-id-reuse.json`: a `request_id` already used for another `send_id`,
  refused before anything is written, `detail.field: "send_id"`.
- `07-refused-invalid-params.json`: an unknown member, named in `detail.field`.
- `08-refused-scope-owner-mismatch.json`: a caller that is neither the session's owner
  nor a plan sender with the steer grant.
- `09-refused-unsupported.json`: a session whose harness doesn't declare `send_promote`.
- `10-role-describe-opencode2.json`, `11-role-describe-pi.json`,
  `12-role-describe-claude-code.json`: each host's `role.describe`, declaring
  `send_promote` beside `steer` and listing `session.promote`, with the module's version
  written as `<v>`.

The samples are written by the runner's own test
(`runner::tests::promote_tests::send_promote_samples_are_pinned`), which checks every
reply against them byte for byte, and `SHA256SUMS` lists each file's digest. They are
published: never change a file here; publish a new set beside this one instead.
