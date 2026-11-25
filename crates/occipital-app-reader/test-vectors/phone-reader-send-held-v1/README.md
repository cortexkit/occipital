# phone-reader-send-held-v1

Messages sent while the chat's runner restarts for an update. Served through
Occipital's real request handling by `occipital-module`'s
`broca::tests::send_held_samples`, so each reply is byte for byte what a phone
receives. `NN-name.request.json` is the request the app sends and
`NN-name.json` the reply body. `SHA256SUMS` lists every file but this README.

## What Occipital does

When a send can't reach the chat's runner (it is restarting, or the call got
no answer), Occipital keeps the message and answers the app at once with the
usual accepted receipt plus `held: true`. It sends the message again by itself
until the runner takes it, under the same send id. A runner answers a repeat
of a send id with the send it already has, so a message never runs twice.

- **Order:** a later message to the same chat waits behind the held ones, even
  once the runner is back, and they go out in the order they were sent.
- **The waiting list:** a held message is listed in `queuedSends` as `queued`
  with `held: {sinceMs, untilMs}`: when Occipital kept it, and when it gives up.
  Once the runner takes it, `held` is gone and the entry goes on as any queued
  message: it leaves the list when it lands in the chat.
- **Giving up:** a message the runner never takes within 10 minutes is listed
  `unsent` with reason `runner_unavailable`. The app keeps the words and offers
  to send them again as a new message. A refusal the runner says can never pass
  ends it the same way, with that refusal's code as the reason.
- **A steer:** sent again as a steer that may start a turn (`steer`, then
  `idle_start`). It joins the running turn if one runs, and otherwise starts
  one. A restart usually ends the turn it was sent into, so it most often
  starts a new turn.
- **Send now:** a held message hasn't reached the runner, so
  `session.promote_send` on it answers `{"outcome": "not_queued"}`, as for any
  message the runner doesn't hold. An app that reads `held` doesn't offer it.
- **Repeats:** an app that sends the same `clientMessageId` again gets the held
  receipt back with `duplicate: true`. The app never needs to resend a held
  message itself.

A worker's steer is not held: it may only join the run it was sent into, which
a restart ends, so it is answered `not_sent` as before.

## Files

The chat is an invented Broca head whose runner runs a turn throughout.

- `01-send-held.json`: "Is the release ready?" (`c-first`), sent while the
  runner restarts: accepted with `held: true`.
- `02-page-held.json`: the newest page, listing `c-first` held.
- `03-send-behind.json`: "And tag it when it is." (`c-second`), sent two
  seconds later with the runner back: held behind `c-first`, and the runner
  got nothing.
- `04-page-two-held.json`: both held, in the order sent.
- `05-send-now-held.json`: "Send now" on `c-first`: `not_queued`.
- `06-page-taken.json`: the runner took both, in order, and holds them for the
  next turn: both `queued`, neither `held`.
- `07-send-steer-held.json`: a steer, "Skip the staging step." (`c-steer`),
  sent during a second restart: held.
- `08-page-steer-held.json`: `c-steer` held after the two queued messages.
- `09-page-given-up.json`: 10 minutes later the runner is still down, and
  `c-steer` is `unsent` with reason `runner_unavailable`.
