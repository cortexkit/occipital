# Model name fixtures

Replies from the two modules `display.model_names` reads, copied byte for byte
from their owners' own sample sets, so the tests decode what those modules
actually serve.

- `fusiform-*.json`: the `response` of five cases in fusiform's
  `crates/fusiform-module/fixtures/served-payloads.json` at commit
  `47d19ebe3a555971f31f6ed3e37f54335ca9ab2d`, which fusiform's CI regenerates
  from its real serving path. Each is written as compact JSON.
  - `fusiform-first-provider-page.json`: "catalog.get — first provider page",
    a paged reply with `next_page_token`.
  - `fusiform-one-model-current.json`: "catalog.get — one model, current"
    (`anthropic/claude-sonnet-4-5`).
  - `fusiform-provider-default-endpoint.json`: "catalog.get — provider default
    endpoint" (`openai/gpt-6.1-sol`).
  - `fusiform-curated-decision-model.json`: "catalog.get — a curated decision
    model", a hand-added model with no `provider.name`.
  - `fusiform-named-retired-model.json`: "catalog.get — a named read of a
    retired model", whose `models` is empty.
- `fusiform-attachment-bounds.json` and `fusiform-gateway-without-bounds.json`:
  the `response` of "catalog.get — first-party attachment bounds"
  (`anthropic/claude-opus-5-5` with `limit.images_per_request`,
  `limit.image_bytes` and `limit.image_long_edge_px`) and "catalog.get — a
  gateway without attachment bounds" (`anyapi/anthropic/claude-opus-4-6`), from
  the same file at commit `b98cdd2c92e2b3e8223c99e25418bf6478d4649b`, written
  the same way.
- `routing-resolved.json` and `routing-warming.json`: `resolved.json` and
  `warming.json` from prefrontal's tag `route-catalog-targets-vectors-v1`
  (commit `0afec2ec0c1206057656bd97a6980e6b1e435d90`), folder
  `test-vectors/route-catalog-targets-v1/`. Their SHA-256 sums equal that
  folder's `SHA256SUMS`.

The tests serve these replies as pages in an order they choose; only the page
tokens joining them are the tests' own.
