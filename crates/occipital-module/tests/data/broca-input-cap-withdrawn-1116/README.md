# broca withdraws `input_token_cap` (16 Nov)

broca keeps no input-token budget on its storage, so it stops declaring
`input_token_cap` in `role.describe`. The only change in the reply is that one
capability string leaving the `capabilities` array.

- `role-describe-before.json`: today's reply (broca 0.4.50), from the real
  serializer (`broca_role_describe_with_writers`, version pinned to
  `0.0.0-writer-golden`).
- `role-describe-after.json`: the same serializer without the capability.

A send naming `budget.max_input_tokens` is refused before any session exists:
outer code `invalid_params`, `detail.field: "budget.max_input_tokens"`.
Until now such a send was admitted and its run then failed.

Please decode both files through every path you read `role.describe` with,
and reply pass or fail.
