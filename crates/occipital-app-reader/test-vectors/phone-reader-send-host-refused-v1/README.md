# phone-reader-send-host-refused-v1

What the apps receive for a message the agent's host refused for good (runner
contract 0.8.58, `not_delivered` with reason `host_refused`). The host can't take
the message as sent: sending the same words and files again would be refused
the same way. Show it as **couldn't be sent**, say why from `cause`, and offer the
words back in the draft so the person can change something first. Never resend
it automatically, and don't offer a plain Try again.

Each reply was served through Occipital's real request handling, so it is byte
for byte what a phone receives. The runner's answers are the runner contract's
own wire vectors (`send.json`, `send-status.json`, `run-result.json`). Each
`NN-*.request.json` is the request the app sends, and `NN-*.json` beside it is the
reply.

## 01: a send refused at once

`session.enqueue_user_message` for a send the runner answers `host_refused`
straight away, as it does for a repeat of a send that already ended so:

```json
{"result":{"code":"not_delivered","effectId":"app-…","ok":false,"reason":"host_refused","terminal":true}}
```

`terminal: true` means retrying under the same `clientMessageId` can't succeed.

## 02: a held send the host then refused

The runner first held the send (`pending`), then the host refused it because it
can't read the attached file. The chat's newest page lists it in `queuedSends`:

```json
{"acceptedAtMs":1794850000000,"cause":"attachment_unreadable","clientMessageId":"c-unsent",
 "effectId":"app-…","endedAtMs":1794850000000,"reason":"host_refused",
 "runnerCode":"provider_error","state":"unsent"}
```

- `cause` is new. It appears only with reason `host_refused`, and only with one
  of the runner contract's four named values:
  - `attachment_unreadable`: the host can't read a file the message carries.
  - `host_configuration_invalid`: the agent's host is set up wrongly.
  - `host_version_unsupported`: the agent's host is a version that isn't supported.
  - `host_policy_refused`: an administrator's policy forbids the request. Its
    `runnerCode` is `host_policy_refused` too.
  A cause the contract doesn't name is left out, so word `host_refused` without
  one as a plain "couldn't be sent".
- `runnerCode` is the code the send's run ended with: `provider_error`, or
  `host_policy_refused` for a policy refusal. Neither is a wait, so there is no
  `retryAfterMs`.

`SHA256SUMS` covers every file except this README. A test in `occipital-module`
(`broca::tests::host_refused_samples`) serves the set again and fails if any
byte changes.
