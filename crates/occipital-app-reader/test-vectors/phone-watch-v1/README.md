# Display watch samples

These are the four `session.display_watch` samples from the `phone-watch-v1.3`
sample set the phone and desktop apps pin. They were captured when Prefrontal core served
the apps' display watch, and copied byte for byte from the Prefrontal repository
(`test-vectors/phone-watch-v1/` at commit
`9f124a33b318d309e66b3524cd964a1861a6d9ac`), together with the code this crate
ports. `SHA256SUMS` keeps the original four checksum rows. The set's other watch
operations aren't part of this subset.

The fake-host test
`display_watch::tests::display_watch_waits_for_a_projection_then_paces_frames_and_marks_oversize`
drives the event mapper, batcher and projection, then compares the resulting
watch notices with all four samples. Run it with:

```sh
cargo test --locked -p thalamus-app-reader display_watch_waits_for_a_projection_then_paces_frames_and_marks_oversize
```

The display-read fixture at `../../tests/fixtures/harness_display/contract_vectors.json`
was copied unchanged from the same commit
(`crates/prefrontal-core-module/tests/fixtures/harness_display/`). Its provenance fields describe the
server that captured it; the test here drives this crate's own read parser and
projection against it.
