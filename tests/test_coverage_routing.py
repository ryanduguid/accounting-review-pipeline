"""Every Codecov download must have a producer on the actual Python matrix."""
from __future__ import annotations

import re
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]


class CoverageRoutingTests(unittest.TestCase):
    def test_coverage_artifact_has_a_live_matrix_producer(self) -> None:
        native_jobs = yaml.safe_load((ROOT / '.github/workflows/ci.yml').read_text())['jobs']
        self.assertIn('rust', native_jobs)
        self.assertNotIn('codecov', native_jobs)
        for filename in ('ci-package.yml',):
            with self.subTest(workflow=filename):
                jobs = yaml.safe_load((ROOT / '.github/workflows' / filename).read_text())['jobs']
                versions = jobs['test']['strategy']['matrix']['python']
                download = next(step for step in jobs['codecov']['steps']
                                if step.get('uses', '').startswith('actions/download-artifact@'))
                name = download['with']['name']
                uploads = [step for step in jobs['test']['steps']
                           if step.get('uses', '').startswith('actions/upload-artifact@')
                           and step.get('with', {}).get('name') == name]
                self.assertEqual(len(uploads), 1, f'{filename}: {name} has no unique producer')
                condition = re.fullmatch(r"matrix\.python\s*==\s*['\"]([^'\"]+)['\"]", uploads[0]['if'])
                self.assertIsNotNone(condition, f'{filename}: unsupported coverage selection')
                assert condition is not None
                self.assertIn(condition[1], versions, f'{filename}: coverage leg is absent from the matrix')


if __name__ == '__main__':
    unittest.main()
