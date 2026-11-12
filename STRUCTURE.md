# Codebase Structure

A Cargo workspace of Rust crates, plus the Swift package the phone links. This file is a map; to find a specific symbol or file, search the code rather than this document. `ARCHITECTURE.md` explains how the parts work together.

## Workspace

```text
crates/
  occipital-module/      the ck-occipital daemon module: app reader, sends, uploads, tool display, notices, reply teaching
  occipital-app-reader/  the transcript engine: app rows, pages, claims, live-text ring, watches (no daemon code)
  occipital-lang/        the interface language: parser, checker, evaluator, layout, fact wording (cortexkit-occipital-lang)
  occipital-ffi/         the C interface apps embed, with its round-trip vectors
  occipital-registry/    the tool template contract and the occipital-check command (cortexkit-occipital-registry)
  occipital-refs/        issue and pull request mentions for links, no dependencies (cortexkit-occipital-refs)
  occipital-files/       no-replace move, file identity and free space, for placing uploads
ports/swift/             OccipitalCore.xcframework (built, committed) and the OccipitalKit sources
Package.swift            the OccipitalKit Swift package
scripts/                 check.sh, build-xcframework.sh, xcframework-source-hash.sh
```

`.cortexkit/` is git-ignored: internal design notes, scratch and saved answers never reach the repository.

## `crates/occipital-module/src`

- `main.rs`, `startup.rs`: the command line (`--manifest`, `--version`, `--subc`), the launch nonce, logging and exit codes.
- `serve.rs`: the module handler on `subc-client-rs`: opening the stores at HELLO_ACK, routing requests, watch pools, health, and the tool-provider route.
- `manifest.rs`: the module manifest, capabilities and catalog.
- `app_reader.rs`: the app operations' names, admission and dispatch.
- `app_reader/broca.rs`: reading any runner through the llm-runner contract (heads, pages, claims, live events, display facts). Despite the name it serves every runner.
- `app_reader/resolve.rs`: Entorhinal lookups and which harnesses each runner hosts.
- `app_reader/route.rs`: each app request's path back to the app (the `Exchange`), and reads and watches run over it.
- `app_reader/observe.rs`: read deadlines, request logging and the slow-request warning.
- `app_reader/display_hub.rs`: one live subscription per open chat, shared by its watches.
- `app_reader/send.rs`, `send/`: sends, Stop and Stop everything. `send_store.rs` keeps the receipts that answer a repeated send; `send_queue.rs` the sends a runner accepted but hasn't stored, or didn't deliver.
- `app_reader/uploads.rs`, `uploads/`: uploads, their store, and placing files in a project.
- `display.rs`, `display/`: `display.write`, the display stores, view fetching, call pins, the apps' tool reads, model names, and `display/daemon.rs`, the connection those view fetches and name lookups call other modules on (the app reader connects on its own to call runners and Entorhinal, in `app_reader/broca.rs`).
- `notices.rs`: riding notices, their store, and the apps' notice reads.
- `notices/wakes.rs`: wake records in the same file: the `display.write` wake kind, the sends page reads name, and `session.wake_records`.
- `media.rs`: image reads and thumbnails.
- `request_context.rs`: the plan's system text and tools, from the runner's `session.plan_view`.
- `commands.rs`: the person's commands: listing the modules that declare `user-commands/v1`, and relaying previews and runs with the caller's verified facts.
- `store.rs`, `backup.rs`: `store.db` and the Engram backup catalog.
- `system_text.rs`: the reply teaching. `show_ui.rs` and `native.rs` exist only with the `show-ui` feature.

## `crates/occipital-app-reader/src`

- `broca_entries.rs`: stored messages to app rows, for every harness. `tool_input.rs` and `tool_times.rs` shape tool parts.
- `page.rs`, `claim.rs`, `source.rs`, `details.rs`: pages, claims and session facts.
- `display.rs`, `display_events.rs`, `display_watch.rs`: the live-text ring and its frames, including tool frames.
- `transcript_watch.rs`, `watches.rs`, `signals.rs`: transcript watches, their per-route bookkeeping and per-session wakes. `activity.rs`: activity tokens.
- `send.rs`: the send request and its codes, which the module's send path uses.

## Tests and sample data

- **Unit tests** sit beside the code: a `tests.rs` or `tests/` next to the module they cover. Each crate's `tests/` holds its integration tests (`phone_reader_samples.rs` writes and checks the app sample sets).
- **App sample sets** live in `crates/occipital-app-reader/test-vectors/<name>/`, each with a README and SHA256SUMS; a set the apps pin is tagged with its folder's name. The apps pin the tags, so a set is never edited once tagged; a change ships as the next version (`-v2`, `-v3`).
- **Runner recordings** live in `crates/occipital-module/tests/data/<recording>/`, copied byte for byte from the runner that recorded them, and are served through Occipital's real handlers in tests.
- **Generated vectors:** `crates/occipital-ffi/vectors/ffi/` and `crates/occipital-refs/vectors/text-references-v1/` are regenerated by their tests with `OCCIPITAL_BLESS_VECTORS=1`; without it a test fails when a regeneration would change a byte.

## Where to add new code

- **An app operation:** its name in one of `app_reader.rs`'s method lists, which `operations()` publishes as a query or a mutation and `admit` checks, its handler in the feature's own module, a sample set, and a line in the module guide's operation table. The phone needs a Callosum grant for it.
- **A new runner or harness:** the runner's hosted harnesses in `app_reader/resolve.rs`, the row shape in `broca_entries.rs` (through the runner contract's display projection when one exists), and tests on the runner's own recorded pages.
- **A new field on app rows or session facts:** `broca_entries.rs` or `source.rs`, and a new tagged sample set.
- **A store table or column:** a new migration on that store's chain. Then an older build refuses the store, so rolling back needs the store files copied aside before the new build first starts; the request to place the build must say so.
- **A C entry point:** a module in `crates/occipital-ffi/src/` through `boundary.rs`, its text in `include/occipital.h`, a Swift wrapper in `ports/swift/Sources/OccipitalKit/`, an FFI vector, and a rebuilt phone library.
- **A catalog component:** the catalog JSON in `crates/occipital-lang/catalog/`, layout and checks in the library, the digest re-pinned in `catalog.rs`, and the old digest kept as an alias.
- **A kind of mention:** a rule in `crates/occipital-refs/src/lib.rs`, its spec in that crate's README, `RULES_VERSION` raised, and a new vector set.

## Conventions

- **Builds:** Rust builds and tests run on the Linux build server; release builds of `ck-occipital` on the build Mac, from a tree with no modified tracked files. `scripts/check.sh` runs on the Mac, because it reads the local git index.
- **No panics in shipped code:** the language library, the C interface, the registry and the module deny `unwrap`, `expect` and `panic!` outside tests. A broken assumption becomes a status or an error code.
- **Logging:** through `cortexkit-log` only. Never log message text, file names or credentials.
- **Public wording:** the repository is public. Prose calls the phone and desktop apps "the apps"; module names are fine.
