# Claude Code groups, v1

These JSON bodies come from `thalamus-harness-module` tests driving the pinned
Linux Claude Code 2.1.330 through the module's MCP server and model gateway.
The upstream model and tool providers are local test stand-ins; no live account
or user history was used. The files contain synthetic session, provider and tool
identifiers. IDs, timestamps and digests are examples, not constants for readers
to match. `SHA256SUMS` pins this recording, not another run of the tests.

| Files | Reader and case |
| --- | --- |
| `role-describe.json` | Prefrontal: test-support capability surface. Lists `pre_user`, `send_ids`, `call_yield` and `notice_yield`; neither `tool_versions` nor `refresh_relaunch` is declared. |
| `pre-user-read.json` | Prefrontal and Occipital: `session.read` showing the transformed first and later prompts. Retrying a send did not run the hook again. |
| `send-ids-read.json`, `send-ids-raw-history.json` | Prefrontal and Occipital: opted-in `session.read` and raw `session.history`. The owner and a named sender both used `shared-key`; each user entry carries that sender's own key beside `sender`. Assistant entries omit it. |
| `cc-call-yield-*`, `cc-notice-yield-*` | AFT: the actual outbound `tool.yield` request for each trigger and the `tool_finished` subscription event for the interim completion. Prefrontal and Occipital: send reply, settled send and run status, and the later provider notice's receipt and read page. The tool finishes on interim content; the late result appears once as a provider-authored notice naming the original call. |
| `refresh-reply.json`, `refresh-baseline-applied.json`, `refresh-history-{before,after}.json` | Prefrontal: accepted required refresh, applied baseline and raw history across the successful next-run relaunch. Previous entries, ordinals and `history_version` are unchanged. The host restarts once, only for that session. |
| `refresh-run-error.json`, `refresh-baseline-failed.json`, `session-refresh-failed.json` | Prefrontal: `host_relaunch_failed` status, the pending-failure record and the subscription event when the required process launch fails before any model request. |

The refresh recordings cover a due tool change known before handoff. They are
not evidence for deferred changes that first become due inside the gateway's
first-request reconciliation, where Claude Code has already built its request.

To record again on Linux, fetch the pinned Claude Code with
`bash probes/claude-code-linux/fetch.sh "$TMPDIR/cc"` and set
`THALAMUS_CLAUDE_CODE_BIN` to that folder's `claude`. Set
`THALAMUS_CLAUDE_GROUPS_RECORD_DIR` to the recording directory. Run the ignored
`runner::tests_claude_code_real::parity` tests and
`real_claude_code_ends_a_run_when_a_required_tools_change_cant_be_installed`
with `--features test-support`; this set was captured from their actual replies
and events without substituting field values.
