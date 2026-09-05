"""The release workflow is the shared archive policy, not a local copy."""

from pathlib import Path
import re
import shlex
import unittest

ROOT = Path(__file__).resolve().parents[1]
MONOREPO = ROOT.parents[1]

POLICY_REF = "ryanduguid/release-policy/.github/workflows/release-archive.yml@"


class ReleaseArchiveTests(unittest.TestCase):
    def test_release_workflow_uses_the_shared_archive_policy(self) -> None:
        workflow = (MONOREPO / ".github" / "workflows" / "release-accounting-excel-toolkit.yml").read_text(
            encoding="utf-8",
        )
        self.assertIn(POLICY_REF, workflow)
        # The policy must be pinned to an immutable 40-hex commit, never a
        # branch or a tag. Which commit it names moves with every policy bump
        # and is reviewed in that bump's own diff, so it is not frozen here.
        pin = workflow.split(POLICY_REF, 1)[1].split()[0]
        self.assertEqual(len(pin), 40, pin)
        self.assertTrue(set(pin) <= set("0123456789abcdef"), pin)
        self.assertIn("artifact-stem: accounting-excel-toolkit", workflow)
        self.assertIn("source-directory: adapters/accounting-excel-toolkit", workflow)
        self.assertIn("tag-prefix: accounting-excel-toolkit", workflow)
        self.assertIn('"accounting-excel-toolkit/v*"', workflow)
        self.assertNotIn("\n          git archive ", workflow)

    def test_release_guide_verifies_spdx_for_both_archives(self) -> None:
        guide = (ROOT / "RELEASING.md").read_text(encoding="utf-8")
        loop = re.search(r"for archive in ([^\n]+); do\n(.*?)\ndone", guide, re.DOTALL)
        self.assertIsNotNone(loop, "The guide must verify each archive's SPDX attestation")
        assert loop is not None
        self.assertEqual(shlex.split(loop.group(1)), [
            "accounting-excel-toolkit-${tag##*/v}.zip",
            "accounting-excel-toolkit-${tag##*/v}.tar.gz",
        ])
        for required in (
            'gh attestation verify "$archive" -R "$repo"',
            "--predicate-type https://spdx.dev/Document/v2.3",
            '--source-digest "$release_commit"',
            '--source-ref "refs/tags/$tag"',
            "--signer-workflow ryanduguid/release-policy/.github/workflows/publish-archives.yml",
            "--signer-digest 787db4590e725cfd37104c8a9dd9e75f7fd4c018",
        ):
            self.assertIn(required, loop.group(2))


if __name__ == "__main__":
    unittest.main()
