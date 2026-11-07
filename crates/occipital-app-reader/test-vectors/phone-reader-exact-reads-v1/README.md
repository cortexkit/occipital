# phone-reader-exact-reads-v1

Exact reads, `session.transcript_page {session, harness?, messageOrdinal}`,
which serve one message with its tool calls whole, for a step log. Served
through Occipital's real page path by
`occipital-module/src/app_reader/broca/tests/exact_read_samples.rs`, from three
chats: an OpenCode 2 head (`ses_head_oc2`, harness `opencode2`), a Pi head
(`ses_head_pi`, harness `pi`) and a Broca worker (`alfonso:worker`, no
harness). Each `NN-name.json` is the reply the app receives for
`NN-name.request.json`. `SHA256SUMS` lists every file but this README.

| File | Shows |
| --- | --- |
| `01-opencode2-long-output` | a `bash` call with 94,000 bytes of output (2,000 lines of 47 bytes) |
| `02-opencode2-error` | a call that failed |
| `03-pi-long-output` | the same long call on Pi, whose result is stored as its own message |
| `04-pi-error` | a failed call on Pi |
| `05-broca-long-output` | the same long call on Broca |
| `06-broca-error` | a failed call on Broca |
| `07-broca-no-output-yet` | a call whose result the runner hasn't recorded yet |

- **A full read** (01 to 06) has `fullContent: true`. Each call is a `tool`
  part with `arguments` as an object, `content` (the result's text) and
  `isError`.
- **Nothing is cut.** Occipital serves `content` whole, at any size. The one
  limit is the transport's 4 MiB frame. A message the runner can't serve whole
  comes back as in 07, with the runner's reason. In the rare case the runner's
  message fits but Occipital's reply doesn't, the read is refused
  `frame_too_large`.
- **A result not recorded yet** (07): the read isn't full. It has no
  `fullContent`, carries `fullContentUnavailable` with the reason, and holds the
  message's page row, whose call has no `content` or `outputBytes`. On OpenCode
  2 this case doesn't arise: a message is served only once it has finished.

**Text:** the chat text is invented.
