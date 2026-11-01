# Pinned Unicode 16.0 data

Language 1.0 uses Unicode **16.0.0**, independently of Rust's bundled Unicode
version (§4.3). The committed Rust tables in `src/unicode/` are generated only
from the following official inputs, downloaded from
`https://www.unicode.org/Public/16.0.0/ucd/`:

| Input | SHA-256 |
| --- | --- |
| `UnicodeData.txt` | `ff58e5823bd095166564a006e47d111130813dcf8bf234ef79fa51a870edb48f` |
| `SpecialCasing.txt` | `8d5de354eef79f2395a54c9c7dcebbaf3d30fc962d0f85611ea97aa973a0c451` |
| `PropList.txt` | `53d614508e2a0b2305a8aa21cd60d993de9326cdf65993660dfcce4503548583` |
| `DerivedCoreProperties.txt` | `39d35161f2954497f69e08bdb9e701493f476a3d30222de20028feda36c1dabd` |

`tools/unicode_tables.py` verifies every input hash before writing anything. It
uses Python's standard library, not Python's Unicode database, locale rules or
case/whitespace helpers. To regenerate from the repository root:

```sh
mkdir -p target/scratch/ucd-16.0.0
for name in UnicodeData.txt SpecialCasing.txt PropList.txt DerivedCoreProperties.txt; do
  curl --fail --location "https://www.unicode.org/Public/16.0.0/ucd/$name" -o "target/scratch/ucd-16.0.0/$name" || exit
done
python3 crates/occipital-lang/tools/unicode_tables.py target/scratch/ucd-16.0.0
```

Use the same Python command with `--check` to compare all seven committed tables
without modifying them. `cargo test -p occipital-lang unicode::tests` independently
pins each generated file's SHA-256 to a literal; intentional regeneration requires
reviewing both the generated changes and those literals. Generated arrays are
excluded from rustfmt so their byte identity is stable.

## Table semantics

- `upper.rs` and `lower.rs` combine UnicodeData's simple case mappings with
  **unconditional full mappings** from SpecialCasing. Identity entries are omitted.
  The locale-independent `Final_Sigma` rule is retained in `lower.rs`; the runtime
  iterator applies its original-input context with the `Cased` and `Case_Ignorable`
  tables. Locale-tailored Turkish, Azeri and Lithuanian rules are excluded. The
  generator refuses an unknown conditional rule instead of silently dropping it.
- `white_space.rs` contains exactly `White_Space` from PropList, not
  `Pattern_White_Space`, category Zs or the host's whitespace predicate.
- `bidi_control.rs` contains PropList's `Bidi_Control` (directional marks included).
  `default_ignorable.rs` contains DerivedCoreProperties' `Default_Ignorable_Code_Point`,
  including reserved ranges. Along with explicit C0/C1 ranges these support the
  §3.11 **display escape predicate**, not canonical URL rewriting or §5.6 control
  replacement. Actual URL parsing and the sanitizer that replaces most controls
  are separate consumers; neither is implemented here.
- `cased.rs` and `case_ignorable.rs` contain their namesake derived properties for
  the context-sensitive lowercasing rule. Adjacent property ranges are merged.

The case iterator yields scalars without allocating a result, allowing a consumer
to calculate output length before building it (§6). No normalization occurs.
`Upper("ß") == "SS"` is tested directly through that iterator. Golden assertions
also cover dotted/dotless i, full expansions, final sigma, all White_Space members,
U+180E/U+200B, newly cased Cyrillic/Latin/Garay characters in 16.0, and the identity
mapping of A7CE/A7CF and Beria Erfe pairs that were added only in 17.0. The latter
expectations are independently visible in the 17.0 `UnicodeData.txt`, but 17.0 data
is **not** a generator input.

The pinned syntax dependency already decodes §3.4 named and numeric Markdown
entities, including non-scalar numeric references. `tests/syntax_entities.rs`
checks plain text, paired children and text beside references. Quoted prop strings
remain ordinary expression string literals. No entity table is added here.

Unicode data is distributed under the accompanying `LICENSE.txt` (Unicode-3.0).
