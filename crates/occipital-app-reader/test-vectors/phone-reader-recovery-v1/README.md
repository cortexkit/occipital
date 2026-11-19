# App recovery replies v1

Each numbered `.request.json` is sent through the app reader; the matching `.json` is the response or refusal body the app receives. The runner and storage fixtures are synthetic.

| Files | What they show |
| --- | --- |
| 01 | The runner finished the send without delivering it. The reply is terminal `not_delivered`, not a successful receipt. |
| 02 | Stop could not verify how the run ended. It answers `provider_unavailable`, not a remembered `not_active`. |
| 03 | A prompt send cannot read `role.describe`, the runner's list of supported capabilities. The app can retry rather than placing the upload in the project. |
| 04 | A media read cannot obtain that capability list either; it does not report the file as unsupported. |
| 05 | The runner's plan is too large and no previous read established whether it includes the `say` tool. A page containing `say` calls is refused rather than returned with guessed parts. |
| 06 | Callosum did not attest a device. The reader refuses the route rather than admitting an operator. |
| 07 | A send with neither `clientMessageId` nor `send_id` now returns `invalid_request` before the runner can deliver it even once. |
