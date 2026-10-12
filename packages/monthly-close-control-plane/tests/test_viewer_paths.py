"""Viewer refusals for unreadable artefacts and edited packs, and the sheet's optional sections.

Every pack here is built by the real writer and then edited the way a person
or a damaged copy would edit it, as in test_viewer.py.
"""

from __future__ import annotations

import re
from dataclasses import replace
from datetime import date
from pathlib import Path

import pytest
from closecontrol.engine import review_close
from closecontrol.errors import ControlInputError
from closecontrol.models import ReviewerAcknowledgement
from closecontrol.report import write_review_pack
from closecontrol.viewer import render_review_sheet, verify_pack

from test_calculation_evidence import build_record
from test_calculation_evidence import run as evidence_run
from test_calculation_evidence import write as write_evidence
from test_equity import _inputs as equity_inputs
from test_viewer import _legacy_pack, _pack, _read_json, _rewrite_json

SUMMARY = "close-summary.md"
EVIDENCE_HEADER = "| Calculation | Status | Period | Figures | Relied on | Entry digest |"


def _write(tmp_path: Path, pack, name: str = "pack") -> Path:
    output = tmp_path / name
    write_review_pack(pack, output)
    return output


@pytest.fixture()
def pack_dir(tmp_path: Path) -> Path:
    return _write(tmp_path, _pack())


def _edit_summary(pack_dir: Path, old: str, new: str) -> None:
    path = pack_dir / SUMMARY
    text = path.read_text(encoding="utf-8")
    assert old in text, old  # nosec B101: guards the fixture edit itself
    path.write_text(text.replace(old, new, 1), encoding="utf-8")


# --- unreadable artefacts ---------------------------------------------------


def _refuse_read(monkeypatch: pytest.MonkeyPatch, name: str, error: OSError) -> None:
    real_read_bytes = Path.read_bytes

    def read_bytes(self: Path) -> bytes:
        if self.name == name:
            raise error
        return real_read_bytes(self)

    monkeypatch.setattr(Path, "read_bytes", read_bytes)


@pytest.mark.parametrize(("error", "message"), [
    (IsADirectoryError(21, "Is a directory"), "close-summary.md: expected a file, found a directory"),
    (PermissionError(13, "Permission denied"), "close-summary.md: could not be read from"),
])
def test_unreadable_required_artefact_is_named(pack_dir: Path, monkeypatch: pytest.MonkeyPatch,
                                               error: OSError, message: str) -> None:
    _refuse_read(monkeypatch, SUMMARY, error)
    with pytest.raises(ControlInputError, match=message):
        verify_pack(pack_dir)


def test_unreadable_query_register_is_not_read_as_an_older_pack(
        pack_dir: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _refuse_read(monkeypatch, "client-queries.csv", PermissionError(13, "Permission denied"))
    with pytest.raises(ControlInputError, match="client-queries.csv: could not be read from"):
        verify_pack(pack_dir)


@pytest.mark.parametrize(("name", "message"), [
    ("exceptions.csv", "exceptions.csv: not valid UTF-8"),
    (SUMMARY, "close-summary.md: not valid UTF-8"),
])
def test_artefact_that_is_not_utf8_is_refused(pack_dir: Path, name: str, message: str) -> None:
    path = pack_dir / name
    path.write_bytes(path.read_bytes() + b"\xff")
    with pytest.raises(ControlInputError, match=message):
        verify_pack(pack_dir)


# --- edited JSON pack -------------------------------------------------------


@pytest.mark.parametrize(("value", "message"), [
    (1000, "thresholds.absolute_variance must be a string"),
    ("-1000.00", "thresholds.absolute_variance must be finite and non-negative"),
])
def test_threshold_must_be_a_non_negative_decimal_string(pack_dir: Path, value: object, message: str) -> None:
    document = _read_json(pack_dir)
    document["thresholds"]["absolute_variance"] = value
    _rewrite_json(pack_dir, document)
    with pytest.raises(ControlInputError, match=message):
        verify_pack(pack_dir)


def test_exception_status_must_be_a_pack_status(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["exceptions"][0]["status"] = "OPEN"
    _rewrite_json(pack_dir, document)
    with pytest.raises(ControlInputError, match=r"exceptions\[0\]\.status is not a pack status"):
        verify_pack(pack_dir)


def test_summary_without_source_digests_is_refused(pack_dir: Path) -> None:
    path = pack_dir / SUMMARY
    lines = path.read_text(encoding="utf-8").splitlines()
    kept = [line for line in lines if not re.match(r"- `[a-z_]+`: `[0-9a-f]{64}`$", line)]
    path.write_text("\n".join(kept) + "\n", encoding="utf-8")
    with pytest.raises(ControlInputError, match="no source-evidence digest lines found"):
        verify_pack(pack_dir)


# --- calculation evidence ---------------------------------------------------


@pytest.fixture()
def evidence_pack(tmp_path: Path) -> Path:
    pack = evidence_run(tmp_path, calculation_evidence_paths=[write_evidence(tmp_path, build_record())],
                        required_calculations=("coal-lsl-levy",))
    return _write(tmp_path, pack)


def _evidence_row(pack_dir: Path) -> str:
    text = (pack_dir / SUMMARY).read_text(encoding="utf-8")
    (row,) = [line for line in text.splitlines() if line.startswith("| coal-lsl-levy |")]
    return row


def _summary_edit(pack_dir: Path, edit: str) -> None:
    row = _evidence_row(pack_dir)
    digest = row.rstrip(" |").rsplit(" ", 1)[-1]
    old, new = {
        "unterminated": (row + "\n\n## Exceptions", row + "\n## Exceptions"),
        "required_not_followed_by_blank": ("Required: `coal-lsl-levy`\n\n", "Required: `coal-lsl-levy`\n"),
        "required_malformed": ("Required: `coal-lsl-levy`", "Required: coal-lsl-levy"),
        "delimiter": ("| --- | --- | --- | --- | --- | --- |\n| coal", "| --- | --- | --- | --- | --- |\n| coal"),
        "stray_line": (row + "\n", row + "\nEdited by hand.\n"),
        "five_cells": (row, row.replace(" | yes |", " |", 1)),
        "duplicate_row": (row + "\n", row + "\n" + row + "\n"),
        "digest_case": (digest, digest.upper()),
        "no_rows": (row + "\n", ""),
    }[edit]
    _edit_summary(pack_dir, old, new)


@pytest.mark.parametrize(("edit", "message"), [
    ("unterminated", "does not end where the writer ends it"),
    ("required_not_followed_by_blank", "required line is missing or not where the writer puts it"),
    ("required_malformed", "the calculation-evidence required line is malformed"),
    ("delimiter", "the calculation-evidence table delimiter is missing or altered"),
    ("stray_line", "holds a line the writer never emits: 'Edited by hand.'"),
    ("five_cells", "a calculation-evidence row holds 5 cells, not 6"),
    ("duplicate_row", "calculation 'coal-lsl-levy' appears twice in the table"),
    ("digest_case", "carries an entry digest that is not a lowercase SHA-256"),
    ("no_rows", "the calculation-evidence table has a header and no rows"),
])
def test_edited_calculation_evidence_section_is_refused(evidence_pack: Path, edit: str, message: str) -> None:
    _summary_edit(evidence_pack, edit)
    with pytest.raises(ControlInputError, match=re.escape(message)):
        verify_pack(evidence_pack)


def _block_edit(document: dict, edit: str) -> None:
    block = document["calculation_evidence"]
    entry = block["supplied"][0]
    if edit == "not_object":
        document["calculation_evidence"] = []
    elif edit == "missing_effect":
        del block["effect"]
    elif edit == "supplied_not_list":
        block["supplied"] = {"coal-lsl-levy": entry}
    elif edit == "entry_not_object":
        block["supplied"] = ["coal-lsl-levy"]
    elif edit == "usable_not_boolean":
        entry["usable"] = "yes"
    elif edit == "values_not_strings":
        entry["values"] = {"levy": 192.38}
    else:
        block["supplied"] = []


@pytest.mark.parametrize(("edit", "message"), [
    ("not_object", "calculation_evidence must be an object"),
    ("missing_effect", "calculation_evidence is missing member(s): effect"),
    ("supplied_not_list", "calculation_evidence.supplied must be a list"),
    ("entry_not_object", "calculation_evidence.supplied holds a non-object entry"),
    ("usable_not_boolean", "a calculation_evidence entry has a non-string label or a non-boolean usable flag"),
    ("values_not_strings", "calculation_evidence['coal-lsl-levy'].values must map names to decimal strings"),
    ("supplied_emptied", "holds a calculation-evidence table while close-review-pack.json records no supplied"),
])
def test_edited_calculation_evidence_block_is_refused(evidence_pack: Path, edit: str, message: str) -> None:
    document = _read_json(evidence_pack)
    _block_edit(document, edit)
    _rewrite_json(evidence_pack, document)
    with pytest.raises(ControlInputError, match=re.escape(message)):
        verify_pack(evidence_pack)


def test_evidence_table_in_a_pack_without_evidence_is_refused(pack_dir: Path) -> None:
    _edit_summary(pack_dir, "\n## Exceptions", "\n" + EVIDENCE_HEADER + "\n## Exceptions")
    with pytest.raises(ControlInputError, match="a calculation-evidence table is present while"):
        verify_pack(pack_dir)


def test_sheet_names_a_required_calculation_with_no_file(tmp_path: Path) -> None:
    pack = evidence_run(tmp_path, required_calculations=("coal-lsl-levy",))
    sheet, _ = render_review_sheet(_write(tmp_path, pack))
    assert "- Required: coal-lsl-levy" in sheet  # nosec B101: pytest outcome assertion
    assert "- Supplied: none. The exceptions say which calculation is missing." in sheet  # nosec B101: pytest outcome assertion


# --- client-query register removed from an older pack ----------------------


@pytest.mark.parametrize(("kept", "message"), [
    ("| Query | Control | Tenant | Account | Difference | Question | Evidence requested |",
     "holds a client-query table while the pack carries no client-query register"),
    ("- Client queries drafted: 1.", "states a client-query count while the pack carries no client-query register"),
])
def test_older_pack_keeping_part_of_the_register_is_refused(pack_dir: Path, kept: str, message: str) -> None:
    _legacy_pack(pack_dir)
    _edit_summary(pack_dir, "\n## Exceptions", "\n" + kept + "\n## Exceptions")
    with pytest.raises(ControlInputError, match=message):
        verify_pack(pack_dir)


def test_older_pack_without_control_coverage_opens_without_that_line(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    del document["controls_not_run"]
    _rewrite_json(pack_dir, document)
    path = pack_dir / SUMMARY
    lines = path.read_text(encoding="utf-8").splitlines()
    path.write_text("\n".join(line for line in lines if not line.startswith("- Controls not run: ")) + "\n",
                    encoding="utf-8")
    sheet, _ = render_review_sheet(pack_dir)
    assert "Overall status: REVIEW" in sheet  # nosec B101: pytest outcome assertion
    assert "Controls not run" not in sheet  # nosec B101: pytest outcome assertion


# --- equity evidence --------------------------------------------------------


@pytest.fixture()
def equity_pack(tmp_path: Path) -> Path:
    inputs, _ = equity_inputs(tmp_path)
    return _write(tmp_path, review_close(**inputs))


def test_sheet_shows_the_equity_reconciliation(equity_pack: Path) -> None:
    sheet, _ = render_review_sheet(equity_pack)
    assert "\nEquity reconciliation\n" in sheet  # nosec B101: pytest outcome assertion
    assert "Synthetic closing transfer" in sheet  # nosec B101: pytest outcome assertion


def test_calculated_equity_dates_must_match_the_pack(equity_pack: Path) -> None:
    document = _read_json(equity_pack)
    document["equity_reconciliation"]["opening_date"] = "2026-08-30"
    _rewrite_json(equity_pack, document)
    with pytest.raises(ControlInputError, match="Equity dates disagree with the pack"):
        verify_pack(equity_pack)


def test_equity_source_without_its_evidence_is_refused(pack_dir: Path) -> None:
    digest = "c" * 64
    document = _read_json(pack_dir)
    document["source_sha256"]["equity_schedule"] = digest
    _rewrite_json(pack_dir, document)
    _edit_summary(pack_dir, "\n\n## Exceptions", f"\n- `equity_schedule`: `{digest}`\n\n## Exceptions")
    with pytest.raises(ControlInputError, match="Equity schedule source has no reconciliation evidence"):
        verify_pack(pack_dir)


# --- acknowledgement and empty packs ---------------------------------------


def _acknowledged(comment: str):
    return replace(_pack(), acknowledgement=ReviewerAcknowledgement(
        reviewer_initials="RD", reviewed_on=date(2026, 8, 3), comment=comment))


def test_multi_line_comment_verifies_as_a_quoted_block(tmp_path: Path) -> None:
    output = _write(tmp_path, _acknowledged("Discussed with the controller.\n# Not a heading"))
    lines = verify_pack(output)[1].splitlines()
    start = lines.index("- Comment:")
    assert lines[start + 1:start + 3] == [  # nosec B101: pytest outcome assertion
        "  > Discussed with the controller.", "  > \\# Not a heading"]


def test_sheet_omits_an_empty_comment(tmp_path: Path) -> None:
    sheet, _ = render_review_sheet(_write(tmp_path, _acknowledged("")))
    assert "- Reviewer initials: RD" in sheet  # nosec B101: pytest outcome assertion
    assert "- Comment:" not in sheet  # nosec B101: pytest outcome assertion


def test_sheet_for_a_pack_with_no_exceptions(tmp_path: Path) -> None:
    sheet, _ = render_review_sheet(_write(tmp_path, replace(_pack("PASS"), exceptions=())))
    assert "No exceptions were raised. A human must still decide whether the close is appropriate." in sheet  # nosec B101: pytest outcome assertion
