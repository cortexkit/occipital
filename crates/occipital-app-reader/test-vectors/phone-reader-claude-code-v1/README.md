# phone-reader-claude-code-v1

Claude Code chats as the apps receive them. Each chat is a real Claude Code
2.1.318 session that Thalamus's runner (`thalamus-harness`, harness
`claude-code`) recorded against a stand-in model, served through Occipital's
own frame loop. The words are the recording's scripted ones. The requests name
harness `claude-code`.

The recording predates runner contract 0.8.33, which lets a runner name the
text a host wrote into a person's message (`host_parts`). For `02` the test
adds that fact as a runner now serves it for Claude Code's interrupt marker:
once with its code and once without, so the apps see both.

`NN-*.request.json` is the request the app sends, and `NN-*.json` is the reply
body it receives. `09` to `11` are the notices one display watch sends. A tool
frame's `asOfMs` is the computer's time when the notice was built; the set
fixes it at 1794208800000 so its bytes don't change from run to run.

| Files | What they show |
| --- | --- |
| `01-page-reasoning` | Claude's thinking is a `reasoning` part. The second answer's is empty (`text: ""`) and still listed, so part positions stay put. |
| `02-page-tool-endings` | How each call ended, once stored: answered; a tool the session doesn't offer (`isError`); cut at the step limit (`unanswered: "not_executed"`, no `schemaPin`); stopped by Stop (`unanswered: "cancelled"`). A message holding only tool results is a `tool` row with no parts: the call carries its output's size (`outputBytes`), and an exact read serves the text. Two person messages begin with Claude Code's `[Request interrupted by user]`: each is an `injected` part with `host: true`, the first with `hostCode: "interrupted"`, the second with none, and neither is in the row's `text`. |
| `03-page-files` | A shared photo (`image/png`) and PDF (`application/pdf`) as `file` parts with type, size and sha256. |
| `04-photo-full` | `session.media` with `size: "full"`: the photo's bytes, base64, under the digest its part names. |
| `05-page-marks` | Core's notice carries `notice: {sender, deliveredAs}`; words core passed on for an agent carry `speaksFor: {agentId}`; core's ordinary messages carry `sentBy: {module}`. |
| `06-notices` | `session.notices` for the call a module's note rode (sender `magic-context`, the context-management module): the note, with `deliveredAs: "tool_result"`. |
| `07-page-phone-message` | The phone's message carries `clientMessageId`, the id it was sent under. |
| `08-session.display_watch.request` | A display watch with `acceptToolFrames: true`, opened while the chat is idle. |
| `09-watch-notice` | The watch's first notice, a `display_gap` baseline. |
| `10-watch-notice` | The reply's words stream, then the call shows `running` and `answered` together: Claude Code reports a call's start only once it has returned, so a long call never shows as running while it runs. |
| `11-watch-notice` | The second reply's words, then two `display_end` frames, as that reply is stored and the run ends. |

Draw a host part as a note under the person's message, written from its
`hostCode`, or not at all; never as the person's words. For a code an app
doesn't know, or none, showing nothing is right.

`SHA256SUMS` covers every file except this README. A test in
`occipital-module` (`broca::tests::claude_code_samples`) serves the set again
and fails if any byte changes.
