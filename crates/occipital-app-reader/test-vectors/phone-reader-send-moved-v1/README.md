# phone-reader-send-moved-v1

What the apps receive when the person sends a message to a chat whose agent is
moving, or has moved, to another computer. Each reply is the error frame
Occipital sends for `session.enqueue_user_message`, `{code, message, detail?}`.
Nothing was sent to the agent: the app keeps the words as a draft and offers
them again once it knows where the agent lives. It never resends on its own.

| Files | What it shows |
| --- | --- |
| `01-send-moving` | The agent is moving now. Occipital holds every send to its chats from the move's start, so none lands after the cut: `session_moving`. Sending again after the move ends reaches the agent wherever it lives. |
| `02-send-moved-here` | The move finished, and Occipital on this computer let the chat go: `session_held_elsewhere`, with the new computer's box id as `detail.holder`. |
| `03-send-runner-moved` | The runner on this computer refuses the send because the chat moved (llm-runner 0.8.63's own sample), served as the same `session_held_elsewhere` with `holder`. |
| `04-send-runner-moved-no-holder` | The same, when the runner doesn't know the new computer: no `detail`. |

Both codes are final for this computer: the same send here can't succeed. The
app looks up the agent's computer again before it offers the words.

`<name>.request.json` is the request's `{method, params}`; `<name>.json` is the
reply. The test `send_moved_samples::the_send_moved_samples_are_what_the_reader_serves`
in `occipital-module` serves them again and fails if a reply's bytes change.
