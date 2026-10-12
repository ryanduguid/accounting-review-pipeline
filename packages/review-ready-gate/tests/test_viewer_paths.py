"""Viewer refusals for tampered packs the main viewer tests do not construct.

Every pack starts from the real writer, then one file is edited the way a
careless or hostile hand would, and the viewer must name the disagreement.
"""

from __future__ import annotations

from dataclasses import replace
from datetime import date
from pathlib import Path

import pytest
from reviewready import viewer
from reviewready.engine import review_pack
from reviewready.errors import GateInputError
from reviewready.models import ReviewerAcknowledgement
from reviewready.report import write_review_pack
from reviewready.viewer import render_review_sheet, verify_pack
from tests.support import EXAMPLES
from tests.test_viewer import _read_json, _rewrite_json, _synthetic_pack

SUMMARY = "readiness-summary.md"
CSV = "findings.csv"
HEADER = "| Status | Code | Slot | Repeat | Reason |"
DIVIDER = "| --- | --- | --- | --- | --- |"


@pytest.fixture()
def pack_dir(tmp_path: Path) -> Path:
    output = tmp_path / "pack"
    write_review_pack(_synthetic_pack(), output)
    return output


@pytest.fixture()
def acknowledged_pack(tmp_path: Path) -> Path:
    output = tmp_path / "acknowledged"
    note = ReviewerAcknowledgement("RD", date(2026, 4, 12), "Checked.")
    write_review_pack(replace(_synthetic_pack(), acknowledgement=note), output)
    return output


@pytest.fixture()
def three_findings(tmp_path: Path) -> Path:
    output = tmp_path / "not-ready"
    write_review_pack(review_pack(profile="bas", pack_dir=EXAMPLES / "bas-not-ready"), output)
    return output


def _read(pack: Path, name: str) -> str:
    return (pack / name).read_text(encoding="utf-8")


def _write(pack: Path, name: str, text: str) -> None:
    (pack / name).write_text(text, encoding="utf-8", newline="")


def _edit_summary(pack: Path, edit) -> None:
    lines = _read(pack, SUMMARY).split("\n")
    _write(pack, SUMMARY, "\n".join(edit(lines)))


def test_artefact_replaced_by_a_directory_during_reading_is_named(
    pack_dir: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    read_bytes = Path.read_bytes

    def swapped(self: Path) -> bytes:
        if self.name == "readiness-pack.json":
            raise IsADirectoryError(str(self))
        return read_bytes(self)

    with monkeypatch.context() as patch:
        patch.setattr(type(pack_dir), "read_bytes", swapped)
        with pytest.raises(GateInputError, match="readiness-pack.json: expected a file, found a directory"):
            render_review_sheet(pack_dir)


def test_acknowledgement_member_that_is_not_a_string_is_refused(acknowledged_pack: Path) -> None:
    document = _read_json(acknowledged_pack)
    document["acknowledgement"]["comment"] = 5
    _rewrite_json(acknowledged_pack, document)
    with pytest.raises(GateInputError, match="acknowledgement.comment must be a string"):
        verify_pack(acknowledged_pack)


@pytest.mark.parametrize(
    "reviewed_on,match",
    [
        ("20260412", "must be an extended-format ISO date"),
        ("2026-02-30", "acknowledgement.reviewed_on is not a date"),
    ],
)
def test_acknowledgement_date_must_be_a_real_extended_date(
    acknowledged_pack: Path, reviewed_on: str, match: str
) -> None:
    document = _read_json(acknowledged_pack)
    document["acknowledgement"]["reviewed_on"] = reviewed_on
    _rewrite_json(acknowledged_pack, document)
    with pytest.raises(GateInputError, match=match):
        verify_pack(acknowledged_pack)


def test_finding_with_a_pack_status_is_refused(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["findings"][0]["status"] = "READY"
    _rewrite_json(pack_dir, document)
    with pytest.raises(GateInputError, match=r"findings\[0\].status is not a finding status"):
        verify_pack(pack_dir)


def test_summary_without_evidence_lines_is_refused(pack_dir: Path) -> None:
    _edit_summary(pack_dir, lambda lines: [line for line in lines if not line.startswith("- `")])
    with pytest.raises(GateInputError, match="no source-evidence digest lines found"):
        verify_pack(pack_dir)


def test_summary_digests_without_json_digests_are_refused(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["source_sha256"] = {}
    _rewrite_json(pack_dir, document)
    with pytest.raises(GateInputError, match="lists digests but readiness-pack.json lists none"):
        verify_pack(pack_dir)


@pytest.mark.parametrize(
    "edit,match",
    [
        (lambda lines: [line for line in lines if line != HEADER],
         "expected exactly one findings-table header, found 0"),
        (lambda lines: [HEADER if line == DIVIDER else line for line in lines],
         "expected exactly one findings-table header, found 2"),
        (lambda lines: [line for line in lines if line != DIVIDER],
         "the findings table has no delimiter row"),
        (lambda lines: lines[: lines.index(HEADER) + 5],
         "document does not match its canonical rendering"),
        (lambda lines: [line + " extra |" if line.startswith("| NOT_READY") else line
                        for line in lines],
         "finding 1 has 6 cells, expected 5"),
        (lambda lines: [line for line in lines if "OPEN_ITEM_BLOCKING" not in line],
         "finding counts disagree: readiness-pack.json holds 3, readiness-summary.md tabulates 2"),
        (lambda lines: [*lines[:3], "- Engagement type: bas", *lines[3:]],
         "expected exactly one 'Engagement type' scope line, found 2"),
    ],
)
def test_findings_table_and_scope_tampering_is_refused(
    three_findings: Path, edit, match: str
) -> None:
    _edit_summary(three_findings, edit)
    with pytest.raises(GateInputError, match=match):
        verify_pack(three_findings)


def test_findings_table_with_no_json_findings_is_refused(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["findings"] = []
    _rewrite_json(pack_dir, document)
    _write(pack_dir, CSV, _read(pack_dir, CSV).splitlines()[0] + "\r\n")
    with pytest.raises(GateInputError, match="tabulates findings but readiness-pack.json lists none"):
        verify_pack(pack_dir)


def test_repeat_flag_outside_true_or_false_is_refused(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["findings"][0]["repeat"] = "maybe"
    _rewrite_json(pack_dir, document)
    _write(pack_dir, CSV, _read(pack_dir, CSV).replace(",false,", ",maybe,", 1))
    with pytest.raises(GateInputError, match=r"findings\[0\].repeat must be 'true' or 'false'"):
        verify_pack(pack_dir)


@pytest.mark.parametrize(
    "content,match",
    [
        (b"\xff\xfe", "findings.csv: not valid UTF-8"),
        (b"code,status\r\n", "findings.csv: header row does not match the written contract"),
    ],
)
def test_unreadable_findings_csv_is_refused(pack_dir: Path, content: bytes, match: str) -> None:
    (pack_dir / CSV).write_bytes(content)
    with pytest.raises(GateInputError, match=match):
        verify_pack(pack_dir)


def test_summary_that_is_not_utf8_is_refused(pack_dir: Path) -> None:
    (pack_dir / SUMMARY).write_bytes(b"\xff")
    with pytest.raises(GateInputError, match="readiness-summary.md: not valid UTF-8"):
        verify_pack(pack_dir)


def test_row_splitter_keeps_cells_of_a_row_without_outer_pipes() -> None:
    assert viewer._md_row_cells("a | b \\| c") == ["a", "b \\| c"]  # nosec B101: pytest outcome assertion


def test_cross_checks_refuse_shapes_the_file_parsers_never_return(pack_dir: Path) -> None:
    """Guards behind the CSV reader and the schema check, met by a direct caller."""
    document, summary, rows, _ = verify_pack(pack_dir)
    short = [{key: value for key, value in rows[0].items() if key != "code"}]
    with pytest.raises(GateInputError, match="findings.csv: row 1 has no code column value"):
        viewer._verify_cross_file_agreement(document, summary, short)
    document["findings"][0]["code"] = 5
    with pytest.raises(GateInputError, match=r"findings\[0\].code must be a string"):
        viewer._verify_findings_table(document, summary)
