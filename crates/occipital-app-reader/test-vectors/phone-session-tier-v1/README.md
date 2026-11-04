# phone-session-tier-v1

A chat's service tier, as an app reads it: `serviceTier` beside `model` on
`session.details`, on the newest transcript page, and on claim replies that carry
session facts.

- `serviceTier` is the tier a message that names none would use next, such as
  `"fast"`. Occipital passes it on exactly as the runner's head states it.
- A missing `serviceTier` means the head named none. On a runner that lets a head's
  model and tier be chosen (runner contract `head_selection`), that means
  `standard`; on any other runner it is unknown. An app offers the choice only for
  heads where core does, so where the app shows a picker it reads a missing tier as
  `standard`.
- Like `model`, it changes when the choice changes, which changes `activityToken`,
  so an app's next poll or transcript-watch notice brings the new value.
- An `unchanged` reply and an older page carry neither member.

## Where the replies come from

Occipital's app reader served each request at the commit that adds this folder,
through the same serving path a phone's route takes, over a test Broca session.
`<name>.json` is the whole response body. `<name>.request.json` is the request's
`{method, params}`. The test
`app_reader::broca::tests::session_facts::the_service_tier_samples_are_what_the_handlers_answer`
in `occipital-module` serves every request again and fails if a byte changes, so a
change that alters them needs a new sample set, never an edit to this one.

## Cases

| Files | What it shows |
| --- | --- |
| `01-details-fast` | `session.details` while the head names the `fast` tier. |
| `02-newest-page-fast` | The newest page from the same head: `serviceTier` beside `model`. |
| `03-details-no-tier` | `session.details` once the head names no tier: no `serviceTier`. |
