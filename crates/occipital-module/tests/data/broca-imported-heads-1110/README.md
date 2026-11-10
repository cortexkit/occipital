# Imported head reads from broca

Recorded from the source tree committed as broca
`9d4ff8d8091c7bc01d07620ea2615e79d8601399`.

Inputs are the real Thalamus **session-export-v2** recording in
`../head-import-entries-1107/thalamus/` (OpenCode 2.2.3 and Pi 1.5.0,
Thalamus commit `204a6f27e`). All nine files passed that folder's
`SHA256SUMS` before import. Export SHA-256:

- OpenCode: `e5d0f5fd584bb79b47bfd194395bc6c2daef22193c79c21fb26d9d84d4629b45`
- Pi: `520a4b4d878916b646abb1766e065f73ef69158527d7c594e3b612b22f2e931c`

## Files and requests

The same three files exist under `opencode2/` and `pi/`:

| File | Method | Exact request params |
| --- | --- | --- |
| `read-default-1.json` | `session.read` | `{}` |
| `read-display-facts-1.json` | `session.read` | `{"include_display_facts":true}` |
| `history-display-1.json` | `session.history` | `{"form":"display"}` |

Each file is the **exact JSON body of broca's Response frame**, including
its outer `{"result":...}` envelope. Bytes were copied directly from
`Frame.body`; they were not pretty-printed, scrubbed, reserialized, or given
a trailing newline. The transport's binary header is not included.
`SHA256SUMS` covers all six JSON files and this README, not itself.

Both default pages cover the entire imported transcript, starting at ordinal
1, with no continuation: **13 messages for OpenCode** and **12 for Pi**.
The display-fact pages cover the same ranges. OpenCode's tool results split
from the two source assistant rows become two additional broca rows. Pi's
system message is dropped by import; its journal header and model/thinking
bookkeeping are not transcript messages. This is broca's imported transcript,
not a relabeling of Thalamus's `session.entries` numbering.

## Calls and notices

Both default and opted-in reads retain original keys and carriers in
`messages[].tool_calls`. The opted-in pages also retain them in `tool_times`;
display history retains them on tool-call parts. The samples show:

| Host | Assistant ordinal | Original `call_key` | `call_carrier` |
| --- | --- | --- | --- |
| OpenCode | 2 | `msg_1b5fa8dea0010w1FHaCCIhgG5k/call_probe` | `thalamus-harness` |
| OpenCode | 11 | `msg_1b5fa9c0a001BvMuQLEmMEQeV3/call_each` | `thalamus-harness` |
| Pi | 2 | `5b9fb962/call_probe_1` | `thalamus-pi` |

Both exports contain real notices:

- **OpenCode**, tool-result ordinal **3**, part **0**: `notices[0]` names
  the original `call_probe` key above and carrier `thalamus-harness`,
  `send_id: "ride"`, `sender: "magic-context"`, `text: "RIDE_TEXT"`,
  `delivered_as: "tool_result"`. Ordinal **8**, mid
  `msg_1b5fa9a4000182QkAmT7kVMImp`, is the separate `NOTE 3` user row with
  `notice: true` and `sender: "magic-context"`.
- **Pi**, tool-result ordinal **3**, mid `38128df8`, part **0**:
  `notices[0]` names `5b9fb962/call_probe_1`, carrier `thalamus-pi`,
  `send_id: "ride"`, `sender: "notifier"`, `text: "RIDE_REMINDER"`,
  `delivered_as: "tool_result"`. Ordinal **7**, mid `e55a56e1`, is the
  separate `NOTE 3` user row with `notice: true` and `sender: "notifier"`.

These notice facts appear in the display-fact reads and display history.
Default reads intentionally omit display-only `notices`, `notice`, and
`tool_times` facts. Notice text is not joined into the native tool output.
Neither export needed a fabricated notice.

## Capture and regression rig

`crates/broca-module-serve/tests/it/head_import_serve.rs` contains
`thalamus_imported_heads_keep_calls_and_notices_in_wire_reads`. It reuses the
existing import/serve rig, loads each export plus its sends side file and
export metadata, imports through `session.import`, and reads over
`serve_connection` (the `session.read` handler and `send_json_response`, not a
hand-built response). Each host has a fresh disposable new-storage state
root. Bind session names `runner` / `head` and source reference
`thalamus:sample-head` are synthetic; source session ids and call keys come
from the throwaway Thalamus recording. No production store or path is used.
Fresh-state incarnation, cursor digest, and broca-added mids may differ on a
new recording; the original imported call keys do not.

To record from a repository checkout, substitute scratch paths below:

```sh
export CARGO_BUILD_JOBS=8
export BROCA_THALAMUS_EXPORT_DIR=/scratch/head-import-entries-1107/thalamus
export BROCA_IMPORTED_READ_RECORD_DIR=/scratch/imported-head-reads-1110
nice -n 10 env CARGO_INCREMENTAL=0 cargo nextest run --locked \
  -p broca-module-serve --test it \
  -E 'test(thalamus_imported_heads_keep_calls_and_notices_in_wire_reads)'
```

Without the export override, the regression uses broca's checked-in v1
exports; without the recording override, it writes no sample files. The test
also checks a one-result page starting at ordinal 3 so the notice survives a
page boundary. The supplied v2 capture passed locally; the default v1
regression and native read/subscribe byte pins passed on Linux.
