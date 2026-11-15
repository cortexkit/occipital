# phone-reader-promote-v1

"Send now", as an app sends it: `session.promote_send {session, harness?,
clientMessageId, clientRequestId}`. It moves a message the person sent, still
waiting for the agent's running turn to end, into that turn. Each request is served
through Occipital's own frame loop on a paired phone's route, against a test runner.
The runner's answers are the runner contract's own promote vectors
(`send-promote.json`, llm-runner 0.8.56, group `send_promote`).
`NN-*.request.json` is the request the app sends, and `NN-*.json` is the reply body
it receives.

`clientMessageId` names the waiting message, the id the app sent it with. Occipital
looks up the send id its receipt recorded and asks the runner's `session.promote`
about that send. Beside each request that reaches the runner,
`NN-*.runner-request.json` is the body Occipital sent. The apps never see it. It
shows that the runner is asked about the message's own send (`send_id`), and that
the app's `clientRequestId` goes to the runner as its `request_id`.

`clientRequestId` names the press. Retry a press whose reply was lost under the same
id: the runner answers with its first answer and nothing runs again. A new press
takes a new id.

The runner decides in one step with its queue, so the message never runs twice. The
reply is the runner's, passed on unchanged:

- `01-promoted`: `{outcome: "promoted"}`. The message was still waiting and a turn
  was running, so it joins that turn at its next step. If that turn ends before then,
  the message runs as the next turn, as it would have anyway. Keep showing it as
  queued: it leaves `queuedSends` when it lands, as any queued message does.
- `02-repeat`: the same press again after a lost reply. Same request to the runner,
  same answer.
- `03-delivered`: `{outcome: "delivered"}`. The turn ended first, and the message
  already started or joined a turn. Nothing changes.
- `04-no-active-run`: `{outcome: "not_promoted", reason: "no_active_run"}`. The
  message still waits, but no turn is running to join. It runs when the next one
  starts.
- `05-not-queued`: `{outcome: "not_queued"}`. The runner has no such message
  waiting, or it ended without being delivered.

An outcome or a reason the app doesn't know never means the message was moved.

Refusals, where nothing changed:

- `06-unknown-message`: `not_found` with `detail.field: "clientMessageId"`. No
  message with this id was sent through Occipital (or its receipt is over 48 hours
  old). The runner isn't asked.
- `07-unsupported`: `unsupported`. The chat's runner doesn't offer "Send now". The
  runner isn't asked.
- `08-harness-unsupported`: `unsupported` too. The runner offers it, but this chat's
  host takes no steer (Claude Code).
- `09-press-id-reused`: `client_request_id_conflict` with `detail.field:
  "clientRequestId"`. The press id was used before for another message.
- `10-not-authorized`: `enqueue_not_authorized`. The chat's plan doesn't let the apps
  steer it.

Offer "Send now" only on a `queued` entry of `queuedSends`, in a chat whose
`sender.modes` include `steer`.
