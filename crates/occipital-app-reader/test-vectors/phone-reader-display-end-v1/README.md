# phone-reader-display-end-v1

The phone's live display of one Canary Head run on an OpenCode head, recorded by the
phone app on 2026-10-31 at about 07:54Z, with `session.display_watch` open from before
the send. The run wrote some text, ran `date -u`, then wrote a closing sentence, so it
has two assistant messages with a tool call between them.

| Files | What they show |
| --- | --- |
| `00-warmup-details`, `01-newest-page-before` | The session before the send. |
| `02-display-watch.request`, `.open`, `.end` | The watch request and when it opened and ended. |
| `02-display-watch.notice-1` | `display_gap`: the watch attached with nothing live yet. |
| `02-display-watch.notice-2` to `-5` | The first message's reasoning, then its text in three pieces. |
| `02-display-watch.notice-6` | `display_end` alone: the first message was stored, so its live words are final. |
| `02-display-watch.notice-7` | One notice holding the second message's text and its own `display_end`. |
| `02-display-watch.notice-8` | `display_end` alone: the run finished. |
| `03-send`, `04-details-0` to `-5` | The send and the session's details while the run ran. |
| `05-newest-page-after`, `06-details-after` | The stored transcript and details after the run. |

Two kinds of `display_end` appear. A message's end is sent only while that message's
words are the live ones, and the run's end once, when the run finishes or pauses. Both
come from one ordered stream: a run's end always precedes the next run's first words,
and a message's end is never sent once a later message's words have started. So two
ends in a row, as in notices 7 and 8, mean "these words are final" twice, never
"end the next message".

Each `.request.json` is what the app sent, and each `.timing.txt` records when. No
file needed redaction. `SHA256SUMS` covers the files as published here.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
