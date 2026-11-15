# phone-session-send-now-v1

Whether an app may offer "Send now" for a queued message (`session.promote_send`,
see `phone-reader-promote-v1`). The chat's `sender`, on `session.details`, the
newest page and every claim reply that carries session facts, gains
`promote: true` when both hold:

- the chat's runner declares `send_promote` for this session (the runner contract
  allows it only where the runner also declares `steer`);
- the grant's `modes` include `steer`.

Otherwise `promote` is absent. Show "Send now" only on a `queued` entry of
`queuedSends` in a chat whose `sender.promote` is `true`. Each file is the reply
Occipital's own handlers serve against a test runner.

- `details-offered.json` and `newest-page-offered.json`: both hold, so
  `sender.promote` is `true`.
- `details-runner-without-send-now.json`: the runner doesn't declare
  `send_promote`. No `promote`.
- `details-grant-without-steer.json`: the runner declares it, but the grant
  doesn't include `steer`. No `promote`.

`sender`'s other members are the runner's grant, unchanged.
