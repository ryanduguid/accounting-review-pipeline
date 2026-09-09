"""Phase B viewer tests: verify, then display, or refuse.

The acceptance set from issue #35: valid review, blocked/incomplete evidence,
altered digest, malformed artefacts, and structural proof that the viewer
carries no network or accounting-mutation code. The pilot fixtures are built
by the real writer, never hand-authored, so a renderer change that breaks
agreement fails here before it can mislead a reviewer.
"""

from __future__ import annotations

import ast
import csv
import json
from datetime import date
from decimal import Decimal
from pathlib import Path

import pytest

from closecontrol.cli import main
from closecontrol.engine import CloseReviewPack, review_close
from closecontrol.errors import ControlInputError
from closecontrol.models import ExceptionItem, ReviewerAcknowledgement
from closecontrol.report import write_review_pack
from closecontrol.viewer import PACK_FILE_NAMES, render_review_sheet, verify_pack


def _pack(status: str = "REVIEW", *, with_acknowledgement: bool = False) -> CloseReviewPack:
    exceptions = (
        ExceptionItem(
            control="subledger_reconciliation",
            status=status,
            tenant="Varrock Ventures Pty Ltd",
            account_id="100",
            account_code="100",
            account_name="Operating Bank",
            current_value=Decimal("15000.00"),
            prior_value=Decimal("15000.00"),
            difference=Decimal("15.00"),
            threshold=Decimal("0.01"),
            percentage_change=None,
            reason="GL to subledger difference 15.00 exceeds tolerance 0.01.",
            reviewer_action="Reconcile the Operating Bank subledger before review sign-off.",
        ),
    )
    acknowledgement = (
        ReviewerAcknowledgement(
            reviewer_initials="RD",
            reviewed_on=date(2026, 8, 3),
            comment="Subledger difference discussed with the controller.",
        )
        if with_acknowledgement
        else None
    )
    return CloseReviewPack(
        status="REVIEW",
        current_report_dates=("2026-07-31",),
        prior_report_dates=("2026-06-30",),
        source_hashes={"current_trial_balance": "a" * 64, "prior_trial_balance": "b" * 64},
        absolute_threshold=Decimal("1000"),
        percentage_threshold=Decimal("0.10"),
        reconciliation_tolerance=Decimal("0.01"),
        exceptions=exceptions,
        acknowledgement=acknowledgement,
    )


@pytest.fixture()
def pack_dir(tmp_path: Path) -> Path:
    output = tmp_path / "pack"
    write_review_pack(_pack(), output)
    return output


def _read_json(pack_dir: Path) -> dict:
    return json.loads((pack_dir / "close-review-pack.json").read_text(encoding="utf-8"))


def _rewrite_json(pack_dir: Path, document: dict) -> None:
    (pack_dir / "close-review-pack.json").write_text(
        json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


# --- valid evidence -------------------------------------------------------


def test_valid_pack_renders_sheet(pack_dir: Path) -> None:
    sheet, digests = render_review_sheet(pack_dir)
    assert "Overall status: REVIEW" in sheet
    assert "difference 15.00" in sheet
    assert "does not approve a close" in sheet
    for name in PACK_FILE_NAMES:
        assert f"{name}: sha256 " in sheet
    assert len(digests) == 4
    assert "No reviewer acknowledgement was supplied" in sheet


def test_acknowledged_pack_renders_the_acknowledgement(tmp_path: Path) -> None:
    output = tmp_path / "acknowledged-pack"
    write_review_pack(_pack(with_acknowledgement=True), output)
    sheet, _ = render_review_sheet(output)
    assert "- Reviewer initials: RD" in sheet
    assert "- Reviewed on: 2026-08-03" in sheet
    assert "- Comment: Subledger difference discussed with the controller." in sheet
    assert "does not change the control status or approve a close" in sheet


def test_sheet_shows_the_client_queries_and_says_nothing_was_sent(pack_dir: Path) -> None:
    """The fixture's one exception is a subledger difference, which is a
    question for the client. The sheet has to carry the boundary with it: a
    list of a client's accounts and unexplained movements reads as ready-to-send
    correspondence unless it says otherwise."""
    sheet, _ = render_review_sheet(pack_dir)
    assert "Client queries" in sheet
    assert "Client queries drafted: 1." in sheet
    assert "Nothing has been sent." in sheet
    assert "ask: What makes up the difference between the general ledger balance" in sheet
    assert "evidence: The reconciling items" in sheet


def test_a_pack_whose_query_boundary_was_removed_fails_closed(pack_dir: Path) -> None:
    summary = pack_dir / "close-summary.md"
    text = summary.read_text(encoding="utf-8")
    summary.write_text(
        text.replace("Nothing has been sent to anyone.", ""), encoding="utf-8"
    )
    with pytest.raises(ControlInputError, match="client-query boundary statement"):
        render_review_sheet(pack_dir)


def test_a_dropped_query_row_fails_closed(pack_dir: Path) -> None:
    path = pack_dir / "client-queries.csv"
    lines = path.read_text(encoding="utf-8-sig").splitlines(keepends=True)
    del lines[1]
    path.write_text("".join(lines), encoding="utf-8-sig")
    with pytest.raises(ControlInputError, match="client query counts disagree"):
        render_review_sheet(pack_dir)


def test_a_reworded_question_fails_closed(pack_dir: Path) -> None:
    """A question edited in the CSV alone is the failure that matters here: it
    is the cell somebody would change before sending, and the pack would then
    no longer be evidence of what the run actually asked."""
    path = pack_dir / "client-queries.csv"
    text = path.read_text(encoding="utf-8-sig")
    path.write_text(
        text.replace("What makes up the difference", "Please explain the difference"),
        encoding="utf-8-sig",
    )
    with pytest.raises(ControlInputError, match="question disagrees on client query 1"):
        render_review_sheet(pack_dir)


def test_a_duplicated_query_id_fails_closed(pack_dir: Path) -> None:
    path = pack_dir / "close-review-pack.json"
    document = json.loads(path.read_text(encoding="utf-8"))
    document["client_queries"].append(dict(document["client_queries"][0]))
    path.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    with pytest.raises(ControlInputError, match="more than once"):
        render_review_sheet(pack_dir)


def _legacy_pack(pack_dir: Path) -> Path:
    """Turn a pack into one written before the client-query register existed."""
    (pack_dir / "client-queries.csv").unlink()
    path = pack_dir / "close-review-pack.json"
    document = json.loads(path.read_text(encoding="utf-8"))
    del document["client_queries"]
    path.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    summary = pack_dir / "close-summary.md"
    text = summary.read_text(encoding="utf-8")
    summary.write_text(text.split("## Client queries")[0], encoding="utf-8")
    return pack_dir


def test_a_pack_written_before_the_register_still_opens(pack_dir: Path) -> None:
    """An archived three-file pack is evidence a firm may still have to show.
    Requiring a file that did not exist when it was written would make the
    older evidence unreadable by the command that exists to display it."""
    sheet, digests = render_review_sheet(_legacy_pack(pack_dir))
    assert "Overall status: REVIEW" in sheet
    assert "written before the client-query register existed" in sheet
    assert "Client queries drafted" not in sheet
    assert set(digests) == {"close-review-pack.json", "close-summary.md", "exceptions.csv"}


def test_half_a_register_fails_closed(pack_dir: Path) -> None:
    """A pack holding the file without the member, or the member without the
    file, was assembled from two runs or edited. It is evidence of neither
    format, so it is refused rather than read as the older one."""
    file_only = _legacy_pack(pack_dir)
    (file_only / "client-queries.csv").write_text(
        "query_id,control,tenant,account_id,account_code,account_name,"
        "review_group,difference,question,evidence_requested\n",
        encoding="utf-8-sig",
    )
    with pytest.raises(ControlInputError, match="half present"):
        render_review_sheet(file_only)


def test_member_without_file_fails_closed(pack_dir: Path) -> None:
    (pack_dir / "client-queries.csv").unlink()
    with pytest.raises(ControlInputError, match="half present"):
        render_review_sheet(pack_dir)


def test_a_surplus_cell_cannot_ride_along_unverified(pack_dir: Path) -> None:
    """csv.DictReader would file a surplus cell under a key nothing compares,
    so an appended formula would verify clean and be live the moment the CSV
    was opened. The writer's guard covers named fields only."""
    for name in ("client-queries.csv", "exceptions.csv"):
        path = pack_dir / name
        lines = path.read_text(encoding="utf-8-sig").splitlines()
        lines[1] += ",=cmd|'/c calc'!A0"
        path.write_text("\n".join(lines) + "\n", encoding="utf-8-sig")
        with pytest.raises(ControlInputError, match="cells, but the header declares"):
            render_review_sheet(pack_dir)
        # Put it back so the next file is tested against an otherwise sound pack.
        path.write_text(
            "\n".join(line.replace(",=cmd|'/c calc'!A0", "") for line in lines) + "\n",
            encoding="utf-8-sig",
        )


def test_a_question_edited_only_in_the_summary_fails_closed(pack_dir: Path) -> None:
    """The summary is what a preparer reads and copies a question out of, so a
    question changed there alone is the divergence that matters most: the pack
    would verify while the file somebody works from asks something else."""
    summary = pack_dir / "close-summary.md"
    text = summary.read_text(encoding="utf-8")
    summary.write_text(
        text.replace(
            "What makes up the difference between the general ledger balance",
            "Please explain the difference between the general ledger balance",
        ),
        encoding="utf-8",
    )
    with pytest.raises(ControlInputError, match="question for Q-.* is missing or altered"):
        render_review_sheet(pack_dir)


def test_a_query_dropped_only_from_the_summary_fails_closed(pack_dir: Path) -> None:
    summary = pack_dir / "close-summary.md"
    text = summary.read_text(encoding="utf-8")
    summary.write_text(
        text.replace("- Client queries drafted: 1.", "- Client queries drafted: 0."),
        encoding="utf-8",
    )
    with pytest.raises(ControlInputError, match="client-query counts disagree"):
        render_review_sheet(pack_dir)


def test_verify_returns_artefact_digests_matching_files(pack_dir: Path) -> None:
    import hashlib

    *_, artefact_digests = verify_pack(pack_dir)
    assert set(artefact_digests) == set(PACK_FILE_NAMES)
    for name, digest in artefact_digests.items():
        assert len(digest) == 64
        assert digest == hashlib.sha256((pack_dir / name).read_bytes()).hexdigest()


def test_view_command_renders_and_exits_zero(pack_dir: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert main(["view", "--pack-dir", str(pack_dir)]) == 0
    out = capsys.readouterr().out
    assert "Close Review Sheet" in out
    assert "review aid" in out


# --- missing and incomplete evidence --------------------------------------


def test_missing_file_fails_closed(pack_dir: Path) -> None:
    (pack_dir / "exceptions.csv").unlink()
    with pytest.raises(ControlInputError, match="not found"):
        render_review_sheet(pack_dir)


def test_view_on_empty_directory_fails_closed(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert main(["view", "--pack-dir", str(tmp_path)]) == 1
    assert "verification failed" in capsys.readouterr().err


def test_pack_dir_pointing_at_a_file_fails_closed(pack_dir: Path) -> None:
    # A file where the pack directory should be is a reviewer's mistake, not a
    # traceback. Windows raises FileNotFoundError for this path while POSIX
    # raises NotADirectoryError; both must remain a controlled input failure.
    with pytest.raises(
        ControlInputError,
        match=r"^close-review-pack\.json: (could not be read from|not found in) ",
    ):
        render_review_sheet(pack_dir / "close-summary.md")


# --- tampered JSON ---------------------------------------------------------


def test_altered_status_in_json_fails_closed(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["overall_status"] = "PASS"
    _rewrite_json(pack_dir, document)
    with pytest.raises(ControlInputError, match="status disagrees"):
        render_review_sheet(pack_dir)


def test_unknown_json_member_fails_closed(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["injected"] = True
    _rewrite_json(pack_dir, document)
    with pytest.raises(ControlInputError, match="unknown top-level member"):
        render_review_sheet(pack_dir)


def test_missing_json_member_fails_closed(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    del document["thresholds"]
    _rewrite_json(pack_dir, document)
    with pytest.raises(ControlInputError, match="missing top-level member"):
        render_review_sheet(pack_dir)


def test_duplicate_json_member_fails_closed(pack_dir: Path) -> None:
    text = (pack_dir / "close-review-pack.json").read_text(encoding="utf-8")
    poisoned = text.replace('"overall_status": "REVIEW",', '"overall_status": "PASS",\n  "overall_status": "REVIEW",', 1)
    assert poisoned != text
    (pack_dir / "close-review-pack.json").write_text(poisoned, encoding="utf-8")
    with pytest.raises(ControlInputError, match="more than once"):
        render_review_sheet(pack_dir)


def test_acknowledgement_that_is_not_an_object_fails_closed(tmp_path: Path) -> None:
    output = tmp_path / "acknowledged-pack"
    write_review_pack(_pack(with_acknowledgement=True), output)
    document = _read_json(output)
    document["acknowledgement"] = "RD reviewed and approved this close."
    _rewrite_json(output, document)
    with pytest.raises(ControlInputError, match="acknowledgement must be null or an object"):
        render_review_sheet(output)


def test_invalid_threshold_in_json_fails_closed(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["thresholds"]["absolute_variance"] = "not-a-number"
    _rewrite_json(pack_dir, document)
    with pytest.raises(ControlInputError, match="absolute_variance"):
        render_review_sheet(pack_dir)


def test_bad_digest_shape_in_json_fails_closed(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["source_sha256"]["current_trial_balance"] = "ZZZ"
    _rewrite_json(pack_dir, document)
    with pytest.raises(ControlInputError, match="SHA-256"):
        render_review_sheet(pack_dir)


def test_non_string_exception_field_fails_closed(pack_dir: Path) -> None:
    document = _read_json(pack_dir)
    document["exceptions"][0]["difference"] = 15.0
    _rewrite_json(pack_dir, document)
    with pytest.raises(ControlInputError, match=r"exceptions\[0\].difference must be a string"):
        render_review_sheet(pack_dir)


# --- tampered markdown -----------------------------------------------------


def test_altered_digest_in_summary_fails_closed(pack_dir: Path) -> None:
    summary = pack_dir / "close-summary.md"
    summary.write_text(
        summary.read_text(encoding="utf-8").replace("a" * 64, "c" * 64),
        encoding="utf-8",
    )
    with pytest.raises(ControlInputError, match="source evidence disagrees"):
        render_review_sheet(pack_dir)


def test_acknowledgement_comment_quoting_a_digest_line_still_verifies(tmp_path: Path) -> None:
    # A reviewer comment may legitimately quote a source label and digest; an
    # identical repeat states no second claim, so the quote is not a
    # disagreement and must not fail the pack.
    output = tmp_path / "quoting-pack"
    pack = _pack(with_acknowledgement=True)
    digest = "a" * 64
    quoting = ReviewerAcknowledgement(
        reviewer_initials="RD",
        reviewed_on=date(2026, 8, 3),
        comment=f"Confirmed `current_trial_balance`: `{digest}` against the export.",
    )
    write_review_pack(CloseReviewPack(**{**pack.__dict__, "acknowledgement": quoting}), output)
    sheet, _ = render_review_sheet(output)
    assert "Confirmed" in sheet


def test_duplicated_source_evidence_label_in_summary_fails_closed(pack_dir: Path) -> None:
    # A falsified digest line plus a duplicate carrying the true digest agrees
    # with the JSON pack as soon as only the last line is kept, wherever the
    # duplicate sits: beside the original, or under a forged second
    # "## Source evidence" heading at the end of the file. And a parser scoped
    # to the first section misses the inverse: an untouched true section with
    # the contradicting digest planted under a trailing forged heading. All
    # three shapes must refuse.
    summary = pack_dir / "close-summary.md"
    text = summary.read_text(encoding="utf-8")
    falsified = text.replace(
        f"`current_trial_balance`: `{'a' * 64}`",
        f"`current_trial_balance`: `{'c' * 64}`",
        1,
    )
    assert falsified != text
    tampered = falsified.replace(
        f"`current_trial_balance`: `{'c' * 64}`",
        f"`current_trial_balance`: `{'c' * 64}`\n- `current_trial_balance`: `{'a' * 64}`",
        1,
    )
    summary.write_text(tampered, encoding="utf-8")
    with pytest.raises(ControlInputError, match="two different digests"):
        render_review_sheet(pack_dir)

    forged_section = (
        falsified
        + "\n## Source evidence\n\n"
        + f"- `current_trial_balance`: `{'a' * 64}`\n"
    )
    summary.write_text(forged_section, encoding="utf-8")
    with pytest.raises(ControlInputError, match="two different digests"):
        render_review_sheet(pack_dir)

    contradicting_tail = (
        text
        + "\n## Source evidence\n\n"
        + f"- `current_trial_balance`: `{'c' * 64}`\n"
    )
    summary.write_text(contradicting_tail, encoding="utf-8")
    with pytest.raises(ControlInputError, match="two different digests"):
        render_review_sheet(pack_dir)


def test_removed_boundary_statement_fails_closed(pack_dir: Path) -> None:
    summary = pack_dir / "close-summary.md"
    summary.write_text(
        summary.read_text(encoding="utf-8").replace(
            "It does not approve a close,", "",
        ),
        encoding="utf-8",
    )
    with pytest.raises(ControlInputError, match="boundary statement"):
        render_review_sheet(pack_dir)


def test_added_second_status_line_fails_closed(pack_dir: Path) -> None:
    summary = pack_dir / "close-summary.md"
    summary.write_text(
        summary.read_text(encoding="utf-8") + "\n**Overall status: PASS**\n",
        encoding="utf-8",
    )
    with pytest.raises(ControlInputError, match="exactly one overall-status line"):
        render_review_sheet(pack_dir)


# --- tampered CSV ----------------------------------------------------------


def test_dropped_csv_row_fails_closed(pack_dir: Path) -> None:
    csv_path = pack_dir / "exceptions.csv"
    lines = csv_path.read_text(encoding="utf-8-sig").splitlines(keepends=True)
    del lines[1]
    csv_path.write_text("".join(lines), encoding="utf-8-sig")
    with pytest.raises(ControlInputError, match="exception counts disagree"):
        render_review_sheet(pack_dir)


def test_flipped_csv_cell_fails_closed(pack_dir: Path) -> None:
    csv_path = pack_dir / "exceptions.csv"
    text = csv_path.read_text(encoding="utf-8-sig")
    csv_path.write_text(text.replace("15.00", "16.00"), encoding="utf-8-sig")
    with pytest.raises(ControlInputError, match="disagrees on exception 1"):
        render_review_sheet(pack_dir)


def test_guarded_csv_field_survives_verification(pack_dir: Path) -> None:
    # A tenant whose name starts with '=' is guarded as "'..." on the CSV side
    # by the writer; the verifier must expect that exact guard, not flag it.
    # The writer guards after lstrip, so leading whitespace is guarded too.
    pack = _pack()
    item = pack.exceptions[0]
    guarded = ExceptionItem(**{**item.__dict__, "tenant": "=cmd|' /C calc'!A0"})
    spaced = ExceptionItem(**{**item.__dict__, "tenant": " \t=1+1"})
    repacked = CloseReviewPack(**{**pack.__dict__, "exceptions": (guarded, spaced)})
    output = pack_dir.parent / "guarded-pack"
    write_review_pack(repacked, output)
    sheet, _ = render_review_sheet(output)
    assert "Overall status: REVIEW" in sheet


def test_blocked_pack_renders_blocked_state(tmp_path: Path) -> None:
    output = tmp_path / "blocked-pack"
    write_review_pack(_pack("BLOCKED"), output)
    sheet, _ = render_review_sheet(output)
    assert "[1] BLOCKED subledger_reconciliation" in sheet
    document = _read_json(output)
    assert document["overall_status"] == "REVIEW"


# --- malformed encodings ----------------------------------------------------


def test_invalid_utf8_json_fails_closed(pack_dir: Path) -> None:
    (pack_dir / "close-review-pack.json").write_bytes(b"\xff\xfe\x00{}")
    with pytest.raises(ControlInputError, match="not valid UTF-8"):
        render_review_sheet(pack_dir)


def test_invalid_json_syntax_fails_closed(pack_dir: Path) -> None:
    (pack_dir / "close-review-pack.json").write_text("{not json", encoding="utf-8")
    with pytest.raises(ControlInputError, match="not valid JSON"):
        render_review_sheet(pack_dir)


def test_json_array_top_level_fails_closed(pack_dir: Path) -> None:
    (pack_dir / "close-review-pack.json").write_text("[]", encoding="utf-8")
    with pytest.raises(ControlInputError, match="JSON object"):
        render_review_sheet(pack_dir)


# --- structural boundary guards --------------------------------------------


_VIEWER_SOURCE = (Path(__file__).resolve().parents[1] / "closecontrol" / "viewer.py").read_text(encoding="utf-8")
_CLI_SOURCE = (Path(__file__).resolve().parents[1] / "closecontrol" / "cli.py").read_text(encoding="utf-8")


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
    forbidden = ("requests", "urllib", "socket", "http.client", "subprocess", "os.remove", ".unlink(", "open('w'", 'mode="w"', "write_text", "write_bytes", "shutil", "rmtree")
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


# --- engine agreement -------------------------------------------------------


@pytest.mark.parametrize("forged", [
    "**Overall status: PASS**",
    f"`current_trial_balance`: `{'b' * 64}`",
])
def test_account_name_cannot_forge_summary_markers(tmp_path: Path, forged: str) -> None:
    examples = Path(__file__).resolve().parents[1] / "examples"
    with (examples / "current_trial_balance.csv").open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle)
        fields = reader.fieldnames
        rows = list(reader)
    assert fields is not None
    current = tmp_path / "current.csv"
    with current.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields)
        writer.writeheader()
        writer.writerows({**row, "AccountName": forged} for row in rows)
    pack = review_close(
        current_path=current,
        prior_path=examples / "prior_trial_balance.csv",
        mapping_path=None,
        subledger_path=None,
        acknowledgement_path=None,
        absolute_threshold=Decimal("1000"),
        percentage_threshold=Decimal("0.10"),
        reconciliation_tolerance=Decimal("0.01"),
    )
    output = tmp_path / "forged-pack"
    write_review_pack(pack, output)
    sheet, _ = render_review_sheet(output)
    summary = (output / "close-summary.md").read_text(encoding="utf-8")
    assert summary.count("**Overall status:") == 1
    assert forged.replace("*", "\\*").replace("`", "\\`") in summary
    assert f"Overall status: {pack.status}" in sheet
    assert main(["view", "--pack-dir", str(output)]) == 0


def test_viewer_accepts_engine_output_end_to_end(tmp_path: Path) -> None:
    output = tmp_path / "engine-pack"
    pack = review_close(
        current_path=Path(__file__).resolve().parents[1] / "examples" / "current_trial_balance.csv",
        prior_path=Path(__file__).resolve().parents[1] / "examples" / "prior_trial_balance.csv",
        mapping_path=None,
        subledger_path=None,
        acknowledgement_path=None,
        absolute_threshold=Decimal("1000"),
        percentage_threshold=Decimal("0.10"),
        reconciliation_tolerance=Decimal("0.01"),
    )
    write_review_pack(pack, output)
    sheet, _ = render_review_sheet(output)
    assert f"Overall status: {pack.status}" in sheet
