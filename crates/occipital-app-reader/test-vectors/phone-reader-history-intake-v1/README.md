# phone-reader-history-intake-v1

A chat whose history the runner is still taking in, as an app reads it:
`historyIntake: "running"` on `session.details`, on the newest transcript page,
and on claim replies that carry session facts.

- When a head moves onto a runner, the runner takes in the history its host
  already held, oldest first. Until it's done, the newest page holds the newest
  message taken in so far, which is old, not the chat's latest. The runner says
  so on its head (runner contract `history_intake`), and Occipital passes it on
  as `historyIntake: "running"`.
- Any value the runner gives reads as `"running"`, since the contract treats an
  unknown one as still running. Once the intake is done the member is absent.
- While it's present, an app shouldn't keep the page as the chat's latest, for
  example in a cache that reopens the chat after a restart. Once it's gone, the
  app should read the newest page whole rather than claim against the page it
  holds.
- The rest of the history is stored after what was there, so the page version
  doesn't change when the intake ends. The facts do, which changes
  `activityToken`, so an app's next poll or transcript-watch notice brings the
  change.
- An `unchanged` reply and an older page carry no `historyIntake`.

## Where the replies come from

Occipital's app reader served each request at the commit that adds this folder,
through the same serving path a phone's route takes, over a test OpenCode 2 session
on Thalamus's runner (module `thalamus-harness`). The text is invented. Creation
times are the test runner's write times. `<name>.json` is the whole response body.
`<name>.request.json` is the request's `{method, params}`. The test
`app_reader::broca::tests::history_intake_samples::the_history_intake_samples_are_what_the_reader_serves`
in `occipital-module` serves every request again and fails if a byte changes, so a
change that alters them needs a new sample set, never an edit to this one.

## Cases

| Files | What it shows |
| --- | --- |
| `01-details-intake-running` | `session.details` while the runner is taking in the history: `historyIntake: "running"`. |
| `02-newest-page-intake-running` | The newest page then: the two oldest messages, the newest taken in so far, with `historyIntake: "running"`. |
| `03-newest-page-intake-done` | The newest page once the intake is done: all four messages under the same `version`, a new `activityToken`, no `historyIntake`. |
