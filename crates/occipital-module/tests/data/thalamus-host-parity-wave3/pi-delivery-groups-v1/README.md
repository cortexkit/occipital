# Pi delivery groups v1

These are runner reply bodies and subscription frames from real Pi 1.6.0 hosted by the module with `test-support`, using a loopback stand-in model and tool provider. The JSON is pretty-printed; the keys surrounding the individual replies name the calls that produced them, not additional wire fields. IDs, cursors, and times are those of the test sessions. `role-describe.json` is the staged test-build description, not a shipping promise.

| File | What it shows | Main readers |
|---|---|---|
| `role-describe.json` | `retract`, `interrupt`, `user_commands`, `streaming`, `retention`; retention limits and owner-relay operations; no `media_only_send`. | All callers |
| `retract.json` | A submission id on a pending send, `retracted`, `not_pending`, `already_started`, and the keyed retry's `send_retracted` refusal. The retained send's landed receipt no longer has a submission id. | Prefrontal |
| `interrupt.json` | Sender grant, pending state during a running tool, refusal to retract a stop already underway, the cancelled run, and the succeeding run's delivery and completion. | Prefrontal |
| `interrupt-with-steer.json` | Pi cancels the old run; an untaken steer starts a distinct turn before the interrupting message starts its own. OpenCode 2.3.2 sent the same prompt order but attributed the steer as `step` on the previous run. Pi preserves its native finish reason, `aborted` rather than OpenCode's `error`. | Prefrontal, Occipital |
| `user-commands.json` | Frozen commands, a private invocation reply, and private status. The same invocation id does not invoke the provider twice; its input, name, and reply do not reach Pi's journal, transcript reads or the next model request. | Prefrontal |
| `streaming.json` | A read page with its `head`, a request resuming at that `head`, the durable controls strictly after it, and the later read page. Provisional live IDs and display deltas are a separate opt-in and do not replay. | Occipital, apps |
| `retention.json` | A listed head refuses a new retention value. A worker kept for one second after its last activity returns `expired` and the same expiry time from reads, run operations, and retract. Pi's billing journal still follows the existing Astrocyte acknowledgement policy. | Prefrontal |

Record these files by building the holder with `cargo build --locked -p thalamus-harness-module --features test-support --bin ck-thalamus-harness`, setting `PI_BIN` to the pinned `pi-extension/thalamus/node_modules/.bin/pi`, `THALAMUS_TEST_HOLDER_IMAGE` to `target/debug/ck-thalamus-harness`, and `THALAMUS_PI_DELIVERY_SAMPLES` to the output directory. Run `cargo nextest run --locked -p thalamus-harness-module --features test-support --lib --run-ignored only -E 'test(/delivery_groups::real_pi_/)'` on Linux. Verify the captures with `shasum -a 256 -c SHA256SUMS`.
