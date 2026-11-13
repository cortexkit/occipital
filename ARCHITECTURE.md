# Architecture

Occipital is how agents' work reaches and renders in the CortexKit phone and desktop apps. It is two things at once:

- **A daemon module, `ck-occipital`** (`crates/occipital-module`), the reserved `occipital` module of the subconscious daemon (subc). It is the one module the apps use for a chat: it reads every session from the llm-runner hosting it, composes the pages and live text the apps draw, forwards sends, Stop and uploads, keeps the display data tool providers write for each call, keeps the notices modules send agents, and relays the person's commands to the modules that own them. It also serves the reply teaching, the part of a head's system prompt that teaches the interface tags.
- **The libraries:** the interface language (`crates/occipital-lang`, published as `cortexkit-occipital-lang`), its C binding packaged for iOS (`crates/occipital-ffi`, wrapped in Swift as `OccipitalKit`) and the mention finder for links (`crates/occipital-refs`, published as `cortexkit-occipital-refs`), which the apps embed, so a reply, a tool row or a link is drawn the same way on the phone, on the desktop and in the module; and the tool template contract and checker (`crates/occipital-registry`, published as `cortexkit-occipital-registry`), which tool providers run in their CI.

`STRUCTURE.md` has the directory map and where to add new code. The module guide, `crates/occipital-module/README.md`, lists every operation.

## Terms

- **Runner:** a module implementing the llm-runner contract, which hosts sessions and stores their messages. Broca is one; Thalamus's `thalamus-harness` hosts OpenCode 2, Claude Code and Pi sessions, and `thalamus-pi` hosts Pi sessions until they move. Occipital talks to runners only through the published contract (`cortexkit-role-llm-runner`).
- **Head, worker:** a head is an agent a person works with; a worker is one an agent started. Both are sessions on a runner, and the apps read both.
- **Harness:** the shape of a session's stored messages (`broca`, `opencode2`, `claude-code`, `pi`). A request names it; a request without one is read as Broca.
- **Page:** a window of a chat's messages as app rows, newest first or before an ordinal. Every page carries a **version**, `history:<n>:incarnation:<id>` (just `history:<n>` when the runner names no log), taken from the runner's history counter and log identity. The apps cache pages by version and reload a chat whose version changes.
- **Claim:** what an app sends back to resume a chat it holds: the version it holds and the newest message it has. The answer is `matched` (with any new rows), `diverged`, `cursor_expired` (with the whole page again) or `outside_window`. A newest-page read that names an unchanged activity token, or a claim on a chat still empty, is answered `unchanged`.
- **Live text:** the words of a reply as the model writes them, before the runner stores it. A **display watch** streams them; the stored row replaces them once it lands.
- **Tool call identity:** each tool part carries `callKey` (the runner's key for the call, unique across that runner) and `schemaPin` (which version of the tool ran). A call moved to another runner with its history keeps its key, and the runner names the one that minted it as `call_carrier`.
- **View, template, record, completion, detail:** a tool provider's **view** of a tool version says how its calls draw: an `occipital.template/1` **template** for the row and the page, plus the shape of what it writes. For each call the provider writes a **record** (drawn on the row), a **completion** and a **detail** (drawn on the page).
- **Notice:** a module's message to an agent, such as a reminder or a status line. It reaches the agent either as its own message, which is a chat row like any other, or **riding** the next tool result, which makes no row.
- **Command:** something a module lets the person do from the apps directly, without asking a model, such as Magic Context's flush. A **preview** says what it would do now, in **facts** (a label with a number and unit, or a fixed state); a **run** does it.
- **Reply stamp:** the interface dialect and catalog digest a runner records on a head's reply, so the apps know which tags it may hold. Replies use catalog 1.0; tool templates use tool catalog 1.1.

## How the apps reach Occipital

The phone reaches the computer through Callosum, the federation module: every phone call arrives on a route stamped `reserved:callosum` with the device it came from. The desktop app connects to the daemon directly. Admission reads only what the daemon attests on the route: the route's principal, and the operator-app stamp that names the desktop. While the desktop is in development, a plain local caller is admitted too. Everything else is refused before the request is read further.

Occipital in turn calls runners, Entorhinal, tool providers and command modules as `reserved:occipital`, proved by its launch nonce. A runner admits Occipital's reads and sends by the session's plan, which core writes: Occipital is listed as a reader and as a sender, with the send modes the plan grants.

## Reading a chat

1. **Find the session.** Entorhinal maps the app's session id and harness to the runner hosting it. A harness recorded on a runner that doesn't host it is refused, never read in another harness's shape.
2. **Read the runner.** `session.head` gives the history counter, log identity, run state and send grant. `session.read` serves every page (newest and older, by ordinal range) and every exact read, with display facts (the app's message ids, tool times, call identity, creation and finish times, notices, message marks, host-written parts); raw `session.history` only finds the ordinal of a message that an app's claim (see Terms) or exact read names, and its rows are never shown.
3. **Build rows.** `crates/occipital-app-reader` turns stored messages into app rows. Broca's messages are decoded directly; OpenCode 2, Claude Code and Pi messages go through the runner contract's own display projections first, so every harness reaches the apps in one row shape. Occipital builds rows from the runner's raw entries, never from text a runner pre-renders. A stored message that can't be read at all becomes an `unreadable` placeholder row in its place; a single part that can't be shown becomes an `unreadable` part, and the rest of the message reads. Either way Occipital logs why. Text a host wrote into a person's message (Claude Code's `[Request interrupted by user]`), which the runner names in the read's `host_parts`, becomes an `injected` part marked `host` and stays out of the row's text, so it is never drawn as the person's words. OpenCode 2's own `synthetic` messages, which the runner contract counts as host text whole, become a person's row holding only such a part. A host part in a reply is the host's own bookkeeping (the empty reasoning blocks that record a fallback model) and is left out of the row; when the runner knows another model answered in place of the one asked for, its `model_fallback` fact reaches the row as `modelFallback`, naming both.
4. **Answer.** The reply carries the session facts (model, last run, send grant, service tier, history intake, lost host) at its top level, never inside a page, so a reply whose page is trimmed to fit still carries them.

A page's version moves only when the runner's history counter or log identity does. A change in how Occipital renders rows doesn't move it, so apps holding a chat keep their copy until it reloads.

## Live text and tool frames

Each open chat has one subscription to its runner's live events, shared by every app watching it (`app_reader/display_hub.rs`). Text arrives in a ring that batches deltas for 300 ms; a watch or read that starts mid-reply first gets the text written so far. A reader that opts in with `acceptToolFrames` also gets a frame for each call as it starts and ends, matched to the stored part by `callId`; a run that ends with calls still running marks them unanswered. A run's end ends the live view only when it names the run that is running, since a new run's start can arrive before the previous run's end. A display `run_status`, which says the run's state changed (a provider retry starting or ending), makes the chat's watches re-read the head and resets the live view, so an abandoned attempt's words never stay on screen.

Watches run in a long-held pool of handler slots, so open chats never hold up reads and sends. A route may hold at most 24 watches, which leaves room for its reads and sends; the 25th is refused `watch_capacity`.

## Sending

`session.enqueue_user_message` forwards the person's message to the runner's `session.send`. Its send id comes from the app's message id, the chat and the runner, so a retry reaches the runner under the same key and is answered with the existing send, never delivered twice. A receipt answers a repeated send for 48 hours. Modes are queue (the default), steer and interrupt, as the plan grants. Occipital never sends anything on its own and never resends after an unknown outcome.

`session.stop` cancels the running turn through the runner; `session.stop_all` asks the runner to stop everything the agent started, and reports what stopped and what kept running.

## Files and images

An uploaded file goes into the model's prompt or into the session's project folder. An app that asks for the prompt gets it when the session's runner takes uploads and its model accepts the file's type; the send naming the file copies it to the runner. Otherwise the file is placed in the project and the message names its path. When the runner refuses a prompt file anyway (`media_unsupported`, `media_too_large`), the app asks Occipital to place it in the project (`upload.place`) and sends the path instead. Partial uploads wait in `upload-blobs/` and resume from the bytes already written.

How many files a message may carry follows the session's model: `session.details` reports it as `attachments` to an app that asks (images from Fusiform's `limit.images_per_request`, else 4; other files 4; each image bound the stricter of the catalog's and the runner's, the catalog's edge being the smallest of its base and `above_images` conditions), and a send naming more is refused `too_many_files` before anything is copied. Both come from one function, so the details and the send never disagree.

`session.media` reads an image a chat references: a thumbnail Occipital makes from the runner's bytes (at most 1024 pixels on the longest side, upright), or the original in pieces. Page reads ask runners that support it for photo references, so no page brings image bytes with it; an image reaches the apps as a file part with its digest and size, and its bytes only through `session.media`.

## What the model is given

`session.request_context` answers the system text and the tools of the plan in force on a session, from the runner contract's `session.plan_view`, which a runner admits as it admits `session.read`. Occipital checks the reply against the contract's closed type and passes the text, the tool schemas and the UTF-8 byte offsets on unchanged, with the members spelled the apps' way. It is the request as the plan built it, not a copy of one sent to a provider, and it holds none of Magic Context's head messages (memories, summaries, profile, docs), which the apps read from Magic Context. A runner that doesn't declare `plan_view` is refused `unsupported` without a call.

## Tool display

- **Writing:** a tool provider writes each call's record, completion and detail with `display.write` on Occipital's tool-provider route. The writer is the module the daemon attests, never a name in the body; an unverified caller is refused.
- **Views:** Occipital fetches each provider's views (`tool.views`) off the write path: when it first meets a pin, when the provider's build changes, and when a read finds the last fetch an hour old. A template that fails Occipital's checks is left out, and its calls draw as a plain chip.
- **Joining:** a record joins a call only through a view its own writer serves for the pin the runner stored for that call. If two writers qualify, neither is served.
- **Reading:** the apps read views, templates, records and details beside the pages (`display.views`, `display.template`, `session.tool_records`, `session.tool_detail`), cache them for good, and hear of new ones from `session.tool_records_watch`. Keeping them beside the pages keeps page versions the runner's alone: a record that lands after its row never reloads a chat.
- **Moved calls:** Occipital files a call under the runner that minted its key and finds it by the session and key the app asks about, so a chat moved to another runner keeps its tool rows and notices.

`display.model_names` resolves model ids to the names the apps show, from Fusiform's catalog and routing's credential-provider map (`route.catalog_targets`, on whichever module the daemon's catalog lists as providing `routing/v1`), cached for 30 minutes (60 seconds when the map couldn't be read). The same read takes each model's attachment limits.

## Notices

Occipital keeps each riding notice it learns of, from a page read's notices fact or the runner's live event, under its call (the runner that minted the call's key, and the key) and its sender and send id. The apps read a call's notices with `session.notices` and hear of new ones from `session.notices_watch`. A notice sent as its own message stays a chat row, marked so the apps draw it as a notice rather than the person's words.

**Wake records** say what a wake held, so the apps draw it as chips rather than the prompt text. The waking module (core) writes each record before it sends the wake, with `display.write` and `kind: "wake"`, keyed by its own send id; the sender is the route's attested writer. Occipital keeps it once per session, sender and send id, never changed. A row names its send only through the runner's read facts, so every page read records which send each message came from, in memory at once and in the file. The apps read the records of the rows they draw with `session.wake_records {messageIds}`, and a riding notice carries its send's record in `session.notices`. A row whose history moved from another session takes the one record its send has in any session, and none when two sessions hold one. There is no watch: the apps fetch a row's record before inserting the row, and a record that arrives later never replaces its plain card.

## Commands

A module that has commands provides `user-commands/v1` and serves `commands.describe`, `commands.preview` and `commands.run` on its management surface, admitting them only from `reserved:occipital`; the shapes are the `cortexkit-role-user-commands` crate's (`crates/occipital-module/src/commands.rs`).

- **Subjects:** a session subject names the session as its runner keys it, `{runner, harness, session}`. `session.details` serves it as `subject` to an app that asks (`commandSubject: true`), so the apps never derive it from the runner name Entorhinal records.
- **Listing:** `commands.list {subject}` merges the commands of every ready module whose manifest provides the capability, for the subject's kind (session, agent or computer), each named by its module. Each module's description is kept per build and read again when the module is placed on another; one that fails its checks is left out, so a broken module never hides the others.
- **Relaying:** a preview or run is checked against its command's declaration, then sent to the module with `caller`, the facts Occipital verified on the app's route (the signed desktop, its development build, or a paired device with its fingerprint). The module records them beside Occipital's principal and never treats them as authority; it confirms what needs confirming itself. The module's answer is passed on as it came.
- **Runs are never sent twice.** One sent but unanswered is reported `command_outcome_unknown`, and the app asks again under the same `request_id`, which the module answers from its stored reply.
- **Nothing enters a chat.** A run is the person's action, never part of a transcript or of what a model reads.

## Reply teaching and the interface language

The language library parses, checks, evaluates and lays out the closed interface language: the tags a head's reply may hold, `show_ui` programs and tool templates. It has no panic paths, because apps embed it. Its catalogs are pinned by publication digest; when a catalog's text changes, the previous digest stays an accepted alias, so a stored reply keeps drawing.

- **Reply teaching:** asked for its system text, the module answers with teaching generated from the catalog, plus the stamp a runner records on each reply.
- **Tool templates:** a row template reads the call's view (its record, completion and timing) and the clock only inside a fixed-size `Slot`, so a row's height never changes when they arrive. `occipital-check` gives tool providers the same checks in their CI.
- **The phone:** `crates/occipital-ffi` exposes the library as a C interface that turns any panic into a status code, built into `ports/swift/OccipitalCore.xcframework`; `OccipitalKit` wraps it in Swift.
- **Facts:** `format_fact` words a command preview's or result's facts ("about 18,200 tokens", "Building, 40%") and gives each a tone, so both apps draw them alike; a fact's value is never the module's own text.
- **Links:** `crates/occipital-refs` finds "Issue N", "PR #N" and `owner/repo#N` mentions in text and returns byte ranges with URLs, from the project record core serves beside each agent. The apps draw the links; the text is never changed.

## Storage

All stores sit in Occipital's data folder, normally `<data home>/cortexkit/occipital/`, named by the daemon in HELLO_ACK.

| File | Holds | Backed up |
|---|---|---|
| `store.db` | native `show_ui` records, send receipts, held sends, the state of uploads in progress | yes |
| `display.db` | tool records, completions, views, call pins | yes |
| `display-details.db` | tool details (large, may hold file contents) | no |
| `notices.db` | riding notices, wake records, the send each read message came from | yes |
| `upload-blobs/` | partial upload contents | no |

- **One writer:** `store.db` is opened with cortexkit-store, which holds a single-writer lease; every write is fenced.
- **Migrations only move forward.** Each file, and each table group in `store.db`, has its own version, and a binary refuses to start on one ahead of what it knows. So a build that adds a migration makes rollback a store step, not a plain binary swap: copy the changed files aside before placing it.
- **Backups:** at every start the module writes `engram-catalog.json`, so Engram backs up the files marked yes, live.
- **Restores are safe:** an older restored copy never makes Occipital resend: sends are deduplicated by the runner, and uploads resume.

## Failure handling

- **Health** comes from in-memory counters only, never a store or network read, and reports `failing` naming any background task that stopped.
- **Runner refusals are mapped, never passed raw.** A read the runner refuses reaches the apps as `cursor_expired` or `session_unknown` when that is what it says, and otherwise as `provider_unavailable`, which the apps retry; the runner's code stays in the message. A media read keeps `unknown_media` and `invalid_params`, and a plan view `no_plan` and `too_large`, which are final. A plan view the runner refuses `transient` with `detail.cause: "awaiting_plan_applied"` reaches the apps as `context_not_ready`: the runner serves it from the session's next applied plan, so the app reads again later instead of retrying. A send's refusal reaches the apps under a send code (`enqueue_not_authorized`, `client_message_id_conflict`, `media_unsupported`, `media_too_large` and others) or as `runner_refused`, with the runner's code, whether the same send may succeed later, and how long to wait first. An account wait (the account that would serve the send is at its usage limit, or routing can't choose an account yet) counts as one that may succeed later, after the wait the runner names; it is never reported as failed. When `session.head` names the code the session's last run ended with, the session facts carry it as `lastRun.error.providerCode`, if it is one of the codes the runner contract names, so the apps can say a run ended on an account wait. When the head says what the run waits on, the session facts carry it as `lastRun.awaits` (who ends the wait, why, and when it moves or gives up); its quoted `message` passes with `from` naming whose words they are (runner, sign-in plugin, host or provider; the apps open it on tap, never in the line), is left out when `from` is a source the contract doesn't name, and an `awaits` that doesn't decode is left out rather than failing the read. A queued send whose turn ended in an error is listed unsent with that run's code (`runnerCode`, read once from `run.status`) and, for an account wait, `retryAfterMs` from `endedAtMs`; the app sends the words again as a new message, since the runner answers the old send id with its finished record.
- **Mutations are never retried blindly.** A send, Stop or command run whose outcome is unknown is reported, not resent; the app's retry carries the same key.
- **Logs** record every app request with its method, session, outcome and the calls it made: a send, Stop, Stop everything, command run or finished, aborted or placed upload at INFO, any other success at debug level unless it took over 2 seconds, and refusals and deadlines as warnings. A request other than a watch that is still running after 10 seconds gets one extra warning naming its current step. Message text and file names never reach the log.

## How changes are verified

- **Sample sets:** every wire shape the apps decode is pinned by a tagged sample set with SHA256SUMS, under `crates/occipital-app-reader/test-vectors/` (and the FFI and refs vectors). The apps pin those tags, so a wire change ships as a new tagged set and the old one stays frozen.
- **Runner recordings:** runners' own recorded replies are copied byte for byte into the module's test data and served through Occipital's real handlers.
- **Break checks:** a fix is proven by breaking it on purpose and watching its test fail.
- **The phone library** is rebuilt reproducibly on the build server, and `scripts/check.sh` refuses a committed library that no longer matches its sources.
