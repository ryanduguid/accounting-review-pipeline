# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""Hold each native release caller and its crate to the binary release contract.

release-binary.yml refuses a release whose caller or crate breaks these rules,
but only after a tag is pushed. Checking them on every change keeps a tag from
being the first place a drift shows up.
"""

import subprocess
import tomllib
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
# Component directory leaf -> the Cargo binary its release ships.
NATIVE = {
    "monthly-close-control-plane": "close-control",
    "review-ready-gate": "review-ready",
}
PERMISSIONS = {
    "actions": "read",
    "attestations": "write",
    "contents": "write",
    "id-token": "write",
}


class NativeReleaseWorkflowTests(unittest.TestCase):
    def test_callers_use_the_binary_release_family(self) -> None:
        for component, binary in NATIVE.items():
            with self.subTest(component=component):
                document = yaml.safe_load(
                    (ROOT / f".github/workflows/release-{component}.yml").read_text(
                        encoding="utf-8"
                    )
                )
                # PyYAML reads the bare key `on` as True.
                self.assertEqual(
                    document[True], {"push": {"tags": [f"{component}/v*"]}}
                )
                self.assertEqual(document["permissions"], {"contents": "read"})
                self.assertEqual(set(document["jobs"]), {"release"})
                job = document["jobs"]["release"]
                self.assertEqual(job["permissions"], PERMISSIONS)
                self.assertRegex(
                    job["uses"],
                    r"^ryanduguid/release-policy/\.github/workflows/"
                    r"release-binary\.yml@[0-9a-f]{40}$",
                )
                inputs = job["with"]
                self.assertEqual(inputs["artifact-stem"], component)
                self.assertEqual(inputs["tag-prefix"], component)
                self.assertEqual(inputs["source-directory"], f"packages/{component}")
                self.assertEqual(inputs["binary"], binary)

    def test_crates_provide_what_the_binary_release_requires(self) -> None:
        for component, binary in NATIVE.items():
            with self.subTest(component=component):
                crate = ROOT / "packages" / component
                version = (crate / "VERSION").read_text(encoding="utf-8")
                self.assertRegex(version, r"^(0|[1-9][0-9]*)(\.(0|[1-9][0-9]*)){2}\n$")
                version = version.strip()
                manifest = tomllib.loads((crate / "Cargo.toml").read_text(encoding="utf-8"))
                package = manifest["package"]
                self.assertEqual(package["version"], version)
                self.assertIs(package["publish"], False)
                self.assertIn(binary, {item["name"] for item in manifest.get("bin", [])})
                toolchain = tomllib.loads(
                    (crate / "rust-toolchain.toml").read_text(encoding="utf-8")
                )
                self.assertRegex(toolchain["toolchain"]["channel"], r"^\d+\.\d+\.\d+$")
                notes = (crate / "RELEASE_NOTES.md").read_text(encoding="utf-8")
                # The release gate requires exactly `# v<version>` at tag time;
                # an `(unreleased)` suffix is allowed between releases.
                self.assertTrue(notes.startswith(f"# v{version}"), notes.splitlines()[0])
                for document in ("LICENSE", "NOTICE", "README.md"):
                    self.assertTrue((crate / document).is_file(), document)
                tracked = subprocess.run(
                    ["git", "ls-files", "--error-unmatch", f"packages/{component}/Cargo.lock"],
                    cwd=ROOT,
                    capture_output=True,
                    check=False,
                )
                self.assertEqual(tracked.returncode, 0, "Cargo.lock is not tracked")


if __name__ == "__main__":
    unittest.main()
