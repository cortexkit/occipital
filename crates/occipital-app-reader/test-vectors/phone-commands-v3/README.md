# The person's commands through Occipital (phone-commands-v3)

What the apps get from Occipital's `user-commands/v1` relay against the live
AFT module (the `ck-aft` build of commit `f0ecf7f9`) and Magic Context module
(the `ck-mc` build of commit `292c6a419c`), served through the Occipital
build of commit `877fb4d`, recorded on 13 November 2026. The apps build their
command palette from these bytes.

`phone-commands-v1` and `phone-commands-v2` stay frozen and still hold the
`flush` and `status` steps of Magic Context, the session details, and the
refusals. This set adds what came after them:

- AFT's `status` command, which now answers on an OpenCode 2 head and on a
  Broca head (a head is an agent a person works with; its chat is a session
  on that runner), and `not_bound` for a session AFT doesn't serve.
- Magic Context's `wrapup` command (summarize the history, keeping the newest
  messages raw), and `not_managed` for a session Magic Context doesn't manage.
  No `wrapup` run is recorded: a run would summarize the real history of the
  OpenCode 2 head these previews were taken on.

Each step is a request file (`*.request.json`, the `{method, params}` the app
sent) and the reply (`*.json`): `{"result": …}` for an answer, or the error
body `{code, message, detail?}` for a refusal. Nothing was edited after
recording.

Recorded with the `compare_readers` example in `crates/occipital-module`,
whose `COMPARE_RAW=1` prints each reply as one line of compact JSON, as a
plain local caller, which Occipital admits as the desktop's development
build. The modules received `caller {app: "desktop", admission:
"development"}` with no device.

| Files | Step |
| --- | --- |
| `01-list-opencode2` | `commands.list` for an OpenCode 2 head: AFT's `status`, then Magic Context's `flush`, `status` and `wrapup`. |
| `02-list-broca` | The same for a Broca head: the same four commands. |
| `03-preview-aft-status-opencode2` | AFT `status` preview on the OpenCode 2 head: `available: true`, facts for text search, code map, errors, warnings and running shells, and a table of each index's state and file count. |
| `04-preview-aft-status-broca` | The same on the Broca head. |
| `05-preview-aft-status-not-bound` | AFT `status` for a session AFT serves no project for: `available: false`, reason `not_bound`. |
| `06-preview-wrapup-opencode2` | Magic Context `wrapup` preview with `messages_to_keep: 20`: `available: true`, its effect, and facts for the messages to summarize, the raw history's size, the messages that stay raw and the summarizer's state. |
| `07-preview-wrapup-not-managed` | `wrapup` for a session Magic Context doesn't manage: `available: false`, reason `not_managed`. |

## Verify

```sh
(cd crates/occipital-app-reader/test-vectors/phone-commands-v3 && shasum -a 256 -c SHA256SUMS)
```
