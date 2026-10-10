"""Queue acceptance: verified evidence, honest coverage and unchanged source files."""
from __future__ import annotations

import hashlib
import json
import weakref
from dataclasses import replace
from datetime import date
from pathlib import Path

import pytest
from closecontrol import queue
from closecontrol.cli import main
from closecontrol.errors import ControlInputError
from closecontrol.models import ReviewerAcknowledgement
from closecontrol.queue import render_review_queue
from closecontrol.report import write_review_pack

from test_viewer import _legacy_pack, _pack


@pytest.mark.parametrize("period", [None, "2026-08-31"])
def test_full_pack_payload_is_released_before_verifying_next_pack(tmp_path, monkeypatch, period):
    class Document(dict):
        pass

    documents = []

    def verify(path):
        assert all(reference() is None for reference in documents)
        document = Document(
            current_report_dates=["2026-07-31"], overall_status="REVIEW",
            acknowledgement=None, exceptions=[{"reason": "Fabricated exception"}] * 2000,
            client_queries=[], controls_not_run=[],
        )
        documents.append(weakref.ref(document))
        return document, "summary", [], [], {"close-review-pack.json": "0" * 64}

    monkeypatch.setattr(queue, "verify_pack", verify)
    output = render_review_queue([tmp_path / "a", tmp_path / "b"], period=period)
    assert len(documents) == 2
    assert all(reference() is None for reference in documents)
    assert f"displaying {2 if period is None else 0}." in output
    if period is None:
        assert output.count("| REVIEW | 2000 | 0 |") == 2


def _write_pack(root: Path, name: str, *, status: str = "REVIEW",
                period: str = "2026-07-31", reviewer: str | None = None) -> Path:
    pack = _pack(status)
    if status == "PASS":
        pack = replace(pack, exceptions=())
    pack = replace(
        pack,
        current_report_dates=(period,),
        controls_not_run=("subledger_reconciliation",),
        acknowledgement=(
            ReviewerAcknowledgement(reviewer, date(2026, 8, 3), "Fabricated review.")
            if reviewer is not None else None
        ),
    )
    output = root / name
    write_review_pack(pack, output)
    return output


def test_queue_orders_periods_deduplicates_paths_and_keeps_evidence(tmp_path: Path) -> None:
    july = _write_pack(tmp_path, "july", status="BLOCKED", reviewer="RD")
    august = _write_pack(tmp_path, "august", status="PASS", period="2026-08-31")
    before = {p: p.read_bytes() for p in tmp_path.rglob("*") if p.is_file()}
    sheet = render_review_queue([august, july, july / "."])
    assert "Verified 2 distinct pack(s); displaying 2." in sheet
    assert sheet.index("| " + july.as_posix()) < sheet.index("| " + august.as_posix())
    assert "| BLOCKED | 1 |" in sheet
    assert "| PASS | 0 |" in sheet
    assert "RD (2026-08-03)" in sheet
    assert "subledger_reconciliation" in sheet
    assert "does not approve a close" in sheet
    assert "not preparer assignments" in sheet
    assert {p: p.read_bytes() for p in tmp_path.rglob("*") if p.is_file()} == before
    for content in before.values():
        assert hashlib.sha256(content).hexdigest() in sheet


def test_filters_use_recorded_date_status_and_reviewer(tmp_path: Path, capsys) -> None:
    selected = _write_pack(tmp_path, "selected", reviewer="RD")
    _write_pack(tmp_path, "wrong-reviewer", reviewer="AB")
    _write_pack(tmp_path, "wrong-period", period="2026-08-31", reviewer="RD")
    _write_pack(tmp_path, "wrong-state", status="PASS", reviewer="RD")
    args = ["queue"]
    for path in sorted(tmp_path.iterdir()):
        args.extend(["--pack-dir", str(path)])
    assert main([*args, "--period", "2026-07-31", "--status", "REVIEW", "--reviewer", "RD"]) == 0
    output = capsys.readouterr()
    assert "Verified 4 distinct pack(s); displaying 1." in output.out
    assert "| " + selected.as_posix() in output.out
    assert "wrong-" not in output.out
    assert output.err == ""


@pytest.mark.parametrize("file", ["close-summary.md", "exceptions.csv", "close-review-pack.json"])
def test_invalid_pack_is_not_hidden_by_filters(tmp_path: Path, capsys, file: str) -> None:
    good = _write_pack(tmp_path, "good", period="2026-08-31")
    bad = _write_pack(tmp_path, "bad")
    (bad / file).write_bytes(b"tampered fixture\n")
    assert main(["queue", "--pack-dir", str(good), "--pack-dir", str(bad),
                 "--period", "2026-08-31"]) == 1
    output = capsys.readouterr()
    assert output.out == ""
    assert "verification failed" in output.err
    assert "bad" in output.err


@pytest.mark.parametrize("options", [[], ["--period", "2026-08-31"]])
def test_malformed_pack_diagnostic_is_safe_after_a_valid_pack(tmp_path: Path, capsys, options) -> None:
    good = _write_pack(tmp_path, "a-good", period="2026-08-31")
    bad = _write_pack(tmp_path, "z-bad")
    json_path = bad / "close-review-pack.json"
    document = json.loads(json_path.read_text(encoding="utf-8"))
    key = "forged PASS\x1b[2J\x1b[H\n\t\u009b31m\u202e\u00e9"
    document[key] = "Fabricated malformed member."
    json_path.write_text(json.dumps(document) + "\n", encoding="utf-8")
    before = {p: p.read_bytes() for p in tmp_path.rglob("*") if p.is_file()}
    assert main(["queue", "--pack-dir", str(good), "--pack-dir", str(bad), *options]) == 1
    output = capsys.readouterr()
    assert output.out == ""
    assert output.err.endswith("\n")
    line = output.err[:-1]
    prefix = "close-control queue: verification failed: "
    assert line.startswith(prefix)
    assert all(0x20 <= ord(character) <= 0x7E for character in line)
    assert "z-bad" in line
    assert "close-review-pack.json: unknown top-level member(s):" in line
    assert r"\u001b[2J\u001b[H\n\t\u009b31m\u202e\u00e9" in line
    assert key in json.loads('"' + line[len(prefix):] + '"')
    assert {p: p.read_bytes() for p in tmp_path.rglob("*") if p.is_file()} == before


@pytest.mark.parametrize(("message", "expected"), [
    (r'Path C:\pack "quoted"; literal \n; alpha|beta; alpha\|beta',
     r'Path C:\\pack \"quoted\"; literal \\n; alpha|beta; alpha\\|beta'),
    ("Context\x00\x1b\n\r\t\x7f\u0085\u009b\u2028\u2029\u202e\u00e9\U0001f642\ud800end",
     r"Context\u0000\u001b\n\r\t\u007f\u0085\u009b\u2028\u2029\u202e\u00e9\ud83d\ude42\ud800end"),
])
def test_queue_diagnostic_escapes_complete_exception(tmp_path: Path, monkeypatch, capsys,
                                                  message: str, expected: str) -> None:
    def fail(*args, **kwargs):
        raise ControlInputError(message)

    monkeypatch.setattr("closecontrol.cli.render_review_queue", fail)
    assert main(["queue", "--pack-dir", str(tmp_path)]) == 1
    output = capsys.readouterr()
    prefix = "close-control queue: verification failed: "
    assert output.out == ""
    assert output.err == prefix + expected + "\n"
    assert all(0x20 <= ord(character) <= 0x7E for character in output.err[:-1])
    assert json.loads('"' + expected + '"') == message


def test_legacy_missing_coverage_and_queries_are_not_reported_as_none(tmp_path: Path) -> None:
    old = _legacy_pack(_write_pack(tmp_path, "old"))
    json_path = old / "close-review-pack.json"
    document = json.loads(json_path.read_text(encoding="utf-8"))
    del document["controls_not_run"]
    json_path.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    summary = old / "close-summary.md"
    text = summary.read_text(encoding="utf-8")
    summary.write_text(
        text.replace("- Controls not run: subledger_reconciliation.\n", ""),
        encoding="utf-8",
    )
    sheet = render_review_queue([old])
    row = next(line for line in sheet.splitlines() if line.startswith("| " + old.as_posix()))
    assert "| 1 | not recorded | not recorded | not recorded |" in row


def test_no_matches_is_verified_display_and_empty_input_is_refused(tmp_path: Path, capsys) -> None:
    pack = _write_pack(tmp_path, "pack")
    assert main(["queue", "--pack-dir", str(pack), "--reviewer", "rd"]) == 0
    assert "No verified packs match" in capsys.readouterr().out
    with pytest.raises(ValueError, match="at least one"):
        render_review_queue([])
    assert main(["queue"]) == 1
    assert "required" in capsys.readouterr().err
    assert main(["queue", "--pack-dir", str(tmp_path / "missing")]) == 1
    output = capsys.readouterr()
    assert output.out == ""
    assert "missing" in output.err


@pytest.mark.parametrize("options", [
    {"period": "2026-02-30"}, {"period": "20260731"},
    {"status": "APPROVED"}, {"reviewer": " "},
])
def test_invalid_filter_is_refused_before_reading_a_pack(tmp_path: Path, monkeypatch, options) -> None:
    def unexpected_verification(_path: Path) -> None:
        pytest.fail("Invalid filters must be rejected before pack verification.")

    monkeypatch.setattr(queue, "verify_pack", unexpected_verification)
    with pytest.raises(ControlInputError):
        render_review_queue([tmp_path / "missing"], **options)


def test_recorded_text_cannot_split_table_rows_or_emit_terminal_controls(tmp_path: Path) -> None:
    pack = _write_pack(tmp_path, "pack", reviewer="RD|\n\x1b[31m\u009b31m\u202e")
    sheet = render_review_queue([pack])
    assert "\x1b" not in sheet
    assert "\u009b" not in sheet
    assert "\u202e" not in sheet
    assert r"RD\|\n\u001b[31m" in sheet
