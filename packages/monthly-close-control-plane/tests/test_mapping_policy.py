"""A balanced ledger can still carry an incompatible reporting-group mapping."""

import hashlib
import json
from pathlib import Path

import pytest
from closecontrol.cli import main
from closecontrol.engine import review_close
from closecontrol.errors import ControlInputError
from closecontrol.loader import SourceSnapshot, load_mapping_policy
from closecontrol.report import write_review_pack
from closecontrol.viewer import render_review_sheet

HEADER = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit"


def _inputs(tmp_path):
    paths = {}
    for name, when in (("prior", "2026-08-31"), ("current", "2026-09-30")):
        path = tmp_path / f"{name}.csv"
        path.write_text(
            HEADER + "\n" + f"{when},Synthetic,Expenses,E,Expense,400,100,0,100,0\n"
            + f"{when},Synthetic,Liabilities,L,Payable,200,0,100,0,100\n", encoding="utf-8")
        paths[f"{name}_path"] = path
    mapping = tmp_path / "mapping.csv"
    mapping.write_text("AccountID,ReviewGroup\nE,Assets\nL,Payables\n", encoding="utf-8")
    policy = tmp_path / "policy.csv"
    policy.write_text("Section,ReviewGroup\nAssets,Assets\nLiabilities,Payables\n", encoding="utf-8")
    return {**paths, "mapping_path": mapping, "mapping_policy_path": policy}


def test_incompatible_mapping_requires_review_and_keeps_original_evidence(tmp_path):
    inputs = _inputs(tmp_path)
    before = {key: path.read_bytes() for key, path in inputs.items()}
    pack = review_close(**inputs)
    assert pack.status == "REVIEW"
    assert len(pack.exceptions) == 1
    item = pack.exceptions[0]
    assert item.control == "mapping_compatibility"
    assert item.account_id == "E" and item.review_group == "Assets"
    assert "Section 'Expenses'" in item.reason and "ReviewGroup 'Assets'" in item.reason
    assert "Permitted Section values: 'Assets'" in item.reason
    assert pack.client_queries == ()  # The firm's policy is not a question sent to a client.
    assert pack.source_hashes["mapping_policy"] == hashlib.sha256(
        before["mapping_policy_path"]).hexdigest()
    assert {key: path.read_bytes() for key, path in inputs.items()} == before
    output = tmp_path / "pack"
    write_review_pack(pack, output)
    sheet, _ = render_review_sheet(output)
    assert "mapping_compatibility" in sheet and "Expenses" in sheet
    payload = json.loads((output / "close-review-pack.json").read_text(encoding="utf-8"))
    assert payload["source_sha256"]["mapping_policy"] == pack.source_hashes["mapping_policy"]
    assert payload["exceptions"][0]["review_group"] == "Assets"


def test_no_policy_preserves_existing_behaviour(tmp_path):
    inputs = _inputs(tmp_path)
    inputs.pop("mapping_policy_path")
    pack = review_close(**inputs)
    assert pack.status == "PASS" and pack.exceptions == ()
    assert "mapping_policy" not in pack.source_hashes


def test_section_matching_preserves_case(tmp_path):
    inputs = _inputs(tmp_path)
    inputs["mapping_policy_path"].write_text(
        "Section,ReviewGroup\nexpenses,Assets\nLiabilities,Payables\n", encoding="utf-8")
    assert review_close(**inputs).exceptions[0].control == "mapping_compatibility"


def test_cli_malformed_policy_writes_no_pack(tmp_path):
    inputs = _inputs(tmp_path)
    inputs["mapping_policy_path"].write_text("Wrong,Header\n", encoding="utf-8")
    output = tmp_path / "pack"
    assert main(["review", "--current", str(inputs["current_path"]),
                 "--prior", str(inputs["prior_path"]), "--mapping", str(inputs["mapping_path"]),
                 "--mapping-policy", str(inputs["mapping_policy_path"]),
                 "--output", str(output)]) == 1
    assert not output.exists()


def test_multiple_permitted_sections_and_reordered_headers_are_supported(tmp_path):
    inputs = _inputs(tmp_path)
    inputs["mapping_policy_path"].write_text(
        "ReviewGroup,Section\nAssets,Assets\nAssets,Expenses\nPayables,Liabilities\n",
        encoding="utf-8-sig")
    assert review_close(**inputs).status == "PASS"


def test_group_missing_from_policy_is_review_required(tmp_path):
    inputs = _inputs(tmp_path)
    inputs["mapping_policy_path"].write_text(
        "Section,ReviewGroup\nLiabilities,Payables\n", encoding="utf-8")
    assert "no permitted Section" in review_close(**inputs).exceptions[0].reason


def test_unmapped_account_is_not_also_a_compatibility_exception(tmp_path):
    inputs = _inputs(tmp_path)
    inputs["mapping_path"].write_text("AccountID,ReviewGroup\nL,Payables\n", encoding="utf-8")
    assert [item.control for item in review_close(**inputs).exceptions] == ["account_mapping"]


@pytest.mark.parametrize("text", [
    "", "Section,ReviewGroup\n", "Section,Wrong\nAssets,Assets\n",
    "Section,Section,ReviewGroup\nAssets,Assets,Assets\n",
    "Section,ReviewGroup\nAssets,Assets\nAssets,Assets\n",
    "Section,ReviewGroup\n,Assets\n", "Section,ReviewGroup\nAssets,\n",
    "Section,ReviewGroup\nAssets\n", "Section,ReviewGroup\nAssets,Assets,extra\n",
    "Section,ReviewGroup\nAssets,A\u202esets\n",
])
def test_malformed_policy_is_refused(tmp_path, text):
    path = tmp_path / "policy.csv"
    path.write_text(text, encoding="utf-8")
    with pytest.raises(ControlInputError):
        load_mapping_policy(path)


def test_missing_or_non_utf8_policy_is_refused(tmp_path):
    path = tmp_path / "policy.csv"
    with pytest.raises(ControlInputError, match="does not exist"):
        load_mapping_policy(path)
    path.write_bytes(b"\xff")
    with pytest.raises(ControlInputError, match="UTF-8"):
        load_mapping_policy(path)


def test_policy_requires_mapping_before_reading_sources(tmp_path, monkeypatch):
    def unexpected(*args, **kwargs):
        pytest.fail("read a source before validating policy configuration")
    monkeypatch.setattr(SourceSnapshot, "capture", unexpected)
    with pytest.raises(ControlInputError, match="requires"):
        review_close(current_path=tmp_path / "absent", prior_path=tmp_path / "absent",
                     mapping_policy_path=tmp_path / "policy.csv")


def test_policy_is_read_once_and_bound_to_its_original_bytes(tmp_path, monkeypatch):
    inputs = _inputs(tmp_path)
    original = inputs["mapping_policy_path"].read_bytes()
    capture = SourceSnapshot.capture
    reads = []
    def replace_after_read(path, *, label):
        snapshot = capture(path, label=label)
        if path == inputs["mapping_policy_path"]:
            reads.append(path)
            path.write_text("Section,ReviewGroup\nExpenses,Assets\nLiabilities,Payables\n",
                            encoding="utf-8")
        return snapshot
    monkeypatch.setattr(SourceSnapshot, "capture", replace_after_read)
    pack = review_close(**inputs)
    assert pack.status == "REVIEW" and len(reads) == 1
    assert pack.source_hashes["mapping_policy"] == hashlib.sha256(original).hexdigest()


@pytest.mark.parametrize("command", ["review", "workbench"])
def test_cli_returns_review_and_view_verifies_pack(tmp_path, command):
    inputs = _inputs(tmp_path)
    output = tmp_path / "pack"
    assert main([command, "--current", str(inputs["current_path"]),
                 "--prior", str(inputs["prior_path"]), "--mapping", str(inputs["mapping_path"]),
                 "--mapping-policy", str(inputs["mapping_policy_path"]),
                 "--output", str(output)]) == 2
    assert main(["view", "--pack-dir", str(output)]) == 0


def test_cli_refuses_policy_output_collision_without_touching_it(tmp_path):
    policy = tmp_path / "close-summary.md"
    content = "Section,ReviewGroup\nAssets,Assets\n"
    policy.write_text(content, encoding="utf-8")
    assert main(["review", "--current", str(tmp_path / "missing-current.csv"),
                 "--prior", str(tmp_path / "missing-prior.csv"),
                 "--mapping", str(tmp_path / "missing-mapping.csv"),
                 "--mapping-policy", str(policy), "--output", str(tmp_path)]) == 1
    assert policy.read_text(encoding="utf-8") == content


@pytest.mark.parametrize("command", ["review", "workbench"])
@pytest.mark.parametrize("error", [OSError, RuntimeError, ValueError])
def test_cli_handles_policy_path_resolution_failure(tmp_path, monkeypatch, capsys, command, error):
    inputs = _inputs(tmp_path)
    before = {key: path.read_bytes() for key, path in inputs.items()}
    output = tmp_path / "pack"
    resolve = Path.resolve

    def fail_policy_path(path, *args, **kwargs):
        if path == inputs["mapping_policy_path"]:
            raise error("synthetic resolution failure")
        return resolve(path, *args, **kwargs)

    def unexpected_read(*args, **kwargs):
        pytest.fail("Read a source after path resolution failed")

    monkeypatch.setattr(Path, "resolve", fail_policy_path)
    monkeypatch.setattr(SourceSnapshot, "capture", unexpected_read)
    assert main([command, "--current", str(inputs["current_path"]),
                 "--prior", str(inputs["prior_path"]), "--mapping", str(inputs["mapping_path"]),
                 "--mapping-policy", str(inputs["mapping_policy_path"]),
                 "--output", str(output)]) == 1
    message = capsys.readouterr().err
    assert "--mapping-policy" in message and "synthetic resolution failure" in message
    assert "Traceback" not in message
    assert not output.exists()
    assert {key: path.read_bytes() for key, path in inputs.items()} == before
