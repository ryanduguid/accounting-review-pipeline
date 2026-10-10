"""Characterise decision order and recovery before complexity extraction."""
from __future__ import annotations

import json
from decimal import Decimal
from pathlib import Path

import pytest
from reviewready import cli, publication, viewer
from reviewready.engine import review_pack
from reviewready.errors import GateInputError
from reviewready.report import _as_json, write_review_pack
from tests.support import EXAMPLES
from tests.test_gate import _not_ready_pack, _ready_pack
from tests.test_publication import Cancelled, assert_clean, payloads


@pytest.mark.parametrize(
    ("member", "value", "message"),
    [
        ("overall_status", "PASS", "overall_status must be one of READY, NOT_READY, BLOCKED; got 'PASS'"),
        ("engagement_type", [], "engagement_type must be one of bas, month_end, year_end; got []"),
        ("period_end", None, "period_end must be a string"),
        ("preparer_initials", False, "preparer_initials must be a string"),
        ("review_boundary", 1, "review_boundary must be a string"),
        ("review_boundary", "changed", "review_boundary does not match the written contract"),
        ("thresholds", {}, "thresholds must hold exactly tieout_tolerance"),
        ("thresholds", {"tieout_tolerance": 1}, "thresholds.tieout_tolerance must be a string"),
        ("thresholds", {"tieout_tolerance": "bad"}, "thresholds.tieout_tolerance is not a decimal: 'bad'"),
        ("thresholds", {"tieout_tolerance": "NaN"}, "thresholds.tieout_tolerance must be finite and non-negative"),
        ("source_sha256", [], "source_sha256 must be an object"),
        ("source_sha256", {"tb": {}}, "source_sha256['tb'] must hold exactly filename and sha256"),
        ("source_sha256", {"tb": {"filename": "", "sha256": "a" * 64}}, "source_sha256['tb'].filename must be a non-empty string"),
        ("source_sha256", {"tb": {"filename": "tb.csv", "sha256": "A" * 64}}, "source_sha256['tb'].sha256 is not a lowercase SHA-256 digest"),
        ("acknowledgement", {}, "acknowledgement must be null or hold exactly comment, effect, reviewed_on, reviewer_initials"),
        ("findings", {}, "findings must be a list"),
        ("findings", [None], "findings[0] must be an object"),
        ("controls_not_run", None, "controls_not_run must be a list"),
        ("controls_not_run", [{}], "controls_not_run[0] does not hold the fields the writer emits: expected ['filename', 'reason', 'slot']"),
        ("controls_not_run", [{"slot": "bank", "filename": "", "reason": "missing"}], "controls_not_run[0].filename must be a non-empty string"),
    ],
)
def test_schema_exact_refusals(member: str, value: object, message: str) -> None:
    document = _as_json(_ready_pack())
    document[member] = value
    with pytest.raises(GateInputError) as caught:
        viewer._verify_json_schema(document)
    assert str(caught.value) == "readiness-pack.json: " + message


@pytest.mark.parametrize("first", ["overall_status", "thresholds", "source_sha256", "acknowledgement", "findings"])
def test_schema_checks_keep_their_order(first: str) -> None:
    document = _as_json(_ready_pack())
    invalid = {
        "overall_status": "PASS", "thresholds": {}, "source_sha256": [],
        "acknowledgement": {}, "findings": {}, "controls_not_run": None,
    }
    keys = list(invalid)
    for key in keys[keys.index(first):]:
        document[key] = invalid[key]
    with pytest.raises(GateInputError) as caught:
        viewer._verify_json_schema(document)
    assert str(caught.value).startswith(f"readiness-pack.json: {first} ")


@pytest.mark.parametrize("value", ["NaN", "Infinity", "-0.01"])
def test_tolerance_refusal_precedes_unknown_profile_and_missing_pack(value: str, tmp_path: Path) -> None:
    with pytest.raises(GateInputError) as caught:
        review_pack(profile="unknown", pack_dir=tmp_path / "missing", tieout_tolerance=Decimal(value))
    assert str(caught.value) == "tie-out tolerance must be a finite non-negative decimal."


def test_profile_refusal_precedes_missing_pack(tmp_path: Path) -> None:
    with pytest.raises(GateInputError) as caught:
        review_pack(profile="unknown", pack_dir=tmp_path / "missing")
    assert str(caught.value) == "\"unknown engagement profile 'unknown'; expected one of: bas, month_end, year_end.\""


def test_acknowledgement_identity_refusal_precedes_date_refusal(tmp_path: Path) -> None:
    note = tmp_path / "note.json"
    note.write_text(json.dumps({"reviewer_initials": " ab ", "reviewed_on": "2026-03-30", "comment": "Reviewed"}), encoding="utf-8")
    with pytest.raises(GateInputError) as caught:
        review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready", acknowledgement_path=note)
    assert str(caught.value) == "reviewer initials must differ from the preparer initials."


@pytest.mark.parametrize("command", ["intake", "view-document", "compare", "view", "gate-input", "gate-output"])
def test_cli_command_error_prefixes(monkeypatch, tmp_path: Path, capsys, command: str) -> None:
    def refuse(*args, **kwargs):
        raise GateInputError("characterised refusal")

    path = str(tmp_path / "absent")
    cases = {
        "intake": ("create_intake", ["intake", "--synthetic", "--source", path, "--text", path, "--text-origin", "manual", "--entity", "Synthetic", "--period-end", "2026-03-31", "--output", path], "review-ready intake: "),
        "view-document": ("load_document", ["view-document", "--bundle", path], "review-ready view-document: verification failed: "),
        "compare": ("compare_packs", ["compare", "--previous-pack-dir", path, "--current-pack-dir", path], "review-ready compare: verification failed: "),
        "view": ("render_review_sheet", ["view", "--pack-dir", path], "review-ready view: verification failed: "),
        "gate-input": ("review_pack", ["gate", "--profile", "bas", "--pack", path, "--output", path], "review-ready: input error: "),
        "gate-output": ("require_output_outside_repository", ["gate", "--profile", "bas", "--pack", path, "--output", path], "review-ready: output error: "),
    }
    function, argv, prefix = cases[command]
    monkeypatch.setattr(cli, function, refuse)
    assert cli.main(argv) == 1
    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == prefix + "characterised refusal\n"


def test_publication_slot_refusal_precedes_destination_inspection(tmp_path: Path) -> None:
    with pytest.raises(ValueError) as caught:
        publication.publish(tmp_path / "missing", ())
    assert str(caught.value) == "publication requires the three fixed pack slots"


@pytest.mark.parametrize("point", ["backup", "stage"])
def test_preparation_failure_restores_without_changing_payloads(monkeypatch, tmp_path: Path, point: str) -> None:
    output = tmp_path / "pack"
    write_review_pack(_ready_pack(), output)
    before = payloads(output)
    copy, write = publication.shutil.copyfileobj, Path.write_text

    def fail_copy(source, sink, *args, **kwargs):
        if Path(sink.name).name == "backup-1":
            raise Cancelled("backup interrupted")
        return copy(source, sink, *args, **kwargs)

    def fail_write(path, *args, **kwargs):
        if path.name == "stage-1":
            raise Cancelled("stage interrupted")
        return write(path, *args, **kwargs)

    with monkeypatch.context() as inner:
        inner.setattr(publication.shutil, "copyfileobj", fail_copy if point == "backup" else copy)
        inner.setattr(Path, "write_text", fail_write if point == "stage" else write)
        with pytest.raises(Cancelled, match=f"{point} interrupted"):
            write_review_pack(_not_ready_pack(), output)
    assert payloads(output) == before
    assert_clean(output)


def test_corrupt_backup_is_retained_on_failed_recovery(monkeypatch, tmp_path: Path) -> None:
    output = tmp_path / "pack"
    write_review_pack(_ready_pack(), output)
    replace = publication.os.replace

    def interrupt(source, destination):
        replace(source, destination)
        source = Path(source)
        if source.name == "stage-0":
            (source.parent / "backup-0").write_bytes(b"changed backup")
            raise Cancelled()

    monkeypatch.setattr(publication.os, "replace", interrupt)
    with pytest.raises(OSError) as caught:
        write_review_pack(_not_ready_pack(), output)
    assert str(caught.value) == "publication or recovery incomplete; retain control and backups for review"
    assert str(caught.value.__cause__) == "recovery backup differs; retain recovery evidence"
    state = publication._read_state(output / publication.CONTROL_NAME)
    assert state["status"] == "in_progress"
    transaction = output / publication.CONTROL_NAME / ("tx-" + state["transaction"])
    assert (transaction / "backup-0").read_bytes() == b"changed backup"


def test_publication_checkpoint_order(monkeypatch, tmp_path: Path) -> None:
    output = tmp_path / "pack"
    write_review_pack(_ready_pack(), output)
    checkpoints = []

    def observe():
        state = publication._read_state(output / publication.CONTROL_NAME)
        entries = [] if state["transaction"] is None else list(
            (output / publication.CONTROL_NAME / ("tx-" + state["transaction"])).iterdir()
        )
        checkpoints.append((
            state["status"], state["transaction"] is None,
            sum(path.name.startswith("backup-") for path in entries),
            sum(path.name.startswith("stage-") for path in entries),
            sum(value is not None for value in state["previous"]),
        ))

    monkeypatch.setattr(publication, "_check_cancellation", observe)
    write_review_pack(_not_ready_pack(), output)
    assert checkpoints == [
        ("settled", True, 0, 0, 0), ("settled", True, 0, 0, 0),
        ("in_progress", False, 0, 0, 0), ("in_progress", False, 1, 0, 0),
        ("in_progress", False, 2, 0, 0), ("in_progress", False, 3, 0, 0),
        ("in_progress", False, 3, 1, 0), ("in_progress", False, 3, 2, 0),
        ("in_progress", False, 3, 3, 0), ("in_progress", False, 3, 3, 3),
        ("in_progress", False, 3, 2, 3), ("in_progress", False, 3, 1, 3),
        ("in_progress", False, 3, 0, 3), ("settled", False, 3, 0, 3),
        ("settled", True, 0, 0, 0),
    ]


def test_prepared_state_write_that_commits_then_raises(monkeypatch, tmp_path: Path) -> None:
    output = tmp_path / "pack"
    write_review_pack(_ready_pack(), output)
    before, revision = payloads(output), publication.snapshot_revision(output)
    write = publication._write_state

    def interrupt(control, state):
        write(control, state)
        if state["status"] == "in_progress" and all(state["previous"]):
            raise Cancelled("prepared state interrupted")

    monkeypatch.setattr(publication, "_write_state", interrupt)
    with pytest.raises(Cancelled, match="prepared state interrupted"):
        write_review_pack(_not_ready_pack(), output)
    assert payloads(output) == before
    assert publication.snapshot_revision(output) != revision
    assert_clean(output)


@pytest.mark.parametrize("absent", publication.NAMES)
def test_rollback_restores_each_previously_absent_slot(monkeypatch, tmp_path: Path, absent: str) -> None:
    output = tmp_path / "pack"
    write_review_pack(_ready_pack(), output)
    (output / absent).unlink()
    before = payloads(output)
    replace = publication.os.replace

    def interrupt(source, destination):
        replace(source, destination)
        if Path(source).name == "stage-2":
            raise Cancelled()

    monkeypatch.setattr(publication.os, "replace", interrupt)
    with pytest.raises(Cancelled):
        write_review_pack(_not_ready_pack(), output)
    assert payloads(output) == before
    assert not (output / absent).exists()
    assert publication._read_state(output / publication.CONTROL_NAME)["transaction"] is None


@pytest.mark.parametrize("command", ["view", "compare"])
def test_display_commands_preserve_all_publication_bytes(tmp_path: Path, command: str) -> None:
    output = tmp_path / "pack"
    write_review_pack(_ready_pack(), output)

    def snapshot():
        return {path.relative_to(output): (path.read_bytes(), path.stat().st_mtime_ns)
                for path in output.rglob("*") if path.is_file()}

    before = snapshot()
    argv = ["view", "--pack-dir", str(output)] if command == "view" else [
        "compare", "--previous-pack-dir", str(output), "--current-pack-dir", str(output),
    ]
    assert cli.main(argv) == 0
    assert snapshot() == before
