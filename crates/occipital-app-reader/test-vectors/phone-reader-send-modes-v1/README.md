# phone-reader-send-modes-v1

How an app chooses when its message reaches the agent: the `mode` member of
`session.enqueue_user_message`. Each request is served through Occipital's own
frame loop on a paired phone's route, against a test runner that keeps Broca's
rules. The set is generated, so its text is invented. `NN-*.request.json` is the
request the app sends, and `NN-*.json` is the reply body it receives.

Beside each send that reaches the runner, `NN-*.runner-send.json` is the
`session.send` body Occipital wrote to the runner. The apps never see it. It is
here so the set also shows which delivery each mode becomes.

Both chats have a running turn. Their grants are what core gives the apps, and an
app reads them from `sender` on `session.details`, the newest page, a claim's
answer and transcript notices:

- the head (`head-session`): `modes: ["steer", "idle_start"]`, `interrupt: true`;
- the worker (`worker-session`): `modes: ["tool_result", "steer"]`,
  `interrupt: false`.

| Files | What they show |
| --- | --- |
| `01-steer` | `mode: "steer"` to the head. The message joins the running turn at its next step. If that turn ends first, it starts the next turn, so it is never dropped and has no deadline. The runner is sent `delivery: ["steer", "idle_start"]`. |
| `02-steer-repeat` | The app's retry after a lost reply, with the same id, text and mode. It answers the first receipt with `duplicate: true` and the same `effectId`. Nothing reaches the runner. |
| `03-repeat-without-mode` | A retry under the same id that drops the mode is a different message, refused `client_message_id_conflict`. A retry must repeat the first attempt's mode. The refusal's message names text and attachments, but a changed mode is refused the same way. |
| `04-queue` | No `mode`, which means `queue`. The message waits and starts the next turn after the running one ends. The runner is sent `delivery: ["idle_start"]`. |
| `05-interrupt` | `mode: "interrupt"` to the head, whose grant has `interrupt: true`. The running turn ends and a new one starts with the message. The runner is sent `delivery: "interrupt"`. |
| `06-worker-steer` | `mode: "steer"` to the worker. A worker has no idle start, so the message joins only its running turn, with a deadline 30 minutes after the send (`expires_at_ms`). No `mode` is sent the same way to a worker. If the turn ends before taking it, `queuedSends` lists it as `unsent` (see `phone-reader-steer-run-ended-v1`). |
| `07-worker-interrupt` | `mode: "interrupt"` to the worker, whose grant has no interrupt. It is refused `enqueue_not_authorized` and nothing is sent. |

`SHA256SUMS` covers every file except this README. A test in
`occipital-module` (`send::tests::mode_samples`) serves the set again and fails
if any byte changes.
