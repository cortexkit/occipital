# phone-reader-host-unknown-v1

A chat whose host program the runner has lost touch with, as an app reads it:
`host` on `session.details`, on the newest transcript page, and on claim replies
that carry session facts.

- A runner that keeps runs alive across its own restarts (runner contract
  `restart_survival`) says `host: "unknown"` on its head while it can't tell
  whether the session's host program is still running. Occipital passes it on
  as `host`, spelled as the runner gives it.
- While it's present, the run stays `active` and `activity` stays `busy`, but
  no more words or tool calls arrive until the host is found. Stop still works
  and ends the run at once: it answers `stopped`, and the run ends `interrupted`
  with reason `host_unknown`.
- `unknown` is the spelling today. The contract may add others later; any value
  means the host isn't known to be alive, never that it is.
- Once the host is found, or the run ends, the member is absent. The page
  version doesn't change; the facts do, which changes `activityToken`, so an
  app's next poll or transcript-watch notice brings the change.
- An `unchanged` reply and an older page carry no `host`.

## Where the replies come from

Occipital's app reader served each request at the commit that adds this folder,
through the same serving path a phone's route takes, over a test OpenCode 2
session on Thalamus's runner (module `thalamus-harness`). The text is invented.
`<name>.json` is the whole response body. `<name>.request.json` is the request's
`{method, params}`. The test
`app_reader::broca::tests::host_unknown_samples::the_host_unknown_samples_are_what_the_reader_serves`
in `occipital-module` serves every request again and fails if a byte changes, so a
change that alters them needs a new sample set, never an edit to this one.

## Cases

| Files | What it shows |
| --- | --- |
| `01-details-host-unknown` | `session.details` while the runner can't tell whether the host is alive: `host: "unknown"`, with the run still `active`. |
| `02-newest-page-host-unknown` | The newest page then, with `host: "unknown"`, `activity: "busy"` and the run `active`. |
| `03-details-host-found` | `session.details` once the host is found and the run goes on: no `host`, the run still `active`. |
| `04-newest-page-host-found` | The newest page then: the same `version`, a new `activityToken`, no `host`. |
