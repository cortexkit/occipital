# AFT notice samples

These samples were produced by AFT commit `01289469c7094d35f728011afa545df7d3a8b2cb` with a temporary test-only exporter. The commands, session and output are synthetic; no shell commands ran. The exporter used AFT's normal runner send-ID formatter, masking, record builder, durable display store, announcement encoder, and views snapshot and bucket-key encoder. It did not publish to a live bus.

## Records and announcements

Each `records/<name>.write.json` is the exact stored `display.read` reply: the arguments of the corresponding `display.write` (including `kind`, `runner`, `session`, `send_id`, and `body`). To send it as a push, use it as the `arguments` in `{ "name": "display.write", "arguments": … }` on the AFT-attested route. There is no claimed sender in the payload.

Each `records/<name>.ready.json` is the corresponding exact `display_record_ready.v1` event body. AFT uses its `record_id` as the NATS deduplication message ID. These files were emitted by the announcement callback only after the exported bytes had been read back from the durable store.

| Name | Case |
| --- | --- |
| `finished` | Exit 0 after 22 min; has a description and output preview. |
| `failed` | Exit 1 after 3 min; the preview's cut flag is set. |
| `stopped` | Command killed after 3 min. |
| `timed-out` | Command timed out after 30 min; the title falls back to its command. |
| `still-running` | Reminder after 20 min; shares a fold key with `finished`. |
| `watch-match` | Watch matched `error: build failed`; no fold key. |
| `watch-task-exit` | Watched command exited with 0; no fold key. |

## Views

Each `views/<hex>.json` contains the exact whole one-pin `tool.views` reply stored in `aft.views`. The filename stem is the lowercase hex encoding of the pin's UTF-8 bytes, i.e. the bucket key. The pin is also the only key under `views` in that file.

| Pin | Bucket key |
| --- | --- |
| `nv1:aft:bash_finished:1` | `6e76313a6166743a626173685f66696e69736865643a31` |
| `nv1:aft:bash_still_running:1` | `6e76313a6166743a626173685f7374696c6c5f72756e6e696e673a31` |
| `nv1:aft:bash_watch:1` | `6e76313a6166743a626173685f77617463683a31` |

`SHA256SUMS` covers every file in this folder except itself. Run `shasum -a 256 -c SHA256SUMS` from this directory to verify the files.
