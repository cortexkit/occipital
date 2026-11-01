# Phone reader samples with part identities (v2-keys)

These samples pin the three members that transcript rows carry so that Magic
Context can place drop markers and summary ranges on the phone's transcript:
`partKey` and `partIndex` on every display part, and `callId` on every tool call.
Magic Context uses these members to place drop markers and summary ranges on
transcript parts. It keys nothing durable by its own tag numbers, so these stored
values are what it keys by.

Every `NN-*.request.json` is the params the phone sends. Every `.reply.json` (a
read's whole response body, `{"result":…}`) or `.notice-1.json` (a watch's first
notice) is what the module answers. Each file is compact JSON with no trailing
newline, written the way `serde_json::to_vec` writes it. `SHA256SUMS` lists every
file.

## How they were produced

The test
`app_reader::broca::tests::part_keys::the_part_key_samples_are_what_the_handlers_answer`
in `crates/thalamus-harness-module` builds the session on a fake Broca route and
serves each request through the module's own device-route handlers. It then
compares every body byte for byte with its file here, and compares `SHA256SUMS`
with the digests of those bytes. To record them again, point
`THALAMUS_RECORD_KEY_SAMPLES` at a directory and copy what it writes here:

```sh
THALAMUS_RECORD_KEY_SAMPLES=/tmp/part-keys \
  cargo test --locked -p thalamus-harness-module --lib the_part_key_samples_are_what_the_handlers_answer
```

Verify:

```sh
(cd crates/thalamus-app-reader/test-vectors/phone-reader-broca-v2-keys && shasum -a 256 -c SHA256SUMS)
```

The head values (`historyVersion` 7, `activity`, the activity token and time, the
attachment fields) come from the fake route's fixed head. They're opaque test
values, not tokens a live session would accept. No row has `createdAtMs`, and every
`network` is `unknown`, because Broca's `session.read` entries don't carry a write
time yet (see `phone-reader-broca-v1`).

| File | Scenario |
| --- | --- |
| `01-page-newest` | Newest page, `m40` to `m43`. |
| `02-claim-behind` | Claim on that page holding only `m40`. It's matched, and its `newTurns` hold `m41` to `m43`. |
| `03-watch-behind` | Transcript watch opened with the same claim; its first notice. |
| `04-message-full` | Full read of `m41` by `messageOrdinal`. |

## The members

- **`partKey`** (string): the runner's own key for the part in its `session.read`
  entries. For Broca, and the Claude Code gateway it serves, that's the block's
  index in the stored message's `content`, written as a decimal string. It's a
  string so the member has one type across runners: OpenCode's keys are its `prt_…`
  part ids.
- **`partIndex`** (number): the block's position in the **stored** message,
  counting from 0 and counting every stored block. Display rows leave some stored
  blocks out (encrypted reasoning, turn ends, tool results folded into their
  call), so a part's position in `parts` is not its `partIndex`. File parts
  already carried `partIndex`, with the same meaning; the `files` list keeps the
  shape it had, a `partIndex` and no `partKey`.
- **`callId`** (string): on every tool part and every `toolCalls` entry, the
  stored block's `tool_call_id`. On a full read, the part that shows a tool result
  carries the id of the call it answers, so a dropped result can land on its call.

A member is left out when the stored message has no value for it, never
invented: a tool block stored without a `tool_call_id` has no `callId`, and the
placeholder row the reader shows for an unreadable entry (`"unreadable": true`) has
one synthetic text part with neither `partKey` nor `partIndex`.

The members are additive. Every other member keeps its value, and members are
written in sorted order, so removing the three gives the bytes a reader served
before them. The test
`app_reader::broca::tests::part_keys::without_part_identities_the_v1_replies_are_the_recorded_bytes`
holds today's replies to `phone-reader-broca-v1` that way. The values are the same
on page reads, claim replies, watch notices and full reads, and the same after the
module restarts, because they come from the stored message alone.

## The reasoning-and-tool message, `m41`

Stored `content`:

| Stored index | Block | Shown? |
| --- | --- | --- |
| 0 | `redacted_reasoning` | no |
| 1 | `reasoning` | yes |
| 2 | `text` "Checking the clock." | yes |
| 3 | `tool_call` `call_date_1` (`bash`, `date -u +%Y`) | yes |
| 4 | `tool_result` `call_date_1` (`2026\n`) | folded into its call; shown on a full read |
| 5 | `text` "It is 2026. Listing the worktree next." | yes |
| 6 | `tool_call` `call_ls_2` (`bash`, `ls`); its result is stored in `m42` | yes |

Parts served on the page, the claim and the watch notice:

| Row position | `type` | `partKey` | `partIndex` | `callId` |
| --- | --- | --- | --- | --- |
| 0 | `reasoning` | `"1"` | 1 | none |
| 1 | `text` | `"2"` | 2 | none |
| 2 | `tool` | `"3"` | 3 | `call_date_1` |
| 3 | `text` | `"5"` | 5 | none |
| 4 | `tool` | `"6"` | 6 | `call_ls_2` |

On the full read, the result at stored index 4 is its own part between rows 2
and 3: `partKey` `"4"`, `partIndex` 4, `callId` `call_date_1`.
