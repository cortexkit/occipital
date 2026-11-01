# phone-reader-live-ids-v1

The live display of a message whose host names it only when storing it, as a Pi
head's runner streams it.

## What the apps see

A runner declaring `live_ids` streams such a message's words under a
runner-minted `live_id` until the host stores it. Occipital shows them with the
`live_id` as `messageID`, and keeps that key for the whole message: words the
runner sends under the stored message's `mid` after storing it still show under
the `live_id`, and the message's `display_end` ends it. An app hands the words
over to the stored copy by matching the words, so it never sees one message
under two keys.

The `live_id` is opaque, never equals any `mid` in the session, and never
appears on the stored page. A message named when it's created, as every OpenCode
and Broca message is, streams under its `mid` exactly as before. Nothing in a
frame says which kind of key it carries, and no frame gains a member.

A watch notice has the same shape as a read reply, so these files show both.

## Where the files come from

These replies come from Occipital's display read at the commit that adds this
folder, fed a runner's own events: text deltas carrying `live_id`, then
`message_started {mid, live_id}`, a delta carrying the `mid`, and
`message_finished {mid}`. Each `.json` file is the response body,
`{"result": …}`, and each `.request.json` file is the request. The test
`display_events::live_ids::the_live_id_samples_are_what_the_reader_serves` in
`occipital-app-reader` serves every request again and fails if a byte changes,
so a reader change that alters them needs a new sample set.

## Cases

| Files | What it shows |
| --- | --- |
| `01-unstored-message-cold` | A cold read while the message streams unstored: its words so far under `messageID` `live-7f3a`. |
| `02-unstored-message-frames` | The same words as frames from `seq` 0. |
| `03-stored-then-ended` | From `02`'s `seq`: the runner stores the message as `m41` and sends one more word under `m41`, which shows under `live-7f3a`; then its `display_end`. |
| `04-next-message-and-run-end` | The run's next message, named at creation, streams under its `mid` `m42`; then the run's `display_end`. |
