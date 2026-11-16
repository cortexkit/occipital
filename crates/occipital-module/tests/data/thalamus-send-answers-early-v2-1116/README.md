# send-answers-early-v2

One body, recorded by the real Pi early-answer test (`pi_hosted::tests::real::send_answers_early`) on
Agent Runner commit 5f278103f with Pi 1.5.0, through THALAMUS_SEND_EARLY_RECORD_DIR.

`02-pi-send-reply.json` replaces v1's file of the same name for Pi only: a Pi head declares no
`retract`, so its `pending` reply names no `submission_id` (llm-runner §9, "a pending reply to any send
carries submission_id exactly when the runner declares retract"). The send is still followed with
`session.send_status` by `send_id`, and "Send now" names it by `send_id`. OpenCode 2 and Claude Code
replies are unchanged (v1 files 01 and 03).
