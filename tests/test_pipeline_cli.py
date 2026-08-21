from decimal import Decimal
from pathlib import Path
from closecontrol.pipeline_cli import parse_trial_balance, run_integrity_checks, calculate_ato_benchmarks, generate_markdown_report

SAMPLE_CSV = Path(__file__).resolve().parents[1] / "examples" / "current_trial_balance.csv"

def test_pipeline_parsing():
    rows = parse_trial_balance(SAMPLE_CSV)
    assert len(rows) == 7
    assert any(r.account_code == "2000" for r in rows)

def test_pipeline_integrity():
    rows = parse_trial_balance(SAMPLE_CSV)
    integrity = run_integrity_checks(rows)
    assert integrity["is_balanced"] is True
    assert integrity["movement_diff"] == Decimal("0")
    assert integrity["ytd_diff"] == Decimal("0")

def test_pipeline_benchmarks():
    rows = parse_trial_balance(SAMPLE_CSV)
    benchmarks = calculate_ato_benchmarks(rows)
    assert benchmarks["turnover"] == Decimal("130000.00")
    assert benchmarks["cost_of_sales"] == Decimal("80000.00")
    assert benchmarks["gross_profit"] == Decimal("50000.00")

def test_pipeline_report_generation():
    rows = parse_trial_balance(SAMPLE_CSV)
    integrity = run_integrity_checks(rows)
    benchmarks = calculate_ato_benchmarks(rows)
    report = generate_markdown_report(integrity, benchmarks, rows)
    assert "# Monthly Close & ATO Benchmark Review Packet" in report
    assert "Acme Demo Pty Ltd" in report
    assert "BALANCED & VERIFIED" in report
