# Routing capacity wait samples

These are JSON bodies from one run of
`real_opencode2_routing_capacity_wait_resumes_the_same_run_on_its_account`,
using OpenCode 2.3.2, a stand-in auth plugin and loopback routing and model
providers. The first model request was admitted and ran. Routing refused the
second, the plugin waited until the named reset, and the same run's second
provider request went out on the held account.

* `accessor.request.json` and `accessor.reply.json`: the `/oc2/routing` request
  and response bodies when routing refused the second model request. The
  accessor opts in with `waits_for_capacity: true`; the refusal names only a
  reset time, not a relative delay.
* `route.account_for.request.json` and `route.account_for.re-ask.json`: the
  routing parameters for that second model request, before the pause and after
  the reset. The provider, model and conversation are unchanged. The bridge
  makes a fresh call and the relay uses a fresh admission ID for the re-ask.
* `run.status.paused.json`: the run's status before reset. `awaits` is a single
  object; `since_ms` is when the relay received the refusal and `next_ms` is the
  instant routing named. The usage is from the first, completed model step.

The stand-in router uses the `account-for.reply.json` body from the STRIATUM
routing module's tag `route-concurrency-full-v1.2`, commit `05c36d11`, copied byte-for-byte in
`../striatum-route-concurrency-full-v1.2/`. The real-host fixture substitutes
only the absolute reset timestamp because it uses the live test clock.

Set `THALAMUS_ROUTING_CAPACITY_RECORD=1` and run the named ignored test with
nextest's `--success-output immediate` to print the bodies. The IDs are minted
by the test hosts; there are no real accounts, credentials or user messages
in the files. `SHA256SUMS` pins the five JSON bodies.
