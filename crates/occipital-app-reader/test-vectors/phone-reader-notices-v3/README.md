# phone-reader-notices-v3

How a turn-scoped system note reaches the apps beside a riding notice. Both
are served by the same reads, `session.notices` and `session.notices_watch`,
under the call they follow, and `deliveredAs` tells them apart. This set adds
turn notes; the `phone-reader-notices-v1` and `-v2` sets are unchanged.

A module's message to the agent can reach it two ways after a tool call:

- **A ride**, `deliveredAs: "tool_result"`: the text goes with the call's
  result, as today.
- **A turn note**, here `deliveredAs: "turn_system_cleared"`: a system message
  the model reads for one step after the call (runner contract 0.8.29). It is
  recorded on the call it follows, like a ride, and makes no chat row.

Occipital passes `deliveredAs` through as the runner names it. Show a form you
don't know like a ride.

The set is served through Occipital's own frame loop on a test OpenCode 2
session on Thalamus's runner. The text is invented.

`NN-*.request.json` is the request the app sends, and `NN-*.json` is the reply body
it receives.

| Files | What they show |
| --- | --- |
| `01-page` | The newest page: one answer with four `bash` calls, `ck_one` to `ck_four`. No notice text is on it. |
| `02-notices` | The notices read for `ck_one` and `ck_two`: a ride under the first, a turn note after the second. |
| `03-notices-watch-open` | The watch opens and names no call yet. |
| `04-notices-watch-hint` | A turn note for `ck_three` arrives on the runner's live stream, naming its form, and the watch names the call. |
| `05-notices-watch-hint-no-form` | A note for `ck_four` arrives live from a runner older than llm-runner 0.8.28, which names no form. The watch names the call. |
| `06-notices-live` | The notices read for both: `ck_three` reads as a turn note, `ck_four` as a ride, since its live event named no form. |
| `07-page-again` | The newest page read again, byte-identical to `01`. The runner's read now lists `ck_four`'s note with the form it stored. |
| `08-notices-watch-hint-corrected` | The watch names `ck_four` again, because the read corrected its form. |
| `09-notices-corrected` | The notices read for `ck_four`: now a turn note. |

A notice's `deliveredAs` can change once, from `tool_result` to the form the
runner stored, when the live event named none. The watch names the call when
it does, so read its notices again on every hint rather than keeping a first
answer for good.

`SHA256SUMS` covers every file except this README. A test in `occipital-module`
(`broca::tests::notices_samples`) serves the set again and fails if any byte
changes.
