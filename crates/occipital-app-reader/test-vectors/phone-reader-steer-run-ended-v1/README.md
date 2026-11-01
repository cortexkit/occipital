# phone-reader-steer-run-ended-v1

A phone message to a Broca worker whose run is cancelled before the message reaches
it. Recorded by the phone app on 2026-10-31 at about 05:10Z against Broca 0.3.292,
the first Broca release that lets a message's sender withdraw it.

The worker (`alfonso:bg_8e6c01bdc64221c2`) is running a foreground `sleep 300`, so a
message sent to it waits. Core then cancels the worker. The message is tied to the
run it was sent into: when that run ends, Thalamus withdraws the message, and the
phone shows it as not delivered with reason `run_ended`, instead of waiting for the
30-minute deadline.

| Files | What they show |
| --- | --- |
| `80-warmup-details` | The worker's run is `active`, nothing queued. |
| `81-send` | The send is accepted (`ok: true`), `clientMessageId` `a6b02c3c-…`. |
| `82-details-queued`, `83-newest-page-queued` | The run is still `active`, and the message is listed in `queuedSends` as `queued`. |
| `84-details-unsent`, `85-newest-page-unsent` | After the cancel: the run is `interrupted`, and the message is `unsent` with `reason: "run_ended"` and `endedAtMs`. |

Each `.request.json` is the request the app sent, and each `.timing.txt` records when.
The two page files had the computer's user name in a worktree path, replaced by the
same-length placeholder `user-redact`. `SHA256SUMS` covers the files as published here.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
