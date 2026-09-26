from __future__ import annotations

import json
from decimal import localcontext
from pathlib import Path

import pytest
from reviewready.cli import main
from reviewready.documents import (
    INTAKE_NAME,
    REVIEW_NAME,
    create_intake,
    load_document,
    render_document,
)
from reviewready.engine import review_pack
from reviewready.errors import GateInputError
from reviewready.report import write_review_pack
from reviewready.viewer import render_review_sheet
from tests.support import EXAMPLES, copy_example_pack

FIXTURE = EXAMPLES / "document-intake"
TEXT = (FIXTURE / "invoice.txt").read_text(encoding="utf-8")
CASES = json.loads((FIXTURE / "benchmark.json").read_text(encoding="utf-8"))["cases"]
ENTITY = "Cedar and Pine Consulting Pty Ltd"


def bundle_at(tmp_path: Path, *, text: str = TEXT, entity: str = ENTITY) -> Path:
    tmp_path.mkdir(parents=True, exist_ok=True)
    source = tmp_path / "source.txt"
    source.write_text(text, encoding="utf-8")
    return create_intake(source_path=source, text_path=source, output_dir=tmp_path / "bundle",
                         entity=entity, period_end="2026-03-31", text_origin="manual")


def reviewed(bundle: Path) -> dict:
    review = json.loads((bundle / "document-review.example.json").read_text())
    review.update(reviewer_initials="XY", reviewed_on="2026-04-10", source_text_checked=True)
    (bundle / REVIEW_NAME).write_text(json.dumps(review), encoding="utf-8")
    return review


def save_review(bundle: Path, review: dict) -> None:
    (bundle / REVIEW_NAME).write_text(json.dumps(review), encoding="utf-8")


@pytest.mark.parametrize("case", CASES, ids=lambda case: case["id"])
def test_fabricated_text_benchmark(tmp_path: Path, case: dict) -> None:
    text = TEXT
    for before, after in case["replace"]:
        text = text.replace(before, after)
    bundle = bundle_at(tmp_path, text=text + case["append"])
    draft = json.loads((bundle / "document-review.example.json").read_text())
    for field, expected in case["expected"].items():
        assert draft["fields"][field]["value"] == expected
    assert sorted(name for name, item in draft["fields"].items() if item["value"] is None) == sorted(
        case["unresolved"]
    )
    if "total_page" in case:
        assert draft["fields"]["total"]["page"] == case["total_page"]
    assert load_document(bundle).issues == ("Human transcription review is missing.",)
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready", document_paths=(bundle,))
    assert pack.status == "NOT_READY"
    reviewed(bundle)
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready", document_paths=(bundle,))
    assert pack.status == ("NOT_READY" if case["unresolved"] else "READY")


def test_review_preserves_observation_and_requires_a_correction_reason(tmp_path: Path) -> None:
    bundle = bundle_at(tmp_path, text=TEXT.replace("110.00", "unreadable"))
    review = reviewed(bundle)
    review["fields"]["total"].update(value="110.00", page=1)
    save_review(bundle, review)
    assert any("explain the correction" in issue for issue in load_document(bundle).issues)
    review["fields"]["total"]["note"] = "Read 110.00 on the original page."
    save_review(bundle, review)
    result = load_document(bundle)
    assert result.issues == ()
    assert result.manifest["fields"]["total"][0]["raw"] == "unreadable"
    assert '"110.00"' in render_document(result)
    assert '"unreadable"' in render_document(result)


@pytest.mark.parametrize("filename", ["original.txt", "extracted.txt", INTAKE_NAME])
def test_source_and_extraction_tampering_is_refused(tmp_path: Path, filename: str) -> None:
    bundle = bundle_at(tmp_path)
    reviewed(bundle)
    target = bundle / filename
    target.write_bytes(target.read_bytes().replace(b"110.00", b"120.00"))
    with pytest.raises(GateInputError, match="identical bytes|does not match"):
        load_document(bundle)


@pytest.mark.parametrize("change,match", [
    (lambda doc: doc.update(intake_sha256="0" * 64), "different intake bytes"),
    (lambda doc: doc.update(source_text_checked="true"), "boolean"),
    (lambda doc: doc.update(reviewed_on="01/02/2026"), "ISO date"),
    (lambda doc: doc["fields"]["total"].update(value="1e2"), "decimal string"),
    (lambda doc: doc["fields"]["total"].update(value=110.0), "string"),
    (lambda doc: doc["fields"]["total"].update(value=True), "string"),
    (lambda doc: doc["fields"]["total"].update(page=True), "identify a page"),
    (lambda doc: doc["fields"]["total"].update(page=2), "identify a page"),
    (lambda doc: doc["fields"].pop("gst_observed"), "every field"),
    (lambda doc: doc["fields"]["total"].update(approved=True), "review shape"),
    (lambda doc: doc.update(approved=True), "review schema"),
    (lambda doc: doc.update(reviewer_initials="XY\x1b[2J"), "control"),
])
def test_invalid_review_records_fail_closed(tmp_path: Path, change, match: str) -> None:
    bundle = bundle_at(tmp_path)
    review = reviewed(bundle)
    change(review)
    save_review(bundle, review)
    with pytest.raises(GateInputError, match=match):
        load_document(bundle)


def test_ambiguous_observations_need_explicit_resolution(tmp_path: Path) -> None:
    bundle = bundle_at(tmp_path, text=TEXT + "\nTotal: 100.00\n")
    review = reviewed(bundle)
    review["fields"]["total"].update(value="110.00", page=1,
                                      note="110.00 is the final total; 100.00 is the subtotal.")
    save_review(bundle, review)
    assert not load_document(bundle).issues
    assert render_document(load_document(bundle)).count("observed page 1") == 7


@pytest.mark.parametrize("gst", ["-10.00", "120.00"])
def test_observed_tax_checks_do_not_infer_tax_entitlement(tmp_path: Path, gst: str) -> None:
    bundle = bundle_at(tmp_path, text=TEXT.replace("GST: 10.00", "GST: " + gst))
    reviewed(bundle)
    assert "Observed GST" in load_document(bundle).issues[0]


def test_missing_tax_never_becomes_zero_implicitly(tmp_path: Path) -> None:
    bundle = bundle_at(tmp_path, text=TEXT.replace("GST: 10.00", "No tax details"))
    review = reviewed(bundle)
    assert review["fields"]["gst_observed"]["value"] is None
    assert "gst_observed: unresolved." in load_document(bundle).issues
    review["fields"]["gst_observed"].update(value="0.00", page=1)
    save_review(bundle, review)
    assert any("explain the correction" in item for item in load_document(bundle).issues)


def test_wrong_entity_or_period_blocks_and_documents_never_replace_tb(tmp_path: Path) -> None:
    bundle = bundle_at(tmp_path, entity="Another fabricated entity")
    reviewed(bundle)
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready", document_paths=(bundle,))
    assert pack.status == "BLOCKED"
    assert pack.findings[0].code == "DOCUMENT_CONTEXT_MISMATCH"
    workpapers = copy_example_pack("bas-ready", tmp_path / "pack")
    (workpapers / "trial_balance.csv").unlink()
    pack = review_pack(profile="bas", pack_dir=workpapers, document_paths=(bundle,))
    assert any(item.code == "MISSING_ARTEFACT" and item.slot == "trial_balance"
               for item in pack.findings)


def test_wrong_review_period_blocks_even_with_a_reviewed_document(tmp_path: Path) -> None:
    source = FIXTURE / "invoice.txt"
    bundle = create_intake(source_path=source, text_path=source, output_dir=tmp_path / "bundle",
                           entity=ENTITY, period_end="2026-02-28", text_origin="manual")
    reviewed(bundle)
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready", document_paths=(bundle,))
    assert pack.status == "BLOCKED"
    assert pack.findings[0].code == "DOCUMENT_CONTEXT_MISMATCH"


@pytest.mark.parametrize("filename", ["trial_balance.csv", "self_review.json"])
@pytest.mark.parametrize("empty", [False, True])
@pytest.mark.parametrize("has_review", [False, True])
def test_unavailable_pack_context_remains_not_ready(
    tmp_path: Path, filename: str, empty: bool, has_review: bool,
) -> None:
    workpapers = copy_example_pack("bas-ready", tmp_path / "pack")
    if empty:
        (workpapers / filename).write_bytes(b"")
    else:
        (workpapers / filename).unlink()
    bundle = bundle_at(tmp_path / "document")
    if has_review:
        reviewed(bundle)
    assert review_pack(profile="bas", pack_dir=workpapers).status == "NOT_READY"
    pack = review_pack(profile="bas", pack_dir=workpapers, document_paths=(bundle,))
    assert pack.status == "NOT_READY"
    assert not any(item.code == "DOCUMENT_CONTEXT_MISMATCH" for item in pack.findings)
    assert any(item.code == "DOCUMENT_REVIEW_REQUIRED" for item in pack.findings) != has_review
    assert any(item.slot.startswith("document_") for item in pack.source_evidence)


@pytest.mark.parametrize("filename,entity,period_end", [
    ("trial_balance.csv", ENTITY, "2026-02-28"),
    ("self_review.json", "Another fabricated entity", "2026-03-31"),
])
def test_known_document_context_mismatch_still_blocks_with_another_input_missing(
    tmp_path: Path, filename: str, entity: str, period_end: str,
) -> None:
    workpapers = copy_example_pack("bas-ready", tmp_path / "pack")
    (workpapers / filename).unlink()
    source = FIXTURE / "invoice.txt"
    bundle = create_intake(source_path=source, text_path=source, output_dir=tmp_path / "bundle",
                           entity=entity, period_end=period_end, text_origin="manual")
    reviewed(bundle)
    pack = review_pack(profile="bas", pack_dir=workpapers, document_paths=(bundle,))
    assert pack.status == "BLOCKED"
    assert any(item.code == "DOCUMENT_CONTEXT_MISMATCH" for item in pack.findings)


def test_duplicate_bytes_are_flagged_without_collapsing_equal_amounts(tmp_path: Path) -> None:
    first = bundle_at(tmp_path / "first")
    second = bundle_at(tmp_path / "second", text=TEXT.replace("SYN-0001", "SYN-0002"))
    for bundle in (first, second):
        reviewed(bundle)
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready",
                       document_paths=(first, second))
    assert pack.status == "READY"
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready",
                       document_paths=(first, first))
    assert pack.status == "NOT_READY"
    assert any(item.code == "DOCUMENT_DUPLICATE" for item in pack.findings)
    assert len(pack.source_evidence) == 13


def test_saved_pack_rejects_a_changed_review_and_missing_or_reordered_bundles(tmp_path: Path) -> None:
    first = bundle_at(tmp_path / "first")
    second = bundle_at(tmp_path / "second", text=TEXT.replace("SYN-0001", "SYN-0002"))
    for bundle in (first, second):
        reviewed(bundle)
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready",
                       document_paths=(first, second))
    output = tmp_path / "output"
    write_review_pack(pack, output)
    sheet, _ = render_review_sheet(output, document_paths=(first, second))
    assert "observed page 1, line" in sheet
    assert "tax treatment" in sheet
    for paths in ((first,), (second, first)):
        with pytest.raises(GateInputError, match="bytes or order"):
            render_review_sheet(output, document_paths=paths)
    review = reviewed(first)
    review["fields"]["supplier"]["note"] = "Rechecked against original."
    save_review(first, review)
    with pytest.raises(GateInputError, match="bytes or order"):
        render_review_sheet(output, document_paths=(first, second))


def test_document_sources_are_bound_without_changing_readiness_schema(tmp_path: Path) -> None:
    bundle = bundle_at(tmp_path)
    reviewed(bundle)
    plain = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready")
    joined = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready", document_paths=(bundle,))
    from reviewready.report import _as_json

    assert set(_as_json(plain)) == set(_as_json(joined))
    assert plain.findings == joined.findings
    assert plain.controls_not_run == joined.controls_not_run
    assert len(joined.source_evidence) == len(plain.source_evidence) + 4


def test_decimal_context_does_not_round_document_amounts(tmp_path: Path) -> None:
    with localcontext() as context:
        context.prec = 2
        bundle = bundle_at(tmp_path, text=TEXT.replace("110.00", "123456789012345678.90"))
        review = reviewed(bundle)
        assert review["fields"]["total"]["value"] == "123456789012345678.90"
        assert not load_document(bundle).issues


@pytest.mark.parametrize("filename", [INTAKE_NAME, REVIEW_NAME])
def test_duplicate_json_members_are_rejected(tmp_path: Path, filename: str) -> None:
    bundle = bundle_at(tmp_path)
    reviewed(bundle)
    (bundle / filename).write_text('{"source": {}, "source": {}}')
    with pytest.raises(GateInputError, match="Duplicate document JSON"):
        load_document(bundle)


@pytest.mark.parametrize("filename", ["../outside.txt", "C:/outside.txt", [], None])
def test_arbitrary_source_paths_are_rejected(tmp_path: Path, filename) -> None:
    bundle = bundle_at(tmp_path)
    intake = json.loads((bundle / INTAKE_NAME).read_text())
    intake["source"]["filename"] = filename
    (bundle / INTAKE_NAME).write_text(json.dumps(intake))
    with pytest.raises(GateInputError, match="must name original"):
        load_document(bundle)


def test_document_symlink_must_stay_inside_the_bundle(tmp_path: Path) -> None:
    bundle = bundle_at(tmp_path)
    original = bundle / "original.txt"
    original.unlink()
    try:
        original.symlink_to(tmp_path / "source.txt")
    except OSError:
        pytest.skip("This host does not permit creating file symlinks.")
    with pytest.raises(GateInputError, match="inside its document bundle"):
        load_document(bundle)


def test_output_refusal_precedes_reads_and_existing_files_survive(tmp_path: Path) -> None:
    checkout = tmp_path / "checkout"
    checkout.mkdir()
    (checkout / ".git").mkdir()
    with pytest.raises(GateInputError, match="checkout"):
        create_intake(source_path=Path("missing"), text_path=Path("missing"),
                      output_dir=checkout / "bundle", entity=ENTITY,
                      period_end="2026-03-31", text_origin="manual")
    bundle = bundle_at(tmp_path)
    original = (bundle / "original.txt").read_bytes()
    with pytest.raises(GateInputError, match="new output directory"):
        create_intake(source_path=Path("missing"), text_path=Path("missing"),
                      output_dir=bundle, entity=ENTITY, period_end="2026-03-31", text_origin="manual")
    assert (bundle / "original.txt").read_bytes() == original


def test_invalid_utf8_and_oversized_or_empty_text_fail(tmp_path: Path) -> None:
    for index, content in enumerate((b"\xff", b"", b"x" * (1024 * 1024 + 1))):
        source = tmp_path / f"source{index}.txt"
        source.write_bytes(content)
        with pytest.raises(GateInputError):
            create_intake(source_path=source, text_path=source, output_dir=tmp_path / f"out{index}",
                          entity=ENTITY, period_end="2026-03-31", text_origin="manual")
        assert not (tmp_path / f"out{index}").exists()


def test_cli_intake_gate_and_bound_view(tmp_path: Path, capsys) -> None:
    bundle = tmp_path / "bundle"
    args = ["intake", "--source", str(FIXTURE / "invoice.txt"), "--text",
            str(FIXTURE / "invoice.txt"), "--text-origin", "manual", "--entity", ENTITY,
            "--period-end", "2026-03-31", "--output", str(bundle)]
    assert main(args) == 1
    assert main([*args, "--synthetic"]) == 0
    assert main(["view-document", "--bundle", str(bundle)]) == 0
    assert "Human transcription review is missing" in capsys.readouterr().out
    output = tmp_path / "output"
    gate = ["gate", "--profile", "bas", "--pack", str(EXAMPLES / "bas-ready"),
            "--document", str(bundle), "--output", str(output)]
    assert main(gate) == 2
    reviewed(bundle)
    assert main(gate) == 0
    view = ["view", "--pack-dir", str(output), "--document", str(bundle)]
    assert main(view) == 0
    assert "observed page 1" in capsys.readouterr().out
    (bundle / "original.txt").write_text("changed")
    assert main(view) == 1
    assert main(["view-document", "--bundle", str(bundle)]) == 1
    assert main([*args, "--synthetic"]) == 1
