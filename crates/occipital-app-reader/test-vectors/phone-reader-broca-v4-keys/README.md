# phone-reader-broca-v4-keys

Part identities on a Broca chat's rows: `partKey`, `partIndex` and `callId`, the same
on every surface that serves a row. The session's `m41` holds encrypted reasoning,
readable reasoning, text, a tool call with its result in the same message, more text
and a second call whose result is stored in `m42`. The encrypted reasoning is never
shown, and a page folds the first result into its call (a full read shows it as its
own part), so on a page the shown parts' `partIndex` values (1, 2, 3, 5, 6) differ
from their row positions (0 to 4).

## What changed from v3

Runners now serve llm-runner 0.8. Its pages bind the history counter and the log's
identity, `incarnation`, instead of a lineage, so `version` reads
`history:7:incarnation:inc-a` where v3 read `history:7:lineage:cut-a`, and
`activityToken`, a digest of the runner's head, changed with it. Every entry is
readable from ordinal 1, so the newest page, which starts at `m40`, now offers older
messages with `nextBeforeOrdinal: 40`, where v3 claimed `reachedStart`. The replies
also carry what the reader added after v3 was recorded: `lastRun`, and
`completion: "unknown"` on an assistant row without a write time.

## Where the files come from

Occipital's app reader served these replies at the commit that adds this folder,
through the same serving path a phone's route takes, over a test Broca session. Each
`.reply.json` or `.notice-1.json` file is the whole reply as an app receives it, and
each `.request.json` file is the request's `{method, params}`. The test
`app_reader::broca::tests::part_keys::the_part_key_samples_are_what_the_handlers_answer`
in `occipital-module` serves every request again and fails if a reply's bytes change,
so a reader change that alters them needs a new sample set, never an edit to this one.

## Cases

| Files | What it shows |
| --- | --- |
| `01-page-newest` | The newest page of four messages, `m40` to `m43`. |
| `02-claim-behind` | A claim holding only `m40`: `newTurns` carry `m41` to `m43` with the same identities. |
| `03-watch-behind` | The same claim on a watch: its first notice. |
| `04-message-full` | The full read of `m41` (`messageOrdinal: 41`): every stored part, the first result as its own part. |
