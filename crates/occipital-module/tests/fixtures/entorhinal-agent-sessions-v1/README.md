# Entorhinal session lookup samples

These three files are byte-for-byte copies of Entorhinal's published
`test-vectors/agent-sessions-v1/` samples. `session-for-runner.json` and
`for-session.json` come from commit `7947b7cac9c96b6d114d669f517a4f1cfd02634e`
(the session lookup operations arrived in 0.1.41, and 0.1.42 added each
binding's `network` and `network_history` members).
`session-for-ref.json` comes from commit `503fa3f` (0.1.47), where
`agent.session_for_ref` replies also name the binding's recorded `runner`;
`resolve::tests::reference_reply_naming_its_runner_still_decodes` checks that
such a reply still decodes. Success replies include the
`result` envelope; missing registrations carry `code: "not_found"`.

`resolve::tests::published_lookup_samples_decode_and_keep_their_checksums`
decodes every reply and pins the published SHA-256 hashes below. The legacy
samples use Pi/OpenCode bindings; Broca tests use the same wire shape with
Broca session identities.

| File | SHA-256 |
| --- | --- |
| `session-for-runner.json` | `b62878e6c91d98cc81ff0f993205edd4a88b5eaf49d649bab81feca02696203e` |
| `for-session.json` | `dd41914934a9b85b35670a815bef17de84752d8da7d05a1bf8833f3edbd1996b` |
| `session-for-ref.json` | `3e9e99950c167a2b2c5b3315fca35d353f50bc29e8ad93afe616c15e7cb703f6` |

The following files are byte-for-byte copies from Entorhinal's tag
`agent-sessions-v1.13` (0.1.84). They publish held-session placement and
open-session pagination. Since the first copy (`54c22cc`, 0.1.44) a moved
session's machine reads `source-machine`, an ended one's liveness `gone`, and
the module is `prefrontal`.
`resolve::tests::published_placement_samples_keep_their_checksums` pins their
hashes; app-reader tests adapt the listed session bindings to the resolver's
runner and legacy reference reply envelopes.

| File | SHA-256 |
| --- | --- |
| `sessions-held-on.json` | `12d112208ca459bd80dab2f1956a8ad1a4661e7d8f4210a64c6733199b8c5329` |
| `sessions-open.json` | `a92b2ff10aa68854acc39e883e2ba080a84db5cfc28fa1b51d2fe9df5a8661c0` |
