#!/usr/bin/env python3
"""Independent §14.3 catalog canonicalizer, using only Python's standard library."""

import argparse
from decimal import Decimal
import hashlib
import json
from pathlib import Path
import unittest


def reject_constant(token):
    raise ValueError(f"not a JSON number: {token}")


def load(source):
    # Decimal preserves raw JSON numbers without passing through binary floats.
    return json.loads(
        source, parse_int=Decimal, parse_float=Decimal, parse_constant=reject_constant
    )


def number(value):
    if value.is_zero():
        return "0"
    sign, digits, exponent = value.as_tuple()
    digits = list(digits)
    while digits[-1] == 0:
        digits.pop()
        exponent += 1
    if len(digits) + exponent > 20 or exponent < -18:
        raise ValueError("catalog number exceeds the §4.2 decimal range")
    # Tuple construction and fixed formatting do not use Decimal context rounding.
    return format(Decimal((sign, digits, exponent)), "f")


def canonical(value):
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, Decimal):
        return number(value)
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, list):
        return "[" + ",".join(canonical(item) for item in value) + "]"
    if isinstance(value, dict):
        # Python sorts strings by Unicode scalar value, not UTF-16 code units.
        return "{" + ",".join(
            canonical(key) + ":" + canonical(value[key]) for key in sorted(value)
        ) + "}"
    raise TypeError(f"not a JSON value: {type(value).__name__}")


def digest(document):
    document["catalog"]["digest"] = None
    return hashlib.sha256(canonical(document).encode("utf-8")).hexdigest()


def overlaid(base, overlay):
    """The base catalog with an overlay applied, as catalog-tool-v1.1.json needs:
    overlay catalog members replace the base's, components are added and never
    replace one, and profilesAllow lists are extended. Anything else is an error."""
    unknown = set(overlay) - {"catalog", "components", "profilesAllow"}
    if unknown:
        raise ValueError(f"unknown overlay members: {sorted(unknown)}")
    for key, value in overlay.get("catalog", {}).items():
        base["catalog"][key] = value
    for name, component in overlay.get("components", {}).items():
        if name in base["components"]:
            raise ValueError(f"the overlay replaces component {name}")
        base["components"][name] = component
    for profile, names in overlay.get("profilesAllow", {}).items():
        listed = base["profilesAllow"][profile]
        for name in names:
            if name in listed:
                raise ValueError(f"{name} is already allowed in {profile}")
            listed.append(name)
    return base


class CanonicalTests(unittest.TestCase):
    def test_hand_derived_vectors(self):
        path = Path(__file__).with_name("canonical-json-vectors.json")
        vectors = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual(len(vectors), 6)
        for vector in vectors:
            with self.subTest(vector=vector["name"]):
                self.assertEqual(canonical(load(vector["source"])), vector["canonical"])

    def test_rejects_out_of_range_numbers(self):
        for source in ["1e20", "1e-19", "1e999999999999999999", "-1e20"]:
            with self.subTest(source=source), self.assertRaises(ValueError):
                canonical(load(source))

    def test_overlay_adds_and_never_replaces(self):
        base = load('{"catalog":{"version":"1.0"},"components":{"A":0},"profilesAllow":{"p":["A"]}}')
        merged = overlaid(base, load('{"catalog":{"version":"1.1"},"components":{"B":1},"profilesAllow":{"p":["B"]}}'))
        self.assertEqual(merged["catalog"]["version"], "1.1")
        self.assertEqual(sorted(merged["components"]), ["A", "B"])
        self.assertEqual(merged["profilesAllow"]["p"], ["A", "B"])
        for overlay in ['{"components":{"A":2}}', '{"profilesAllow":{"p":["A"]}}', '{"types":{}}']:
            with self.subTest(overlay=overlay), self.assertRaises(ValueError):
                overlaid(load('{"catalog":{},"components":{"A":0},"profilesAllow":{"p":["A"]}}'), load(overlay))

    def test_digest_ignores_published_digest(self):
        unpublished = load('{"catalog":{"digest":null},"value":0.5625}')
        published = load('{"catalog":{"digest":"old"},"value":0.5625}')
        self.assertEqual(digest(unpublished), digest(published))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("catalog", nargs="?", type=Path)
    parser.add_argument("--overlay", type=Path, help="apply this overlay to the catalog first")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=[__file__], verbosity=2)
    elif args.catalog:
        document = load(args.catalog.read_text(encoding="utf-8"))
        if args.overlay:
            document = overlaid(document, load(args.overlay.read_text(encoding="utf-8")))
        print(digest(document))
    else:
        parser.error("supply a catalog path or --self-test")


if __name__ == "__main__":
    main()
