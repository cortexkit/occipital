# broca send state notices (`event.broca.send_state.v1.{sender}`)

Each file is one notice exactly as broca's publisher sends it: the subject, the bus message id
(`Nats-Msg-Id`), the headers, and the body's exact bytes (the `body` string is the UTF-8 of the
published body). They were written by broca's real serializer and read back through its outbox.

When a notice is published: each time a keyed send (one with a `send_id`) that a module sent
changes state, once the record of the change is durable in broca's session store. The subject
ends with the sending module's id, so `m_aft_sendstate` reads AFT's sends and `m_occipital`
Occipital's.

The body is a notice only: `{v, session_ref, send_id, seq, state}`.

- `send_id` is your own key for the send. Re-read `session.send_status` for it; never take a
  state from the body.
- `seq` is the position of the record that changed the send in the session's log. It increases
  within a session, so one send's notices arrive in order.
- `state` is a hint for logs: `pending`, `active`, `paused`, `finished`, `superseded`,
  `expired`, `retracted`, or `not_delivered`.

One change from the shape agreed in #fleet-notices (#10304): the message id is
`{session_ref}:{send_id}:{seq}`, not `{send_id}:{seq}`. A caller's send ids are unique only
within a session, and the stream's duplicate window spans every session, so two sessions using
the same send id at the same log position would otherwise drop one notice as a duplicate.
Nobody needs to parse the id.

Files:

- `01`–`04`: AFT's notices (`.aft`): queued, delivered, replaced by a newer send with the same
  dedupe key, expired.
- `05`–`08`: Occipital's (`.occipital`): queued, started its own run, the run paused, the run
  ended.

Please answer pass or fail: your decoder accepts every file, and your reader re-reads
`session.send_status` for the `send_id` it names.
