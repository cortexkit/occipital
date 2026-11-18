# session_held_elsewhere `detail.holder` (agent-data 0.3.3 holderBox), broca

Written by broca's serve path in `store_move_plain_and_encrypted_roundtrip_release_and_move_back`,
after a store move released the source session:

- `read-refused-holder-box.json`, `send-refused-holder-box.json`: the release carried
  `holderBox` (synthetic `0123456789abcdef0123456789abcdef`), so the refusal names it as
  `detail.holder`, the receiving computer's box id.
- `read-refused.json`, `send-refused.json`: the release carried only `holder` (the backup author
  id), so the refusal names no holder: `detail` is `{}`, as broca 0.4.69 answers today.

The backup author id is never named in either.
