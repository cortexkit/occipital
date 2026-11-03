# phone-session-facts-v3

The session facts a Broca chat's pages, claims and details carry: the next send's
`model`, the session's `usage`, the app's send grant (`sender`, or `senderUnknown`),
the log's `incarnation`, the last run (`lastRun`), and each row's `network`.

## What changed from v2

Runners now serve llm-runner 0.8. Its pages bind the history counter and the log's
identity, `incarnation`, instead of a lineage, so `version` reads
`history:7:incarnation:inc-a` where v2 read `history:7:lineage:cut-a`, and
`activityToken`, a digest of the runner's head, changed with it. The replies also
carry what the reader added after v2 was recorded: `lastRun`, each assistant row's
`completedAtMs`, and each part's `partKey` and `partIndex`. v2's README describes the
cases; they are the same.

## Where the files come from

Occipital's app reader answered these requests at the commit that adds this folder,
through its handlers, over a test Broca session. Each file is the whole reply body.
The test `app_reader::broca::tests::session_facts::the_session_fact_samples_are_what_the_handlers_answer`
in `occipital-module` answers every request again and fails if a reply's bytes change,
so a reader change that alters them needs a new sample set, never an edit to this one.

## Cases

| File | What it shows |
| --- | --- |
| `newest-page-model.json` | The newest page with the next send's model and the app's send grant. |
| `matched-claim-model.json` | A matched claim after one more message: the same facts beside `newTurns`. |
| `details-usage.json` | `session.details` with model, usage, grant and the log's incarnation. |
| `details-no-usage.json` | The same without usage or grant: `senderUnknown: true`. |
| `details-no-grant.json` | A runner that declares `head_sender` and grants nothing: no sender member. |
| `details-sender-refused.json` | A read-only route: the grant is never shown. |
| `newest-page-network.json` | Each row's `network` at its write time: yes, no, and unknown for a row without one. |
