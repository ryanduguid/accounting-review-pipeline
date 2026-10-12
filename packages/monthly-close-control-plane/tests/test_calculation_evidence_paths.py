"""Evidence-file shape gates that make a file unreadable rather than relied on."""
from __future__ import annotations

import json

import pytest
from closecontrol import calculation_evidence as module
from closecontrol.errors import SchemaError

from test_calculation_evidence import build_record, write


@pytest.mark.parametrize("overrides,message", [
    ({"label": 5}, "label must be a string"),
    ({"call.status": "C" * 61}, "call.status is longer than 60 characters"),
    ({"normalised.values": {"levy": "1,000.00"}}, "normalised.values.levy is not a decimal string"),
    ({"normalised.values": {"levy": "1.2.3"}}, "normalised.values.levy is not a decimal"),
])
def test_unreadable_calculation_fields_are_refused(tmp_path, overrides, message):
    with pytest.raises(SchemaError, match=message):
        module.load(write(tmp_path, build_record(**overrides)))


def test_absent_validation_findings_read_as_none(tmp_path):
    evidence = module.load(write(tmp_path, build_record(validation={"accepted": True})))
    assert evidence.findings == ()  # nosec B101: pytest outcome assertion
    assert evidence.usable is True  # nosec B101: pytest outcome assertion


def test_oversized_evidence_file_is_refused(tmp_path, monkeypatch):
    path = write(tmp_path, build_record())
    monkeypatch.setattr(module, "MAX_EVIDENCE_BYTES", 16)
    with pytest.raises(SchemaError, match="evidence file exceeds 16 bytes"):
        module.load(path)


def test_evidence_must_be_a_json_object(tmp_path):
    path = tmp_path / "evidence.json"
    path.write_text(json.dumps([build_record()]), encoding="utf-8")
    with pytest.raises(SchemaError, match="evidence must be a JSON object"):
        module.load(path)


def test_evidence_without_a_calculation_block_is_refused(tmp_path):
    record = build_record()
    record["calculation"] = "computed elsewhere"
    with pytest.raises(SchemaError, match="evidence carries no calculation block"):
        module.load(write(tmp_path, record))
