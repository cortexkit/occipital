# phone-reader-stop-held-v1

Stopping a head's run that waits on a held tool call, with `session.stop`. Recorded on
2026-11-11 at 23:08Z through Occipital's real handlers (`ck-occipital` `b979040`), on a
Broca head (harness `broca`, session `ses_126dd595aa34447d9216fba52636420d`). Its run had
held a `bash` call since about 20:25Z: the command's result was lost in a daemon restart,
so the run would never have continued by itself.

The files are as Occipital answered them, re-encoded compact with sorted keys. They hold
no chat text and no user name.

## Cases

| Files | What it shows |
|---|---|
| `60-held-details` | `session.details` before: `lastRun` is `{state: "paused", reason: "tool_held", runId}`, and the grant has `interrupt: true`. |
| `61-held-stop` | `session.stop` naming that run: `{state: "stopped", runId}`. |
| `62-held-details-after` | Details 8 seconds later: `lastRun` is a new run, `state: "active"`. The messages queued behind the held call started the next turn on the same session. |

## Reading it

- A run that waits on a held tool call reads `paused` with reason `tool_held`. Stop ends
  it like a running one: offer Stop for it too, under the same `interrupt: true` grant.
  Other paused reasons (an account limit, a restart pause) resume by themselves.
- After the stop, read `lastRun` again: a new run may already be going, started by
  messages that were waiting.
- `phone-reader-stop-v1` has the rest of Stop: repeats, an unknown run, and a running
  turn's partial reply.

`SHA256SUMS` lists the files as published here.
