from decimal import Decimal
from pathlib import Path

from closecontrol.engine import _integrity_exceptions
from closecontrol.loader import load_canonical_tb


CONTRACT = Path(__file__).resolve().parents[3] / "contracts" / "xero-trial-balance-v1"
TENANT = "Catherby Fisheries Pty Ltd"


def _reasons(name: str) -> list[str]:
    rows = load_canonical_tb(CONTRACT / "fixtures" / name)
    return [item.reason for item in _integrity_exceptions(rows, "current")]


def test_root_exporter_contract_preserves_monthly_close_outcomes() -> None:
    assert _reasons("passing.csv") == []
    assert _reasons("failing_movement.csv") == [
        "current movement debit and credit totals do not balance."
    ]
    assert _reasons("failing_ytd.csv") == [
        "current YTD debit and credit totals do not balance."
    ]


def test_root_exporter_contract_rows_keep_tenant_and_stable_account_id_identity() -> None:
    rows = load_canonical_tb(CONTRACT / "fixtures" / "passing.csv")

    assert [row.key for row in rows] == [
        (TENANT, "00000000-0000-0000-0000-000000000001"),
        (TENANT, "00000000-0000-0000-0000-000000000002"),
    ]
    assert [row.account_code for row in rows] == ["090", "800"]
    assert sum((row.current_net for row in rows), Decimal("0")) == Decimal("0")
    assert sum((row.ytd_net for row in rows), Decimal("0")) == Decimal("0")
