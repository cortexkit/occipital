# agent-data-v2

Occipital's answers as a participant in agent moves (`cortexkit-role-agent-data`
0.3.3), served through its real handler (`crate::moves`). Each file is a JSON
array of exchanges, in the order a move driver sends them:
`{op, principal, request: {method, params}, reply}`. `reply` is the whole
response body, `{"result": …}`, or the error frame's `{code, message, detail}`.

The move carries agent `agent_0000000000000101` from a source computer to a
receiver. On the source, the moved session `ses_moved` holds one row in each
table a move copies: a call pin, a tool record and detail, a riding notice, a
wake record, a notice record, the message-to-send map, and a waiting send with its wait. A
second session, `ses_stays`, holds the same rows and doesn't move.

| File | Computer | What it shows |
| --- | --- | --- |
| `source.json` | source | `describe` (`optional: true`, the one ref kind `agent`), `busy` (always idle), `prepare` (one stream item, `frozen: false`), `export`, `status` |
| `receiver.json` | receiver | `diverged` (`at_cut`), `import` (the same digest and end), `activate`, `status` |
| `release.json` | source | `release` after the receiver activated: `enforced: true` |
| `empty-agent.json` | both | an agent with no rows here: `prepare` and `import` carry no items |
| `refusals.json` | source | a caller other than Entorhinal (`caller_forbidden`), a runner's session kind (`invalid_request`), `export` of a cut never prepared (`cut_unknown`), and a second `release` naming another holder (`id_reused`) |

The test `moves::tests::the_participant_samples_are_what_it_serves` writes these
files with `OCCIPITAL_BLESS_VECTORS=1` and otherwise fails when a reply's bytes
would change.

This set's moved session also holds a notice record (`notice_records`), so
each stream carries one more row than in the earlier set beside this folder,
`../agent-data-v1/`, which the move driver's tests pin and which never
changes.
