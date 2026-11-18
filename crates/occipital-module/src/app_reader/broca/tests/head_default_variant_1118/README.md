# session.head `model.default_variant` (llm-runner 0.8.64), broca 0.4.70

Written by broca's real serve path (`head_names_the_default_effort_a_send_without_a_variant_sends`),
synthetic models, one session, three runs in order:

- `session.head-default-effort.json`: the send named no variant; the model documents a default
  effort, so broca sent `medium` and the head says `default_variant: "medium"`, no `variant`.
- `session.head-named-variant.json`: the send named `high`; the head says `variant: "high"` only.
- `session.head-no-effort.json`: the model documents no default, so broca sent no effort; the head
  has neither member.

broca sends a default effort only on OpenAI families. Claude and Gemini heads with no chosen effort
look like the third sample. A head whose plan selects a model shows the latest run's
`default_variant` only when that run used the same provider and model.
