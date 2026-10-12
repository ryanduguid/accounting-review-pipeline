# Synthetic close review workflow

The close review page shows verified Monthly Close findings and the journal rows
returned by its driver command. The offline HTML record shows the same results
and compares two verified runs. All demonstration assertions are fabricated.

## Build the shared case

From the repository root, install the existing components with `uv sync --locked`.
Run these commands from this component directory, with new output directories
outside every Git checkout:

```powershell
python -B tools/review_workflow.py build --run C:/Users/-/Documents/review-demo/august --month 8
python -B tools/review_workflow.py build --run C:/Users/-/Documents/review-demo/september
python -B tools/review_workflow.py html --run C:/Users/-/Documents/review-demo/september --previous C:/Users/-/Documents/review-demo/august --output C:/Users/-/Documents/review-demo/review.html
python -B tools/review_workflow.py verify --run C:/Users/-/Documents/review-demo/september
```

Use `--bin-dir` for another installed CLI directory. On Linux, supply the
environment's `bin` directory explicitly. The workflow invokes `review-ready`
and `close-control` as independent commands. It imports no sibling runtime.
It retains the commands, producer manifests, timestamps, exit codes and logs.
Each manifest binds the supported launcher, its declared Python interpreter,
selected distribution and entry point, and all resolved first-party package
files, including empty files and typing markers. Normal uv and PyPA console launchers
are supported. Unknown launchers, linked package paths and changed identities
are refused. Rebuild any earlier demonstration run that lacks these manifests.

The case is ENT001, Varrock Ventures Pty Ltd, in AUD on an accrual basis.
August and September 2024 share the original ledger's first financial year and
explicit opening journal JNL1001. Later years have no closing journals in this
fixture, so the adapter refuses them. It does not invent retained-earnings
transfers. Stable fabricated account identifiers are declared separately from
account codes in `samples/shared-review-case.json`; they are not Xero IDs.

The ten-column trial balance follows the root exporter contract. Movement debit
and credit are summed separately from each month's journal lines. YTD debit and
credit include the explicit opening journal and all eligible July-to-period-end
lines, without netting either side. Their net is the as-at balance. Exact Decimal
ties check each journal, both trial-balance pairs and the movement bridge. The
original six source files stay byte-pinned. Trial balances do not replace journal
evidence. The exporter's corpus runner accepts only its three declared contract
fixtures; the new case is checked against the data-only contract and by both
review producers.

Readiness self-review assertions are synthetic examples. `READY`, the close's
`REVIEW` or `PASS`, technical verification and acknowledgements remain separate.
An optional `--review-note` passes the existing acknowledgement file to Monthly
Close. It records a note without changing control status. No command approves
accounting, posts a journal or closes a period.

## Evidence and portable records

The producer's `view` commands verify their output files. The admission receipt
binds the original sources, derived inputs, results and driver output. Consumers
recheck their hashes and journal ties. The portable record runs the producer
verifiers again before display, comparing complete producer identities before
and after each invocation. Child commands ignore inherited Python search paths
and source-tree bytecode. Duplicate driver account identities are refused. A
comparison invokes the existing `compare`
command after verifying both runs and their common case context.

Variance evidence uses the driver's explicit transaction identifiers. Every
included line is identified by JournalID and LineNumber. The adapter requires
nil unexplained movement and a complete returned identifier list. Unsupported
controls state that line-level evidence is unavailable. Amounts and thresholds
remain text; the report does not reclassify findings or calculate materiality.
Controls not run are listed without a coverage percentage. `NOT_RAISED` in a
comparison does not mean resolved or approved. Acknowledgement changes appear
separately from finding and input changes.
Changed inputs include files added, changed or removed between runs. Ambiguous
JSON, malformed consumed fields and JSON nesting beyond 64 levels are refused
through the workflow's normal error message.

The receipt and HTML are unsigned local records. Hashes detect accidental edits,
but someone who can replace the complete run can replace its receipt too. The
producer identity does not establish publisher authenticity or bind the whole
operating system, Python libraries or environment. It does not prevent concurrent
replacement races or prove that the initial installation was benign. The
HTML is one file with embedded CSS, escaped data and no scripts or external
resources. Keep generated packs and portable records outside the checkout.

Use the finding index to jump to an account's review action and evidence. Each
finding links to the next finding and back to the index. Findings retain the
producer's order; duplicate account names have separate destinations. Journal
tables include source references and exact amount text. On a narrow screen,
scroll within a journal table to read every column. The report follows the
browser's light or dark preference.

Journal rows, run context, comparisons and hash receipts are visible without
opening disclosures. Print from the browser to include those sections;
navigation links are omitted from the printed layout. The report remains a
fabricated demonstration with an unsigned local receipt. Verification,
acknowledgement and printing do not approve accounting or close a period.

Run changes starts with counts of finding groups and queries by the producer's
classification. The finding comparison shows each run's value and status under
its period date. Multiple records appear separately, without totals. Changed
compares complete records, so a change need not be a balance or status change.
Recurring means the complete record is identical. Not raised does not mean
resolved or approved. Scope changes, changed input files and acknowledgement
changes appear separately. Query classifications identify the account and query
ID; full questions, evidence requests, reasons and actions remain in the full
comparison record. Use its link to read that record and return to the summary.
Each comparison account keeps its change label, dated values and statuses
together. The fields stack on narrow screens without horizontal scrolling.
Query entries use the same layout. Both the summary and the full record are
included when printing.

The committed three `sample-review-*.csv` files are fabricated report projections,
not producer review packs. To regenerate them, use `fixtures --run ... --output`
with a new external directory, inspect the results and replace only those three
sample files. The report supports this one September case. Its review tables
form an island with one relationship from evidence to exception; they do not
filter the financial model. The evidence page receives the exception, run,
entity, period and basis, and its display measure suppresses rows when no
exception is selected. Hidden pages are navigation settings, not access controls.
For each finding labelled Journal rows reconciled, refresh requires the complete
account population from the trusted ledger for the run's entity and month. It
checks each identifier, date, reference, description and exact amount, then ties
the source total to that finding's difference. Unavailable findings cannot carry
journal evidence. Each repeated account finding needs its own complete evidence
population. The finding record remains authoritative; this check does not
reconstruct every producer finding or authenticate the whole review pack.

## Export a single review run

Create a ZIP with the sealed run, its preparation assertions and validation
logs, the three CSV projections, the offline HTML report and an unsigned member
inventory. Use a new local destination outside version control and outside the
sealed run directory:

```powershell
python -B tools/review_workflow.py export --run C:/Users/-/Documents/review-demo/september --output C:/Users/-/Documents/review-demo/september-evidence.zip
python -B tools/review_workflow.py verify-export --run C:/Users/-/Documents/review-demo/september-evidence.zip
```

The export supports one fixed fabricated run. It refuses free-form review notes,
extra files or directories, `--review-note` and `--previous`. Before reading the
receipt or sealed inputs, it checks their fixed inventory, regular-file types
and individual and aggregate sizes. Admission covers ordinary directory entries
and default data streams; it does not inspect alternate streams or storage aliases.
It reruns the existing producer checks and
uses their exact bytes. The archive includes the synthetic preparer assertions
in `run/inputs/self_review.json`; it creates no reviewer approval.

Recipients can run `verify-export` with an independently obtained copy of
`tools/review_workflow.py`, Python and the ZIP. They need no samples, installed
producers or original run directory. Verification reads members without
extracting them and checks the exact inventory, sizes, hashes and nested receipt.
The verification command refuses `--previous`, `--review-note` and `--output`.
It reports unsigned internal member-byte consistency. Anyone able to replace the
whole archive can replace its hashes. This check does not authenticate provenance,
prove producer execution, approve accounting or establish complete evidence or
controls. HTML and CSVs are workflow-derived views generated from the captured
run during export. Portable verification binds their bytes but does not establish
their semantic correspondence to the packs. The nested binding covers the
receipt's file map; the existing local checks own the other financial ties.

The stored ZIP is deterministic for an unchanged run and renderer. The fixed-case
limits are 64 files, 512 KiB per member and 2 MiB for the archive and payload.
August and September payloads fit these limits. Logs retain their original local
output paths. Inspect the archive before sharing it; the export sends nothing.
Retain the raw ZIP and the exact verifier revision. This profile has been checked
with Python 3.14.8; compatibility with other runtime serializers is unverified.
Cleanup uses the created file's descriptor identity and checks it before removing
a failed output. A partial file may remain when identity cannot be established.
Publication and cleanup assume cooperating local processes; they do not isolate
the destination from another process replacing it between filesystem operations.

## Prepare Desktop and verify

```powershell
powershell -NoProfile -File tools/prepare_native_project.ps1 -Destination C:/Users/-/Documents/review-demo/native -Launch
```

The destination must be new, local and outside the source project. Preparation
copies project source and nine fabricated CSVs, pins their hashes, sets only the
copy's `SampleFolder`, and records the Desktop process it launched and its child
engine. It adopts no pre-existing Desktop. Without `-Launch`, it prepares files
only. No privacy or refresh result is implied by successful preparation.

In that copy, set the nine fabricated CSV sources to Public through Data source
settings and refresh. Then run:

```powershell
powershell -NoProfile -File tools/test_native_project.ps1 -InstanceFile C:/Users/-/Documents/review-demo/native/instance.json -SourceManifest C:/Users/-/Documents/review-demo/native/source-manifest.json -EvidenceDirectory C:/Users/-/Documents/review-demo/native-evidence
powershell -NoProfile -File tools/test_refresh_guards.ps1 -Server localhost:<recorded-port> -SampleFolder C:/Users/-/Documents/review-demo/native/project/samples
powershell -NoProfile -File tools/test_review_evidence.ps1 -Server localhost:<recorded-port> -SampleFolder C:/Users/-/Documents/review-demo/native/project/samples
```

The runner verifies process creation times, engine ownership, model definitions,
complete source and copied project inventories and hashes before and after the existing
financial, benchmark, review, projection and presentation checks. The projection checks
compare every imported text value and test journal identifiers under each full
context and deliberately wrong contexts. Refresh rejection remains a separate,
explicit operation in the marked disposable copy.

Presentation checks exercise the shared string formatter in a temporary table,
including a 30-digit integer part, negative zero, null and malformed values.
The temporary table is removed before the runner's final source check. They also
compare all displayed amounts with their raw strings and verify full detail text
for every finding under account and exception-key selection, plus unselected and
multiple-selection states. The rejection suite checks malformed difference,
journal amount and absolute-threshold display inputs.

On Close review, select a finding row to read its full details, then activate
View journal evidence. In Desktop edit mode, use Ctrl+click or Ctrl+Enter.
The optional Finding filter narrows the list and keeps the account choice after
Back. Row highlighting is temporary. Check native rendering and keyboard
behaviour after report changes; PBIR validation establishes structure only.
In smaller Desktop windows, collapse the authoring panes and use Fit to page.
Portrait layouts are authored for all six pages. Phone preview checks cover the
finding, journal evidence and Back flow; swipe tables for supporting columns.
NVDA queue text verifies selected announcements. Full keyboard traversal, human
screen-reader acceptance, physical phones, Power BI Service, gateways and
production workloads require separate verification.
