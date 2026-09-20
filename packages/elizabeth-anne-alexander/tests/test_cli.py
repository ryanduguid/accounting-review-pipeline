from __future__ import annotations

import io
import json
import os
import sys
from pathlib import Path

import pytest
from elizabeth_anne_alexander import persist
from elizabeth_anne_alexander.cli import main
from elizabeth_anne_alexander.errors import GatewayError

EVALUATE = [
    "evaluate",
    "--context", "samples/contexts/sample-monthly-variance.context.json",
    "--request", "samples/requests/sample-revenue-variance.request.json",
    "--policy", "policy/demo-policy-v1.json",
]


def test_cli_end_to_end_from_any_working_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.chdir(tmp_path)
    assert main([
        "evaluate",
        "--context", "samples/contexts/sample-monthly-variance.context.json",
        "--request", "samples/requests/sample-revenue-variance.request.json",
        "--policy", "policy/demo-policy-v1.json",
        "--out", "build/demo",
    ]) == 0
    assert (tmp_path / "build" / "demo" / "receipt.json").is_file()
    assert main([
        "validate-review",
        "--evidence", "build/demo/reviewer-evidence.json",
        "--receipt", "build/demo/receipt.json",
        "--decision", "samples/decisions/sample-review-decision.json",
    ]) == 0
    output = capsys.readouterr()
    assert output.out.count("elizabeth-anne-alexander:") == 2
    assert "xero-ai-" + "review-gateway:" not in output.out
    assert "xero-ledger-review-gate:" not in output.out


def test_cli_blocks_output_outside_cwd_build(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    assert main([
        "evaluate",
        "--context", "samples/contexts/sample-monthly-variance.context.json",
        "--request", "samples/requests/sample-revenue-variance.request.json",
        "--policy", "policy/demo-policy-v1.json",
        "--out", str(tmp_path / "elsewhere"),
    ]) == 2


def test_a_finished_run_is_not_failed_by_a_path_the_output_stream_cannot_encode(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Redirected stdout on Windows is cp1252; a working directory outside it must not fail the run."""
    # U+0141 LATIN CAPITAL LETTER L WITH STROKE, written escaped so this file
    # stays pure ASCII.
    workdir = tmp_path / "demo-\u0141ukasz"
    workdir.mkdir()
    monkeypatch.chdir(workdir)
    captured = io.BytesIO()
    monkeypatch.setattr(sys, "stdout", io.TextIOWrapper(captured, encoding="cp1252", errors="strict", newline="\n"))

    code = main([*EVALUATE, "--out", "build/demo"])
    sys.stdout.flush()

    assert code == 0
    assert (workdir / "build" / "demo" / "receipt.json").is_file()
    assert b"\\u0141" in captured.getvalue()


def test_a_run_leaves_the_error_handler_of_every_stream_it_prints_to_alone(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Escaping one write must not reconfigure the interpreter's streams for the rest of the process.

    main() is a library entry point called in-process, so switching
    sys.stdout/sys.stderr to backslashreplace would silently change how every
    later caller's output is encoded.
    """
    workdir = tmp_path / "demo-\u0141ukasz"
    workdir.mkdir()
    monkeypatch.chdir(workdir)
    out, err = io.BytesIO(), io.BytesIO()
    monkeypatch.setattr(sys, "stdout", io.TextIOWrapper(out, encoding="cp1252", errors="strict", newline="\n"))
    monkeypatch.setattr(sys, "stderr", io.TextIOWrapper(err, encoding="cp1252", errors="strict", newline="\n"))

    assert main([*EVALUATE, "--out", "build/demo"]) == 0
    assert main([*EVALUATE, "--out", str(tmp_path / "elsewhere")]) == 2
    sys.stdout.flush()
    sys.stderr.flush()

    assert sys.stdout.errors == "strict"
    assert sys.stderr.errors == "strict"
    # The blocked run names the rejected path, which is also outside cp1252.
    assert b"blocked" in err.getvalue()
    assert b"\\u0141" in out.getvalue()


def test_an_output_path_occupied_by_a_file_is_blocked_not_a_traceback(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    (tmp_path / "build").mkdir()
    (tmp_path / "build" / "occupied").write_text("not a directory\n", encoding="utf-8")

    assert main([*EVALUATE, "--out", "build/occupied"]) == 2


def test_a_validation_output_path_that_is_a_directory_is_blocked_not_a_traceback(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.chdir(tmp_path)
    assert main([*EVALUATE, "--out", "build/demo"]) == 0
    (tmp_path / "build" / "validation.json").mkdir()

    assert main([
        "validate-review",
        "--evidence", "build/demo/reviewer-evidence.json",
        "--receipt", "build/demo/receipt.json",
        "--decision", "samples/decisions/sample-review-decision.json",
        "--out", "build/validation.json",
    ]) == 2


def _run_evaluation(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    monkeypatch.chdir(tmp_path)
    assert main([*EVALUATE, "--out", "build/demo"]) == 0
    run = tmp_path / "build" / "demo"
    decision = run / "decision.json"
    sample = Path(__file__).resolve().parents[1] / "elizabeth_anne_alexander" / "samples" / "decisions" / "sample-review-decision.json"
    decision.write_bytes(sample.read_bytes())
    return run


def _validate(run: Path, out: Path) -> int:
    return main([
        "validate-review",
        "--evidence", str(run / "reviewer-evidence.json"),
        "--receipt", str(run / "receipt.json"),
        "--decision", str(run / "decision.json"),
        "--out", str(out),
    ])


@pytest.mark.parametrize("name", ["reviewer-evidence.json", "receipt.json", "decision.json", "model-result.json"])
def test_a_validation_output_that_names_an_input_is_blocked_and_the_input_survives(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], name: str
) -> None:
    """The summary that said the decision was valid used to overwrite the decision."""
    run = _run_evaluation(tmp_path, monkeypatch)
    before = {path.name: path.read_bytes() for path in run.iterdir()}

    assert _validate(run, run / name) == 2

    assert {path.name: path.read_bytes() for path in run.iterdir()} == before
    assert "must not overwrite a review input" in capsys.readouterr().err
    # The untouched pack still validates to a distinct output.
    assert _validate(run, tmp_path / "build" / "validation.json") == 0
    assert (tmp_path / "build" / "validation.json").is_file()


def test_validate_review_writes_its_output_on_the_platform_it_runs_on(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The staged write has to work where it runs; Windows has no directory descriptors."""
    run = _run_evaluation(tmp_path, monkeypatch)
    out = tmp_path / "build" / "validation.json"

    assert _validate(run, out) == 0

    assert json.loads(out.read_text(encoding="utf-8"))["status"]
    # A staging file left behind would mean the replace never happened.
    assert [path.name for path in out.parent.iterdir() if path.name.endswith(".partial")] == []


def test_the_by_path_staged_write_still_refuses_a_destination_that_aliases_an_input(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The fallback the descriptor-less platforms take keeps the same refusal."""
    monkeypatch.setattr(persist, "_supports_dir_fd", lambda: False)
    protected = tmp_path / "receipt.json"
    protected.write_text("{}\n", encoding="utf-8")
    alias = tmp_path / "validation.json"
    try:
        os.link(protected, alias)
    except (OSError, NotImplementedError) as exc:  # pragma: no cover - filesystem without hard links
        pytest.skip(f"hard links unavailable: {exc}")

    with pytest.raises(GatewayError, match="must not overwrite a review input"):
        persist.write_validation({"status": "DECISION_RECORDED"}, alias, (protected,))

    assert protected.read_text(encoding="utf-8") == "{}\n"
    assert [path.name for path in tmp_path.iterdir() if path.name.endswith(".partial")] == []


def test_a_hard_link_to_an_input_is_blocked(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    run = _run_evaluation(tmp_path, monkeypatch)
    alias = tmp_path / "build" / "alias.json"
    try:
        os.link(run / "decision.json", alias)
    except (OSError, NotImplementedError) as exc:  # pragma: no cover - filesystem without hard links
        pytest.skip(f"hard links unavailable: {exc}")
    before = (run / "decision.json").read_bytes()

    assert _validate(run, alias) == 2
    assert (run / "decision.json").read_bytes() == before


def test_a_symbolic_link_to_an_input_is_blocked(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    run = _run_evaluation(tmp_path, monkeypatch)
    alias = tmp_path / "build" / "alias.json"
    try:
        os.symlink(run / "receipt.json", alias)
    except (OSError, NotImplementedError) as exc:  # pragma: no cover - no symlink privilege
        pytest.skip(f"symbolic links unavailable: {exc}")
    before = (run / "receipt.json").read_bytes()

    assert _validate(run, alias) == 2
    assert (run / "receipt.json").read_bytes() == before
