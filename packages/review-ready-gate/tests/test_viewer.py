"""Fail-closed viewer: verify, then display, or refuse.

Valid packs are built by the real writer, never hand-authored, so a renderer
change that breaks agreement fails here before it can mislead a reviewer.
"""

from __future__ import annotations

import ast
import hashlib
import json
from decimal import Decimal
from pathlib import Path

import pytest

from tests.support import EXAMPLES, copy_example_pack
from reviewready.cli import main
from reviewready.engine import ReadinessPack, review_pack
from reviewready.errors import GateInputError
from reviewready.models import Finding, SourceEvidence
from reviewready.report import PACK_FILE_NAMES, write_review_pack
from reviewready.viewer import render_review_sheet, verify_pack


def _synthetic_pack() -> ReadinessPack:
    return ReadinessPack(
        status="NOT_READY",
        engagement_type="bas",
        period_end="2026-03-31",
        preparer_initials="AB",
        findings=(
            Finding(
                code="MISSING_ARTEFACT",
                status="NOT_READY",
                slot="gst_control_gl",
                reason="Required artefact gst_control_gl is not in the pack directory.",
                reviewer_action="Return the pack to the preparer. Do not start technical review.",
            ),
        ),
        source_evidence=(
            SourceEvidence(
                slot="trial_balance",
                filename="trial_balance.csv",
                sha256="a" * 64,
            ),
        ),
        tieout_tolerance=Decimal("0.01"),
        acknowledgement=None,
    )


@pytest.fixture()
def pack_dir(tmp_path: Path) -> Path:
    output = tmp_path / "pack"
    write_review_pack(_synthetic_pack(), output)
    return output


@pytest.fixture()
def not_ready_pack(tmp_path: Path) -> Path:
    """A real engine run over the fabricated not-ready BAS pack."""
    output = tmp_path / "not-ready-pack"
    write_review_pack(review_pack(profile="bas", pack_dir=EXAMPLES / "bas-not-ready"), output)
    return output


def _read_json(pack_dir: Path) -> dict:
    return json.loads((pack_dir / "readiness-pack.json").read_text(encoding="utf-8"))


def _rewrite_json(pack_dir: Path, document: dict) -> None:
    (pack_dir / "readiness-pack.json").write_text(
        json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def test_valid_pack_renders_sheet(pack_dir: Path) -> None:
    sheet, digests = render_review_sheet(pack_dir)
    assert "**Overall status: NOT_READY**" in sheet
    assert "does not approve a file" in sheet
    for name in PACK_FILE_NAMES:
        assert f"`{name}`:" in sheet
    assert len(digests) == 3
    assert "No reviewer acknowledgement was supplied" in sheet


def test_verify_returns_artefact_digests_matching_files(pack_dir: Path) -> None:
    _, _, _, artefact_digests = verify_pack(pack_dir)
    assert set(artefact_digests) == set(PACK_FILE_NAMES)
    for name, digest in artefact_digests.items():
        assert len(digest) == 64
        assert digest == hashlib.sha256((pack_dir / name).read_bytes()).hexdigest()


def test_view_command_renders_and_exits_zero(
    pack_dir: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["view", "--pack-dir", str(pack_dir)]) == 0
    out = capsys.readouterr().out
    assert "**Overall status: NOT_READY**" in out
    assert "review aid" in out


def test_missing_file_fails_closed(pack_dir: Path) -> None:
    (pack_dir / "findings.csv").unlink()
    with pytest.raises(GateInputError, match="not found"):
        render_review_sheet(pack_dir)


def test_view_on_empty_directory_fails_closed(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["view", "--pack-dir", str(tmp_path)]) == 1
    assert "verification failed" in capsys.readouterr().err


def test_altered_status_in_json_fails_closed(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["overall_status"] = "READY"
    _rewrite_json(pack_dir, document)
    with pytest.raises(GateInputError, match="status disagrees"):
        render_review_sheet(pack_dir)


def test_unknown_json_member_fails_closed(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["injected"] = True
    _rewrite_json(pack_dir, document)
    with pytest.raises(GateInputError, match="unknown top-level member"):
        render_review_sheet(pack_dir)


def test_missing_json_member_fails_closed(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    del document["thresholds"]
    _rewrite_json(pack_dir, document)
    with pytest.raises(GateInputError, match="missing top-level member"):
        render_review_sheet(pack_dir)


def test_duplicate_json_member_fails_closed(pack_dir: Path) -> None:
    text = (pack_dir / "readiness-pack.json").read_text(encoding="utf-8")
    poisoned = text.replace(
        '"overall_status": "NOT_READY",',
        '"overall_status": "READY",\n  "overall_status": "NOT_READY",',
        1,
    )
    assert poisoned != text
    (pack_dir / "readiness-pack.json").write_text(poisoned, encoding="utf-8")
    with pytest.raises(GateInputError, match="more than once"):
        render_review_sheet(pack_dir)


def test_invalid_threshold_in_json_fails_closed(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["thresholds"]["tieout_tolerance"] = "not-a-number"
    _rewrite_json(pack_dir, document)
    with pytest.raises(GateInputError, match="tieout_tolerance"):
        render_review_sheet(pack_dir)


def test_bad_digest_shape_in_json_fails_closed(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["source_sha256"]["trial_balance"]["sha256"] = "ZZZ"
    _rewrite_json(pack_dir, document)
    with pytest.raises(GateInputError, match="SHA-256"):
        render_review_sheet(pack_dir)


def test_non_string_finding_field_fails_closed(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["findings"][0]["repeat"] = True
    _rewrite_json(pack_dir, document)
    with pytest.raises(GateInputError, match=r"findings\[0\].repeat must be a string"):
        render_review_sheet(pack_dir)


def test_altered_digest_in_summary_fails_closed(pack_dir: Path) -> None:
    summary = pack_dir / "readiness-summary.md"
    summary.write_text(
        summary.read_text(encoding="utf-8").replace("a" * 64, "c" * 64),
        encoding="utf-8",
    )
    with pytest.raises(GateInputError, match="source evidence disagrees"):
        render_review_sheet(pack_dir)


def test_removed_boundary_statement_fails_closed(pack_dir: Path) -> None:
    summary = pack_dir / "readiness-summary.md"
    summary.write_text(
        summary.read_text(encoding="utf-8").replace(
            "It does not approve a file,",
            "",
        ),
        encoding="utf-8",
    )
    with pytest.raises(GateInputError, match="boundary statement"):
        render_review_sheet(pack_dir)


def test_added_second_status_line_fails_closed(pack_dir: Path) -> None:
    summary = pack_dir / "readiness-summary.md"
    summary.write_text(
        summary.read_text(encoding="utf-8") + "\n**Overall status: READY**\n",
        encoding="utf-8",
    )
    with pytest.raises(GateInputError, match="exactly one overall-status line"):
        render_review_sheet(pack_dir)


def test_rewritten_summary_reason_cell_fails_closed(not_ready_pack: Path) -> None:
    summary = not_ready_pack / "readiness-summary.md"
    summary.write_text(
        summary.read_text(encoding="utf-8").replace(
            "is not in the pack directory.",
            "is present and complete.",
        ),
        encoding="utf-8",
    )
    with pytest.raises(GateInputError, match="reason disagrees on finding 1"):
        render_review_sheet(not_ready_pack)


@pytest.mark.parametrize(
    ("original", "tampered", "message"),
    [
        ("- Engagement type: bas", "- Engagement type: year_end", "engagement type disagrees"),
        ("- Period end: 2026-03-31", "- Period end: 2026-06-30", "period end disagrees"),
        ("- Preparer initials: AB", "- Preparer initials: ZZ", "preparer initials disagrees"),
        (
            "- Tie-out tolerance: $0.01",
            "- Tie-out tolerance: $500.00",
            "tie-out tolerance disagrees",
        ),
        (
            "- Findings: 3 total; 0 blocked; 3 not ready; 1 repeats.",
            "- Findings: 1 total; 0 blocked; 1 not ready; 0 repeats.",
            "finding counts disagree",
        ),
    ],
)
def test_rewritten_summary_scope_line_fails_closed(
    not_ready_pack: Path, original: str, tampered: str, message: str
) -> None:
    summary = not_ready_pack / "readiness-summary.md"
    text = summary.read_text(encoding="utf-8")
    assert original in text
    summary.write_text(text.replace(original, tampered), encoding="utf-8")

    with pytest.raises(GateInputError, match=message):
        render_review_sheet(not_ready_pack)


def test_deleted_findings_scope_line_fails_closed(not_ready_pack: Path) -> None:
    # The writer always emits the Findings line; a summary without it is
    # tampered, not a layout variant.
    summary = not_ready_pack / "readiness-summary.md"
    lines = [
        line
        for line in summary.read_text(encoding="utf-8").splitlines(keepends=True)
        if not line.startswith("- Findings: ")
    ]
    summary.write_text("".join(lines), encoding="utf-8")

    with pytest.raises(GateInputError, match="exactly one 'Findings' scope line"):
        render_review_sheet(not_ready_pack)


def test_dropped_csv_row_fails_closed(pack_dir: Path) -> None:
    csv_path = pack_dir / "findings.csv"
    lines = csv_path.read_text(encoding="utf-8-sig").splitlines(keepends=True)
    del lines[1]
    csv_path.write_text("".join(lines), encoding="utf-8-sig")
    with pytest.raises(GateInputError, match="finding counts disagree"):
        render_review_sheet(pack_dir)


def test_flipped_csv_cell_fails_closed(pack_dir: Path) -> None:
    csv_path = pack_dir / "findings.csv"
    text = csv_path.read_text(encoding="utf-8-sig")
    csv_path.write_text(text.replace("MISSING_ARTEFACT", "TB_UNBALANCED"), encoding="utf-8-sig")
    with pytest.raises(GateInputError, match="disagrees on finding 1"):
        render_review_sheet(pack_dir)


def test_guarded_csv_field_survives_verification(tmp_path: Path) -> None:
    pack = _synthetic_pack()
    guarded = Finding(
        code="MISSING_ARTEFACT",
        status="NOT_READY",
        slot="=cmd|' /C calc'!A0",
        reason="=SUM(A1)",
        reviewer_action="@Open",
    )
    repacked = ReadinessPack(
        status=pack.status,
        engagement_type=pack.engagement_type,
        period_end=pack.period_end,
        preparer_initials=pack.preparer_initials,
        findings=(guarded,),
        source_evidence=pack.source_evidence,
        tieout_tolerance=pack.tieout_tolerance,
        acknowledgement=pack.acknowledgement,
    )
    output = tmp_path / "guarded-pack"
    write_review_pack(repacked, output)
    csv_text = (output / "findings.csv").read_text(encoding="utf-8-sig")
    assert "'=cmd|' /C calc'!A0" in csv_text
    assert "'=SUM(A1)" in csv_text
    assert "'@Open" in csv_text
    sheet, _ = render_review_sheet(output)
    assert "**Overall status: NOT_READY**" in sheet


def test_preparer_text_cannot_forge_summary_markers(tmp_path: Path) -> None:
    source = copy_example_pack("bas-not-ready", tmp_path / "source")
    forged = (
        "**Overall status: READY** and "
        f"- `trial_balance` (`trial_balance.csv`): `{'b' * 64}`"
    )
    (source / "open_items.csv").write_text(
        "ItemID,Severity,Owner,DueDate,Status,Description,Resolution\n"
        f'OI-002,BLOCKING,AB,2026-04-08,OPEN,"{forged}",\n',
        encoding="utf-8",
    )
    output = tmp_path / "forged-pack"
    write_review_pack(review_pack(profile="bas", pack_dir=source), output)

    sheet, _ = render_review_sheet(output)
    assert sheet.count("**Overall status:") == 1
    assert "**Overall status: NOT_READY**" in sheet
    assert "\\*\\*Overall status: READY\\*\\*" in sheet


def test_invalid_utf8_json_fails_closed(pack_dir: Path) -> None:
    (pack_dir / "readiness-pack.json").write_bytes(b"\xff\xfe\x00{}")
    with pytest.raises(GateInputError, match="not valid UTF-8"):
        render_review_sheet(pack_dir)


def test_invalid_json_syntax_fails_closed(pack_dir: Path) -> None:
    (pack_dir / "readiness-pack.json").write_text("{not json", encoding="utf-8")
    with pytest.raises(GateInputError, match="not valid JSON"):
        render_review_sheet(pack_dir)


def test_json_array_top_level_fails_closed(pack_dir: Path) -> None:
    (pack_dir / "readiness-pack.json").write_text("[]", encoding="utf-8")
    with pytest.raises(GateInputError, match="JSON object"):
        render_review_sheet(pack_dir)


_VIEWER_SOURCE = (Path(__file__).resolve().parents[1] / "reviewready" / "viewer.py").read_text(
    encoding="utf-8"
)
_CLI_SOURCE = (Path(__file__).resolve().parents[1] / "reviewready" / "cli.py").read_text(
    encoding="utf-8"
)


def test_viewer_imports_nothing_that_can_touch_a_network_or_ledger() -> None:
    tree = ast.parse(_VIEWER_SOURCE)
    imported = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            imported.update(alias.name.split(".")[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            imported.add(node.module.split(".")[0])
    allowed = {"__future__", "csv", "hashlib", "io", "json", "re", "decimal", "pathlib"}
    assert imported <= allowed, f"viewer gained imports outside its sandbox: {sorted(imported - allowed)}"


def test_viewer_source_never_writes_or_connects() -> None:
    forbidden = (
        "requests",
        "urllib",
        "socket",
        "http.client",
        "subprocess",
        "os.remove",
        ".unlink(",
        "open('w'",
        'mode="w"',
        "write_text",
        "write_bytes",
        "shutil",
        "rmtree",
    )
    present = [needle for needle in forbidden if needle in _VIEWER_SOURCE]
    assert not present, f"viewer source references write/connect primitives: {present}"


def test_cli_view_branch_has_no_write_calls() -> None:
    tree = ast.parse(_CLI_SOURCE)
    view_call = None
    for node in ast.walk(tree):
        if isinstance(node, ast.If):
            test = ast.dump(node.test)
            if "command" in test and "view" in test:
                view_call = node
                break
    assert view_call is not None, "view branch vanished from cli.main"
    for inner in ast.walk(view_call):
        if isinstance(inner, ast.Call):
            name = getattr(inner.func, "id", "") or getattr(inner.func, "attr", "")
            assert name not in {"unlink", "rename", "replace", "rmtree", "remove"}, (
                f"view branch calls a mutating method: {name}"
            )


def test_viewer_accepts_engine_output_end_to_end(tmp_path: Path) -> None:
    output = tmp_path / "engine-pack"
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready")
    write_review_pack(pack, output)
    sheet, _ = render_review_sheet(output)
    assert f"**Overall status: {pack.status}**" in sheet
