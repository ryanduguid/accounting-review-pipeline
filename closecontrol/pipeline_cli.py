"""
OpenAccountants Australian Monthly Close & ATO Benchmark Review CLI.
"""

from __future__ import annotations

import argparse
import csv
import sys
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation, ROUND_HALF_UP
from pathlib import Path
from typing import List, Dict, Any


@dataclass
class AccountRow:
    report_date: str
    tenant: str
    section: str
    account_id: str
    account_name: str
    account_code: str
    debit: Decimal
    credit: Decimal
    ytd_debit: Decimal
    ytd_credit: Decimal

    @property
    def net_movement(self) -> Decimal:
        return self.debit - self.credit

    @property
    def net_ytd(self) -> Decimal:
        return self.ytd_debit - self.ytd_credit



def _money(raw: object, field: str) -> Decimal:
    try:
        value = Decimal(str(raw if raw not in (None, "") else "0"))
    except InvalidOperation as exc:
        raise ValueError(f"{field} is not a decimal amount: {raw!r}") from exc
    if not value.is_finite():
        raise ValueError(f"{field} is not a finite decimal amount: {raw!r}")
    return value


def parse_trial_balance(csv_path: Path) -> List[AccountRow]:
    rows: List[AccountRow] = []
    with open(csv_path, "r", encoding="utf-8-sig") as f:
        reader = csv.DictReader(f)
        for r in reader:
            report_date = r.get("ReportDate", "")
            tenant = r.get("Tenant", "Entity")
            section = r.get("Section", "")
            account_id = r.get("AccountID", "")
            account_name = r.get("AccountName", r.get("Account", ""))
            account_code = r.get("AccountCode", r.get("Code", ""))
            
            debit = _money(r.get("Debit", "0"), "Debit")
            credit = _money(r.get("Credit", "0"), "Credit")
            ytd_debit = _money(r.get("YTDDebit", debit), "YTDDebit")
            ytd_credit = _money(r.get("YTDCredit", credit), "YTDCredit")
            
            rows.append(AccountRow(
                report_date=report_date,
                tenant=tenant,
                section=section,
                account_id=account_id,
                account_name=account_name,
                account_code=account_code,
                debit=debit,
                credit=credit,
                ytd_debit=ytd_debit,
                ytd_credit=ytd_credit
            ))
    return rows


def run_integrity_checks(rows: List[AccountRow]) -> Dict[str, Any]:
    total_debit = sum(r.debit for r in rows)
    total_credit = sum(r.credit for r in rows)
    movement_diff = total_debit - total_credit

    total_ytd_debit = sum(r.ytd_debit for r in rows)
    total_ytd_credit = sum(r.ytd_credit for r in rows)
    ytd_diff = total_ytd_debit - total_ytd_credit

    is_movement_balanced = abs(movement_diff) < Decimal("0.01")
    is_ytd_balanced = abs(ytd_diff) < Decimal("0.01")

    exceptions: List[str] = []
    for r in rows:
        sec = r.section.lower()
        if "asset" in sec or "bank" in sec:
            if r.net_ytd < Decimal("0"):
                exceptions.append(f"Abnormal credit balance in Asset/Bank account: `{r.account_code} - {r.account_name}` (${r.net_ytd:,.2f})")
        elif "revenue" in sec or "income" in sec or "sales" in sec:
            if r.ytd_debit > r.ytd_credit:
                exceptions.append(f"Debit balance in Revenue account: `{r.account_code} - {r.account_name}` (${r.ytd_debit - r.ytd_credit:,.2f})")

    return {
        "total_debit": total_debit,
        "total_credit": total_credit,
        "movement_diff": movement_diff,
        "total_ytd_debit": total_ytd_debit,
        "total_ytd_credit": total_ytd_credit,
        "ytd_diff": ytd_diff,
        "is_balanced": is_movement_balanced and is_ytd_balanced,
        "exceptions": exceptions
    }


def calculate_ato_benchmarks(rows: List[AccountRow]) -> Dict[str, Any]:
    turnover = Decimal("0")
    cost_of_sales = Decimal("0")
    labour_costs = Decimal("0")
    rent_costs = Decimal("0")

    for r in rows:
        name = r.account_name.lower()
        sec = r.section.lower()
        
        if "revenue" in sec or "income" in sec or "sales" in sec:
            turnover += (r.ytd_credit - r.ytd_debit)
        elif "cost of" in sec or "cogs" in sec or "direct cost" in sec or "cost of sales" in name or "cogs" in name:
            cost_of_sales += (r.ytd_debit - r.ytd_credit)
        elif any(k in name for k in ["wage", "salary", "salaries", "superannuation", "payroll", "subcontractor"]):
            labour_costs += (r.ytd_debit - r.ytd_credit)
        elif "rent" in name or "lease" in name or "occupancy" in name:
            rent_costs += (r.ytd_debit - r.ytd_credit)

    cos_pct = (cost_of_sales / turnover * Decimal("100")) if turnover > 0 else Decimal("0")
    labour_pct = (labour_costs / turnover * Decimal("100")) if turnover > 0 else Decimal("0")
    rent_pct = (rent_costs / turnover * Decimal("100")) if turnover > 0 else Decimal("0")
    gross_profit = turnover - cost_of_sales
    gp_pct = (gross_profit / turnover * Decimal("100")) if turnover > 0 else Decimal("0")

    return {
        "turnover": turnover,
        "cost_of_sales": cost_of_sales,
        "gross_profit": gross_profit,
        "gross_profit_margin_pct": gp_pct.quantize(Decimal("0.1"), rounding=ROUND_HALF_UP),
        "cos_pct": cos_pct.quantize(Decimal("0.1"), rounding=ROUND_HALF_UP),
        "labour_costs": labour_costs,
        "labour_pct": labour_pct.quantize(Decimal("0.1"), rounding=ROUND_HALF_UP),
        "rent_costs": rent_costs,
        "rent_pct": rent_pct.quantize(Decimal("0.1"), rounding=ROUND_HALF_UP),
    }


def generate_markdown_report(integrity: Dict[str, Any], benchmarks: Dict[str, Any], rows: List[AccountRow]) -> str:
    tenant = rows[0].tenant if rows else "Australian Business"
    date_str = rows[0].report_date if rows else "2026-06-30"

    status_badge = "✅ BALANCED & VERIFIED" if integrity["is_balanced"] else "❌ BALANCE EXCEPTION"

    md = f"""# Monthly Close & ATO Benchmark Review Packet

**Entity:** {tenant}  
**Report Date:** {date_str}  
**Integrity Status:** {status_badge}  
**Pipeline Source:** `openaccountants-au review`

---

## 1. Deterministic Control Plane Integrity Check

| Metric | Current Movement | Year to Date (YTD) | Status |
| :--- | :---: | :---: | :---: |
| **Total Debits** | `${integrity['total_debit']:,.2f}` | `${integrity['total_ytd_debit']:,.2f}` | Recorded |
| **Total Credits** | `${integrity['total_credit']:,.2f}` | `${integrity['total_ytd_credit']:,.2f}` | Recorded |
| **Net Out-of-Balance** | `${integrity['movement_diff']:,.2f}` | `${integrity['ytd_diff']:,.2f}` | {'✅ Balanced' if integrity['is_balanced'] else '❌ Out of Balance'} |

### Exception Log
"""
    if integrity["exceptions"]:
        for ex in integrity["exceptions"]:
            md += f"- ⚠️ {ex}\n"
    else:
        md += "- ✅ Zero abnormal balance exceptions detected.\n"

    md += f"""
---

## 2. ATO Small Business Benchmark Analysis

Financial ratios calculated against Year-To-Date turnover of **${benchmarks['turnover']:,.2f}**:

| Operating Metric | Actual Amount | % of Turnover | ATO Benchmark Typical Range | Variance / Health |
| :--- | :---: | :---: | :---: | :---: |
| **Gross Profit** | `${benchmarks['gross_profit']:,.2f}` | **{benchmarks['gross_profit_margin_pct']}%** | Use ato-benchmark-compare | Not evaluated |
| **Cost of Sales** | `${benchmarks['cost_of_sales']:,.2f}` | **{benchmarks['cos_pct']}%** | Use ato-benchmark-compare | Not evaluated |
| **Labour & Wages** | `${benchmarks['labour_costs']:,.2f}` | **{benchmarks['labour_pct']}%** | Use ato-benchmark-compare | Not evaluated |
| **Rent & Occupancy**| `${benchmarks['rent_costs']:,.2f}` | **{benchmarks['rent_pct']}%** | Use ato-benchmark-compare | Not evaluated |

---

## 3. Human Review & Sign-Off Gate

> [!IMPORTANT]
> **Statutory Disclaimer (TASA 2009 / DrDebits Protocol):**  
> This packet was generated using deterministic calculation rules and is presented for professional accounting review. No automated tax lodgement or legal advice is implied.

- [ ] Month-end bank reconciliations confirmed
- [ ] ATO benchmarks reviewed against industry code
- [ ] Signed off by Registered Tax Agent / Authorised Signatory

**Signatory:** ___________________________ &nbsp;&nbsp;&nbsp;&nbsp; **Date:** ______________
"""
    return md


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="openaccountants-au",
        description="Quarantined. Use close-control and ato-benchmark-compare.",
    )
    parser.parse_args()
    print(
        "openaccountants-au is quarantined. It printed canned ATO range "
        "verdicts and is not a benchmark engine. Use close-control for the "
        "close pack and ato-benchmark-compare for range tests.",
        file=sys.stderr,
    )
    return 2


def _legacy_review_main() -> int:
    parser = argparse.ArgumentParser(prog="openaccountants-au", description="OpenAccountants Australian Review Tool")
    subparsers = parser.add_subparsers(dest="command", help="Command to execute")
    
    review_parser = subparsers.add_parser("review", help="Execute trial balance integrity and ATO benchmark review")
    review_parser.add_argument("--tb", type=str, required=True, help="Path to trial balance CSV")
    review_parser.add_argument("-o", "--output", type=str, default="monthly_review_packet.md", help="Output Markdown report path")
    
    args = parser.parse_args()
    
    if args.command != "review":
        parser.print_help()
        return 1
        
    tb_path = Path(args.tb)
    if not tb_path.exists():
        print(f"Error: File '{args.tb}' not found.")
        sys.exit(1)
        
    rows = parse_trial_balance(tb_path)
    integrity = run_integrity_checks(rows)
    benchmarks = calculate_ato_benchmarks(rows)
    report_md = generate_markdown_report(integrity, benchmarks, rows)
    
    out_path = Path(args.output)
    out_path.write_text(report_md, encoding="utf-8")
    print(f"Review packet generated successfully -> {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
