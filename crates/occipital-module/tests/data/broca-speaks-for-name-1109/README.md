# Named speakers: captured reader samples

Captured with BROCA_CAPTURE_SAMPLES=1 from broca's own serve loop and serializers (task branch
merge 30ef7203d, writer commit "declare notice_with_speaks_for and store speaker names"), with
synthetic session names and text.

- named-marks-send.json: `session.send` reply for the scope owner's send carrying
  `{"speaks_for":{"agent_id":"agent-a","name":"Ada"},"notice":true}`
  (tests/it/plan_admission/message_marks.rs a_named_agent_notice_is_stored_and_served_with_both_marks).
- named-marks-read.json: `session.read` with `include_display_facts`. Every message of the run's
  input carries the stored `speaks_for` with its `name`, beside `notice: true`.
- named-marks-run-status.json: `run.status` of that run. The run.status surface has no marks
  member, so it carries no `speaks_for`; it is here so readers can check the session's replies.
- named-marks-reuse.json, named-marks-reuse-2.json: `send_id_reuse` refusals with
  `detail.field: "speaks_for"`, for a retry that changed the name and one that dropped it
  (a_changed_speaker_name_refuses_the_retry_naming_speaks_for).
- compaction-named-speaker.json: a `session.entries` page served to a compaction provider whose
  catalog entry declares `compaction-entry-speaks-for/v1`. The user entry carries
  `speaks_for {agent_id, name}` beside its notice origin, outside its digest
  (tests/contract_samples.rs a_named_speaker_reaches_a_declaring_compactors_entries).
