# `run.status` step liveness reader samples

Raw response bodies from the real serve connection, with loopback HTTP and a held tool.
Regenerate with `BROCA_STEP_LIVENESS_SAMPLE_DIR` pointing at this directory and the
`step_liveness_reader_samples` test. Timestamps are Unix milliseconds and vary per run.
Verify with `shasum -a 256 -c SHA256SUMS`.
