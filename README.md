# Occipital

Occipital is how data reaches and renders in the CortexKit phone and desktop apps. It turns what agents and tools produce into what the apps show: a chart or a callout in a reply, a tool's own view of a command it ran.

It holds:

- **The language library** (`crates/occipital-lang`). One Rust library parses, checks and evaluates the closed, declarative interface language, so an interface is checked and drawn the same way in every app and in this module. It has no panic paths, because apps embed it. Parsing builds on [`cortexkit-rich-ui-syntax`](https://crates.io/crates/cortexkit-rich-ui-syntax).
- **The app binding** (`crates/occipital-ffi`). A C interface to the library that turns any panic into an error code, packaged as a static xcframework for iOS. The Swift package in `Package.swift` wraps it as `OccipitalKit`.
- **The tool template contract** (`crates/occipital-registry`, published as `cortexkit-occipital-registry`). The `occipital.template/1` document with its canonical JSON and digest, the checks a tool provider's view must pass, and the `occipital-check` command providers run in CI. The [template guide](crates/occipital-registry/TEMPLATES.md) explains how to write one.
- **The `ck-occipital` binary** (`crates/occipital-module`). It runs on the person's computer as the reserved `occipital` module under the CortexKit daemon. It serves the apps' chat operations (transcript pages, live text, sends, Stop and uploads) and the reply teaching, the part of a head's system prompt that teaches the interface tags, which Core adds for runners that stamp replies. See the [module guide](crates/occipital-module/README.md).
- **The transcript engine** (`crates/occipital-app-reader`). The pages, claims, live-text ring and watches behind the apps' chat reads, independent of the daemon.
- **Mentions for links** (`crates/occipital-refs`, published as `cortexkit-occipital-refs`). Finds "Issue N", "PR #N" and `owner/repo#N` in text so the apps can draw them as links, without changing the text.
- **File placement helpers** (`crates/occipital-files`). A no-replace move, a file's identity and free space, on Windows, Linux and macOS, for placing uploads.

## Where it's going

Occipital is becoming the one module the apps use for a chat:
- **Reading:** it reads every llm-runner through the shared runner contract and composes the pages and live text the apps show.
- **Writing:** it forwards sends, uploads and stop to the runner hosting the session.
- **Tool views:** it keeps the display data tool providers write for each call, such as a command's exit code or an edit's diff. Runners keep only what the model reads and writes.

Reading and writing are served for Broca sessions and for Thalamus's OpenCode 2, Claude Code and Pi sessions, and both apps read their chats through Occipital. Tool views are served for every provider that publishes them, and the notices modules send agents are kept beside the chats.

`ARCHITECTURE.md` explains how the parts work together, and `STRUCTURE.md` maps the code.

## Build

Rust runs on the Linux build server, never with cargo on the Mac that builds the apps.

```sh
cargo build --release -p occipital-module --bin ck-occipital
cargo test --workspace
cargo test -p occipital-module --features show-ui
```
