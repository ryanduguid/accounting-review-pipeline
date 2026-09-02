from __future__ import annotations

import os
from pathlib import Path

import pytest

from closecontrol.engine import review_close
from closecontrol.loader import CANONICAL_COLUMNS


ROOT = Path(__file__).resolve().parents[1]
TEN_COLUMN_CONTRACT = (
    "ReportDate",
    "Tenant",
    "Section",
    "AccountID",
    "AccountName",
    "AccountCode",
    "Debit",
    "Credit",
    "YTDDebit",
    "YTDCredit",
)
GATEWAY_SAMPLE_CURRENT = "sample-tb-2026-06-30.csv"
GATEWAY_SAMPLE_PRIOR = "sample-tb-2026-05-31.csv"


def _gateway_sample_inputs() -> Path | None:
    """Return the sibling (or env-configured) gateway sample inputs directory."""
    configured = os.environ.get("ELIZABETH_ANNE_ALEXANDER_ROOT")
    candidates = []
    if configured:
        candidates.append(Path(configured))
    candidates.append(ROOT.parent / "xero-ledger-review-gate")
    for root in candidates:
        inputs = root / "elizabeth_anne_alexander" / "samples" / "inputs"
        if (inputs / GATEWAY_SAMPLE_CURRENT).is_file() and (inputs / GATEWAY_SAMPLE_PRIOR).is_file():
            return inputs
    return None


def test_canonical_columns_match_the_ten_column_contract() -> None:
    schema_header = (ROOT / "schemas" / "canonical_trial_balance.csv").read_text(
        encoding="utf-8-sig"
    ).splitlines()[0]

    assert CANONICAL_COLUMNS == TEN_COLUMN_CONTRACT
    assert ",".join(CANONICAL_COLUMNS) == schema_header


def test_gateway_same_fy_samples_review_without_year_reset() -> None:
    # Close-control's own Varrock June/July fixtures straddle 1 July and cannot
    # feed the gateway. This pin uses the gateway's May/June Demo Entity pair.
    inputs = _gateway_sample_inputs()
    if inputs is None:
        pytest.skip("sibling xero-ledger-review-gate checkout is missing")

    current_path = inputs / GATEWAY_SAMPLE_CURRENT
    prior_path = inputs / GATEWAY_SAMPLE_PRIOR
    for path in (current_path, prior_path):
        header = path.read_text(encoding="utf-8-sig").splitlines()[0]
        assert header == ",".join(CANONICAL_COLUMNS)

    pack = review_close(current_path=current_path, prior_path=prior_path)

    assert pack.status == "REVIEW"
    assert pack.current_report_dates == ("2026-06-30",)
    assert pack.prior_report_dates == ("2026-05-31",)
    assert not [item for item in pack.exceptions if item.control == "financial_year_reset"]
    assert {"acct-100", "acct-300", "acct-400", "acct-900"} <= {
        item.account_id for item in pack.exceptions
    }
