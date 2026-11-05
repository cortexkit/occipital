# phone-reader-provider-tools-v1

How a tool call that the model's provider ran itself reaches the apps: a hosted
web search, for example, which nothing on this computer ran. Its tool part carries
`providerExecuted: true`. A call this computer ran leaves the member out.

The set is served through Occipital's own frame loop on a paired phone's route,
over the recorded OpenCode 2 session in
`crates/occipital-module/tests/data/runner-pages-opencode2-v1/` (Thalamus's tag
`runner-pages-opencode2-v1`). One thing is changed: its `bash` call is stored with
`executed: true`, which is how OpenCode 2 stores a call its provider ran. The
runner contract (llm-runner 0.8.10) serves that as `provider_executed` on the
call's display parts. So the call is a stand-in for a hosted tool. Everything else
is as recorded, and the chat text is the recording's probe text.

`NN-*.request.json` is the request the app sends, and `NN-*.json` is the reply body
it receives.

| Files | What they show |
| --- | --- |
| `01-newest-page` | The newest page. Message 2's tool part (`callId: "call_probe"`) has `providerExecuted: true`, beside its usual members. |
| `02-exact-read` | The exact read of message 2, with the same mark and the call's whole output. |

Such a call has no `schemaPin` and no tool provider module, so it never gets a
template or tool records. Broca stores no tools its provider ran, so the mark
appears only on OpenCode 2 chats today.

`SHA256SUMS` covers every file except this README. A test in `occipital-module`
(`broca::tests::opencode2_pages`) serves the set again and fails if any byte
changes.
