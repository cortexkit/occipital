# phone-reader-runner-entry-v1

An entry the runner wrote itself, as the apps receive it (runner contract 0.9.6,
`runner_entry`). Served through Occipital's real request handling by
`occipital-module`'s `broca::tests::contract_096_samples`, so the reply is byte
for byte what a phone receives. `01-newest-page.request.json` is the request
the app sends and `01-newest-page.json` the reply body. `SHA256SUMS` lists every
file but this README.

When Claude Code saves no reply for a model step that finished, Agent Runner
writes an empty assistant entry in its place, so the run always names a reply.
The read marks it with `runner_entry`, and the row carries it as
`runnerEntry: {"code": "host_stored_no_reply"}`. A model's own empty reply
carries no mark.

The chat is Agent Runner's recorded Claude Code chat (`host-parity-wave1-v1`),
whose last entry is such an empty entry, with the fact the runner now serves
added. Rows 4 and 5 are copies of that entry, invented to show the other two
cases:

| Row | `role` | `runnerEntry` | What it is |
| --- | --- | --- | --- |
| 1 | `user` | none | the person's message |
| 2 | `assistant` | none | Claude Code's own "No response requested." |
| 3 | `assistant` | `{"code": "host_stored_no_reply"}` | the empty reply the runner wrote |
| 4 | `assistant` | `{"code": "a_later_code"}` | a code from a later contract: still an entry the runner wrote |
| 5 | `assistant` | none | an empty reply from the model itself |

A row with `runnerEntry` is never the model failing to answer, whatever its
code. A row without one keeps its meaning as before.
