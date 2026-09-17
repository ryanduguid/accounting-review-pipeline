"""The evidence flags end to end, with the network blocked.

These drive `close-control` through `main()` rather than the library, because
the contract that matters to an operator is the command's: which flags exist,
what the pack ends up holding, and which exit code comes back.
"""

from __future__ import annotations

import json
import socket
from pathlib import Path

import pytest
from closecontrol.cli import main

EXAMPLES = Path(__file__).resolve().parents[1] / "examples"
GOOD = EXAMPLES / "calculation-evidence-coal-lsl-levy.json"
REFUSED = EXAMPLES / "calculation-evidence-refused.json"


@pytest.fixture(autouse=True)
def no_network(monkeypatch):
    def refuse(*args, **kwargs):  # noqa: ANN002, ANN003
        raise AssertionError("close-control opened a socket")

    monkeypatch.setattr(socket, "create_connection", refuse)
    monkeypatch.setattr(socket.socket, "connect", refuse)


def review(tmp_path, *extra, name="pack"):
    output = tmp_path / name
    code = main([
        "review",
        "--current", str(EXAMPLES / "current_trial_balance.csv"),
        "--prior", str(EXAMPLES / "prior_trial_balance.csv"),
        "--output", str(output),
        *extra,
    ])
    pack = None
    path = output / "close-review-pack.json"
    if path.is_file():
        pack = json.loads(path.read_text(encoding="utf-8"))
    return code, pack


def test_the_committed_example_evidence_files_are_valid_and_fabricated():
    for path in (GOOD, REFUSED):
        record = json.loads(path.read_text(encoding="utf-8"))
        assert record["calculation"]["synthetic_input"] is True, path.name
    good = json.loads(GOOD.read_text(encoding="utf-8"))
    assert good["calculation"]["call"]["status"] == "COMPUTED"
    refused = json.loads(REFUSED.read_text(encoding="utf-8"))
    assert refused["calculation"]["call"]["status"] == "UPSTREAM_REFUSED"
    assert refused["calculation"]["normalised"]["values"] == {}


def test_without_the_flags_the_pack_has_no_evidence_block(tmp_path):
    code, pack = review(tmp_path)
    assert code == 2  # the fabricated example is REVIEW, as it always was
    assert "calculation_evidence" not in pack


def test_supplying_evidence_records_it_and_keeps_the_exit_contract(tmp_path):
    code, pack = review(
        tmp_path, "--calculation-evidence", str(GOOD), "--require-calculation", "coal-lsl-levy",
    )
    assert code == 2
    block = pack["calculation_evidence"]
    assert block["required"] == ["coal-lsl-levy"]
    assert block["supplied"][0]["usable"] is True
    assert block["supplied"][0]["values"]["levy"] == "192.38"
    assert "calculation_evidence:coal-lsl-levy" in pack["source_sha256"]
    assert not [item for item in pack["exceptions"] if item["control"] == "calculation_evidence"]


def test_a_required_calculation_with_no_evidence_raises_an_exception(tmp_path):
    code, pack = review(tmp_path, "--require-calculation", "coal-lsl-levy")
    assert code == 2
    reasons = [item["reason"] for item in pack["exceptions"]
               if item["control"] == "calculation_evidence"]
    assert any("configured as required" in reason for reason in reasons)


def test_a_recorded_refusal_never_becomes_a_figure(tmp_path):
    code, pack = review(
        tmp_path, "--calculation-evidence", str(REFUSED),
        "--require-calculation", "coal-lsl-levy-unresolved",
    )
    assert code == 2
    supplied = pack["calculation_evidence"]["supplied"][0]
    assert supplied["status"] == "UPSTREAM_REFUSED"
    assert supplied["values"] == {}
    assert supplied["usable"] is False


def test_a_missing_evidence_file_is_an_input_error_not_a_pass(tmp_path, capsys):
    code, pack = review(tmp_path, "--calculation-evidence", str(tmp_path / "nowhere.json"))
    assert code == 2
    assert pack["overall_status"] == "BLOCKED"
    assert any(item["control"] == "calculation_evidence" and item["status"] == "BLOCKED"
               for item in pack["exceptions"])


def test_evidence_cannot_be_written_over_by_its_own_run(tmp_path, capsys):
    """The existing source-and-destination guard covers the new flag too."""
    output = tmp_path / "pack"
    output.mkdir()
    collision = output / "close-review-pack.json"
    collision.write_text("{}", encoding="utf-8")
    code = main([
        "review",
        "--current", str(EXAMPLES / "current_trial_balance.csv"),
        "--prior", str(EXAMPLES / "prior_trial_balance.csv"),
        "--calculation-evidence", str(collision),
        "--output", str(output),
    ])
    assert code == 1
    assert "would destroy it" in capsys.readouterr().err
