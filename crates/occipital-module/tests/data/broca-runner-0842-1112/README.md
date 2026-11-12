# broca on llm-runner 0.8.42: reply samples for readers

Written by broca's real serializers (`broca-wire`), synthetic values only.

- `role-describe-before.json` / `role-describe-after.json`: `role.describe` as broca 0.4.20
  serves it and as the next build will. The only difference: capability `plan_view` and op
  `session.plan_view` are added (contract 4.9). Broca already serves the read on the new
  storage, where every head lives; a session still kept in today's storage refuses it by name.
- `session-head-account-limit-before.json` / `-after.json`: `session.head` while the latest run
  is paused for an account limit. After: `last_run_state.resume_not_before_ms` (0.8.42), the
  same time `run.status` already serves. Present only while the run is paused for an account
  limit, and only on the new storage; absent in every other state.
- `session-head-completed.json`: an idle head, unchanged by this build.

Also in this build, with no reply change: `session_class: "summary"` sessions start on the
new storage.
