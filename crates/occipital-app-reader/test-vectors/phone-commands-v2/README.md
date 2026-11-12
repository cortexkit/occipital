# The person's commands through Occipital (phone-commands-v2)

What the apps get from Occipital's `user-commands/v1` relay against the live
Magic Context module (the `ck-mc` build of commit `501726199e`, served
through the Occipital build of commit `4199868`), recorded on
13 November 2026. The apps build their command palette from these bytes.

This set replaces `phone-commands-v1`, which stays frozen. Magic Context
changed three things since then:

- A flush preview's `version` covers only what the person confirms, not the
  token figures, so a new turn on a working head no longer makes the run
  `preview_stale`. `10` and `11` were taken on the head that recorded this set
  (OCCI, Occipital's own agent) while it was working, and the run landed on
  the first try.
- The `status` table's one column is `Now`.
- The flush run's answer no longer carries the view-number fact.

Each step is a request file (`*.request.json`, the `{method, params}` the app
sent) and the reply (`*.json`): `{"result": …}` for an answer, or the error
body `{code, message, detail?}` for a refusal. Nothing was edited after
recording.

Recorded with the `compare_readers` example in `crates/occipital-module`,
whose `COMPARE_RAW=1` prints each reply as one line of compact JSON, as a
plain local caller, which Occipital admits as the desktop's development
build. Magic Context received `caller {app: "desktop", admission:
"development"}` with no device.

| Files | Step |
| --- | --- |
| `01-details-broca` | `session.details` with `commandSubject: true` on a Broca head. `subject` is `{runner: "broca", harness: "broca", session}`. |
| `02-details-opencode2` | The same on an OpenCode 2 head: `{runner: "thalamus-harness", harness: "opencode2", session}`. |
| `03-details-pi` | The same on a Pi head: runner `thalamus-harness`, harness `pi`. |
| `04-details-opencode1` | An OpenCode 1 plugin head (harness `opencode`): refused `invalid_request`. Occipital doesn't read those heads, so they have no subject and no commands. |
| `05-list-opencode2` | `commands.list` for the OpenCode 2 head: Magic Context's `flush` and `status`. A request wraps the subject from details as `{kind: "session", session: <subject>}`. |
| `06-preview-flush` | `flush` preview on that head, which now has drops it may land: `available: true` with its facts. The unavailable case (`queued_in_recent_tail`) is in `phone-commands-v1`. |
| `07-preview-status` | `status` preview: the palette facts and the expanded table under the column `Now`. |
| `08-run-flush-stale` | A run under a version no preview gave: `outcome: "preview_stale"`. Nothing ran. |
| `09-unknown-module` | A preview naming a module that declares no commands: refused `unknown_command`. |
| `10-preview-flush-available` | `flush` preview on the recording head: `available: true`, the facts, and the version to run under. |
| `11-run-flush-scheduled` | The run under that version: `outcome: "scheduled"` with three facts. The view takes effect on the head's next request. |
| `12-run-flush-repeat` | The same request again, same `request_id`: the module answers from its stored reply, byte-identical to `11`, and nothing runs twice. |

The phone route's recordings (`phone-NN-*`) were taken against the earlier
Magic Context build and stay in `phone-commands-v1`.

## Not here yet

The `status` command of AFT (the code-tools module), which follows once its
build with commands is live.

## Verify

```sh
(cd crates/occipital-app-reader/test-vectors/phone-commands-v2 && shasum -a 256 -c SHA256SUMS)
```
