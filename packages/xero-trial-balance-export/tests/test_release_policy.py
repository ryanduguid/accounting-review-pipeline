"""The active release caller is the shared Python policy with a caller-side PyPI job."""

from pathlib import Path
import unittest

MONOREPO_ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = MONOREPO_ROOT / ".github" / "workflows" / "release-xero-trial-balance-export.yml"


class ReleasePolicyTests(unittest.TestCase):
    def workflow(self) -> str:
        if not (MONOREPO_ROOT / ".git").exists() and not (MONOREPO_ROOT / "IMPORTS.md").is_file():
            self.skipTest("release workflow is not included in source distributions")
        return WORKFLOW.read_text(encoding="utf-8")

    def test_release_is_triggered_only_by_namespaced_tags(self) -> None:
        workflow = self.workflow()
        self.assertIn('tags:\n      - "xero-trial-balance-export/v*"', workflow)
        self.assertNotIn("workflow_dispatch:", workflow)
        self.assertNotIn("inputs.tag", workflow)

    def test_release_uses_the_shared_python_policy(self) -> None:
        release_job = self.workflow().split("  release:\n", 1)[1].split("\n  pypi:", 1)[0]
        self.assertIn(
            "uses: ryanduguid/release-policy/.github/workflows/release-python.yml@"
            "3ff09b654a17b9a3b55548e25e6108ee582b00c4",
            release_job,
        )
        self.assertIn("source-directory: packages/xero-trial-balance-export", release_job)
        self.assertIn("tag-prefix: xero-trial-balance-export", release_job)
        self.assertIn("upload-dist-artifact: true", release_job)
        self.assertNotIn("artifact-stem:", release_job)
        self.assertNotIn("version-parser:", release_job)

    def test_pypi_publishes_only_the_exact_attested_distribution(self) -> None:
        pypi_job = self.workflow().split("  pypi:\n", 1)[1]
        self.assertIn("needs: release", pypi_job)
        self.assertIn("name: pypi-xero-trial-balance-export", pypi_job)
        self.assertIn("id-token: write", pypi_job)
        self.assertIn(
            "name: dist-${{ needs.release.outputs.stem }}-${{ needs.release.outputs.version }}",
            pypi_job,
        )
        self.assertIn("pypa/gh-action-pypi-publish@dc37677b2e1c63e2034f94d8a5b11f265b73ba33", pypi_job)
        self.assertNotIn("python -m build", pypi_job)


if __name__ == "__main__":
    unittest.main()
