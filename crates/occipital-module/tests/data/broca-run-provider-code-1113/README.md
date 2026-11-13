# Required provider-code reader samples

This set follows `run-awaits-1112/` and corrects its codeless-error omission rule. Served run
errors have always required `{class, message, provider_code}`. Broca now supplies a missing
code at the shared `SharedProviderError` boundary, without rewriting stored errors:

| Caller-facing class | Derived missing code |
| --- | --- |
| transient | `transient` |
| auth_required | `auth_required` |
| context_overflow | `context_overflow` |
| permanent or an unrecognized class | `provider_error` |

`provider_error` is the open wire catch-all named by llm-runner 0.8.47: the provider refused
or failed and no named code applies. The existing dependency pin stays at 0.8.45; unknown
code strings already decode. Existing codes are preserved rather than reclassified.

The provider's finer `provider_subcode` and `provider_details` bytes stay unchanged. The
`provider_subcode` versus contract `provider_detail_code` spelling is a separate gap, not
renamed or moved by this change.

## Provenance and filenames

- `before` is the served behavior at commit `92f9ade6b6cb152ac987e0e9c5fab917b3035151`.
- `after` is this follow-up's normalized served behavior.
- `legacy` and `store` identify the two actual storage implementations; before/after do not
  mean different storages in this set.
- `terminal-unknown-code` is a codeless permanent terminal error.
- `transient-codeless` is a codeless transient terminal error.
- `auth-required-codeless` follows the real loop's authentication pause, not a fabricated
  terminal authentication failure.

Every body comes from the real actor's typed status/result/head serializer. Subscribe files
use captured control units and broca's real verified-cursor frame serializer. They are raw
serializer JSON bytes, with no added file LF. The providers, messages and finer diagnostic
values are synthetic. The tests read the stored errors after serving every surface and
assert that their code is still absent and their finer diagnostic is unchanged.

Before and after are independent fixture runs. Random incarnation/digest values and capture
or recorded timestamps can therefore differ as well as the contract fields; the byte-change
ledger below lists those differences explicitly rather than rewriting the samples.

## Every served surface

| Surface | Code location | Semantic change |
| --- | --- | --- |
| run.status | `error.provider_code` | Added on both storages |
| run.result | `error.provider_code` | Added on both storages, including the auth pause's error |
| session.head | `last_run_error.provider_code` | Added for both terminal error cases on both storages; absent for an auth pause, which has no last_run_error |
| subscribe error | `unit.error.provider_code` | Added for terminal errors on both storages |
| subscribe paused | `unit.error.provider_code` | Added for the authentication pause on both storages |
| subscribe run_finished | No error object, unchanged shape | Only new-storage `unit.awaits` changes; no provider_code member is invented here |

`awaits` remains new-storage-only. Codeless permanent endings now carry `party: nobody`;
codeless transient endings carry `party: caller`. Status, result, head and both ending
controls use the same normalized error and recorded end time. Legacy endings still omit
`awaits`. Authentication pause awaits remain person/sign_in; nonterminal results have none.

Thus every terminal permanent/transient surface changes on `store`. On `legacy`, status,
result, head and error frames change, while the finished frame retains its prior fields.
For authentication, status/result/paused error bodies change on both storages; the head has
no error summary and no finished/error unit is produced for that nonterminal pause.

The permanent, transient and authentication tests all fail on the before boundary with a
missing provider_code. After normalization, each applicable error decodes against the
contract's typed RunError, and complete status bodies decode against its typed RunStatus.

`SHA256SUMS` covers the 56 JSON bodies and this README. No sample is committed.

## Exact byte-change ledger

Each row compares the raw before and after file, then lists every changed JSON path. An
added awaits object is listed at its object path; it contains the party and recorded time.
Cursor proof identities and epoch timestamp changes are fixture-run variation, not record
rewrites caused by serving.

| Scenario / storage / surface | Raw bytes changed | Changed JSON paths |
| --- | --- | --- |
| `auth-required-codeless.legacy.run.result` | yes | `$.error.provider_code` |
| `auth-required-codeless.legacy.run.status` | yes | `$.error.provider_code` |
| `auth-required-codeless.legacy.session.head` | yes | `$.head.digest`, `$.head.incarnation`, `$.incarnation`, `$.updated_at` |
| `auth-required-codeless.legacy.subscribe.paused` | yes | `$.cursor.digest`, `$.cursor.incarnation`, `$.unit.error.provider_code` |
| `auth-required-codeless.store.run.result` | yes | `$.error.provider_code` |
| `auth-required-codeless.store.run.status` | yes | `$.awaits.since_ms`, `$.error.provider_code` |
| `auth-required-codeless.store.session.head` | yes | `$.incarnation`, `$.last_run_state.awaits.since_ms`, `$.updated_at` |
| `auth-required-codeless.store.subscribe.paused` | yes | `$.cursor.digest`, `$.cursor.incarnation`, `$.unit.awaits.since_ms`, `$.unit.error.provider_code` |
| `terminal-unknown-code.legacy.run.result` | yes | `$.error.provider_code` |
| `terminal-unknown-code.legacy.run.status` | yes | `$.error.provider_code` |
| `terminal-unknown-code.legacy.session.head` | yes | `$.head.digest`, `$.head.incarnation`, `$.incarnation`, `$.last_run_error.provider_code`, `$.updated_at` |
| `terminal-unknown-code.legacy.subscribe.error` | yes | `$.cursor.digest`, `$.cursor.incarnation`, `$.unit.error.provider_code` |
| `terminal-unknown-code.legacy.subscribe.run_finished` | yes | `$.cursor.digest`, `$.cursor.incarnation` |
| `terminal-unknown-code.store.run.result` | yes | `$.awaits`, `$.error.provider_code` |
| `terminal-unknown-code.store.run.status` | yes | `$.awaits`, `$.error.provider_code` |
| `terminal-unknown-code.store.session.head` | yes | `$.incarnation`, `$.last_run_error.provider_code`, `$.last_run_state.awaits`, `$.updated_at` |
| `terminal-unknown-code.store.subscribe.error` | yes | `$.cursor.digest`, `$.cursor.incarnation`, `$.unit.awaits`, `$.unit.error.provider_code` |
| `terminal-unknown-code.store.subscribe.run_finished` | yes | `$.cursor.digest`, `$.cursor.incarnation`, `$.unit.awaits` |
| `transient-codeless.legacy.run.result` | yes | `$.error.provider_code` |
| `transient-codeless.legacy.run.status` | yes | `$.error.provider_code` |
| `transient-codeless.legacy.session.head` | yes | `$.head.digest`, `$.head.incarnation`, `$.incarnation`, `$.last_run_error.provider_code`, `$.updated_at` |
| `transient-codeless.legacy.subscribe.error` | yes | `$.cursor.digest`, `$.cursor.incarnation`, `$.unit.error.provider_code` |
| `transient-codeless.legacy.subscribe.run_finished` | yes | `$.cursor.digest`, `$.cursor.incarnation` |
| `transient-codeless.store.run.result` | yes | `$.awaits`, `$.error.provider_code` |
| `transient-codeless.store.run.status` | yes | `$.awaits`, `$.error.provider_code` |
| `transient-codeless.store.session.head` | yes | `$.incarnation`, `$.last_run_error.provider_code`, `$.last_run_state.awaits`, `$.updated_at` |
| `transient-codeless.store.subscribe.error` | yes | `$.cursor.digest`, `$.cursor.incarnation`, `$.unit.awaits`, `$.unit.error.provider_code` |
| `transient-codeless.store.subscribe.run_finished` | yes | `$.cursor.digest`, `$.cursor.incarnation`, `$.unit.awaits` |
