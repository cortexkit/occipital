# Architecture

Occipital is how agents' work reaches and renders in the CortexKit phone and desktop apps. It is two things at once:

- **A daemon module, `ck-occipital`** (`crates/occipital-module`), the reserved `occipital` module of the subconscious daemon (subc). It is the one module the apps use for a chat: it reads every session from the llm-runner hosting it, composes the pages and live text the apps draw, forwards sends, Stop and uploads, keeps the display data tool providers write for each call, and keeps the notices modules send agents. It also serves the reply teaching, the part of a head's system prompt that teaches the interface tags.
- **The libraries the apps embed:** the interface language (`crates/occipital-lang`, published as `cortexkit-occipital-lang`), its C binding packaged for iOS (`crates/occipital-ffi`, wrapped in Swift as `OccipitalKit`), the tool template contract and checker (`crates/occipital-registry`, published as `cortexkit-occipital-registry`), and the mention finder for links (`crates/occipital-refs`, published as `cortexkit-occipital-refs`). The apps draw from these, so a reply, a tool row or a link is drawn the same way on the phone, on the desktop and in the module.

`STRUCTURE.md` has the directory map and where to add new code. The module guide, `crates/occipital-module/README.md`, lists every operation.

## Terms

- **Runner:** a module implementing the llm-runner contract, which hosts sessions and stores their messages. Broca is one; Thalamus's `thalamus-harness` hosts OpenCode 2, Claude Code and Pi sessions, and `thalamus-pi` hosts Pi sessions until they move. Occipital talks to runners only through the published contract (`cortexkit-role-llm-runner`).
- **Head, worker:** a head is an agent a person works with; a worker is one an agent started. Both are sessions on a runner, and the apps read both.
- **Harness:** the shape of a session's stored messages (`broca`, `opencode2`, `claude-code`, `pi`). A request names it; a request without one is read as Broca.
- **Page:** a window of a chat's messages as app rows, newest first or before an ordinal. Every page carries a **version**, `history:<n>:incarnation:<id>`, taken from the runner's history counter and log identity. The apps cache pages by version and reload a chat whose version changes.
- **Claim:** what an app sends back to resume a chat it holds: the version it holds and the newest message it has. The answer is `matched` (with any new rows), `diverged`, `cursor_expired` (with the whole page again) or `outside_window`.
- **Live text:** the words of a reply as the model writes them, before the runner stores it. A **display watch** streams them; the stored row replaces them once it lands.
- **Tool call identity:** each tool part carries `callKey` (the runner's key for the call, unique across that runner) and `schemaPin` (which version of the tool ran). A call moved to another runner with its history keeps its key, and the runner names the one that minted it as `call_carrier`.
- **View, template, record, completion, detail:** a tool provider's **view** of a tool version says how its calls draw: an `occipital.template/1` **template** for the row and the page, plus the shape of what it writes. For each call the provider writes a **record** (drawn on the row), a **completion** and a **detail** (drawn on the page).
- **Notice:** a module's message to an agent, such as a reminder or a status line. It reaches the agent either as its own message, which is a chat row like any other, or **riding** the next tool result, which makes no row.
- **Reply stamp:** the interface dialect and catalog digest a runner records on a head's reply, so the apps know which tags it may hold. Replies use catalog 1.0; tool templates use tool catalog 1.1.

## How the apps reach Occipital

The phone reaches the computer through Callosum, the federation module: every phone call arrives on a route stamped `reserved:callosum` with the device it came from. The desktop app connects to the daemon directly. Admission reads only what the daemon attests on the route: the route's principal, and the operator-app stamp that names the desktop. While the desktop is in development, a plain local caller is admitted too. Everything else is refused before the request is read further.

Occipital in turn calls runners, Entorhinal and tool providers as `reserved:occipital`, proved by its launch nonce. A runner admits Occipital's reads and sends by the session's plan, which core writes: Occipital is listed as a reader and as a sender, with the send modes the plan grants.

## Reading a chat

1. **Find the session.** Entorhinal maps the app's session id and harness to the runner hosting it. A harness recorded on a runner that doesn't host it is refused, never read in another harness's shape.
2. **Read the runner.** `session.head` gives the history counter, log identity, run state and send grant. `session.read` serves the newest window and exact reads, with display facts (the app's message ids, tool times, call identity, creation and finish times, notices, message marks); raw `session.history` serves older pages and anchors claims.
3. **Build rows.** `crates/occipital-app-reader` turns stored messages into app rows. Broca's messages are decoded directly; OpenCode 2, Claude Code and Pi messages go through the runner contract's own display projections first, so every harness reaches the apps in one row shape. Occipital builds rows from the runner's raw entries, never from text a runner pre-renders.
4. **Answer.** The page carries the session facts (run state, send grant, service tier, history intake, lost host) at its top level, so a page trimmed to fit still carries them.

A page's version moves only when the runner's history counter or log identity does. A change in how Occipital renders rows doesn't move it, so apps holding a chat keep their copy until it reloads.

## Live text and tool frames

Each open chat has one subscription to its runner's live events, shared by every app watching it (`app_reader/display_hub.rs`). Text arrives in a ring that batches deltas for about 300 ms; a watch opened late gets the ring's words first. A reader that opts in with `acceptToolFrames` also gets a frame for each call as it starts and ends, matched to the stored part by `callId`; a run that ends with calls still running marks them unanswered. A run's end ends the live view only when it names the run that is running, since a new run's start can arrive before the previous run's end.

Watches run in a long-held pool of handler slots, so open chats never hold up reads and sends. A route may hold at most 24 watches, which leaves room for its reads and sends; the 25th is refused `watch_capacity`.

## Sending

`session.enqueue_user_message` forwards the person's message to the runner's `session.send`. Its send id comes from the app's message id, the chat and the runner, so a retry reaches the runner under the same key and is answered with the existing send, never delivered twice. A receipt answers a repeated send for 48 hours. Modes are queue (the default), steer and interrupt, as the plan grants. Occipital never sends anything on its own and never resends after an unknown outcome.

`session.stop` cancels the running turn through the runner; `session.stop_all` asks the runner to stop everything the agent started, and reports what stopped and what kept running.

## Files and images

An uploaded file goes into the model's prompt or into the session's project folder. An app that asks for the prompt gets it when the session's runner takes uploads and its model accepts the file's type; the send naming the file copies it to the runner. Otherwise, or when the runner refuses it, the file is placed in the project and the message names its path. Partial uploads wait in `upload-blobs/` and resume from the bytes already written.

`session.media` reads an image a chat references: a thumbnail Occipital makes from the runner's bytes (at most 1024 pixels on the longest side, upright), or the original in pieces. Pages read photos as references from runners that support it, so a page never carries image bytes.

## Tool display

- **Writing:** a tool provider writes each call's record, completion and detail with `display.write` on Occipital's tool-provider route. The writer is the module the daemon attests, never a name in the body; an unverified caller is refused.
- **Views:** Occipital fetches each provider's views (`tool.views`) off the write path: when it first meets a pin, when the provider's build changes, and when a read finds the last fetch an hour old. A template that fails Occipital's checks is left out, and its calls draw as a plain chip.
- **Joining:** a record joins a call only through a view its own writer serves for the pin the runner stored for that call. If two writers qualify, neither is served.
- **Reading:** the apps read views, templates, records and details beside the pages (`display.views`, `display.template`, `session.tool_records`, `session.tool_detail`), cache them for good, and hear of new ones from `session.tool_records_watch`. Keeping them beside the pages keeps page versions the runner's alone: a record that lands after its row never reloads a chat.
- **Moved calls:** Occipital files a call under the runner that minted its key and finds it by the session and key the app asks about, so a chat moved to another runner keeps its tool rows and notices.

`display.model_names` resolves model ids to the names the apps show, from Fusiform's catalog and core's provider aliases, cached for 30 minutes (60 seconds when the aliases couldn't be read).

## Notices

Occipital keeps each riding notice it learns of, from a page read's notices fact or the runner's live event, keyed by its sender and send id. The apps read a call's notices with `session.notices` and hear of new ones from `session.notices_watch`. A notice sent as its own message stays a chat row, marked so the apps draw it as a notice rather than the person's words.

## Reply teaching and the interface language

The language library parses, checks, evaluates and lays out the closed interface language: the tags a head's reply may hold, `show_ui` programs and tool templates. It has no panic paths, because apps embed it. Its catalogs are pinned by publication digest; when a catalog's text changes, the previous digest stays an accepted alias, so a stored reply keeps drawing.

- **Reply teaching:** asked for its system text, the module answers with teaching generated from the catalog, plus the stamp a runner records on each reply.
- **Tool templates:** a row reads its call's record or completion only inside a fixed-size `Slot`, so a row's height never changes when they arrive. `occipital-check` gives tool providers the same checks in their CI.
- **The phone:** `crates/occipital-ffi` exposes the library as a C interface that turns any panic into a status code, built into `ports/swift/OccipitalCore.xcframework`; `OccipitalKit` wraps it in Swift.
- **Links:** `crates/occipital-refs` finds "Issue N", "PR #N" and `owner/repo#N` mentions in text and returns byte ranges with URLs, from the project record core serves beside each agent. The apps draw the links; the text is never changed.

## Storage

All stores sit in Occipital's data folder, normally `<data home>/cortexkit/occipital/`, named by the daemon in HELLO_ACK.

| File | Holds | Backed up |
|---|---|---|
| `store.db` | native `show_ui` records, send receipts, held sends, the state of uploads in progress | yes |
| `display.db` | tool records, completions, views, call pins | yes |
| `display-details.db` | tool details (large, may hold file contents) | no |
| `notices.db` | riding notices | yes |
| `upload-blobs/` | partial upload contents | no |

- **One writer:** `store.db` is opened with cortexkit-store, which holds a single-writer lease; every write is fenced.
- **Migrations only move forward.** Each table group has its own chain, and a binary refuses to start on a store ahead of its chain. So a build that adds a migration makes rollback a store step, not a plain binary swap: copy the changed files aside before placing it.
- **Backups:** at every start the module writes `engram-catalog.json`, so Engram backs up the files marked yes, live.
- **Restores are safe:** an older restored copy never makes Occipital resend: sends are deduplicated by the runner, and uploads resume.

## Failure handling

- **Health** comes from in-memory counters only, never a store or network read, and reports `failing` naming any background task that stopped.
- **A runner that can't answer** is `provider_unavailable`, which the apps retry. A runner's own refusals reach the apps with their code.
- **Mutations are never retried blindly.** A send or Stop whose outcome is unknown is reported, not resent; the app's retry carries the same key.
- **Logs** record every app request's method, session, outcome and the calls it made, and every mutation at INFO. A request other than a watch that is still running after 10 seconds gets one extra warning naming its current step. Message text and file names never reach the log.

## How changes are verified

- **Sample sets:** every wire shape the apps decode is pinned by a tagged sample set with SHA256SUMS, under `crates/occipital-app-reader/test-vectors/` (and the FFI and refs vectors). The apps pin those tags, so a wire change ships as a new tagged set and the old one stays frozen.
- **Runner recordings:** runners' own recorded replies are copied byte for byte into the module's test data and served through Occipital's real handlers.
- **Break checks:** a fix is proven by breaking it on purpose and watching its test fail.
- **The phone library** is rebuilt reproducibly on the build server, and `scripts/check.sh` refuses a committed library that no longer matches its sources.
