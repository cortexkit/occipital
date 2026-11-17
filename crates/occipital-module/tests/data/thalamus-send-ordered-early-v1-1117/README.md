# Idle ordered send: early Pi reply

`pi-after-send-reply.json` is the unmodified UTF-8 body returned by `session.send`
on an idle, adopted Pi 1.5.2 head. It has no wrapper or trailing newline. The request
named `delivery: "queue"`, which Pi normalizes to the ordered `["idle_start"]`
delivery list. The reply is `{"state":"pending"}`: acceptance stored the send but
not a run, so it names no `run_id`. Pi declares no `retract` group, so it names no
`submission_id` either. Mode selection creates the run later; the test also checks
that the send lands as a turn and that the run completes.

Recorded on Linux from clean commit
`7499b889a14ca80e0a8fdeea197e577c5f964d45`, with llm-runner 0.8.60, by
`pi_hosted::tests::real::send_answers_early::real_hosted_pi_first_send_answers_pending_then_runs_the_turn`.
The recorder is `THALAMUS_SEND_ORDERED_RECORD_PATH`; the same test still covers the
idle own-run `steer` case separately. The filtered real-Pi run passed all 13 selected
send and steer tests.

## Before the change

There is deliberately no fabricated before-reply sample. The old idle ordered
path answered only after its synchronous observation pass handed over the prompt;
the restart regression on an in-memory OpenCode 2 host substitute observed
`active` rather than `pending`. With the old `send.rs` restored from base commit
`607cbbb00286d71079d73e3daf52b9104d6b36f8`, the Pi ordered-list and legacy-queue
pause tests both failed with:

```text
idle ordered session.send reply waited for blocked Host::deliver: Elapsed(())
```

The equivalent OpenCode 2 and Claude Code tests also failed, and the restart test
failed with `left: String("active"), right: "pending"` (0 passed, 5 failed).
That red run is recorded in
`.cortexkit/internal-docs/investigations/ordered-send-early-answer.md`, under
"Red proof". A real-Pi before capture was attempted in the required disposable
scratch tree, but its fixture refused the build because that tree has no Git
metadata, so no before capture was made; the fixture's clean-build provenance guard
was not bypassed.

No existing sample set was edited. `SHA256SUMS` pins this README and the captured
reply body.
