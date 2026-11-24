# plan-notes-1124

Reader samples for broca's plan-notes writer (llm-runner `plan_notes`, §10.2), from broca's
real serializers. Not shipped yet: broca 0.4.115 carries the reader half only.

- `role-describe.json`: `role.describe` with `plan_notes` added to `capabilities`, right after
  `session_change`. Nothing else changes.
- `baseline-pending.json`: `session.baseline` after an owner's `session.refresh` with policy
  `notes_until_rebuild` was accepted: `pending.policy` is `notes_until_rebuild`. `rungs` do not
  list it, because the contract makes it a policy, not a rung.
- `baseline-notes-delivered.json`: the same after a run delivered notes about the pending
  change (the baseline is unchanged; notes are runner records, never transcript entries).
- `baseline-applied.json`: after a `session.flush_prefix` and the next run applied the change
  in full (`effective.applied_by: "fold"`, nothing pending).
