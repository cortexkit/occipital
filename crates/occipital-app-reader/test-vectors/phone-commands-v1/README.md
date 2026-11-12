# The person's commands through Occipital (phone-commands-v1)

What the apps get from Occipital's `user-commands/v1` relay against the live
Magic Context module (the `ck-mc` build of commit `ff7e9140df`, served
through the Occipital build of commit `4199868`), recorded on
12 November 2026. The apps build their command palette from these bytes.

Each step is a request file (`*.request.json`, the `{method, params}` the app
sent) and the reply (`*.json`): `{"result": …}` for an answer, or the error
body `{code, message, detail?}` for a refusal. Nothing was edited after
recording, except one path noted below.

## Through the relay (`NN-*`)

Recorded with the `compare_readers` example in `crates/occipital-module`, whose `COMPARE_RAW=1`
prints each reply as one line of compact JSON, as a plain local caller,
which Occipital admits as the desktop's development build. Magic Context
received `caller {app: "desktop", admission: "development"}` with no device.

| Files | Step |
| --- | --- |
| `01-details-broca` | `session.details` with `commandSubject: true` on a Broca head. `subject` is `{runner: "broca", harness: "broca", session}`. |
| `02-details-opencode2` | The same on an OpenCode 2 head: `{runner: "thalamus-harness", harness: "opencode2", session}`. |
| `03-details-pi` | The same on a Pi head: runner `thalamus-harness`, harness `pi`. |
| `04-details-opencode1` | An OpenCode 1 plugin head (harness `opencode`): refused `invalid_request`. Occipital doesn't read those heads, so they have no subject and no commands. |
| `05-list-opencode2` | `commands.list` for the OpenCode 2 head: Magic Context's `flush` and `status`. A request wraps the subject from details as `{kind: "session", session: <subject>}`. |
| `06-preview-flush` | `flush` preview while every queued drop is still in the recent tail: `available: false`, reason `queued_in_recent_tail`. |
| `07-preview-status` | `status` preview: four palette facts and the expanded table, `Held in recent tail` among its rows. |
| `08-run-flush-stale` | A run under a version no preview gave: `outcome: "preview_stale"`. Nothing ran. |
| `09-unknown-module` | A preview naming a module that declares no commands: refused `unknown_command`. |
| `10-preview-flush-available` | `flush` preview on a head with drops it may land: `available: true`, the facts and the `Context after` figure, and the version to run under. |
| `11-run-flush-scheduled` | The run under that version: `outcome: "scheduled"` with its facts. The view takes effect on the head's next request. |
| `12-run-flush-repeat` | The same request again, same `request_id`: the module answers from its stored reply, byte-identical to `11`, and nothing runs twice. |

`10` to `12` ran on OCCI's own head, whose context is its own to flush. The
first try at `11` came back `preview_stale`, because OCCI's own turn moved
the context between preview and run; `10` and `11` were then taken back to
back.

## Through the phone route (`phone-NN-*`)

CKIOS's recording from the paired iPhone simulator over Callosum
(`fed-phone:`), at 22:28:15Z to 22:28:18Z, on the OpenCode 2 head of
`02`. Each step also has a `*.timing.txt` with when it was sent and how long it
took. These request files hold only the params; the step's name gives the
method. CKIOS's own checksum file for these 12 files has SHA-256 `b6b995da…`.

| Files | Step |
| --- | --- |
| `phone-01-details` | `session.details` with `commandSubject: true`. The phone's connection layer adds its ambient identity (`callerDirectory`, `sessionId`) to this read; the `/Users/user/` in `callerDirectory` was rewritten to `/Users/user-redact/`, this folder's redaction, so its bytes differ from CKIOS's file. |
| `phone-02-list` | `commands.list`, with no ambient identity: `commands.*` params refuse unknown members. |
| `phone-03-preview-flush` | `flush` preview: `queued_in_recent_tail`. |
| `phone-04-preview-status` | `status` preview. |

Magic Context's log for the two previews shows what Occipital forwarded:
`caller {app: "phone", admission: "phone", device: "ac2a520b6dd57fc83bd5d36bd4899688"}`,
the simulator's device fingerprint from its Callosum route. `commands.list` is
answered by Occipital from the declarations it keeps, so Magic Context logs no
line for it.

The simulator is granted list and preview only; `commands.run` is granted to
the attested iPhone alone, so no phone run is recorded here.

## Not here yet

AFT's `status` command, which follows once AFT's build with commands is live.

## Verify

```sh
(cd crates/occipital-app-reader/test-vectors/phone-commands-v1 && shasum -a 256 -c SHA256SUMS)
```
