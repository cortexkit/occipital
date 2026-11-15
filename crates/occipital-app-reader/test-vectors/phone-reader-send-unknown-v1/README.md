# phone-reader-send-unknown-v1

A chat's `queuedSends` list with every state a person's message can be in before,
or instead of, landing in the chat. New in this set: `state: "unknown"`, a message
the runner handed on but can't confirm ran. It is one OpenCode 2 chat's newest
page, read through Occipital's real request handling, so the reply is byte for
byte what a phone receives.

`01-newest-page.request.json` is the request the app sends, and
`01-newest-page.json` is the reply body it receives. The chat holds three stored
messages. Its `queuedSends` list, oldest first:

| `clientMessageId` | `state` | What it means | What to draw |
| --- | --- | --- | --- |
| `c-unsent` | `unsent`, `reason: "handover_failed"` | The runner held the message but couldn't bring the agent's host back, so it never ran. | "Didn't reach the agent", with a way to send the words again as a new message. |
| `c-unknown` | `unknown` | The runner handed the message on but can't confirm it ran. It never sends it again. | "May not have arrived", with **no** resend: sending the words again could run them twice. |
| `c-landed` | `unknown` | The same, but its message did land: the chat's stored message at ordinal 3 carries `clientMessageId: "c-landed"`. | The stored message, as an ordinary sent message. A stored message always wins over its list entry. |
| `c-queued` | `queued` | The runner holds the message and hasn't delivered it yet. | Waiting to reach the agent. |

An `unknown` entry has `endedAtMs`, when the runner said it couldn't confirm the
send, and no `reason`. It stays listed for 48 hours after that, like an `unsent`
one, and Occipital never sends it again. A `queued` entry turns into `unsent`
or `unknown`, or leaves the list once its message is stored. Agent Runner,
once it hands a waiting send back (its 5-minute limit), turns a `queued` entry
into `unsent` within 5 minutes; other runners may hold a send longer.

The list is the one Occipital keeps for the apps' sends, written by the test
through the same calls Occipital makes when the runner answers; tests in
`occipital-module` (`app_reader::send::tests`) check that it makes them.

`SHA256SUMS` covers every file except this README. A test in `occipital-module`
(`broca::tests::queued_sends_samples`) serves the set again and fails if any
byte changes.
