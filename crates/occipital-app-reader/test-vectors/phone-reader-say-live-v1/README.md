# phone-reader-say-live-v1

A `say` message shown as the model writes it, on the chat of
`phone-reader-say-v1`, whose plan gives the agent prefrontal's `say`.

## Asking for it

`session.display_read` and `session.display_watch` take
`acceptMessageFrames: true`. A reader that doesn't ask gets exactly what it got
before: the `say` is a tool call to a reader of tool frames, and its text
arrives only with the stored row. The frames below come in the watch's notices
and in reads, numbered by `seq` with the other frames. A reader that asks for
message frames but not tool frames finds the tool frames' numbers skipped.

## The frames

- **Text:** a `display` frame with `partType: "message"`:
  `{kind: "display", sessionID, messageID, partType: "message", partKey?, callId, callKey?, chunk, richUi?}`.
  `chunk` is newly written text of the message: append it to the live message
  with that `callId`, and draw its tags under `richUi`, as for reply text. The
  text is decoded from the call's arguments, so escapes are already resolved.
  `callKey` is known only once the call went to its tool, so it's usually
  absent.
- **Frozen:** `{kind: "message", callId, frozen: true}`. Chunks were lost, so
  the text shown is a correct start of the message that won't grow. Keep
  showing it.
- **Withdrawn:** `{kind: "message", callId, withdrawn: true}`. No message was
  sent from this text: the call failed or got no answer, its run ended first,
  its arguments started again, or the live view was reset. Drop the live text.
  If its arguments start again, text frames for the same `callId` follow.
- **Baseline:** a read without `sinceSeq`, and a watch's first notice, carry
  `messages: [{messageID, partKey?, callId, callKey?, text, richUi?, frozen?}]`,
  each message being written with its text so far.

A `display_gap` frame drops the live reply words and running calls as before;
it doesn't touch a live message, which follows only the frames above. An
answered call sends nothing more: the stored `message` part with the same
`callId` replaces the live text, as a stored reply replaces its live words. A
`say` sends no tool frame to these readers.

## Files

- `01-session.display_watch.request`: the watch, with `acceptToolFrames` and
  `acceptMessageFrames`, opened while the chat is idle.
- `02-watch-notice`: its first notice, an empty baseline.
- `03-watch-notice`: the first message as written, in two text frames: its
  arguments were split inside the `text` key and inside a `\u00e9` escape. The
  call then went to its tool and was answered, and another call's arguments
  streamed; none of that sends a frame here.
- `04-watch-notice`: a second message begins: `Checking the`.
- `05-session.display_read-baseline`: a read without `sinceSeq` at that point,
  carrying the second message in `messages`.
- `06-watch-notice`: chunks are lost (`display_gap`), so the second message is
  frozen; a chunk after the gap shows nothing. A reset (`display_gap` again)
  withdraws it, and the run ends (`display_end`).
- `07-session.display_read-tools-only`: the same run read with
  `acceptToolFrames` alone: the first `say` is a tool call, running then
  answered, and no message text appears. Its `asOfMs` is fixed, since the
  computer stamps it from its clock.
