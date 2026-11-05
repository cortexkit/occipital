# phone-reader-notices-v1

How the notices that rode a chat's tool calls reach the apps. A notice is a
module's message to the agent, such as Magic Context's reminders or core's
status line. A runner can deliver one riding the next tool result instead of as
its own message. Such a notice makes no transcript entry and usually lands after
its call's page was read, so the apps read it beside the page, never on it:
`session.notices` for the calls they show, and `session.notices_watch` to learn
when one arrives.

The set is served through Occipital's own frame loop on a test OpenCode 2
session on Thalamus's runner. The session holds one answer with three `bash`
calls, and the runner's read lists three notices on it in its `notices` display
fact (llm-runner 0.8.11). A fourth notice arrives on the runner's live stream as
`notice_delivered` after the page was read. The text is invented.

`NN-*.request.json` is the request the app sends, and `NN-*.json` is the reply body
it receives.

| Files | What they show |
| --- | --- |
| `01-page` | The newest page. Each tool part carries its `callKey`, and no notice is on the page. |
| `02-notices` | The notices of all three calls. `ck_one` has one notice. `ck_two` has two from different modules, in delivery order, one of them from a module the apps have no name for (`example-notifier`). `ck_three`, which no notice rode yet, is left out. |
| `03-notices-watch-open` | The watch's first notice, `{callKeys: []}`: it is listening, so a read after it misses nothing. |
| `04-notices-watch-hint` | The watch naming `ck_three` once a notice rode it. Hints carry no notice text. |
| `05-notices-after-hint` | The read of `ck_three` after the hint, with the notice that arrived. |

Each notice is `{sender, text, deliveredAs}`. `sender` is the sending module's id,
as the daemon attested it; show an id you have no name for as it is.
`deliveredAs` is how the runner delivered it, `tool_result` for every notice
here. A call's notices can grow after you read them, so read a call again when
the watch names it.

`SHA256SUMS` covers every file except this README. A test in `occipital-module`
(`broca::tests::notices_samples`) serves the set again and fails if any byte
changes.
