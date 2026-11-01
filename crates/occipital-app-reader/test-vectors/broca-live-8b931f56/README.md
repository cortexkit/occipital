# Broca typed live subscription frames

Copied byte-for-byte from Broca master `8b931f56`,
`crates/broca-wire/tests/golden/session_subscribe.live_*.jsonl`, for the Broca
0.3.253 compatibility check. Each line is one real `session.subscribe` frame; `@RUN_ID@` is Broca's
golden-sample placeholder, not a Thalamus substitution.

`SHA256SUMS` pins the original bytes. The harness-module's `broca::tests::live_frames`
tests replay these lines through the production subscription decoder, source,
display hub and app watch handlers with only the Broca transport replaced.

The samples cover text, tools, reasoning, steering and attach replay. On attach,
Broca sends a reset followed by one delta containing all text accumulated so far
for each part of the in-flight message.

Live text and reasoning use the runner's explicit `mid`, even before the durable
message appears in a transcript read. `part` is not yet used by the app display
protocol. Tool argument deltas are not shown as text. These placement limitations
are independent of stream decoding and durable transcript refresh.
