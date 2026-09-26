# Compare two close runs

The source-only command verifies both complete packs before comparing them. It
also checks each supplied current trial balance against its pack's recorded
digest. Those snapshots establish the entity even when a pack has no findings.

From an installed checkout:

```text
close-control compare --previous-pack PREVIOUS_PACK --current-pack CURRENT_PACK --previous-tb PREVIOUS_CURRENT.csv --current-tb CURRENT_CURRENT.csv
```

The command writes JSON to standard output and changes no files. Exit 0 means
verified display, including when either underlying pack requires review; it is
not a passing close. Malformed or mismatched evidence exits 1.

Findings are grouped by control, tenant and account, retaining every finding in
each group. Query comparisons use the existing stable query IDs. Changes are
NEW, RECURRING, CHANGED, NOT_RAISED or NOT_COMPARABLE. NOT_RAISED means absent
from the later run, not resolved. A changed threshold, mapping, omitted-control
coverage, required calculation, financial-year-reset context, non-contiguous
period or blocked run is named under scope_changes. An absent finding under
changed scope is NOT_COMPARABLE. Amount changes remain visible even when scope
differs; interpret them alongside that scope list. A changed source's completeness
is not established by matching totals, and the comparison does not authenticate
source records or reviewer identity. Changed trial-balance account populations
are named. A changed subledger digest also makes absence NOT_COMPARABLE because
the existing pack stores its digest, not the complete covered account list.

Comparison works on findings, not balances, so an error no single run flags can
never appear in it. The fabricated ledger in `tests/test_comparison.py` posts 150
of software subscriptions to Office Expenses every month for a financial year.
At the default thresholds no pack or comparison names either account, and the
June close passes with Office Expenses overstated by 1,800, a quarter of its
correct balance. A steady mis-coding moves like any recurring cost: at an
absolute threshold of 100, Office Expenses and Software Subscriptions are flagged
in the same pattern, so the comparison cannot tell which is wrong. Catching it
needs evidence the pack does not hold, such as a budget, prior-year balances or
a review of how each supplier's bills are coded.

Optional --responses accepts a separate UTF-8 JSON file:

```json
{
  "schema_version": 1,
  "pack_sha256": "REPLACE_WITH_CURRENT_PACK_JSON_SHA256",
  "responses": [
    {
      "query_id": "COPY_FROM_CURRENT_PACK",
      "explanation": "The supporting statement identifies a timing difference.",
      "evidence_reference": "Synthetic statement, item 4",
      "reviewer": "AB",
      "reviewed_on": "2026-10-01"
    }
  ]
}
```

Supply the actual digest and query ID. Dates cannot precede the current close.
Duplicate or unknown queries, extra fields and a different pack digest are
refused. References are labels only; nothing is opened or sent. Responses are
included with their source digest and never change a finding, pack status or
accounting decision. Keep response files and redirected results in the approved
workpaper location outside the checkout.
