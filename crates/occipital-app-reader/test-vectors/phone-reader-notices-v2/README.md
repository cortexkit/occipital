# phone-reader-notices-v2

How a chat says who sent each message the person didn't write, with llm-runner
0.8.12's marks (`message_marks`). This set adds the marks to
`phone-reader-notices-v1`, which stays as it was tagged.

A runner records the module that sent each message it stores, and two marks the
sending module may set:

- **`speaks_for`:** the words are another agent's, carried by the module. A
  head's brief and steers to its worker come this way, and so do peer messages
  to a head.
- **`notice`:** the module's own message to the agent, such as core's status line
  or Magic Context's reminders.

The set is served through Occipital's own frame loop on a test OpenCode 2
session on Thalamus's runner. The text and the agent ids are invented.

`NN-*.request.json` is the request the app sends, and `NN-*.json` is the reply body
it receives.

| Files | What they show |
| --- | --- |
| `01-page-marks` | The newest page. Message 1 is the person's own, sent through Occipital, and carries no mark. Message 2 is a head's brief, with `speaksFor: {agentId}`. Message 3 is core's status line, with `notice: {sender, deliveredAs}`. Message 4 is from a module that set neither mark, with `sentBy: {module}`. Message 5 is the agent's answer. |
| `02-notices-speaks-for` | The notices read for message 5's call. A peer message rode its tool result, so the notice also carries `speaksFor: {agentId}`. |

A row carries at most one of `speaksFor`, `notice`, `sentBy` and the relayed
message's `origin`. Draw a `speaksFor` row as that agent's message, a `notice` row
folded, and a `sentBy` row as a visible message labelled with the module. A row
with none of these is the person's own. `deliveredAs` is the runner's delivery
mode, and is left out when the runner names none. Show a module or agent id you
have no name for as it is.

`SHA256SUMS` covers every file except this README. A test in `occipital-module`
(`broca::tests::notices_samples`) serves the set again and fails if any byte
changes.
