# Pi run limits v1

These are JSON replies recorded by the hosted Pi 1.6.0 tests with `--features test-support`, on Linux, using the repository’s extension, a loopback Anthropic Messages provider and the stand-in sign-in plugin. No credentials or live stores were used. The files preserve the full answer objects, pretty-printed; request and run IDs are from the test runs.

| File | What to check |
| --- | --- |
| `role-describe.json` | Pi lists `max_steps` and `max_output_tokens` in the test build. |
| `pi-surface.json` | Before the capped run’s first request, the surface says to ask for limits and carries the cap of 19. |
| `pi-step.json` | The second model request asks again and gets the same cap. `asks.every_step` instructs the extension to ask before further requests. |
| `run-result-max-steps.json` | The run ends `max_steps` with `finish: tool_calls`, after one model response and no tool dispatch. Pi’s refused continuation is not a model step. |

To record the answers again, run the ignored `real_pi_run_limits_` tests with `THALAMUS_PRINT_PI_RUN_LIMIT_SAMPLES=1`, the holder image built with `--features test-support`, and `PI_BIN` set to Pi 1.6.0. The lines beginning `PI_RUN_LIMIT_SAMPLE` name the sample and give its JSON. Verify the recorded files with `shasum -a 256 -c SHA256SUMS` from this directory.
