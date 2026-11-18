# phone-reader-read-moved-v1

What the apps receive when they read a chat whose agent moved to another
computer, from this computer's Occipital, and the runner here no longer serves
the session.

The runner refuses every read of that session `session_held_elsewhere`, which is
final on this computer. Occipital passes it on as the same
`session_held_elsewhere` a send gets (`phone-reader-send-moved-v1`), never as
`provider_unavailable`. So the app doesn't retry: it looks the agent up again
and reads the chat on the computer where it lives now. `detail.holder` names
that computer's box id when the runner knows it.

Every other refusal of a read keeps its usual code: `cursor_expired` or
`session_unknown` when the runner says so, and otherwise `provider_unavailable`,
which the app retries. That includes a runner's `session_moving`, for a move
still in progress.

## Files

- `01-page-runner-moved`: a newest-page read, refused with Broca's own body
  (recorded from Broca's serve path), which names no new computer yet.
- `02-details-runner-moved`: a details read, refused the same way.
- `03-page-runner-moved-holder`: a page read refused with the runner
  contract's sample that names the new computer, served with it as
  `detail.holder`.

Each `.json` file is the error frame Occipital's real request handling sends,
`{code, message, detail?}`. Each `.request.json` file is the request.
`SHA256SUMS` covers every file but this README.
