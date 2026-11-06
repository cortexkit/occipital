# llm-runner 0.8.11 serve-path samples

The `before/` JSON files are unmodified frame bodies captured by
`contract_samples::upgrade_wire_samples` through the real in-process SUBC serve connection,
on the original 0.8.7/0.17.9/0.1.2 dependencies and original serving implementation.
The fixture uses only synthetic text and a synthetic reserved module principal.

The corresponding `after/` capture is from the final writer-enabled implementation.
The `before-denied/` and `after-denied/` directories pair the denied-call read and live
frames separately. There are 28 before bodies and 29 after bodies; the extra body is
the new notice-delivered stream event. `SHA256SUMS` covers all 57 unmodified JSON bodies.
Dynamic run IDs, WAL incarnation IDs, timestamps, history versions and frame digests differ
between independent fixture runs. No values inside the captured bodies are normalized.
`session.history.raw.json` is the default raw form; the display capture opts into `form: display`.
`session.read.default.json` omits display facts; `session.read.facts.json` requests them.
The numbered subscribe files are captured stream data frame bodies in arrival order.

Expected additive differences:

- Discovery declares `notices` with its already-declared prerequisites.
- Read/head/subscribe cursors include `incarnation` and a 16-hex-character source-frame digest.
- Opted-in read entries and raw history entries report positive module sender evidence.
- Opted-in read entries and display rows carry notice facts on the tool-result message.
- Subscribe adds `notice_delivered`, carrying the runner-minted call key and exact notice text.
- A denied native call gains its terminal-record call key on opted-in reads and live frames.

Legacy position-only wire cursors are refused `cursor_expired`, as approved by the parent.
Malformed cursors and legacy position-only cursors refuse subscribe immediately; a well-formed bound cursor that fails catch-up verification closes the stream with `cursor_expired` before any event.
The compatibility path for legacy materialized snapshots may still mint a position-only head when its frame lies before the already-read WAL suffix; that cursor also expires on subscribe, without a prefix scan.

No compaction Setup, notice, ride, entries-page or view golden is regenerated for this upgrade.
The unchanged v1 runner alias stays pinned to 0.7.48. Optional compaction token counts remain
unset; no 24-hour post-cut ride-pruning behavior is adopted.

Release ordering: the first commit pins the historical `send_delivered` bytes. The next
implementation commit knows `rides-v2/v1` and renders its per-ride envelope, but keeps its
writer switch false. The separate tip commit enables writing that envelope. Ship the reader
commit one release before the tip. The new envelope freezes `form: append`; old records do
not acquire envelope metadata or a different multipart renderer.
