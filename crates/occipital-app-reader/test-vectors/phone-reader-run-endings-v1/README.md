# phone-reader-run-endings-v1

How each way a run can end reaches the apps through Occipital: `lastRun` and
`activity` on a chat, and the mark on a tool call no tool answered. Every file is
served through Occipital's own frame loop on a paired phone's route, over replies
that Thalamus's OpenCode 2 runner recorded from a real OpenCode 2.2.3 server, at
harness build `3d70587c3`, with a loopback stand-in model. Those runner replies are
in `crates/occipital-module/tests/data/thalamus-run-endings-v2/`, copied byte for
byte from the thalamus repository's tag `phone-reader-run-endings-v2`, with a
README that says how each ending was produced. The chat text is the test's own
probe text.

`NN-*.request.json` is the request the app sends, and `NN-*.json` is the reply body
it receives.

| Files | Ending | What the apps get |
| --- | --- | --- |
| `01-completed` | the model answered | `lastRun.state: "completed"`, `activity: "idle"` |
| `02-max-steps` | the step limit ended the run | `state: "max_steps"`. Its `bash` call (`call_probe`) has `isError: true` and `unanswered: "not_executed"`, because the step limit left it unrun. |
| `03-error-provider` | the provider refused the request | `state: "error"`, `error: {class: "permanent"}` |
| `04-error-compaction-unavailable` | no compaction view, so the turn never reached the model | `state: "error"`, `error: {class: "compaction"}` |
| `05-completed-tool-not-offered` | the model called a tool the plan doesn't offer, and the run went on | `state: "completed"`. The call (`call_absent`) has `isError: true` and `unanswered: "denied"`. |
| `06-cancelled` | the owner stopped the run mid-step | `state: "cancelled"` |
| `07-interrupted` | the runner restarted mid-step | `state: "interrupted"`, `reason: "runner_crash"` |

Each of these has `NN-*-details` (`session.details`) and `NN-*-newest-page`
(`session.transcript_page`, `limit: 50`). Every recorded run has ended, so each
page's `activity` is `idle`. The runner marks only calls no tool answered: a
tool's own failure would carry `isError` alone.

`S1` to `S9` (`*-details-synthetic`) are `session.details` for endings this runner
doesn't reach today, each built from a recorded head with only the ending changed:

| File | `lastRun` |
| --- | --- |
| `S1-active-compaction-wait` | `active`, `reason: "compaction_wait"` |
| `S2-active-context-ceiling-wait` | `active`, `reason: "context_ceiling_wait"` |
| `S3-paused-auth-required` | `paused`, `reason: "auth_required"` |
| `S4-paused-restart` | `paused`, `reason: "restart"` |
| `S5-context-ceiling` | `context_ceiling` |
| `S6-error-transient` | `error`, `error: {class: "transient", retryAfterSecs: 2.5}` |
| `S7-error-auth-required` | `error`, `error: {class: "auth_required"}` |
| `S8-error-provider-unavailable` | `error`, `error: {class: "provider_unavailable", resetsAtMs}` |
| `S9-error-context-overflow` | `error`, `error: {class: "context_overflow"}` |

`active` and `paused` are a run still going, and a page then shows `activity:
"busy"`. Every other state the runner contract names is a finished run, and a page
then shows `activity: "idle"`. For a state Occipital doesn't know, a page leaves
`activity` out, and an app should treat that run as finished.

`SHA256SUMS` covers every file except this README. A test in `occipital-module`
(`broca::tests::run_endings_samples`) serves the set again and fails if any byte
changes.
