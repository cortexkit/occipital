# phone-reader-simple-v1

The Simple view of a busy chat, as an app reads it with `view: "simple"` on
`session.transcript_page` and `session.transcript_watch`. Reads without it are
unchanged.

## The rule, version 1

A row is in the view when it is one of these:

1. **The person's message:** role `user`, with none of `notice`, `speaksFor`,
   `sentBy` and `origin`, that still has words, a file or an `unreadable` part
   once balanced `<tag …>…</tag>` spans a host put in are taken out. A text
   block that starts with such a span is served `injected` whole, so the words
   after it count. A part the host wrote (`host: true`) never does.
2. **The agent's `say` message:** an assistant row with a `message` part.
3. **The agent's ask:** an assistant row with an `ask` call whose input files
   an ask (`action` absent or `create`).
4. **The person's answer to an ask:** a wake (`notice`) whose wake record holds
   an `ask_answer` item.

## Rows

Ordinals and message ids are the full history's, so a message is the same in
either copy. Each row has only what the view draws:

- **person:** its parts, without the host's and without `injected` parts that
  are only a host's block; `text` and `files` as served;
- **agent:** only its `message` parts and its `ask` calls that file an ask,
  each whole (`callId`, `callKey`, `partKey`, input, times); `text` is `""` and
  `blockSummaries` and `files` are empty;
- **answer to an ask:** no parts and `text` `""`; draw it from
  `session.wake_records`.

An ask's id is in prefrontal's record for the call (`askId`), read with
`session.tool_records` by the call's `callKey`.

## Versions, paging and claims

- Every reply says `view: "simple"`. Its `version` is the full one with
  `:simple:1` added, so a full claim never matches a Simple read, and a new
  rule reloads only the Simple copies.
- A read goes back through the history until it holds `limit` rows, reaches
  the start, or has read 1,000 messages (2,000 for a newest page). So a page
  can hold fewer rows than `limit`, or none, without being at the start.
  `nextBeforeOrdinal` is where the read stopped; go on from it with
  `beforeOrdinal`. Every page that isn't at the start has one.
- A newest page with no rows at the start says `emptyReason`: `empty` for a
  chat with no messages, `nothing_visible` when none of them is in the view.
- A claim names the Simple version it holds. It is checked against the Simple
  copy its claimant was served, so a row that changed or newly joins the view
  inside the held range answers `diverged`, with a resync from that row.
- A read of one message by position (`messageOrdinal`, `aroundMessageId`,
  `afterOrdinal`) reads the full stream; with `view` it is refused.
- On a chat whose agent has no message tool, `say` calls are tool calls, so
  the view holds only the person's rows, the asks and the answers. Read the
  full stream there.

## Files

The chat: the person's question (`m1`), 1,300 messages of the agent's prose,
then a wake (`m1302`), a person's message that is only a host's block
(`m1303`), one that mixes a block with words (`m1304`), a host-only row
(`m1305`), a photo (`m1306`), a reply with prose, a `say`, an ask it files and
an ask it dismisses (`m1307`, results in `m1308`), a second ask (`m1309`, its
result in `m1310`), a wake holding the person's answer to it (`m1311`), and
more prose.

- `01-newest-page`: `m1304`, `m1306`, `m1307`, `m1309` and `m1311`, each as
  drawn. `m1307` keeps its `say` and the ask it files, not the dismissal.
- `02-older-past-the-bound`: no rows, not at the start, and where to go on.
- `03-older-to-the-start`: `m1`, at the start.
- `04-claim-matched`: after more prose and a new message from the person, only
  the message is new.
- `05-claim-diverged-say-answered`: a `say` the agent was still writing when
  the claimant last read is answered: it joins the view inside the held range.
- `06-claim-diverged-late-record`: prefrontal's record for `m1307`'s `say`
  arrives after the claimant read it; the row comes back with its `replyTo`.
- `07-claim-outside-window`: a claim on the oldest message, with
  `acceptClaims: ["outside_window"]`.
- `08-full-claim-on-the-view`: a full version claimed with `view`:
  `cursor_expired`.
- `09-claim-after-rewrite`: the history is rewritten: `cursor_expired` with
  the new version.
- `10-exact-read-refused`: `messageOrdinal` with `view`, refused.
- `11-nothing-visible`: a chat of the agent's prose only.
- `12-no-message-tool`: the same chat on an agent without `say`.
- `13-ask-records`: the asks' ids from prefrontal's records.

These replies come from Occipital's real handlers, fed these runner entries,
the plan, prefrontal's wake and message records, and its ask records. Each
`.json` file is the response body and each `.request.json` file is the request.
`SHA256SUMS` covers every file but this README.
