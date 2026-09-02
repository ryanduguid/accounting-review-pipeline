# Close-input contract roadmap

`PaydaySuper.Report` is intentionally limited to the documented 18-column
producer contract emitted by `payday-super-checker`. Its checked-in sample is
fabricated. It is not a Xero or MYOB export parser, and it does not make an
accounting, legal or exposure decision.

## Contracted parsers

- `PaydaySuper.Report`: Fixed 18-column CSV producer contract emitted by `payday-super-checker`.
- `Xero.AgedReceivables`: Observed Xero aged receivables summary CSV export contract. Returns the bucket and Total columns as exported, drops the summary total row, and coerces contact keys to text. The bucket-to-total tie-out is the caller's, not this query's.
- `Xero.AgedPayables`: Observed Xero aged payables summary CSV export contract. Returns the bucket and Total columns as exported, drops the summary total row, and coerces supplier keys to text. The bucket-to-total tie-out is the caller's, not this query's.

## Evidence gate: observed Xero aged receivables/payables

`Xero.AgedReceivables` and `Xero.AgedPayables` shipped in `v0.1.5` and are
listed under Contracted parsers above, ahead of this gate: neither parser
header names an observed export mode and date, and no static or native
acceptance case calls either function yet, so the gate's criteria below
remain open. The requirement is kept as the record of what has to be
collected, and it applies to the shipped parsers and to any change in the
observed export shape.

Before adding a Xero aged receivables or aged payables parser, collect a fresh,
non-client interactive export outside this repository and record only the
minimum non-sensitive shape evidence needed for a fabricated fixture. Establish
the exact header names, title rows, ageing-bucket labels, sign convention,
currency treatment, contact/reference fields, totals and report-date semantics.

Do not infer a schema from a trial balance export, the Xero API, screenshots,
help articles or a differently configured report. A future parser must name
the observed export mode and date, accept headers by name, preserve source
values, reject a changed shape clearly, and add fabricated static plus native
Excel acceptance cases before it is documented as supported.

## Remaining evidence gate: MYOB-specific parsers

MYOB-specific parsers follow only after an independently observed MYOB export
is available. Treat each MYOB product, report and export mode as a separate
contract: do not reuse Xero assumptions or label an unobserved layout as MYOB
support. Use the same fabricated-fixture, named-header, hostile-input and
native-engine acceptance requirements as the Xero gate.

The first MYOB contract is now scoped: `MYOB.AgedReceivables` against MYOB
Business (Pro), Excel report export. Product decision, export-mode constraints,
contract skeleton v0.1 and the observation checklist live in
[myob-business-aged-receivables-discovery.md](myob-business-aged-receivables-discovery.md).
No parser exists; the gate stays closed until a real observation fills the
"to observe" entries.

## Boundaries that remain in force

- Keep actual client exports outside the repository; commit fabricated samples
  only.
- Preserve source status, verdicts, caveats, notes, identifiers and amounts.
  Any reconciliation, materiality or close decision remains a human review.
- Do not add OAuth, network calls, Excel automation, write-back or cross-repo
  runtime dependencies to an input adapter.
