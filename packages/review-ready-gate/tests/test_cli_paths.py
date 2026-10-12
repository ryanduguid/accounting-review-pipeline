"""Command-line outcomes the end-to-end gate tests do not reach."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from reviewready.cli import main
from reviewready.report import write_review_pack
from tests.support import EXAMPLES
from tests.test_gate import _ready_pack


def _gate(output: Path, *extra: str, pack: Path = EXAMPLES / "bas-ready") -> list[str]:
    return ["gate", "--profile", "bas", "--pack", str(pack), "--output", str(output), *extra]


@pytest.mark.parametrize(
    "value,message",
    [
        ("abc", "not a decimal: 'abc'"),
        ("-0.01", "must be a finite non-negative decimal"),
        ("NaN", "must be a finite non-negative decimal"),
    ],
)
def test_invalid_tolerance_is_a_usage_error(
    tmp_path: Path, capsys: pytest.CaptureFixture[str], value: str, message: str
) -> None:
    output = tmp_path / "out"
    assert main(_gate(output, f"--tieout-tolerance={value}")) == 1  # nosec B101: pytest outcome assertion
    assert message in capsys.readouterr().err  # nosec B101: pytest outcome assertion
    assert not output.exists()  # nosec B101: pytest outcome assertion


def test_supplied_tolerance_is_recorded(tmp_path: Path) -> None:
    output = tmp_path / "out"
    assert main(_gate(output, "--tieout-tolerance", "0.05")) == 0  # nosec B101: pytest outcome assertion
    document = json.loads((output / "readiness-pack.json").read_text(encoding="utf-8"))
    assert document["thresholds"] == {"tieout_tolerance": "0.05"}  # nosec B101: pytest outcome assertion


def test_help_exit_is_not_turned_into_a_failure(capsys: pytest.CaptureFixture[str]) -> None:
    with pytest.raises(SystemExit) as exited:
        main(["--help"])
    assert exited.value.code == 0  # nosec B101: pytest outcome assertion
    assert "manager review" in capsys.readouterr().out  # nosec B101: pytest outcome assertion


def test_pack_directory_holding_generated_files_is_refused(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    output = tmp_path / "out"
    write_review_pack(_ready_pack(), output)
    before = {path.name: path.read_bytes() for path in output.iterdir() if path.is_file()}
    assert main(_gate(output, pack=output)) == 1  # nosec B101: pytest outcome assertion
    assert "--pack contains " in capsys.readouterr().err  # nosec B101: pytest outcome assertion
    after = {path.name: path.read_bytes() for path in output.iterdir() if path.is_file()}
    assert after == before  # nosec B101: pytest outcome assertion


def test_output_that_cannot_be_published_is_an_output_error(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    output = tmp_path / "out"
    (output / "readiness-pack.json").mkdir(parents=True)
    assert main(_gate(output)) == 1  # nosec B101: pytest outcome assertion
    err = capsys.readouterr().err
    assert err.startswith("review-ready: output error: ")  # nosec B101: pytest outcome assertion
    assert "readiness-pack.json" in err  # nosec B101: pytest outcome assertion
    assert (output / "readiness-pack.json").is_dir()  # nosec B101: pytest outcome assertion
