"""Render the README proof from the fabricated sample trial balance."""

from __future__ import annotations

import argparse
import csv
import html
import sys
from decimal import Decimal
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# Run as a script, sys.path[0] is tools/, so the exporter has to be put on
# the path before the module that owns the header can be imported.
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from export_tb import CANONICAL_COLUMNS  # noqa: E402  needs the sys.path line above

SAMPLE = ROOT / "samples" / "sample-output.csv"
SVG = ROOT / "assets" / "quick-proof.svg"
TRANSCRIPT = ROOT / "assets" / "quick-proof.md"


def _money(value: Decimal) -> str:
    return f"${value:,.2f}"


def _summary() -> dict[str, str | int]:
    with SAMPLE.open(encoding="utf-8-sig", newline="") as handle:
        reader = csv.DictReader(handle)
        columns = tuple(reader.fieldnames or ())
        rows = list(reader)
    if not rows:
        raise ValueError("the fabricated sample has no account rows")
    if columns != CANONICAL_COLUMNS:
        raise ValueError("the fabricated sample has unexpected columns")

    tenants = {row["Tenant"] for row in rows}
    if len(tenants) != 1:
        raise ValueError("the fabricated sample has multiple tenants")
    report_dates = {row["ReportDate"] for row in rows}
    if len(report_dates) != 1:
        raise ValueError("the fabricated sample has multiple report dates")

    totals = {
        column: sum((Decimal(row[column]) for row in rows), Decimal())
        for column in ("Debit", "Credit", "YTDDebit", "YTDCredit")
    }
    if totals["Debit"] != totals["Credit"]:
        raise ValueError("the fabricated movement columns do not balance")
    if totals["YTDDebit"] != totals["YTDCredit"]:
        raise ValueError("the fabricated YTD columns do not balance")
    return {
        "tenant": tenants.pop(),
        "date": report_dates.pop(),
        "rows": len(rows),
        "columns": len(columns),
        "movement": _money(totals["Debit"]),
        "ytd": _money(totals["YTDDebit"]),
    }


def render_transcript(summary: dict[str, str | int]) -> str:
    return f"""# Validated fabricated trial balance

Source: [`samples/sample-output.csv`](../samples/sample-output.csv)

- Tenant: {summary['tenant']}
- Report date: {summary['date']}
- Shape: {summary['rows']} account rows, {summary['columns']} columns
- Movement: debit {summary['movement']} | credit {summary['movement']} | balanced
- YTD: debit {summary['ytd']} | credit {summary['ytd']} | balanced

Reproduce the card and verify that both committed assets are current:

```bash
python tools/render_quick_proof.py --check
```

The source is fabricated. No Xero credentials or organisation data are used.
"""


def render_svg(summary: dict[str, str | int]) -> str:
    tenant = html.escape(str(summary["tenant"]))
    date = html.escape(str(summary["date"]))
    movement = html.escape(str(summary["movement"]))
    ytd = html.escape(str(summary["ytd"]))
    rows = summary["rows"]
    columns = summary["columns"]
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="420" viewBox="0 0 1000 420" role="img" aria-labelledby="title desc">
  <title id="title">Validated fabricated Xero trial balance</title>
  <desc id="desc">A fabricated {rows}-row trial balance with balanced movement and year-to-date totals.</desc>
  <rect width="1000" height="420" rx="20" fill="#000000"/>
  <rect x="34" y="34" width="932" height="352" rx="14" fill="#0A0A0A" stroke="#E5E4E2"/>
  <text x="68" y="84" fill="#FFFFF0" font-family="ui-monospace, SFMono-Regular, Consolas, monospace" font-size="28" font-weight="700">xero-trial-balance-export</text>
  <text x="68" y="116" fill="#C8C8C8" font-family="ui-monospace, SFMono-Regular, Consolas, monospace" font-size="15">FABRICATED OFFLINE PROOF  |  {date}</text>
  <line x1="68" y1="142" x2="932" y2="142" stroke="#606060"/>
  <text x="68" y="184" fill="#E5E4E2" font-family="ui-monospace, SFMono-Regular, Consolas, monospace" font-size="18">tenant</text>
  <text x="260" y="184" fill="#FFFFF0" font-family="ui-monospace, SFMono-Regular, Consolas, monospace" font-size="18">{tenant}</text>
  <text x="68" y="224" fill="#E5E4E2" font-family="ui-monospace, SFMono-Regular, Consolas, monospace" font-size="18">shape</text>
  <text x="260" y="224" fill="#FFFFF0" font-family="ui-monospace, SFMono-Regular, Consolas, monospace" font-size="18">{rows} account rows  |  {columns} columns</text>
  <text x="68" y="264" fill="#E5E4E2" font-family="ui-monospace, SFMono-Regular, Consolas, monospace" font-size="18">movement</text>
  <text x="260" y="264" fill="#FFFFF0" font-family="ui-monospace, SFMono-Regular, Consolas, monospace" font-size="18">DR {movement}  |  CR {movement}</text>
  <text x="68" y="304" fill="#E5E4E2" font-family="ui-monospace, SFMono-Regular, Consolas, monospace" font-size="18">year to date</text>
  <text x="260" y="304" fill="#FFFFF0" font-family="ui-monospace, SFMono-Regular, Consolas, monospace" font-size="18">DR {ytd}  |  CR {ytd}</text>
  <rect x="742" y="238" width="168" height="58" rx="29" fill="#66023C" stroke="#E5E4E2"/>
  <text x="826" y="274" text-anchor="middle" fill="#FFFFF0" font-family="ui-monospace, SFMono-Regular, Consolas, monospace" font-size="18" font-weight="700">BALANCED</text>
  <line x1="68" y1="330" x2="932" y2="330" stroke="#606060"/>
  <text x="68" y="360" fill="#C8C8C8" font-family="ui-monospace, SFMono-Regular, Consolas, monospace" font-size="15">python tools/render_quick_proof.py --check</text>
</svg>
"""


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)
    summary = _summary()
    expected = {
        SVG: render_svg(summary),
        TRANSCRIPT: render_transcript(summary),
    }

    if args.check:
        stale = [path for path, text in expected.items() if not path.is_file() or path.read_text(encoding="utf-8") != text]
        if stale:
            for path in stale:
                print(f"stale: {path.relative_to(ROOT)}")
            return 1
        print("quick proof is current")
        return 0

    for path, text in expected.items():
        path.write_text(text, encoding="utf-8", newline="\n")
        print(f"wrote {path.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
