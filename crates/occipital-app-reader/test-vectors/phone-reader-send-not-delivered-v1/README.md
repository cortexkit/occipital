# phone-reader-send-not-delivered-v1

What the apps receive for a message the runner proved never reached the agent's
host: no part of it ran, and it never will (runner contract 0.8.54,
`not_delivered`). The words may go again, but only as a **new message under a new
`clientMessageId`**: the old send is finished for good, and sent again under its
old id it is refused the same way.

Each reply was served through Occipital's real request handling, so it is byte
for byte what a phone receives. The runner's answers are Agent Runner's recorded
bodies (`occipital-module/tests/data/thalamus-send-not-delivered-1116/`). Each
`NN-*.request.json` is the request the app sends, and `NN-*.json` beside it is the
reply.

## 01: a send refused at once

`session.enqueue_user_message` for a send the runner answers `not_delivered`
straight away, as it does for a repeat of a send that already ended so:

```json
{"result":{"code":"not_delivered","effectId":"app-…","ok":false,"reason":"handover_failed","terminal":true}}
```

`terminal: true` means retrying under the same `clientMessageId` can't succeed.
Try again sends the words as a new message.

## 02: a held send that never reached the host

The runner first held the send (`pending`), then proved it never reached the
host. The chat's newest page lists it in `queuedSends`:

```json
{"acceptedAtMs":1794810000000,"clientMessageId":"c-unsent","effectId":"app-…",
 "endedAtMs":1794810000000,"reason":"handover_failed","runnerCode":"provider_error",
 "state":"unsent"}
```

- `reason` is the runner's. `handover_failed` means the runner couldn't hand the
  message to the agent's host, after trying twice. It is an open string: keep a
  value you don't know (the runner contract also names `withdrawn`).
- `runnerCode` is the code the send's run ended with. `provider_error` is the
  runner contract's catch-all; it is not a wait, so there is no `retryAfterMs`.
- Try again sends the words as a new message, under a new `clientMessageId`.

`SHA256SUMS` covers every file except this README. A test in `occipital-module`
(`broca::tests::not_delivered_samples`) serves the set again and fails if any
byte changes.
