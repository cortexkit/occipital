# phone-reader-request-context-v1

What the apps get from `session.request_context {session, harness}`: the system
text and the tools the session's model is given before the conversation, from
the plan in force on the session (runner contract llm-runner 0.8.40,
`session.plan_view`). It is the request as the plan built it, not a copy of one
sent to a provider: a login plugin or gateway may rename tools or add text when
the request goes out, which `asPlanned: true` says. Magic Context's memories,
summaries, user profile and project docs are not in it; Magic Context serves
those itself.

Each `NN-*.request.json` is the app's request and `NN-*.json` the reply bytes it
receives, served through Occipital's real app handlers from runners' recorded
plan views: Thalamus's from real OpenCode 2.2.3 and Claude Code 2.1.318 hosts
with loopback stand-in models and fixture tool providers, and Broca's from its
real serializer on a synthetic session. So the tool names and texts are
fixtures (`bash from aft`), not real ones. A module test proves every file is
what Occipital serves.

## The answer

```json
{"result":{"generation":0,"newestStep":{"atMs":1794460090630},
 "system":{"text":"Frozen OpenCode 2 system"},
 "tools":[{"name":"bash","description":"bash from aft","inputSchema":{"properties":{},"type":"object"},
   "provider":"aft","tokens":{"count":56,"exact":false}}],
 "pending":{"generation":1,"notes":["…","…"]},
 "asPlanned":true}}
```

- `generation`: the plan generation in force.
- `system.text`: the system text exactly as planned. `system.items`, when present,
  names the UTF-8 byte range `{provider, preset?, start, end}` of each provider's
  part (`end` exclusive). Broca serves them (`08`, `09`). Thalamus's runner
  doesn't keep those ranges, so its samples have none: show the text whole.
- `tools`: in the order the model is offered them. `inputSchema` keeps its keys
  sorted, which is the order the model is offered (the runner contract pins it).
  `tokens.exact: false` is an estimate, not billing.
- `newestStep.atMs`: when the newest request under this plan went out, if the
  runner saw one. `replyMessageId`, when present, is the chat page's id of that
  request's reply. Broca serves it; Thalamus's runner doesn't.
- `pending`: a newer plan generation not yet in force. `notes`, when present,
  holds each text the model was told about it, exactly and in the order it got
  them (Thalamus's `02`). Broca doesn't tell the model of a pending plan, so its
  `pending` has the generation only (`09`).
- `appliedAtMs`: when the plan was applied, if the runner recorded it. Broca
  serves it; Thalamus's runner doesn't.

Absent means the runner doesn't know it. Never show a zero or an empty list for
an absent member.

## Refusals

| `code` | Means | What the app does |
|---|---|---|
| `no_plan` | The session has no plan in force yet. | Show that; asking again won't change it until the session starts. |
| `context_not_ready` | The runner can't yet vouch for which provider serves each tool, and serves the view only from the session's next applied plan, which can be hours away. On Thalamus's runner a few sessions from before it recorded that answer this until their plan next changes. | Say the view isn't available for this agent yet. Read again the next time the person opens the view, never on a timer. |
| `unsupported` | The session's runner doesn't serve plan views (Pi today). | Say the view isn't offered for this agent. |
| `too_large` | The plan doesn't fit one reply. | Say so. Don't retry. |
| `provider_unavailable` | The runner can't be reached now. | Retry later, as for any read. |

## The files

| Files | Shows |
|---|---|
| `01-opencode2-in-force` | OpenCode 2 after its first request: generation 0, tools from two providers, schemas and estimates, the newest request's time. |
| `02-opencode2-pending` | The same session after the model was told of generation 1: generation 0 still in force, `pending.notes` with the owner's note and the `bash` removal sentence. |
| `03-opencode2-applied` | The same session once generation 1 applied: changed system text, no `bash`, no `pending`. |
| `04-claude-code-in-force` | Claude Code after its first request: generation 0, three tools from two providers. |
| `05-no-plan` | A session with no plan applied yet: `no_plan`. |
| `06-awaiting-plan-applied` | A session whose view waits for its next applied plan: `context_not_ready`. Thalamus recorded this one from a unit-test host, since a real one can't be put in that state without editing its record. |
| `07-pi-unsupported` | A Pi session: its runner doesn't list the read, so Occipital answers `unsupported` itself and never asks. |
| `08-broca-applied` | A Broca head at generation 1: two system parts with their byte ranges and presets, three tools from two providers, when the plan was applied, and the newest request's reply message id. |
| `09-broca-pending` | The same Broca head after generation 2 was accepted, not yet applied: `pending` names the generation only, with no notes. |
| `10-broca-no-plan` | A Broca session with no plan applied: `no_plan`. |

`SHA256SUMS` lists every file except this README.
