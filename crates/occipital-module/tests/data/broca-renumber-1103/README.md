# Broca replies across the runner contract 0.8 move

Broca's own replies for one chat, before and after Broca moved to runner contract
0.8. Occipital's sample set `phone-reader-broca-renumber-v1` is served from them. Each
file is the raw reply body Broca's serving path sent, `{"result": ...}` included,
byte for byte. `SHA256SUMS` covers the four replies.

| File | Broca build | Request |
| --- | --- | --- |
| `before/session.head.json` | 0.3.306 (runner contract 0.7.48) | `session.head {}` |
| `before/session.read.json` | 0.3.306 | `session.read {include_display_facts: true}` |
| `after/session.head.json` | 0.3.307 (runner contract 0.8.1) | `session.head {}` |
| `after/session.read.json` | 0.3.307 | `session.read {include_display_facts: true}` |

Both reads ask for the newest page. The chat has four messages: a user request with
the made-up client message id `phone-reader-message-1`, an assistant tool call
`phone-call-1`, its result, and a final reply. Broca 0.3.306 wrote it in a fresh test
store and answered the before reads. Broca 0.3.307 then answered the after reads from
the same store, with no new send. The store's write log was byte-identical before and
after the reads. Project, harness and session are made up.

## What differs

| Field | Before | After |
| --- | --- | --- |
| Message ordinals | `0, 1, 2, 3` | `1, 2, 3, 4` |
| Head first / last ordinal | `0` / `3` | `1` / `4` |
| `history_version` | head `0`; read absent | both `1793749321093` |
| `incarnation` | head only | both, unchanged |
| `lineage_id` | both | neither |
| Per-message part keys | `tags: [{"part": "m0#0"}]` | `part_keys: ["m0#0"]` |

Message ids stay `m0` to `m3`. Content, timestamps, run attribution, tool calls and
display facts (the client message id, and the call's start and finish times and call
key) are the same in both.

Broca recorded these from tags `v0.3.306` (commit `584849bf`) and `v0.3.307`
(commit `e5337831`).
