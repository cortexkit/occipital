# runner-pages-opencode2-v1

Replies of Thalamus's OpenCode 2 runner (module `thalamus-harness`, bind harness
`opencode2`, llm-runner 0.8.2), recorded from a real OpenCode 2.2.3 server by
`runner::tests_opencode2_real::real_opencode2_records_the_pages_apps_read`, with a
loopback stand-in model. The session ran two turns: one with a plan tool call
(`bash`), then a plain reply. Both prompts were sent with a `client_message_id`.

| File | Request |
|---|---|
| `01-session-read.json` | `session.read {from_ordinal: 1, limit: 50}` |
| `02-session-read-display-facts.json` | the same, with `include_display_facts: true` |
| `03-session-head.json` | `session.head` |
| `04-session-history.json` | `session.history {limit: 50}` |
| `05-session-history-display.json` | `session.history {limit: 50, form: "display"}` |
| `06-session-history-newest.json` | `session.history {limit: 2, form: "display"}` |
| `07-session-history-older.json` | the next page back, with that page's `older` cursor |
| `08-run-status.json` | `run.status` for the first turn's run |
| `09-role-describe.json` | `role.describe` |

What the set shows:
- Messages are numbered from 1, every page carries `history_version` and
  `incarnation`, and a cursor names its page.
- Display rows come from the contract's OpenCode 2 rules: a tool call and its result
  are one assistant row.
- Display facts carry the person's `client_message_id` and each tool's
  `started_at` / `finished_at`. Call keys and schema pins are absent: OpenCode 2
  sessions don't declare `call_identity`.
- The owner's head grant lists only `steer` and `idle_start`.

Not shown: live message and text events on `session.subscribe`. OpenCode 2 sessions
don't declare `live_events` yet, so a subscriber sees run lifecycle only.

`SHA256SUMS` lists every file except itself.
