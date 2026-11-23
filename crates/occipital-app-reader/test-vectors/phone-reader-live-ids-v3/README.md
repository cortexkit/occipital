# phone-reader-live-ids-v3

A Pi agent's steps with tool calls, as an app's display watch and page reads
see them. It extends `phone-reader-live-ids-v1`, which has no calls, and
replaces `phone-reader-live-ids-v2`. In v2 the stored pages lacked the call
facts a live Pi page carries (`callKey`, `schemaPin`, `startedAtMs`,
`finishedAtMs`, `input`), and the answered frames' `outputBytes` didn't match
the stored results. Both sets stay frozen.

## What the apps see

Pi's runner streams a step's words under a runner-minted `live_id` until its
host stores the step, then names the step by its `mid`. The step's calls run
only after it's stored, so the runner's events about them name the `mid`.

Occipital shows the step under one key. The words keep the `live_id` to their
`display_end`, as in v1. Every later frame about that `mid` in the same run
also carries the `live_id` as `messageID`: a call's running and answered
frames, the running calls of a baseline read, and a `say` message's frames.
An app holds one reply with the words and the call until the stored copy
replaces it. It matches the words to the stored copy by their text, as in v1,
and each call to its stored tool part by `callId` or `callKey`. `callKey`
always names the stored `mid`.

A stored tool part carries the same `callKey` as its frames, and its result's
size equals the answered frame's `outputBytes`. A call recorded before its
runner served call keys has none, so an app joins by `callKey` when both
sides carry one, and otherwise by `callId`.

A step whose words Occipital didn't show live keeps its `mid` on its calls.
That covers a call alone, whose arguments stream but show no words. It also
covers a step Occipital began following only after its words ended, such as
after a restart, and the frames after a `display_reset` that drops the words.

No frame gains a member. A watch notice has the same shape as a read reply.

## Where the files come from

These replies come from Occipital's real handlers on a Pi chat. A fake runner
declares `live_events` and `live_ids` and feeds Pi-shaped events and stored
entries, with the display facts a live Pi runner serves for each call. The `.request.json` files are the requests. The test
`app_reader::broca::tests::pi_live_steps_samples::the_pi_live_step_samples_are_what_the_reader_serves`
in `occipital-module` serves every request again and fails if a byte changes.
Tool frames carry a fixed `asOfMs`, so the bytes don't change from run to run.

## Cases

| Files | What it shows |
| --- | --- |
| `01-session.display_watch.request.json` | The watch: `acceptToolFrames: true`. |
| `02-watch-notice` | The watch opens on the idle chat. |
| `03-watch-notice` | The first step: reasoning under `live-step-1`, its `display_end`, then the call `call_read` running, also under `live-step-1`. Its `callKey` names the stored `a1`. |
| `04-page-step-stored-call-running` | The newest page while the call runs: the stored step `a1` holds the reasoning and the call. |
| `05-session.display_read-baseline-call-running` | A read without a cursor while the call runs: its running call under `live-step-1`. |
| `06-watch-notice` | The call is answered, still under `live-step-1`. |
| `07-page-step-call-answered` | The newest page once the call's result is stored. |
| `08-watch-notice` | The second step: text under `live-step-2`, and its call running and answered under `live-step-2`. The third step: a call alone, whose frames keep its `mid` `a5`. Then the run's `display_end`. |
| `09-page-run-ended` | The newest page after the run: all three steps and their results. |
