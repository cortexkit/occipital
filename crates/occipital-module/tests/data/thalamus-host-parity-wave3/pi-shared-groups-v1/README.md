# Pi shared groups v1

These are `llm-runner/v1` bodies captured on Linux from a real hosted Pi 1.6.0 and this repository’s Thalamus extension, with `--features test-support`. The fixture uses a loopback model and an in-memory tool-provider catalog, not a live account or a person’s session. Listed heads exercise the owner/reader grants, directives, rich UI and undo. Sessions started by a send exercise default system text and per-run accounting.

The files contain the JSON bodies, without the bus envelopes. `role-describe.json` is the **test build** response; the shipping declaration is unchanged. IDs, cursors and times are as emitted by the fixture. The two rich UI `session.read` bodies replace the Linux builder’s private working and checkout roots with `/scratch` and `/repo` in Pi’s initial system entry; the rest of the bodies are unchanged apart from JSON pretty-printing.

| Group | Bodies | What to check |
| --- | --- | --- |
| `head_sender` | `head-sender-{prefrontal-core,stopper,notifier,occipital}.json` | Owner grant, plan sender with interrupt but without `idle_start`, plan sender with `idle_start` but without interrupt, and a reader with no grant. |
| `turn_directives` | `directives-{refusal,run-status,run-result}.json` | An unrecognized name refuses `invalid_params` on `directives`. The completed run that requested `wake` reports a 90-second wake with a note on both reads; `final_message.text` still holds the tag. |
| `rich_ui_stamp` | `rich-ui-{run-started,head,history-display,read-default,read-opted-in}.json` | The stamp comes from the system-text catalog response. It is on the run start, head, and the tool-calling and final assistant rows for that run only. Raw read entries include it only when requested. |
| `system_text_default` | `system-text-default-{refusal,send,run-status,provider-request}.json` | The same send is refused before any fetch with `system_text_preset_required`, then completes when the catalog advertises `system-text-default/v1`. The `tool.catalog` request’s `arguments.system_text` does not contain a preset; the top-level `arguments.preset` belongs to the tool item, not to the text. The admitted default system text reaches the model request. |
| `run_usage` | `usage-run-{status,result}.json` | Both reads carry two steps under the same model, 22 input and 4 output tokens. |
| `session_usage` | `session-usage-head-{before,after}-undo.json` | Pi’s own tree navigation removes the second turn from visible history and advances `history_version`; spend and its first counted run (`since`) stay unchanged. |

Pi 1.6.0’s OpenAI-completions adapter writes zeros for absent cache and reasoning counts before journaling the message (`dist/api/openai-completions.js`, `parseChunkUsage`). The recorded usage follows those journal counts. An absent provider field and a provider-reported zero are indistinguishable here; preserving that distinction for §6.4 would require evidence upstream of the journal.

## Re-record from the worktree

On the Linux builder, install the pinned Pi and build the holder with the same test seams as the tests:

```sh
npm ci --ignore-scripts --no-audit --no-fund --cache "${TMPDIR:-/tmp}/npm-cache" --prefix pi-extension/thalamus
cargo build --locked -p thalamus-harness-module --features test-support --bin ck-thalamus-harness
export PI_BIN=$PWD/pi-extension/thalamus/node_modules/.bin/pi
export THALAMUS_TEST_HOLDER_IMAGE=$PWD/target/debug/ck-thalamus-harness
export THALAMUS_PI_SHARED_GROUPS_SAMPLES=$PWD/target/pi-shared-samples
mkdir -p target/pi-shared-nextest
printf '[profile.default]\nslow-timeout = { period = "60s", terminate-after = 2 }\n' > target/pi-shared-nextest/config.toml
cargo nextest run --config-file target/pi-shared-nextest/config.toml --locked \
  -p thalamus-harness-module --features test-support --lib --run-ignored only \
  --no-fail-fast -E 'test(real_pi_shared_groups_)'
```

Return the JSON files before the builder discards the job. Redact the two fixture path prefixes noted above and verify the resulting set with `shasum -a 256 -c SHA256SUMS` in this directory.
