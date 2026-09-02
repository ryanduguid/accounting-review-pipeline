from decimal import Decimal
from pathlib import Path

from reviewready.engine import _tb_balanced
from reviewready.loader import SourceSnapshot, load_canonical_tb


CONTRACT = Path(__file__).resolve().parents[3] / "contracts" / "xero-trial-balance-v1"
TENANT = "Catherby Fisheries Pty Ltd"


def _rows(name: str):
    fixture = CONTRACT / "fixtures" / name
    return load_canonical_tb(SourceSnapshot.capture(fixture, label="Trial-balance file"))


def test_root_exporter_contract_preserves_readiness_balance_outcomes() -> None:
    assert _tb_balanced(_rows("passing.csv")) == (Decimal("0.0"), Decimal("0.0"))
    assert _tb_balanced(_rows("failing_movement.csv")) == (Decimal("0.01"), Decimal("0.0"))
    assert _tb_balanced(_rows("failing_ytd.csv")) == (Decimal("0.0"), Decimal("0.01"))


def test_root_exporter_contract_rows_keep_tenant_and_stable_account_id_identity() -> None:
    rows = _rows("passing.csv")

    assert [(row.tenant, row.account_id) for row in rows] == [
        (TENANT, "00000000-0000-0000-0000-000000000001"),
        (TENANT, "00000000-0000-0000-0000-000000000002"),
    ]
    assert [row.account_code for row in rows] == ["090", "800"]
