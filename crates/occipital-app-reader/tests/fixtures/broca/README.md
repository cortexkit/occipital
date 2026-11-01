# Broca entry projection fixtures

Copied without edits from prefrontal-core commit
`a13318fba1581db2776e91f4954744f35dbbbef6`:

- `crates/prefrontal-core-module/tests/fixtures/transcript-page/broca-tool-calls-v1.json`
- `crates/prefrontal-core-module/tests/fixtures/broca-session-read-tool-blocks-live.json`

The pure `broca_entries` tests port the compact row projection checks, including
tool result pairing, argument budgets, typed parts and private reasoning omission.
The first fixture pins the full compact row bytes as JSON values; the second is
the existing sanitized tool-block fixture (a read of `.` that returns
`{"kind":"error","message":"tool plane unavailable"}`).
