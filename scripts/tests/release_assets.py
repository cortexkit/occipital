"""Exercises the release file format and checks in scripts/release/assets.py
without building Rust: a small Python stand-in plays ck-occipital."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from zipfile import ZipFile

from scripts.release import assets


REVISION = "a" * 40
VERSION = "0.1.0"
TARGETS = ("darwin-arm64", "linux-x64", "linux-arm64", "windows-x64", "windows-arm64")


class ReleaseAssets(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)
        self.source = self.root / "source"
        self.source.mkdir()
        self.built = self.root / "built"

    def pack(self, target):
        assets.pack(argparse.Namespace(source=self.source, directory=self.built, target=target))

    def verify(self, target):
        assets.verify_target(argparse.Namespace(directory=self.built, target=target, revision=REVISION,
                                                version=VERSION))

    def build(self, target, *, version=VERSION, revision=REVISION, manifest_revision=REVISION):
        """A stand-in ck-occipital that answers --version and --manifest as the
        real one does, and checks it runs under its `ckdev-` name."""
        manifest = {"module_id": "occipital", "module_version": version,
                    "provenance": {"build_git_sha": manifest_revision}}
        binary = self.source / "ck-occipital"
        binary.write_text(f'''#!/usr/bin/env python3
import pathlib, sys
assert pathlib.Path(sys.argv[0]).name == "ckdev-occipital"
if sys.argv[1] == "--version":
    print("ck-occipital {version} {revision}")
elif sys.argv[1] == "--manifest":
    print({json.dumps(manifest)!r})
else:
    sys.exit(2)
''')
        binary.chmod(0o755)
        self.pack(target)

    def test_a_verified_archive_gets_its_checksum(self):
        self.build("linux-x64")
        self.verify("linux-x64")
        archive = self.built / "ck-occipital-linux-x64.zip"
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        self.assertEqual(archive.with_suffix(".zip.sha256").read_text(), f"{digest}  {archive.name}\n")

    def test_a_binary_reporting_another_release_gets_no_checksum(self):
        cases = {
            "another version": {"version": "0.2.0"},
            "another revision": {"revision": "b" * 40},
            "another manifest revision": {"manifest_revision": "b" * 40},
        }
        for index, (case, wrong) in enumerate(cases.items()):
            with self.subTest(case):
                self.built = self.root / f"built-{index}"
                self.build("linux-x64", **wrong)
                with self.assertRaisesRegex(ValueError, "ck-occipital-linux-x64.zip"):
                    self.verify("linux-x64")
                self.assertEqual(list(self.built.glob("*.sha256")), [])

    def make_inventory(self):
        for target in TARGETS:
            name = "ck-occipital" + (".exe" if target.startswith("windows-") else "")
            path = self.source / name
            path.write_text(f"stand-in for {target}\n")
            path.chmod(0o755)
            self.pack(target)
            archive = self.built / f"ck-occipital-{target}.zip"
            with ZipFile(archive) as zipped:
                self.assertEqual(zipped.namelist(), [name])
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            archive.with_suffix(".zip.sha256").write_text(f"{digest}  {archive.name}\n")

    def assemble(self, output, *, dry_run=False):
        assets.assemble(argparse.Namespace(directory=self.built, output=output, revision=REVISION,
                                           dry_run=dry_run))

    def test_a_tag_needs_the_core_floor_and_a_rehearsal_does_not(self):
        self.make_inventory()
        dry = self.root / "dry"
        release = self.root / "release"
        with patch.dict(os.environ, {"OCCIPITAL_REQUIRES_CORE": ""}):
            with self.assertRaisesRegex(ValueError, "OCCIPITAL_REQUIRES_CORE must be set"):
                self.assemble(release)
            self.assemble(dry, dry_run=True)
        with patch.dict(os.environ, {"OCCIPITAL_REQUIRES_CORE": "1.2.3"}):
            self.assemble(release)
        manifest = {"binaries": {"ck-occipital": {"reports": REVISION}}}
        self.assertEqual(json.loads((dry / "release-manifest.json").read_text()), manifest)
        self.assertEqual(json.loads((release / "release-manifest.json").read_text()),
                         {**manifest, "requires_core": "1.2.3"})
        # Every target ships, Windows included: five ZIPs and five sidecars.
        for output in (dry, release):
            self.assertEqual(sorted(p.name for p in output.glob("*.zip")),
                             sorted(f"ck-occipital-{target}.zip" for target in TARGETS))
            self.assertEqual(len(list(output.glob("*.zip.sha256"))), 5)

    def test_assembly_refuses_a_corrupt_or_missing_file(self):
        self.make_inventory()
        (self.built / "ck-occipital-windows-arm64.zip").write_bytes(b"not the archived binary")
        with patch.dict(os.environ, {"OCCIPITAL_REQUIRES_CORE": "1.2.3"}):
            with self.assertRaisesRegex(ValueError, "ck-occipital-windows-arm64.zip.sha256: checksum does not match"):
                self.assemble(self.root / "release")
        self.built = self.root / "built-missing"
        self.make_inventory()
        (self.built / "ck-occipital-linux-arm64.zip.sha256").unlink()
        with patch.dict(os.environ, {"OCCIPITAL_REQUIRES_CORE": "1.2.3"}):
            with self.assertRaisesRegex(ValueError, "wrong asset inventory: missing .*linux-arm64"):
                self.assemble(self.root / "release-missing")

    def test_a_non_windows_archive_must_hold_an_executable(self):
        binary = self.source / "ck-occipital"
        binary.write_text("not executable\n")
        binary.chmod(0o644)
        self.pack("linux-x64")
        with self.assertRaisesRegex(ValueError, "must be a regular executable"):
            self.verify("linux-x64")


if __name__ == "__main__":
    unittest.main()
