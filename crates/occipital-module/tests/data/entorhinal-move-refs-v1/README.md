# Entorhinal `agent.move_refs` samples

Copied byte for byte from Entorhinal's `test-vectors/agent-move-refs-v1/` at tag
`agent-move-refs-v1` (commit `47943d2`); `SHA256SUMS` is Entorhinal's own.

`agent.move_refs {cutId}` answers the session refs an agent move carries, from
the move's frozen plan, the same on the computer the agent leaves and the one it
reaches. Occipital's move participant asks it in `prepare` to learn which
sessions to hold and carry. `source.json` and `receiver.json` are the same
reply; `unknown-cut.json` is the refusal for a cut Entorhinal never saw.
`moves::remote::tests::entorhinal_s_move_refs_samples_decode` decodes them.
