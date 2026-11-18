# phone-session-default-effort-v1

The reasoning effort a chat's next message uses when no effort is chosen, for
the apps to label, for example "xhigh (default)".

`session.details` and the newest page carry the head's next-send model as
`model`: `{provider, model, variant?, defaultVariant?}`.

- `variant` is the effort chosen for the chat, when one is.
- `defaultVariant` is the effort the runner sends when none is chosen, from the
  vendor's documented default. It appears only without `variant`, and only when
  the runner sends an effort at all.

The two are kept apart on purpose. An app that sends the selection back, for
example when the person changes the model, sends `variant` only, so the vendor's
default never becomes a choice the chat keeps.

## Files

- `01-details-default-effort`: no effort chosen. `model` has `defaultVariant:
  "xhigh"` and no `variant`.
- `02-newest-page-default-effort`: the newest page of the same chat, with the
  same `model`.
- `03-details-chosen-effort`: an effort chosen (`variant: "high"`), so `model`
  names no default.

These replies come from Occipital's real handlers, fed a runner head on
llm-runner 0.8.64. Each `.json` file is the response body, `{"result": …}`, and
each `.request.json` file is the request. `SHA256SUMS` covers every file but
this README.
