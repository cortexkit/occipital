# Phone message idempotency v1

Each JSON file has a descriptive `tag`, an operation request, and the response
returned by the real `session.enqueue_user_message` or
`session.transcript_page` operation handlers in
`phone_message_reuploaded_attachment_retry_deduplicates_by_committed_bytes`.
That test in `crates/prefrontal-core-module/src/phone_message_tests.rs` verifies
the phone user turn in full transcript pages and appended-turn deltas, while
leaving wake-digest turns undecorated. It uses a migrated SQLite store, real
committed attachment uploads, a scripted host landing receipt, and a native Pi
transcript, then compares the serialized records byte-for-byte. The three
`attachment-*.json` records exercise the real enqueue path, including a new upload
ID for a same-byte retry. `SHA256SUMS` covers all seven served JSON records.

- `first-enqueue.json`: durable outbox acceptance, `duplicate: false`.
- `repeat.json`: the original acceptance receipt, `duplicate: true`; no delivery.
- `conflict.json`: named refusal for reusing the identity with changed text.
- `attachment-first-enqueue.json`: acceptance for a message with a committed photo.
- `attachment-repeat.json`: a retry with the same photo bytes under a new upload ID;
  the original receipt is returned with `duplicate: true`.
- `attachment-conflict.json`: named refusal when the identity is reused with
  different committed attachment bytes.
- `transcript-page.json`: only the confirmed phone user turn has `clientMessageId`;
  the earlier user turn, wake-digest user turn, and assistant turn do not.

The effect ID, transcript version, activity token, and generated attachment upload IDs
are opaque values; the producer normalizes them to placeholders, using distinct
attachment placeholders to show the changed ID on retry. No message content, host
message identity, client identity, or refusal is normalized. Production requests do
not supply the `caller*` authority fields; the route derives them from the
authenticated session and paired-phone connection. These records show the
operation's input after route authentication.

`clientMessageId` is optional, 1–64 ASCII letters, digits, hyphens or underscores
(a phone-minted UUID fits). Identity is scoped to the target session. The digest
covers text and every part field; for a committed `attachment_id`, the digest uses
the stored SHA-256 of its bytes in place of that upload ID. Filenames, media types,
and part order remain significant. The accepted phone-message row retains the
attachment-ID-to-SHA map so a retry with that same upload ID can still resolve the
content after attachment adoption removes the upload row. The receipt does not
duplicate message text.
Acceptance and the recoverable effect intent commit in one SQLite transaction.
Host completion is asynchronous: a retry returns the acceptance receipt even if
the host is unavailable or delivery later fails. Entries are retained for at least
48 hours and pruned by the core runtime's periodic retention pass. Reusing an ID
after retention expires is not guaranteed to deduplicate.

Transcript decoration requires a confirmed host message ID. Harness history
landing receipts and history-backed effect results provide it; Pi additionally
reports the native journal entry in an optional `messageId` acknowledgement.
The optional acknowledgement field is sent by the updated Pi plugin. Running
Pi sessions begin reporting it only after the host process next restarts and
loads that plugin.
Normal legacy OpenCode `promptAsync` success returns no host ID
and does not emit the harness callback, so those turns stay undecorated unless
history-based recovery confirms an ID. No synthetic IDs or text matching are
used for decoration.

Regenerate intentionally with `UPDATE_PHONE_MESSAGE_VECTORS=1` and the filtered
Rust module test, then review the JSON and hash manifest together. Normal test
runs never rewrite fixtures.
