"""Render a clearing reconciliation without replacing a previous run."""
from __future__ import annotations

import csv
import html
import io
import json
from datetime import date
from decimal import Context, Decimal, localcontext
from pathlib import Path

from .reconciliation import COLUMNS, DECISION_COLUMNS
from .report import _csv_safe, require_output_outside_repository


def _csv(columns: tuple[str, ...], rows: list[list[str]]) -> str:
    stream = io.StringIO(newline="")
    writer = csv.writer(stream, lineterminator="\n")
    writer.writerow(columns)
    writer.writerows(rows)
    return stream.getvalue()


def render_html(pack: dict) -> str:
    def esc(value: object) -> str:
        return html.escape(str(value), quote=True)

    outstanding = []
    end = date.fromisoformat(pack["period_end"])
    with localcontext(Context(prec=40)):
        for item in pack["outstanding"]:
            amount = Decimal(item["Debit"]) - Decimal(item["Credit"])
            age = (end - date.fromisoformat(item["Date"])).days
            values = [item["TransactionID"], item["Date"], age, item["Reference"],
                      item["Description"], f"{amount:.2f}", pack["notes"].get(item["TransactionID"], "")]
            outstanding.append("<tr>" + "".join(f"<td>{esc(v)}</td>" for v in values) + "</tr>")
    suggestions = "".join(f"<li><strong>{esc(group['group'])}</strong>: "
                          f"{esc(', '.join(group['ids']))}. {esc(group['reason'])}</li>"
                          for group in pack["suggestions"])
    decisions = "".join(f"<li><strong>{esc(group['decision'] or 'pending')}</strong> "
                        f"{esc(group['group'])}: {esc(', '.join(group['ids']))}. "
                        f"{esc(group['note'])}</li>" for group in pack["decisions"])
    identity = " / ".join(pack["identity"].values())
    blocked = ("<p class='notice'>Balance checks failed. Resolve the differences before carrying "
               "items forward. This run has no carry-forward file.</p>" if pack["status"] == "BLOCKED" else "")
    totals = "".join(f"<tr><th scope='row'>{label}</th><td>{esc(pack[key])}</td></tr>" for label, key in (
        ("Opening ledger balance", "opening_balance"), ("Current movement", "movement"),
        ("Closing ledger balance", "closing_balance"), ("Outstanding items", "outstanding_total"),
        ("Opening items minus opening ledger", "opening_difference"),
        ("Opening ledger + movement minus closing ledger", "closing_difference")))
    return f"""<!doctype html>
<html lang="en-AU"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'">
<title>Clearing reconciliation: {esc(identity)}</title>
<style>body{{font:16px/1.55 system-ui,sans-serif;max-width:1100px;margin:40px auto;padding:0 24px;color:#182d34;background:#fafaf7}}
h1{{font-size:2rem;line-height:1.2}}h2{{margin-top:32px}}.meta{{color:#465961}}.status{{font-weight:700}}
table{{border-collapse:collapse;width:100%;background:white}}th,td{{text-align:left;vertical-align:top;padding:10px 12px;border-bottom:1px solid #ccd5d5;overflow-wrap:anywhere}}
th{{font-weight:600}}caption{{text-align:left;padding:8px 0}}.scroll{{overflow-x:auto}}.notice{{padding:16px;border-left:4px solid #9b3c19;background:#fff0e5}}
li{{margin:10px 0}}@media print{{body{{margin:0;font-size:10pt}}table{{font-size:9pt}}}}</style></head>
<body><main><p class="meta">MONTHLY CLOSE CONTROLS</p><h1>Clearing-account reconciliation</h1>
<p>{esc(identity)}<br>{esc(pack['period_start'])} to {esc(pack['period_end'])}</p>
<p class="status">{esc(pack['status'])} · {len(pack['outstanding'])} outstanding items</p>
<p>{esc(pack['review_boundary'])}</p>{blocked}
<h2>Balance checks</h2><table><caption>Debit balances are positive; credit balances are negative.</caption>
<tbody>{totals}</tbody></table>
<h2>Outstanding items</h2><div class="scroll"><table><thead><tr>
<th scope="col">ID</th><th scope="col">Date</th><th scope="col">Age (days)</th><th scope="col">Reference</th>
<th scope="col">Description</th><th scope="col">Amount</th><th scope="col">Review note</th>
</tr></thead><tbody>{''.join(outstanding)}</tbody></table></div>
<h2>Suggested matches</h2><p>Suggestions remain outstanding until a reviewer supplies an accepted group.</p>
<ul>{suggestions or '<li>No suggested matches.</li>'}</ul>
<h2>Review groups</h2><ul>{decisions or '<li>No review groups supplied.</li>'}</ul>
<h2>Continue the review</h2><p>Open suggestions.csv, set Decision to accept or reject and add the same note
on every row of a group. Add manual groups using outstanding transaction IDs. Supply that CSV with
--decisions and rerun into a new output directory. Include your earlier decisions when revising this period.</p>
<p>Use carry-forward.json as --opening-items for the next period after reviewing this pack.
The JSON preserves original text and dates; the spreadsheet export guards formula-like text.</p>
</main></body></html>"""


def write_reconciliation(pack: dict, output: Path) -> Path:
    output = require_output_outside_repository(output)
    if output.exists():
        raise ValueError("Output already exists. Choose a new directory to preserve the previous run.")
    end = date.fromisoformat(pack["period_end"])
    outstanding_rows = [[item[key] if key in {"Debit", "Credit"} else _csv_safe(item[key])
                         for key in COLUMNS] + [str((end - date.fromisoformat(item["Date"])).days),
                         _csv_safe(pack["notes"].get(item["TransactionID"], ""))]
                        for item in pack["outstanding"]]
    decision_rows = [[_csv_safe(group["group"]), key, group["decision"], _csv_safe(group["note"])]
                     for group in pack["decisions"] for key in group["ids"]]
    used = {group["group"] for group in pack["decisions"]}
    for group in pack["suggestions"]:
        name = group["group"]
        while name in used:
            name = "new-" + name
        used.add(name)
        decision_rows.extend([[name, key, "", ""] for key in group["ids"]])
    rendered = {
        "reconciliation.json": (json.dumps(pack, indent=2, ensure_ascii=False) + "\n", "utf-8"),
        "review.html": (render_html(pack), "utf-8"),
        "outstanding.csv": (_csv(COLUMNS + ("AgeDays", "ReviewNote"), outstanding_rows), "utf-8-sig"),
        "suggestions.csv": (_csv(DECISION_COLUMNS, decision_rows), "utf-8-sig"),
    }
    if pack["status"] != "BLOCKED":
        carry = {"schema": "clearing-carry-v1", "identity": pack["identity"],
                 "period_end": pack["period_end"], "balance": pack["closing_balance"],
                 "items": pack["outstanding"],
                 "notes": {item["TransactionID"]: pack["notes"][item["TransactionID"]]
                           for item in pack["outstanding"] if item["TransactionID"] in pack["notes"]}}
        rendered["carry-forward.json"] = (json.dumps(carry, indent=2, ensure_ascii=False) + "\n", "utf-8")
    # Encode everything before creating files, including text a library caller may have supplied.
    encoded = {name: text.encode(encoding) for name, (text, encoding) in rendered.items()}
    output.mkdir(parents=True, exist_ok=False)
    created = []
    try:
        for name, content in encoded.items():
            target = output / name
            with target.open("xb") as stream:
                created.append(target)
                stream.write(content)
    except BaseException:
        for target in reversed(created):
            target.unlink(missing_ok=True)
        output.rmdir()
        raise
    return output / "review.html"
