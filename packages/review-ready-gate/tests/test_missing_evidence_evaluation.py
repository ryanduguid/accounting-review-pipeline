from __future__ import annotations

import json
import re
from pathlib import Path

import pytest
from reviewready.cli import main
from reviewready.engine import review_pack
from reviewready.report import write_review_pack
from tests.support import ROOT

PACK = ROOT / "evaluation" / "missing_evidence"
CONTRACT = json.loads((PACK / "expected_results.json").read_text(encoding="utf-8"))
SCENARIOS = CONTRACT["scenarios"]


def _files(directory: Path) -> dict[str, bytes]:
    return {path.name: path.read_bytes() for path in directory.iterdir() if path.is_file()}


def test_contract_names_its_release_and_pending_review() -> None:
    pyproject = (ROOT / "pyproject.toml").read_text(encoding="utf-8")
    version = re.search(r'(?m)^version = "([^"]+)"$', pyproject).group(1)
    assert CONTRACT["schema_version"] == 1
    assert CONTRACT["product_release"] == version
    assert CONTRACT["practitioner_review"] == "pending"
    assert f"Product release `{version}`" in (PACK / "README.md").read_text(encoding="utf-8")


@pytest.mark.parametrize("scenario", SCENARIOS, ids=[item["id"] for item in SCENARIOS])
def test_scenario_reproduces_the_declared_result(scenario: dict) -> None:
    result = review_pack(profile=scenario["profile"], pack_dir=ROOT / scenario["fixture"])
    assert result.status == scenario["expected_status"]
    assert [item.code for item in result.findings] == scenario["expected_findings"]
    assert [item.slot for item in result.controls_not_run] == scenario[
        "expected_controls_not_run"
    ]


@pytest.mark.parametrize("scenario", SCENARIOS, ids=[item["id"] for item in SCENARIOS])
def test_fixture_differs_from_the_ready_example_only_as_declared(scenario: dict) -> None:
    # A pack that drifted in a second file would still pass the result test, but
    # it would no longer isolate the one missing piece of evidence it names.
    base = _files(ROOT / CONTRACT["base_fixture"])
    pack = _files(ROOT / scenario["fixture"])
    change = scenario["change"]
    removed = set(change.get("removed", []))
    emptied = set(change.get("emptied", []))
    edited = set(change.get("edited", []))
    assert set(base) - set(pack) == removed
    assert set(pack) - set(base) == set()
    for name, content in pack.items():
        if name in emptied:
            assert content == b""
        elif name in edited:
            assert content != base[name]
        else:
            assert content == base[name], name


def test_every_pack_on_disk_is_declared() -> None:
    declared = {Path(item["fixture"]).name for item in SCENARIOS}
    assert {path.name for path in (PACK / "packs").iterdir()} == declared


def test_view_refuses_a_summary_edited_after_the_gate(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    case = CONTRACT["edited_artefact"]
    pack = review_pack(profile=case["profile"], pack_dir=ROOT / case["fixture"])
    output = tmp_path / "pack"
    write_review_pack(pack, output)
    assert main(["view", "--pack-dir", str(output)]) == 0
    capsys.readouterr()

    target = output / case["edit"]["file"]
    content = target.read_bytes()
    before, after = case["edit"]["from"].encode(), case["edit"]["to"].encode()
    assert content.count(before) == 1
    target.write_bytes(content.replace(before, after))

    assert main(["view", "--pack-dir", str(output)]) == case["expected_view_exit"]
    assert case["expected_error"] in capsys.readouterr().err


def test_readme_table_matches_the_contract() -> None:
    readme = (PACK / "README.md").read_text(encoding="utf-8")
    assert CONTRACT["human_decision"] in readme
    for scenario in SCENARIOS:
        row = next(line for line in readme.splitlines() if line.startswith(f"| `{scenario['id']}` |"))
        cells = [cell.strip() for cell in row.strip("|").split("|")]
        assert cells[2] == f"`{scenario['expected_status']}`"
        findings = scenario["expected_findings"]
        assert cells[3] == (", ".join(f"`{code}`" for code in findings) if findings else "none")
        assert cells[4] == ", ".join(f"`{slot}`" for slot in scenario["expected_controls_not_run"])
