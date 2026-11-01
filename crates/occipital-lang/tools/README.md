# Catalog publication digest

`catalog_digest.py` is an independent, standard-library-only Python canonicalizer,
not an invocation or binding of `occipital-lang`. It implements §14.3 identity JSON:
UTF-8 without whitespace, recursively scalar-sorted keys, array order retained,
minimal JSON string escaping, and exact §4.2 decimal tokens (no exponent, trailing
fraction zeros, or negative zero). JSON has no missing-value token: absent fields
stay absent and explicit nulls stay null. It does not implement the ordered codec.

The exact command used from the repository root to produce the committed literal:

```sh
python3 crates/occipital-lang/tools/catalog_digest.py crates/occipital-lang/catalog/catalog-v1.json > crates/occipital-lang/catalog/catalog-v1.digest
```

The tool sets `catalog.digest` to null before canonicalizing and hashing with
SHA-256, as `digestRule` requires. The literal is not the catalog file checksum.
The Rust catalog digest test reads this literal; Rust output must never generate it.

The tool catalog 1.1, which only tool templates name, is catalog 1.0 with the
overlay `catalog-tool-v1.1.json` applied. The tool applies the overlay itself,
by the same rules as the Rust loader but written independently: overlay
`catalog` members replace 1.0's, components are added and never replace one,
and `profilesAllow` lists are extended. Its literal comes from:

```sh
python3 crates/occipital-lang/tools/catalog_digest.py crates/occipital-lang/catalog/catalog-v1.json --overlay crates/occipital-lang/catalog/catalog-tool-v1.1.json > crates/occipital-lang/catalog/catalog-tool-v1.1.digest
```

Rerun it after every edit to the overlay.

Run the independent tool's tests with:

```sh
python3 crates/occipital-lang/tools/catalog_digest.py --self-test
```

The six shared, hand-derived vectors cover catalog numbers, exact decimal/id
boundaries, recursive sorting, Unicode scalar order versus UTF-16 order, minimal
escaping, and empty/null values. Both implementations check them independently;
neither implementation's output supplies expected values for the other.
