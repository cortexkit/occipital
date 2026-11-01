# Where these files come from

Every other file in this directory was copied byte for byte from the Prefrontal
repository, `test-vectors/phone-message-idempotency-v1/` at commit
`6b36afc1456483b0f94ea87769526ac92276fdc6`, when Prefrontal core still served
`session.enqueue_user_message` to the phone and desktop apps. `README.md` is core's own
description of them, unchanged. Where it calls wake-digest turns undecorated, it means
the user turns core writes itself to wake an agent with a digest of its notices: they
come from no app, so they carry no client message id. `SHA256SUMS` is core's manifest; the tests in
`crates/thalamus-app-reader/src/send/tests.rs` check each JSON file against it.

Thalamus now serves the same operation, in
`crates/thalamus-app-reader/src/send.rs` and
`crates/thalamus-harness-module/src/app_reader/send.rs`. The text records (`first-enqueue.json`,
`repeat.json`, `conflict.json`) are replayed through Thalamus's send and must
give the same receipts and refusal, apart from the opaque `effectId`. The three
`attachment-*.json` records are refused by name (`unsupported`) until uploads
reach Thalamus. `transcript-page.json` is kept for the record: it shows which
transcript row carries the app's `clientMessageId`, which the runner now stores.
