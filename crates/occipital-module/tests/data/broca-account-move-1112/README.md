# `account_continue` on a run's error: sample bodies

When a run ends at a quota wall because routing would not move it to another account, the
run's `error` (a `ProviderError`, `provider_code: "account_limit_exhausted"`) now carries one
new optional member, `account_continue`, with routing's answer:

- `routing_code`: routing's code for the hold or the refusal.
- `until_ms`: only for a hold where routing said when an account should have room.

The member is absent when routing gave no answer (it wasn't asked, it was unreachable within
the budget, or its offer couldn't be taken). It is absent on every other error, so those bodies
are unchanged.

## How these were made

Each pair (`run.status` reply and `run.result` reply) comes from the real serializers on a
synthetic store-storage session. The module's registry is opened on a scratch folder, one send
is made whose scripted provider ends the step with the error below, and the registry's
`run_status_reply` and `run_result` replies are serialized with `serde_json::to_vec_pretty`.
The generator was a temporary test in `crates/broca-module/src/store_actor/status_tests.rs`
(kept, uncommitted, at `.cortexkit/scripts/account-move-samples-test.rs`). It was run on the
Linux build server with `BROCA_ACCOUNT_MOVE_SAMPLES` set to this folder:
`cargo nextest run --locked -p broca-module --lib -E 'test(zz_account_move_samples)'`.

| File prefix | The error's `account_continue` |
|---|---|
| `before` | absent (today's body) |
| `after-hold-with-until` | `{"routing_code":"no_compatible_account","until_ms":1800000000000}` |
| `after-hold-without-until` | `{"routing_code":"no_compatible_account"}` |
| `after-refusal` | `{"routing_code":"account_pinned"}` |

The only difference between `before.*` and any `after-*.*` body is the added
`account_continue` member inside `error`, appended after `limit_window`.
The error's `credential_id` and `account_ref` members don't appear in either reply.
