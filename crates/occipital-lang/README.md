# cortexkit-occipital-lang

The rich UI language the CortexKit apps draw with: agents write it in their
replies, tool providers write tool row and page templates in it, and every
app runs this one implementation, so a reply or row draws the same
everywhere. The crate parses and checks programs, evaluates them to a tree of
catalog components, and measures that tree for layout. It does no I/O.

```sh
cargo add cortexkit-occipital-lang
```

```rust
use occipital_lang::{check_program, CheckContext, HostValues, Profile};

let result = check_program(
    b"Tests finished. <Badge tone=\"success\">17 passed</Badge>\n",
    &CheckContext::new(Profile::Reply),
    &HostValues::default(),
)
.expect("the checker runs");
assert!(result.ok());
```

- **Catalogs:** `catalog/catalog-v1.json` lists every component, prop and
  layout rule for replies and pages. `catalog/catalog-tool-v1.1.json` adds the
  parts only tool rows and pages use, such as `Slot`.
- **Tool templates:** `evaluate_template` evaluates a row or page template
  once with a call's input and view.
- **Checking tool templates in CI:** the `occipital-check` command in
  `cortexkit-occipital-registry`.
- **Numbers:** the language's decimals are exact. The crate doesn't turn on
  serde_json's `arbitrary_precision`, which Cargo would apply to every
  serde_json user in your app. Numbers the crate reads from source text stay
  exact. A number your app passes in a `serde_json::Value` is taken as
  serde_json holds it, so it is exact only if your app's serde_json keeps
  exact numbers.

Licensed under MIT.
