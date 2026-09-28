"""Exercise comparison through real fabricated bundles and written packs."""

import hashlib
import json
from pathlib import Path

import pytest
from reviewready.comparison import compare_packs
from reviewready.documents import REVIEW_NAME, create_intake
from reviewready.engine import review_pack
from reviewready.report import write_review_pack
from tests.support import EXAMPLES


def document(root: Path, *, changed: bool = False, period: str = "2026-03-31") -> Path:
    root.mkdir()
    text = (EXAMPLES / "document-intake/invoice.txt").read_text(encoding="utf-8")
    source = root / "source.txt"
    source.write_text(text.replace("110.00", "220.00") if changed else text, encoding="utf-8")
    return create_intake(source_path=source, text_path=source, output_dir=root / "bundle",
                         entity="Cedar and Pine Consulting Pty Ltd", period_end=period,
                         text_origin="manual")


def reviewed(bundle: Path) -> None:
    record = json.loads((bundle / "document-review.example.json").read_text())
    record.update(reviewer_initials="XY", reviewed_on="2026-04-10", source_text_checked=True)
    (bundle / REVIEW_NAME).write_text(json.dumps(record), encoding="utf-8")


def saved(output: Path, documents: tuple[Path, ...]) -> Path:
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready", document_paths=documents)
    write_review_pack(pack, output)
    return output


def digest(bundle: Path) -> str:
    return hashlib.sha256((bundle / "original.txt").read_bytes()).hexdigest()


def findings(result: dict) -> dict[tuple[str, str], str]:
    return {(row["code"], row["slot"]): row["change"] for row in result["findings"]}


def test_removed_document_is_outside_comparison_coverage(tmp_path: Path) -> None:
    bundle = document(tmp_path / "document")
    before = saved(tmp_path / "before", (bundle,))
    after = saved(tmp_path / "after", ())
    result = compare_packs(before, after)
    assert result["previous"]["overall_status"] == "NOT_READY"
    assert result["current"]["overall_status"] == "READY"
    assert findings(result)[("DOCUMENT_REVIEW_REQUIRED", "document_001")] == "NOT_COMPARABLE"
    assert result["scope_changes"] == [{"member": "document_coverage",
                                        "previous": [digest(bundle)], "current": []}]
    assert {row["change"] for row in result["sources"]
            if row["slot"].startswith("document_")} == {"REMOVED"}


@pytest.mark.parametrize("complete", [False, True])
def test_replacement_reports_coverage_without_regrouping_findings(tmp_path: Path, complete: bool) -> None:
    first = document(tmp_path / "first")
    second = document(tmp_path / "second", changed=True)
    if complete:
        reviewed(second)
    result = compare_packs(saved(tmp_path / "before", (first,)),
                           saved(tmp_path / "after", (second,)))
    assert result["scope_changes"] == [{"member": "document_coverage",
                                        "previous": [digest(first)], "current": [digest(second)]}]
    assert findings(result)[("DOCUMENT_REVIEW_REQUIRED", "document_001")] == (
        "NOT_COMPARABLE" if complete else "RECURRING"
    )


def test_added_document_expands_coverage(tmp_path: Path) -> None:
    bundle = document(tmp_path / "document")
    result = compare_packs(saved(tmp_path / "before", ()), saved(tmp_path / "after", (bundle,)))
    assert result["scope_changes"] == [{"member": "document_coverage",
                                        "previous": [], "current": [digest(bundle)]}]
    assert findings(result)[("DOCUMENT_REVIEW_REQUIRED", "document_001")] == "NEW"


def test_reordered_documents_preserve_coverage_and_slot_grouping(tmp_path: Path) -> None:
    first = document(tmp_path / "first")
    second = document(tmp_path / "second", changed=True)
    reviewed(second)
    result = compare_packs(saved(tmp_path / "before", (first, second)),
                           saved(tmp_path / "after", (second, first)))
    assert result["scope_changes"] == []
    assert findings(result) == {
        ("DOCUMENT_REVIEW_REQUIRED", "document_001"): "NOT_RAISED",
        ("DOCUMENT_REVIEW_REQUIRED", "document_002"): "NEW",
    }


def test_reviewing_the_same_document_preserves_coverage(tmp_path: Path) -> None:
    bundle = document(tmp_path / "document")
    before = saved(tmp_path / "before", (bundle,))
    reviewed(bundle)
    result = compare_packs(before, saved(tmp_path / "after", (bundle,)))
    assert result["scope_changes"] == []
    assert findings(result)[("DOCUMENT_REVIEW_REQUIRED", "document_001")] == "NOT_RAISED"


def test_correcting_context_preserves_original_document_coverage(tmp_path: Path) -> None:
    wrong = document(tmp_path / "wrong", period="2026-02-28")
    correct = document(tmp_path / "correct")
    reviewed(wrong)
    reviewed(correct)
    result = compare_packs(saved(tmp_path / "before", (wrong,)),
                           saved(tmp_path / "after", (correct,)))
    assert result["scope_changes"] == []
    assert findings(result)[("DOCUMENT_CONTEXT_MISMATCH", "document_001")] == "NOT_RAISED"
    assert next(row for row in result["sources"] if row["slot"] == "document_001_1")["change"] == "CHANGED"


def test_duplicate_document_removal_preserves_multiplicity(tmp_path: Path) -> None:
    bundle = document(tmp_path / "document")
    result = compare_packs(saved(tmp_path / "before", (bundle, bundle)),
                           saved(tmp_path / "after", (bundle,)))
    assert result["scope_changes"] == [{"member": "document_coverage",
                                        "previous": [digest(bundle)] * 2,
                                        "current": [digest(bundle)]}]
    assert findings(result)[("DOCUMENT_REVIEW_REQUIRED", "document_001")] == "RECURRING"
    assert findings(result)[("DOCUMENT_REVIEW_REQUIRED", "document_002")] == "NOT_COMPARABLE"
    assert findings(result)[("DOCUMENT_DUPLICATE", "document_002")] == "NOT_COMPARABLE"
