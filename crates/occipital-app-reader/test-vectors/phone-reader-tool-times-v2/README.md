# phone-reader-tool-times-v2

When a tool call started and when it was answered, on the chat's tool part.

A `type: "tool"` part may carry three members. Each is left out when unknown, never
zero and never filled in from another clock:

- `startedAtMs`: when the call began running, in Unix milliseconds.
- `finishedAtMs`: when its answer was recorded, in Unix milliseconds. In a parallel
  batch, each call has its own time.
- `unanswered`: why the call ended without an answer: `interrupted`, `cancelled`,
  `denied` or `not_executed`. Decode it as an open string and show an unknown value
  as sent. No case here carries it.

A part without them keeps its earlier bytes, so older sample sets still hold. Only the
`type: "tool"` part carries them, not the message's `toolCalls` summary.

## Where the files come from

This set replaces `phone-reader-tool-times-v1`, which was served from an OpenCode 1
chat. Thalamus's runner now hosts OpenCode 2 chats (harness `opencode2`), so this
set is served from an OpenCode 2 chat. The members and their meaning are the same;
the chat's own bytes differ: message ordinals count from 1, and a part's `partKey` is
the runner's key for its slot (`"0"`, `"1"`, …) rather than an OpenCode 1 part id.

Occipital's app reader served these replies at the commit that adds this folder.
The session is a test session on Thalamus's runner, which serves each call's
`started_at` and `finished_at` beside the read entry. A live session wasn't used, so
that the times are fixed. Each `.json` file is the whole response body,
`{"result": …}`, and each `.request.json` file is the request's `{method, params}`.
The test `app_reader::broca::tests::tool_times_samples` in `occipital-module` serves
every request again and fails if a reply's bytes change, so a reader change that
alters them needs a new sample set, never an edit to this one.

Broca chats carry the same members, from the same read facts.

## Cases

| Files | What it shows |
| --- | --- |
| `01-opencode2-newest-page` | A turn with three calls. `bash` `ls` ran for 250 ms and `read` failed after 40 ms, so both carry the two times. A `bash` `date` served without times carries neither. |
| `02-opencode2-exact-read` | The exact read of the same answer (`messageOrdinal: 2`): the same times beside each call's whole output. |
