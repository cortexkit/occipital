# cortexkit-occipital-registry

Tool templates for the CortexKit apps. A tool provider describes how its
tool's calls draw in the apps with a view: a record schema and row and page
templates written in the `cortexkit-occipital-lang` language. This crate
holds what both sides of that agree on:

- **The template document,** `occipital.template/1`: parsing, its canonical
  JSON and its digest, so a template's digest is the same on every computer
  and phone.
- **The view checks** (`view`): the template checks Occipital runs when it
  fetches a view, and the apps run before they draw one.
- **`occipital-check`:** the same checks as a command, for a provider's CI.

[TEMPLATES.md](TEMPLATES.md) is the guide to writing templates.

```sh
cargo install cortexkit-occipital-registry
occipital-check --catalog
occipital-check my-tool-view.json
```

```rust
use occipital_registry::Template;

let template = Template::parse(br#"{"schema": "occipital.template/1",
    "profile": "tool_row", "executionDialect": "1.0",
    "catalog": {"version": "1.1", "digest": "sha256:a95693cfabc0bf2d7e7276fd1c634128d96c11605c1cdfe480da9ac2418683a4"},
    "reads": [], "source": "export const row = Text(\"done\")\n"}"#)
.expect("a valid template");
assert!(template.digest().expect("canonical").starts_with("sha256:"));
```

The library performs no I/O and never panics on its input.

Licensed under MIT.

## Vectors

`test-vectors/rich-ui-registry-v1/` holds the canonical JSON and template
cases every implementation must reproduce. The `registry_vectors` example
writes them from the public API; regenerate them from the workspace root on the
Linux build server:

```sh
cargo run --locked -p cortexkit-occipital-registry --example registry_vectors
```

- An accepted case has `.input.json`, `.canonical.json` and `.digest` files.
  Canonical JSON has no trailing newline; a digest has the `sha256:` prefix and
  a final line feed. `canon-*` inputs exercise JSON itself; `template-*` inputs
  also pass the template parser.
- A refused case has raw `.input` bytes, some deliberately invalid UTF-8, and a
  `.code` file ending in a line feed. `precedence-*` inputs combine faults to
  pin which one is reported.
- `SHA256SUMS` covers every other file: lowercase hex, two spaces, bytewise
  sorted paths and a final line feed.

The precedence input `{"a":1.0,"a":2}` reports `number_not_integer` at byte 5,
and `{"a":1,"a":1.0}` reports `duplicate_key` at byte 7, which tells a parser
that reports the lowest byte offset from one that checks all numbers or all
keys first.

`tests/vectors.rs` runs the example's writer, compares every file name and
byte with the committed directory, replays each case through the public API,
and recomputes every checksum with its own SHA-256. `tests/canon.rs` checks
the canonical string and ordering cases against hand-written expectations.
