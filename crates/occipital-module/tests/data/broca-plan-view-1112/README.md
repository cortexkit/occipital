# session.plan_view samples for readers (broca 0.4.21)

All synthetic: fixture text and tools, no real system prompt.

- `applied.json`: a head on the new storage after a refresh applied at generation 1. Two system
  items with presets (`prefrontal-core`/`head`, `aft`/`worker`), three tools (`bash` and `read`
  from `aft`, `board` from `prefrontal-core`), `applied_at_ms`, and `newest_step` naming its
  reply's message id (`reply_mid`). Written by broca's real `PlanViewResponse` serializer through
  the store actor's real read (test `plan_view_reader_samples`), framed as the wire frames a
  reply: `{"result": ...}`.
- `pending.json`: the same session after generation 2 was accepted under `on_prefix_rebuild`,
  not yet applied. `pending` is `{"generation": 2}` only. Broca never serves `pending.notes`:
  it does not tell the model about a pending change, so no Broca reply carries notes.
- `no-plan.json`: the refusal for a session on the new storage with no plan (error frame body),
  captured through the real serve path.
- `not-served-on-old-storage.json`: the refusal for a session kept on today's storage, the exact
  error frame body broca 0.4.21 served live on 2026-11-12.
