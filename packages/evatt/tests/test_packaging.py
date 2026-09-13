import json
import re
from pathlib import Path

import evatt
from evatt.errors import EvattError, Halt

ROOT = Path(__file__).resolve().parents[1]


def test_version_is_a_three_part_string() -> None:
    parts = evatt.__version__.split(".")
    assert len(parts) == 3
    assert all(part.isdigit() for part in parts)


def test_halt_is_an_evatt_error_carrying_unknowns() -> None:
    error = Halt(("Jane Roe",))
    assert isinstance(error, EvattError)
    assert error.unknowns == ("Jane Roe",)


def test_gitignore_covers_every_file_that_holds_real_values() -> None:
    """All 4 rules, not just the 2 the map needs.

    ``*.triage.md`` and ``*.tmp`` were unasserted, and each covers a file that
    holds real values: the triage worklist quotes whole residual lines, and
    ``entities.save`` writes the map through a neighbouring .tmp that a hard
    kill can leave behind. A halt now refuses to write a triage file at a path
    git would let you commit, so dropping that rule from this repository stops
    the suite as well as the operator.
    """
    lines = (ROOT / ".gitignore").read_text(encoding="utf-8").splitlines()
    ignored = [line.strip() for line in lines]
    for rule in ("entities.json", "*.entities.json", "*.triage.md", "*.tmp"):
        assert rule in ignored, rule


def test_the_sdist_manifest_carries_every_document_the_readme_points_at() -> None:
    """README.md is the PyPI long description, and it names 4 documents.

    None of them was in either artefact, so an installed reader was sent to a
    DISCLAIMER that was not there, and DISCLAIMER.md is the file carrying the
    Privacy Act position and the "does not authorise disclosure" sentence.
    """
    manifest = (ROOT / "MANIFEST.in").read_text(encoding="utf-8")
    for document in ("DISCLAIMER.md", "DATA-FLOW.md", "SECURITY.md", "CONTRIBUTING.md"):
        assert f"include {document}" in manifest, document
        assert (ROOT / document).exists(), document


SAMPLES = ROOT / "evatt" / "samples"

RESERVED_EMAIL_DOMAIN = re.compile(r"@(?:example\.(?:com|org|net)|.*\.example)\b")
EMAIL_IN_TEXT = re.compile(r"\b[\w.+-]+@[\w.-]+\b")


def test_every_sample_email_uses_a_reserved_domain() -> None:
    for path in SAMPLES.glob("*"):
        for match in EMAIL_IN_TEXT.finditer(path.read_text(encoding="utf-8")):
            assert RESERVED_EMAIL_DOMAIN.search(match.group(0)), f"{path.name}: {match.group(0)}"


def test_sample_map_uses_only_marked_synthetic_names() -> None:
    document = json.loads((SAMPLES / "entities.sample.json").read_text(encoding="utf-8"))
    reserved_people = {"Jane Roe", "Mary O\u2019Brien", "MACDONALD, Alan"}
    for entry in document["entries"]:
        assert entry["value"].startswith("Sample ") or entry["value"] in reserved_people


def test_the_real_map_is_not_committed() -> None:
    assert not (ROOT / "entities.json").exists()
