# Served transcript claim contract v1.1

v1.1 adds one rule to v1: every `matched` reply carries `vouchedFromMessageId`,
the id of the oldest message the server compared and therefore vouches for.
A client must treat a matched reply without it as a decode failure; absence
never means the server vouches for the whole claimed range. On the scan route
(OpenCode) the value is the claim's `heldFloorMessageId` when that message is
inside the retained window, the window's oldest message when the floor is below
it, and `sinceMessageId` when the claimed floor comes after it. On the native
route (Broca) an equal lineage proves the whole claimed range, so the value is
the claim's `heldFloorMessageId`. A `diverged` reply carries the field only when
the held floor is below the window; `cursor_expired` never carries it.

## Where these files come from

These are served response bodies of the apps' `session.transcript_page` operation,
captured before this crate existed, when Prefrontal core served that operation for
the phone and desktop apps. They were copied byte for byte from the Prefrontal repository
(`test-vectors/transcript-claim-v1.1/` at commit
`9f124a33b318d309e66b3524cd964a1861a6d9ac`), together with the code this crate
ports. They're real replies, not hand-written templates: compact JSON with no
trailing newline, as `serde_json::to_vec` wrote them. The phone pins their digests,
so they stay byte-identical here.

The v1.1 files are the v1 captures with `vouchedFromMessageId` added to the three
matched replies (`opencode-matched.json`, `opencode-unchanged.json`,
`broca-matched.json`); every other file is byte-identical to v1.

| File | Scenario |
| --- | --- |
| `opencode-newest.json` | Stable newest page; version equals its activity token and names a retained snapshot. |
| `opencode-matched.json` | Claim holding the newest page's version and `restart-0`; one appended turn `restart-1`. |
| `opencode-unchanged.json` | Claim holding the matched reply's version through `restart-1`; matched with empty `newTurns`. |
| `opencode-diverged.json` | Held history rewritten in place; diverged with resync from the first affected message. |
| `opencode-cursor-expired.json` | Original newest-page snapshot evicted from the server's bounded snapshot cache; cursor_expired with a full resync. |
| `broca-newest.json` | Stable native newest page; version is `lineage-claim-1`, not its activity token. |
| `broca-matched.json` | Claim through `m0` in the same lineage; native read after ordinal 0 returns `m1`. |
| `broca-cursor-expired-window.json` | No retained ordinal hint and the claimed `m0` is outside the returned page; cursor_expired with resync. |
| `broca-cursor-expired.json` | Rewritten history has a new lineage; cursor_expired with resync and version `lineage-claim-2`. |

In this repository, `claim::tests::every_matched_transcript_claim_vector_carries_vouched_from_message_id`
reads every file here and checks each against `SHA256SUMS`.

## Phone interpretation

Both routes use the same diagnostics shape (Pi sessions use it too):
`{identity: "durable", deltaRead: "scan" | "native", invalidation: "none"}`.
Send the non-empty **version**, not an assumed activity token, as `heldVersion`.
A scan route (OpenCode, like Pi) compares retained message identities and content,
so rewriting held history returns `diverged`. A native route (`deltaRead:
"native"`, Broca) uses the producer's opaque lineage generation: a rewrite
changes that lineage and returns `cursor_expired`, **never `diverged`**. The phone
must treat both verdicts as a request to apply `resync`.

Broca can still match without a stored transcript snapshot when its native page
contains the claimed id; the stored snapshot is only an ordinal hint. OpenCode needs
its retained snapshot to vouch for held content. A newest page with
`discardActivityToken: true` has **no version** on either route. OpenCode also omits
version when its projection can't produce a durable-identity snapshot. Activity-token
`unchanged: true` responses remain a separate fast path; the `opencode-unchanged.json`
fixture is instead a claim reply.

The captured activity watermarks and observation timestamps are opaque test-run
values, not usable cursors for a live session. Tests compare the contract and
transcript fields while allowing those volatile observations to vary.

## Verify

```sh
(cd crates/thalamus-app-reader/test-vectors/transcript-claim-v1.1 && shasum -a 256 -c SHA256SUMS)
```

Once Thalamus serves `session.transcript_page` itself, new captures come from its own
handler and keep the same file names, checksums file and contract version.
