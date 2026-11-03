# phone-reader-pi-v1

A Pi chat as the apps read it. Pi chats run on Thalamus's Pi runner (runner
`thalamus-pi`, harness `pi`), and every request names `harness: "pi"`, which is how
Occipital finds the session under that runner.

A Pi chat's rows have the same shape as a Broca chat's:

- Every part has `partKey`, the runner's key for the part's slot in its stored message
  (`"0"`, `"1"`, …), and `partIndex`.
- A tool call is one `type: "tool"` part with `tool`, `callId` and `outputBytes`. Its
  result is folded into it, even though Pi stores the result as a separate message.
- That result message stays in the chat as a row with role `tool` and no parts, as a
  Broca chat's does.
- Pi's empty setup entry, its system message at ordinal 0, is left out. The page is
  still the chat's first page: it doesn't offer an older page to read.

## Where the files come from

Occipital's app reader served these replies at the commit that adds this folder,
through the same serving path a phone's route takes. The chat's stored entries are
Thalamus's recording of a real Pi 1.4.0 agent session on its Pi runner,
`runner-pages-pi-v2`, which Occipital's tests keep byte for byte in
`occipital-module/tests/data`. Each `.json` file is the
whole response body, `{"result": …}`, and each `.request.json` file is the request's
`{method, params}`. The test `app_reader::broca::tests::pi_samples` in
`occipital-module` serves every request again and fails if a reply's bytes change, so
a reader change that alters them needs a new sample set, never an edit to this one.

## Cases

| Files | What it shows |
| --- | --- |
| `01-pi-newest-page` | The newest page: three of the person's messages and the replies to them. The first reply makes a `bash` call, whose result is folded into it (`outputBytes: 10`); the result's own message is a `tool` row with no parts. |
| `02-pi-exact-read` | The exact read of the message with the call (`messageOrdinal: 2`): the call's whole arguments and its result text. |
| `03-pi-display-read` | A display read before any watch attached the chat's live text: `absent: "no_ring_attached"`, not a refusal. |
