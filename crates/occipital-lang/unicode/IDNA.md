# Addresses and pinned Unicode 16 IDNA

`Address::parse` is the one §3.11 parser/serializer for `Open` and address
props. It is pure Rust and uses no platform URL or Unicode database. The
constructor is the only way to obtain an `Address`; serialization exposes the
canonical string, not the original input. All refusals return `E_SCHEME`.
Network policy and the app's full-sheet/second-tap interaction are separate
host responsibilities. No parsing step fetches anything.

## Resolved URL rule set

The owner's ruling resolves §3.11's previously unspecified URL/IDNA rules.
The governing invariant is that the **canonical output** is a fixed point of
WHATWG URL parsing/serialization. Only that output may be opened, never the
original input or the display representation.

1. Only absolute `http://` and `https://`, with a host; lowercase the scheme.
2. UTS #46 rev. 33, Unicode 16.0.0 ToASCII, nontransitional;
   CheckHyphens, CheckBidi, CheckJoiners, UseSTD3ASCIIRules and VerifyDnsLength
   enabled, invalid Punycode refused. Map, normalize to NFC, decode A-labels,
   validate (without repairing decoded A-labels), then encode with RFC 3492.
   Bidi validation covers every label if any decoded label triggers it.
   ASCII labels have 1–63 octets, total host at most 253, lowercase A-labels.
3. Refuse IP literals (including bracketed IPv6 and every IPv4 spelling),
   hosts with a WHATWG-numeric final label, single-label hosts, localhost,
   local suffixes `.localhost`, `.local`, `.internal`, `.home.arpa` (including
   the `home.arpa` zone itself), and trailing dots, including mapped dots.
   A numeric final label is all ASCII digits or `0x` followed by hex; **bare
   `0x` is also refused** because WHATWG's IPv4 number algorithm reads it as 0.
4. Any authority `@` is refused, including empty userinfo.
5. Ports are decimal 1–65535, without leading zeros. Remove 80 for HTTP and
   443 for HTTPS; retain other ports in canonical decimal. Empty, zero,
   signed, nondigit and out-of-range ports are refused.
6. Each `%` must be followed by two ASCII hex digits. Preserve valid escapes
   without decoding, uppercase their hex. Further hex digits are ordinary
   following text (`%001` is escape `%00` and text `1`). Host percent escapes
   fail STD3 rather than being decoded by a platform host parser.
7. UTF-8 percent-encode outside RFC3986 pchar plus `/` in paths, or pchar
   plus `/` and `?` in queries/fragments. **Also encode query apostrophe as
   `%27`**: the owner confirmed this exception because WHATWG's HTTP(S)
   special-query encode set requires it. Apostrophes stay raw in paths and
   fragments. Fragments are allowed, including an empty fragment.
8. Empty path becomes `/`. Refuse path segments `.` or `..`, also when one
   or both dots are any casing of `%2e`. Do not collapse them or repeated
   slashes. Dot-like text in queries/fragments is not a path segment.
9. Refuse backslashes, raw C0/C1 controls (including tab/CR/LF), space
   anywhere, and relative URLs. Never trim or strip browser whitespace.
10. Both original input and normalized output have at most 2,048 Unicode
    scalars; the canonical output is ASCII, so its byte length is its scalar
    count. Mapping/encoding can shorten or lengthen the input.

The existing `DisplayText::address_sheet` escapes C0/C1, bidi controls,
directional marks and default ignorables as uppercase `\u{HEX}`, without
truncation. Canonical URLs are already ASCII: Unicode path/query/fragment
invisibles appear as their percent-encoded bytes, not as decoded controls.
Authority-sheet strings that are not URLs still use the same display helper.

## Generated data and reproduction

The generator verifies these official source files before writing anything:

| Input | SHA-256 |
| --- | --- |
| [IdnaMappingTable.txt](https://www.unicode.org/Public/idna/16.0.0/IdnaMappingTable.txt) | `6db2ef4ed35f3b3de74ebc2e00404a9607f76d499f576b8d4043cf14f1ed175c` |
| [UnicodeData.txt](https://www.unicode.org/Public/16.0.0/ucd/UnicodeData.txt) | `ff58e5823bd095166564a006e47d111130813dcf8bf234ef79fa51a870edb48f` |
| [DerivedNormalizationProps.txt](https://www.unicode.org/Public/16.0.0/ucd/DerivedNormalizationProps.txt) | `4d4c03892dea9146d674b686e495df2d55a28d071ac474041d73518f887abddc` |
| [DerivedBidiClass.txt](https://www.unicode.org/Public/16.0.0/ucd/extracted/DerivedBidiClass.txt) | `71ed943a49c58568d8d92e80ecc2ba2f06e62aee9c8ebb0e6e8bd2c3ed8b180e` |
| [DerivedJoiningType.txt](https://www.unicode.org/Public/16.0.0/ucd/extracted/DerivedJoiningType.txt) | `6bd08b97da66b70ccfdab105a352de2984e02625239ec5695422c99b33d854f0` |

Run from the repository root with these five files in an input directory:

```sh
python3 crates/occipital-lang/tools/idna_tables.py <input-directory>
python3 crates/occipital-lang/tools/idna_tables.py <input-directory> --check
```

The four generated `src/address/tables/*.rs` files each have an independent
literal SHA-256 test. `nfc.rs` contains canonical decompositions, composition
pairs excluding Full_Composition_Exclusion, canonical combining classes, and
General_Category=Mark ranges. Hangul decomposition/composition is algorithmic.
The bidi/joining generator applies UAX #44 `@missing` defaults before explicit
values. Unicode's license is in `LICENSE.txt` beside this file.

## Vectors and independent browser check

`tests/vectors/urls.json` is committed. Each row names its rule and derivation;
repeated-string descriptors spell out exact boundary cases without huge
literals. Expected values are never captured from the Rust implementation.
`tools/url_vectors.py` writes these hand-derived values. Reference A-labels
use RFC 3492; Python's algorithmic Punycode codec was used as an independent
cross-check, not Python's IDNA or Unicode database.

```sh
python3 crates/occipital-lang/tools/url_vectors.py
node crates/occipital-lang/tools/check_url_vectors.mjs
cargo test -p occipital-lang --all-features address::tests
```

The Rust tests compare every accepted output with its committed WHATWG
reference, check canonical idempotence, exact full display and serialization,
and assert `E_SCHEME` for every refusal. Node's independent WHATWG URL parser
checks every committed accepted canonical string without rewriting any vector.
It intentionally does not demand that the original input serialize identically:
WHATWG preserves input percent-escape case, while these rules uppercase it.

`tools/check_url_rules.py` is a build-server-only break check. Stage the live
files first, then run it on `runon` with no concurrent Cargo invocation. Its 24
controls cover every numbered rule, the IDNA flags, both length caps, display
escaping and each table checksum. Each control must fail exactly its named
test while the other 23 pass; every temporary mutation is restored from the
job's private scratch copy, without writing the read-only Git index.
Because the remote index may represent the base commit rather than the locally
staged files, the script's `git diff --no-index --stat` pairs compare each
mutated/restored file against its saved live-source copy.
