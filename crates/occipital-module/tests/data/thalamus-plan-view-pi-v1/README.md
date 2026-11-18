# Pi plan view reader samples (v1)

These are the runner's raw success reply bodies (the `result` envelope, without the
frame header) from `real_hosted_pi_plan_view_follows_request_tool_order`, recorded
with Pi 1.5.2 and llm-runner 0.8.62 at `bc4a54a4049664b0763e0c63eabb9079e84b6119`.
The test runs a hosted Pi against a loopback model and synthetic plan providers; no
login, personal data, or production prompts are used.

| Request on the bound Pi route | Reply | What it shows |
| --- | --- | --- |
| `role.describe {}` | [`role-describe.json`](role-describe.json) | The `plan_view` capability and `session.plan_view` op. |
| `session.plan_view {}` | [`plan-view.reply.json`](plan-view.reply.json) | The applied plan after the attach watch reports the tool order, before any hosted model request (so no `newest_step`). |

The test compares the view with the next request observed by the loopback model.
The plan lists `bash`, `edit`, `read`; Pi offers `read`, `bash`, `edit` first. The
test also confirms that a refresh adding a tool at the front of the plan appends
it to both Pi's request and the plan-view reply, without reordering the existing tools.

To record to a new directory on a Linux builder, from the repository root:

```sh
npm ci --prefix pi-extension/thalamus --cache "$TMPDIR/npm-cache"
cargo build --locked -p thalamus-harness-module --features test-support --bin ck-thalamus-harness
PI_BIN=$PWD/pi-extension/thalamus/node_modules/.bin/pi \
  THALAMUS_TEST_HOLDER_IMAGE=$PWD/target/debug/ck-thalamus-harness \
  THALAMUS_PI_PLAN_VIEW_SAMPLES=$PWD/new-pi-plan-view-samples \
  cargo nextest run --locked -p thalamus-harness-module --features test-support --lib \
    --run-ignored only -E 'test(real_hosted_pi_plan_view_follows_request_tool_order)'
```

Verify the published files with `sha256sum -c SHA256SUMS` in this directory.
