# phone-reader-tool-frames-v1

Tool calls on the live display lane: a running row for each call while it runs,
and its end when it's answered or not.

## Opting in

`session.display_read` and `session.display_watch` take `acceptToolFrames: true`.
Only a reader that sends it gets tool frames. Without it, every reply and notice is
byte for byte what it was before tool frames existed, including its `seq` numbers,
because tool frames live in a ring of their own. `04` shows the same run read
without the flag.

A watch notice has the same shape as a read reply, so these files show both.

## The tool frame

`{"kind": "tool", …}`, with the members of the stored tool part of the same name:

- `sessionID`, and `messageID`: the runner's id for the call's message, as text
  frames carry it.
- `callId`: byte-identical to the stored part's `callId`, so a frame can be matched
  to the stored part that replaces it.
- `tool`, and `phase`: `running`, `answered` or `unanswered`.
- `callKey`, `schemaPin`, `input`, `inputTruncatedFields`: present only when the
  runner serves the call's identity. `input` is cut by the same rule as the stored
  part's, so a running row and its stored row get the same input.
- `startedAtMs`, and on an end `finishedAtMs`: each present only when the runner
  recorded the time.
- On an end: `outputBytes`, `isError: true` for an answer that is an error, and
  `unanswered` with the reason a call got no answer (`cancelled`, `interrupted`,
  `denied`, or a newer runner's value, shown as sent).
- `asOfMs`: the computer's time when the reply or notice was built. A running
  call's elapsed time is `asOfMs - startedAtMs`, plus the time since the app
  received it by its own clock, so the app never compares its clock with the
  computer's.

## Rules

- **Order:** text and calls keep the runner's order. Words held for batching are
  sent before the call that follows them.
- **Baseline:** a read without `sinceSeq`, and a watch's first notice, carry a
  running frame for each call still in flight in `tools`, oldest first. When no
  text has streamed, `frame` is `display_gap`.
- **Gap:** a `display_gap` loses text, never calls. Each call still running is sent
  again right after it.
- **Every running call ends:** when a run ends with a call still running, that call
  gets an `unanswered` (`interrupted`) end before `display_end`.
- **Not known:** a call that was already running when the computer began following
  the session has no live row; the stored page shows its phase. After the runner
  resets its live state, running calls are forgotten the same way.
- **No identity:** a runner that doesn't serve call identity sends frames without
  `callKey`, `schemaPin` and `input`, and the call draws as the plain chip.

## Where the files come from

These replies come from Occipital's display read at the commit that adds this
folder, fed the runner's own `tool_started`, `tool_finished`, text and run events,
at a fixed time (`asOfMs` 1793500100000). Each `.json` file is the response body,
`{"result": …}`, and each `.request.json` file is the request. The test
`display_events::tool_frames::the_tool_frame_samples_are_what_the_reader_serves`
in `occipital-app-reader` serves every request again and fails if a byte changes,
so a reader change that alters them needs a new sample set.

## Cases

| Files | What it shows |
| --- | --- |
| `01-baseline-mid-call` | A baseline while `bash ls -la` runs: the text so far in `frame`, the call in `tools`. |
| `02-calls-answered-and-a-gap` | From `01`'s `seq`: `ls` answered with `outputBytes`; a `read` whose long path is cut (`inputTruncatedFields: ["/path"]`) runs and answers with an error; a call without identity starts; a gap, then that call again. |
| `03-run-end-ends-running-call` | The run is cancelled: the call still running ends `unanswered` (`interrupted`), then `display_end`. |
| `04-same-run-without-tool-frames` | The whole run read without `acceptToolFrames`: text, the gap and the end, with no tool frames. |
