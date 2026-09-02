from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path

import pytest

from elizabeth_anne_alexander.errors import GatewayError
from elizabeth_anne_alexander.gateway import CANONICAL_COLUMNS, _load_tb

CORPUS = Path(__file__).resolve().parents[3] / "contracts" / "xero-trial-balance-v1"
CONTRACT = CORPUS / "expected_results.json"


def _json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def test_root_corpus_preserves_the_exporter_owned_bytes() -> None:
    expected_files = {
        "expected_results.json": "1568687d5ccc809d0267d024b34869061cf10d436a28501e57888314238add91",
        "fixtures/passing.csv": "2cbe9997a8e7210936ff3c59b5d3fdb0041c1b375b0f9c88cf9ee30d0f356a09",
        "fixtures/failing_movement.csv": "702175df967b2854e7897cd27fdc4aca441e21b52438381108fabe88ff3153e4",
        "fixtures/failing_ytd.csv": "ec757f12d13866360fbab189228ebb425893c6f8b299809c6f8567bf5817c64b",
    }

    for relative, expected_digest in expected_files.items():
        content = (CORPUS / relative).read_bytes()
        assert b"\r\n" not in content
        assert hashlib.sha256(content).hexdigest() == expected_digest


def test_gateway_satisfies_the_pinned_exporter_contract() -> None:
    contract = _json(CONTRACT)
    assert contract["schema_version"] == 2
    assert contract["corpus_id"] == "xero-tb-csv.v1"
    assert contract["owner_repository"] == "https://github.com/ryanduguid/xero-trial-balance-export"
    assert tuple(contract["canonical_columns"]) == CANONICAL_COLUMNS

    for scenario in contract["scenarios"]:
        fixture = CORPUS / "fixtures" / scenario["fixture"]
        assert hashlib.sha256(fixture.read_bytes()).hexdigest() == scenario["sha256"]
        expectation = scenario["conformance"]
        if expectation["accept"]:
            assert _load_tb(fixture)
        else:
            with pytest.raises(GatewayError, match=re.escape(expectation["error_contains"])):
                _load_tb(fixture)
