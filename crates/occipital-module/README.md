# occipital-module

The `ck-occipital` binary runs under the CortexKit daemon. It serves three things:

- **The app reader**, on its management surface: the operations the phone and
  desktop apps use for a chat. See [App reader](#app-reader).
- **Tool display data** (`tool-display/v1`): what tool providers write for
  their calls, and the apps' reads of it. See [Tool display](#tool-display).
- **The rich-UI reply teaching**, as a tool provider: Core copies the teaching
  text and its `rich_ui` stamp from the `tool.catalog` answer's `system_text`
  into plans for runners that stamp replies. The default build's catalog lists
  no tools.

Build the first-placement binary with:

```sh
cargo build --release -p occipital-module --bin ck-occipital
```

The optional `show-ui` feature enables the native `show_ui` tool and its
admission checks, alongside the same reply teaching:

```sh
cargo build --release -p occipital-module --bin ck-occipital --features show-ui
```

The phone cannot display `show_ui` records yet; enable the feature only once the
phone client supports those records. Neither build declares `rich-ui-carrier/v1`
or opens carrier routes; carrier support is not implemented yet.

## Running it

```text
ck-occipital --manifest      print the subc manifest, with the build's git commit, and exit 0
ck-occipital --version       print the crate version and full build commit, and exit 0
ck-occipital --subc <file>    serve (how the daemon starts it)
```

`--manifest` and `--version` are answered before anything else. Neither reads the
launch nonce, opens the store or connects to the daemon, so the placement script
can probe a staged binary. Serving also needs `--subc <file>`: the app reader
opens its own connections to the daemon through that file. `--version` prints
`ck-occipital <crate-version> <full-git-commit>` on one line (or `unknown` for the
commit when no usable commit could be derived).
In the manifest, the commit appears as `provenance.build_git_sha` for a
build from a clean tree, and as `provenance.build_git_sha_absence_reason`
(`declined_dirty`, `no_git_dir` or `never_derived`) otherwise.

A served module writes the fleet log through `cortexkit-log`: one file a day
under its data folder's `logs/`, filtered by `CK_LOG`. Each app request ends
with one summary line: the method, session, outcome and step timings, never
message text. It's at WARN for a refusal or deadline, at INFO for a send, a
Stop, an upload's finish or abort, or a read slower than 2 s, and at DEBUG for
other reads.

Every other start first reads the launch nonce from the descriptor the daemon
names in `SUBC_LAUNCH_NONCE_FD` (or, on Windows, the pipe named in
`SUBC_LAUNCH_NONCE_PIPE`). If none is named, the descriptor is closed or it
cannot be read, the binary writes one stderr line starting
`ck-occipital: launch nonce unavailable:` and exits 2. It never uses the
`SUBC_LAUNCH_NONCE` environment copy.

## Daemon configuration

Occipital is installed as a reserved module: only the process the daemon spawned
may register the id `occipital`. The module id is unchanged by the binary rename.
Its entry in `<config home>/cortexkit/subc.jsonc`:

```jsonc
{
  "version": 1,
  "modules": {
    "occipital": {
      "program": "/path/to/ck-occipital",
      "reserved": true
    }
  }
}
```

Neither build declares a `rich-ui-carrier/v1` capability, so the entry needs no
`reserved_capabilities`.

## App reader

The apps reach these operations through Callosum, and the desktop app directly:

| Operation | Kind |
| --- | --- |
| `session.transcript_page`, `session.display_read`, `session.details`, `session.media` | read |
| `display.views`, `display.template`, `session.tool_records`, `session.tool_detail` | read |
| `session.notices`, `session.wake_records` | read |
| `session.transcript_watch`, `session.display_watch`, `session.tool_records_watch`, `session.notices_watch` | watch |
| `session.enqueue_user_message`, `session.stop`, `session.stop_all` | send |
| `upload.begin`, `upload.chunk`, `upload.status`, `upload.finish`, `upload.abort`, `upload.place` | upload |

An uploaded file goes into the model's prompt or into the session's project. An
app that asks for the prompt (`prompt: true` on `upload.begin`) gets it when the
session's runner takes uploads and its model accepts the file's type; the send
that names the file copies it to the runner. Otherwise the file is placed in the
project, and the app puts its path in the message text. `upload.place` is the
app's fallback when a send naming a prompt file is refused.

An image a chat references is read with `session.media`: a thumbnail Occipital
makes from the runner's bytes (at most 1024 pixels on its longest side, upright),
or the original in pieces. Image file parts on a page carry the image's digest and,
once Occipital has read its first bytes, its drawn width and height.

Admission reads only what the daemon attests on the route: a Callosum route
(`reserved:callosum`, with the device label `fed-<class>:<fingerprint>` it
stamps), or, while the desktop app is in development, a plain local caller.
Everything else is refused `carrier_op_not_permitted` before the request is
read further.

A session is found through Entorhinal by its runner and session id. Broca
sessions are read from Broca; OpenCode 2, Claude Code and Pi sessions from
Thalamus's runner (`thalamus-harness`, harnesses `opencode2`, `claude-code`
and `pi`), and Pi sessions also from Thalamus's Pi runner (`thalamus-pi`)
until they move; all through the llm-runner contract. A harness recorded on a
runner that doesn't host it is refused. Broca's stored messages are decoded
directly; the others go through the contract's display projections first, so
every harness reaches the apps in one row shape. The module calls
them, and Entorhinal, as `reserved:occipital`, proved by its launch nonce; the
runners check that principal against the session's plan before taking a send
or a Stop. Any other runner's session is refused `session_unknown`, naming that
runner.

Watches wait in their own pool of handler slots, so open chats never hold up
reads and sends. Each watch ends with a stream end when the app cancels it,
when its route goes, or when the daemon drains the module.

## Tool display

Tool providers write each call's record, completion and detail with
`display.write` on Occipital's tool-provider route, keyed by the writer the
daemon attests. Occipital fetches each provider's view of a tool version
(`tool.views`) off the write path, and again when the provider's version
changes. The apps read views, templates, records and details beside the
transcript pages, which carry each call's `callKey` and `schemaPin`. A write
joins a call only through a view its own writer serves for the pin the runner
stored for the call; Occipital keeps that pin while it builds pages. A
template that fails Occipital's checks is left out, and its calls draw as the
plain chip. The apps' wire is pinned by the sample set
`occipital-app-reader/test-vectors/phone-reader-tool-display-v1/`.

Records, completions, views and call pins live in `display.db` beside the
store, and details in `display-details.db`. Engram backs up `display.db`,
since a provider writes each record once; details are left out, since they
are large and can hold file contents. Records are kept 180 days and details 14
days, at most 512 MiB.

## Notices

A notice is a module's message to the agent, such as Magic Context's
reminders or core's status line. A runner delivers one either as its own
message, which is a transcript entry like any other, or riding the next tool
result, which makes no entry. Occipital keeps each riding notice it learns of,
from a page read's `notices` fact or the runner's `notice_delivered` event, in
`notices.db` beside the store. The apps read a call's notices with
`session.notices` and learn of new ones from `session.notices_watch`, beside
the pages, as `{sender, text, deliveredAs}` in delivery order. They are not
tool records: no tool provider writes them. Engram backs up `notices.db`,
since a runner may prune its own record of a notice. Notices are kept 180
days. The apps' wire is pinned by the sample set
`occipital-app-reader/test-vectors/phone-reader-notices-v1/`.

A wake record says what a wake held, so the apps draw it as chips. The waking
module writes it before the send, with `display.write` and `kind: "wake"` on
the tool-provider route, keyed by its own send id; the sender is the writer the
daemon attests. Occipital declares `wake-record-display/v1` and keeps each
record in `notices.db`, once and unchanged, for 180 days. A page read records
which send each message came from, from the runner's `sender` and `send_id`
read facts. The apps read records by message id with `session.wake_records`,
and a riding notice carries its send's record as `wake` in `session.notices`.
The wire is pinned by `phone-reader-wake-records-v1`.

## Store

The daemon names the store in HELLO_ACK's storage descriptor, normally
`<data_home>/cortexkit/occipital/store.db`. Occipital opens it with
cortexkit-store, which holds the single-writer lease and protects the folder
and file for the current user; every write goes through the store's fenced
write. The schema version is SQLite's `user_version`, and migrations only
move forward. If HELLO_ACK names no store, or the store cannot be opened, the
binary writes one stderr line starting `ck-occipital: store unavailable:` and
exits 1. A store written by a newer build is left untouched: the binary writes
one stderr line starting `ck-occipital: store version unsupported:` and exits 1.

Schema version 1 holds one table, `native_records`, the records of native
`show_ui` calls. Rows are never deleted.

The app reader keeps its own tables beside it, each with its own cortexkit-store
migration chain: the receipts that answer a repeated send (kept 48 hours), the
sends a runner holds or did not deliver, and the uploads in progress. Upload
contents wait in `upload-blobs/` in the store's folder until they are placed in
the session's project folder. Until HELLO_ACK has named the store, app requests are
refused `provider_unavailable`, which the apps retry.

At every start the module writes `engram-catalog.json` beside the store, so
Engram's nightly capture backs the store and `display.db` up whole while the
module runs and leaves `upload-blobs/` and `display-details.db` out. Serving an older restored copy is safe: the module
never sends a message again on its own, a repeated send from an app reaches
the runner under the same key and is answered with the existing send, and an upload
whose partial file is missing resumes from the bytes still there.

## Reply teaching (system text)

In both builds, a `tool.catalog` request with a `system_text` item that has neither
a preset nor params gets the reply teaching text: the display tags a reply may
use, generated by `occipital-lang` from the catalog's `teaching.reply` section.
The answer carries
`rich_ui: {dialect, catalog_digest}` from the language's version descriptor, so a
runner stamps the replies of every run that froze it and the phone draws them
with tags. The manifest declares `system-text-default/v1` because Occipital
serves no system-text presets; a preset or params on the item is refused as
`invalid_request` naming `system_text.preset` or `system_text.params`.
The teaching text and its dialect/catalog stamp are byte-for-byte identical
between builds. The answer's `tool_names` is empty by default and `["show_ui"]`
with the feature; the teaching is not rewritten to enable interactive calls.

## Native call admission (`show-ui` builds only)

Discovery (`role.describe` and `tool.catalog`) works without a scope stamp.
`show_ui` is admitted in the following order; within a step the first member in
name order is reported:

| Order | Code | Refusal |
| --- | --- | --- |
| 1 | `kind_unsupported` | `kind` is `html` |
| 2 | `route_unscoped` | No daemon-stamped route scope |
| 3 | `unknown_argument` | An argument outside the closed input schema |
| 4 | `argument_missing` | A required argument is absent |
| 5 | `argument_invalid` | An argument has an invalid type/value, canonical `data_columns` exceeds 1 MiB, or the reference descriptor is malformed/over 4,096 compact JSON bytes |
| 6 | `argument_forbidden` | `data_columns` without `data_format`, or `data_format` without `data` |
| 7 | `references_unavailable` | A descriptor names a nonempty set whose metadata requires the carrier |
| 8 | `data_reference_unknown` | `data` names a reference, but no records are available |

`output_refs` is an open descriptor object with required `set_id`, `mid`,
`dialect` (nonempty strings), `count` (unsigned integer), `digest` (64 lowercase
hex), and optional nonempty `call_key`. Unknown descriptor members are ignored.
The call does **not** contain the records array. Its records can only be paged
through `session.output_refs` by an admitted carrier, so nonempty sets are refused
until carrier support exists. Empty sets (and calls with no descriptor) prepare
an empty reference map, no data snapshot and no fetched bytes. Validation of
each reference's resolved, unresolved, too-large or held state
belongs to that later metadata-read path, not to call receipt.

The prepared call retains only the scope's owner, ref and epoch and the route's
principal as provenance. Bind identity fields are neither retained nor used for
admission. Native execution checks with `occipital-lang` inside an unwind
boundary. Only `CheckResult::ok()` can produce `stored`, and only after a fenced
write commits. Other check results produce `check_failed`; host errors, panics,
oversized results and failed writes produce `failed` with a stable code.

The complete tool-result/v1 envelope is capped at 32,768 UTF-8 bytes before
writing. Diagnostics in its serialized check are trimmed warnings before errors,
last first, preserving the outcome and increasing the omitted count. The original
check result is never modified. If no diagnostics remain and the envelope still
does not fit, the call returns `result_too_large` without storing anything.

Native records leave `data`, `data_format`, `data_columns` and `output_refs` NULL.
Reference metadata and data persistence require carrier-authorized paging, which
is not implemented here.
`cargo run --locked -p occipital-module --features show-ui --example show_ui_samples` regenerates
the samples in `test-vectors/show-ui-v1/`; tests compare their literal outcomes
and codes, and regenerate the deterministic envelopes for comparison. Record ids
in samples are normalized to a fixed valid id; production uses 128 OS-random bits.
The 32,769-byte `oversize_candidate` is labeled as a candidate, never a reply:
its matching call returns the small `failed`/`result_too_large` envelope instead.
