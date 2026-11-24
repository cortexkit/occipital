# Runner-written empty answers v1

Captured on Linux from Thalamus build `ef470e068a56e31a3cdb74604db27a8a18db22cc` with synthetic transcripts. These are the served entries, not reconstructed examples.

- `opencode2-read-entry.json` is the `session.read` entry with `include_display_facts: true` for the empty assistant entry written by Agent Runner when the host stored no answer. `opencode2-display-entry.json` is the same entry in `session.history` display form. Both carry `runner_entry: {"code":"host_stored_no_reply"}` beside, not inside, the entry content or text parts.
- `pi-native-read-entry.json` and `pi-native-display-entry.json` show the same read and display forms for an actual empty Pi assistant reply. The `runner_entry` fact is absent on both.
- `SHA256SUMS` pins this README and all four JSON entries. The recorder checks the JSON for email addresses and machine home paths before writing it.

To capture a fresh set, set `THALAMUS_RUNNER_ENTRY_SAMPLES` to a new directory and run `a_completed_run_without_a_host_reply_records_an_empty_final_assistant` and `a_pi_native_empty_reply_is_not_a_runner_stand_in` in `thalamus-harness-module` with `--features test-support`. The runner-written entry is enabled in unit tests; the shipping switch remains off.
