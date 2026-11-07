# phone-reader-stop-all-v1

Stop everything, as an app sends it: `session.stop_all {session, harness?, runId?,
clientRequestId}`. It ends the chat's running turn and asks every tool provider in
the session's plan to stop or pause what it runs for that session, such as shell
commands, remote builds, workers and consults. Each request is served through
Occipital's own frame loop on a paired phone's route, against a test runner. The
runner's replies are invented in the runner contract's shape (llm-runner 0.8.17,
`stop_all`). `NN-*.request.json` is the request the app sends, and `NN-*.json` is
the reply body it receives.

Beside each request that reaches the runner, `NN-*.runner-request.json` is the
`session.stop_all` body Occipital sent. The apps never see it. It shows that the
app's `clientRequestId` goes to the runner as its `request_id`, and `runId` as
`run_id`.

The reply is the runner's, passed on unchanged:

- `providers` lists every provider in the plan. `outcome` is `answered` (with
  `items`, and `more` when it listed only some of them), `no_answer` (no reply
  within the runner's 15-second budget), `not_declared` (it doesn't offer the
  operation) or `unreachable`.
- `runner.items` is what the runner itself stopped.
- Each item has `kind`, an optional `id`, a short `label` (at most 120
  characters, never a command line) and `outcome`: `stopped`, `paused` (it can be
  resumed), `already_finished` or `could_not_stop`, with a `reason`. Kinds and
  outcomes are open: show an unknown one by its label.

So an app can show what stopped and what may still be running, and never claim
everything stopped when a provider didn't answer.

The runner keeps its first reply under the request id. A retry with the same
`clientRequestId` gets that reply again, and stops nothing begun since.

| Files | What they show |
| --- | --- |
| `01-full` | Every provider answered: two of AFT's items stopped, a worker and a consult paused by core, and Basal's program already finished. |
| `02-repeat` | The app's retry after a lost reply, under the same `clientRequestId`. Occipital asks the runner the same way, and the runner answers with its first reply. |
| `03-partial` | No `runId`. AFT couldn't stop one shell command, core gave `no_answer`, Cerebellum is `not_declared`, and the runner stopped its own open fork answer. |
| `04-unsupported` | A runner that doesn't declare `stop_all` is never asked. The app gets `unsupported` and can offer Stop instead. |
| `05-not-authorized` | The plan grants Occipital no `interrupt` for this session, so the runner refuses, as it does Stop. The app gets `enqueue_not_authorized`. |

## Where the replies come from

Occipital served each request at the commit that adds this folder. The test
`app_reader::send::tests::stop_all_samples::the_stop_all_samples_are_what_the_reader_serves`
in `occipital-module` serves every request again and fails if a byte changes, so a
change that alters them needs a new sample set, never an edit to this one.
