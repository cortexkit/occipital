# phone-reader-say-v1

An agent's `say` calls as the apps get them. An agent given prefrontal's `say`
tool talks to the person through it, and each call's `text` is a message, drawn
as the agent's chat bubble rather than a tool card.

## When a call is a message

Occipital reads the session's plan (`session.plan_view`) and draws a call named
`say` as a message only when the plan gives the agent prefrontal's `say`. The
stored call doesn't name its provider, so the plan decides. Then:

- **Answered without an error:** the call's tool part is replaced, at the same
  place in the row, by a `message` part.
- **Still running:** the call is left out of the row, since no message has been
  sent yet. A row whose only part is that call has no parts until it's answered.
- **Failed or unanswered:** it stays a tool card, so the full view shows why no
  message was sent.

A message isn't a tool call of its row, so it's left out of `toolCalls` and
`blockSummaries`. On a chat whose plan doesn't give the agent prefrontal's
`say`, every call named `say` is an ordinary tool card.

## The `message` part

`{"type": "message", "partIndex", "partKey", "callId", "callKey"?, "text", "level"?, "key"?, "done"?}`

- `text`: the call's whole `text`, never cut like tool arguments. It holds
  Markdown and the reply tags, drawn with the row's `richUi` stamp, as a reply's
  text is.
- `level`, `key`, `done`: as the call wrote them, each left out when it isn't a
  value `say` takes (`level` is `chat`, `info`, `attention` or `urgent`; `key` is
  at most 64 bytes; `done` counts only with a `key`).
- `callId` and `callKey`: the call's identity, so riding notices and the push
  that opens the chat at the message can find it.

The message the call answers, its verified quote and its media join this part
later, from prefrontal's record. This set has none.

## `session.details`

With `messageTool: true`, the details name the tool the chat's agent talks
through, `messageTool: "say"`, and leave the member out otherwise. The apps
offer their Simple view only where it's present.

## Files

The chat: the person asks a question (`m1`); the agent reads a file and
answers by `say` (`m2`, results in `m3` and `m4`); a second `say` fails (`m5`,
`m6`); a third is still being written (`m7`).

- `01-newest-page`: the newest page on a chat whose plan gives the agent
  prefrontal's `say`. `m2` ends with the `message` part, `m5` keeps its tool
  card, and `m7` has no parts.
- `02-exact-read`: the exact read of `m2`, with the same `message` part.
- `03-details`: `session.details` with `messageTool: true`, naming `say`.
- `04-newest-page-without-say`: the same chat on a plan without `say`. Every
  call is a tool card, `m7`'s still running.
- `05-details-without-say`: the details on that plan, with no `messageTool`.

These replies come from Occipital's real handlers, fed these runner entries and
plans. Each `.json` file is the response body, `{"result": …}`, and each
`.request.json` file is the request. `SHA256SUMS` covers every file but this
README.
