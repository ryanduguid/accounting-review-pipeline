# Monthly Close Controls

```
+----------------------------------------------------------------------+
|                        Monthly Close Controls                        |
+----------------------------------------------------------------------+
|           Deterministic close controls for trial balances            |
+----------------------------------+-----------------------------------+
| DR  what it gives you            | CR  what it needs                 |
+----------------------------------+-----------------------------------+
| close summary with exceptions    | current and prior TB CSV          |
| exceptions CSV for review        | an account mapping CSV            |
| PASS REVIEW BLOCKED states       | -                                 |
+----------------------------------+-----------------------------------+
```

[![tests](https://github.com/ryanduguid/accounting-review-pipeline/actions/workflows/ci.yml/badge.svg)](https://github.com/ryanduguid/accounting-review-pipeline/actions/workflows/ci.yml) [![PyPI](https://img.shields.io/pypi/v/monthly-close-control-plane.svg?color=5C2D91&labelColor=04001F)](https://pypi.org/project/monthly-close-control-plane/) [![License: MIT](https://img.shields.io/badge/License-MIT-4F485E.svg?labelColor=04001F)](LICENSE) [![Python 3.10+](https://img.shields.io/badge/python-3.10%2B-5C2D91.svg?logo=python&logoColor=white&labelColor=04001F)](https://www.python.org/downloads/)

The maintained source is under
[`packages/monthly-close-control-plane`](https://github.com/ryanduguid/accounting-review-pipeline/tree/main/packages/monthly-close-control-plane)
in the Accounting Review Pipeline. The `monthly-close-control-plane`
distribution and `close-control` command remain compatibility identifiers.

A small, **review-first** monthly-close control pack for a validated trial-balance export. You point it at a current and a prior trial balance and it hands you an exception pack for close review:

- `close-summary.md` answers 'what needs my attention this close?': a concise, deterministic review pack with an overall status, the thresholds used, source evidence, and an exception table a reviewer reads top to bottom.
- `exceptions.csv` answers 'which accounts, by how much, and why?': filterable exception detail for Excel or Power BI, one row per exception with values, differences, thresholds, and a suggested reviewer action.
- `client-queries.csv` answers 'what do I have to ask the client?': the exceptions only a client can settle, turned into a question and the evidence that would answer it, with the ones the firm resolves itself left out.
- `close-review-pack.json` answers 'what exactly did this run look at?': structured evidence, thresholds, source hashes, and any supplied review acknowledgement, for archiving or downstream tooling.

The pack surfaces material YTD variances, new and missing accounts, account metadata changes, unmapped accounts, and supplied subledger differences as explicit exceptions. Output has only `PASS`, `REVIEW`, and `BLOCKED` states. A reviewer, not the tool, decides whether a close is acceptable.

It is intentionally narrow:

```text
Validated trial-balance export
            |
            v
Exact control gates and variance checks
            |
            v
Explicit exception queue
            |
            v
Human review and workpaper acknowledgement
```

The first MVP accepts the canonical CSV written by [xero-trial-balance-export](https://github.com/ryanduguid/accounting-review-pipeline/tree/main/packages/xero-trial-balance-export) (`xero-trial-balance-export`). Each file must contain exactly one tenant and one report date; current and prior files must name the same tenant, and the prior date must be earlier. It does **not** connect to Xero, store OAuth tokens, write journals, make payments, lodge BAS, lock a period, distribute a client report, or claim that a close has been approved.

## Quick demo

Source additions: [compare successive verified close packs](docs/close-comparison.md)
and [reconcile ledger equity movements](docs/equity-reconciliation.md).
[Variance drivers](docs/variance-drivers.md) rank the transactions behind each
period variance in a verified pack.
The [evidence schedules](docs/evidence-schedules.md) reconcile expenses,
migration snapshots and inter-entity balances from supplied canonical records.
These commands and options are not established by the published 0.1.4 package.

The [portable workflow recipe](docs/utility-workflows.md) joins close packs,
forecasts, WIP and grant workpapers through each repository's locked environment.

The [architecture note](docs/architecture.md) sets out the control boundary and
the review-pack pipeline. For transaction-level clearing-account matching, see the
[3-month reconciliation example](docs/clearing-reconciliation.md).
The `reconcile` command suggests matches, records reviewed allocations and
carries outstanding items forward. It ships from 0.1.5 and uses a separate
mapped transaction schema; a trial-balance export is insufficient.

The repository contains fabricated data only. Do not commit client trial balances, workpapers, exports, or credentials.

`examples/` is the assault course: every move the tool has, run against
fabricated data, with nothing at stake. Learn the flags here before pointing
it at a real ledger.

```bash
python -m pip install -e ".[dev]"

close-control review \
  --current examples/current_trial_balance.csv \
  --prior examples/prior_trial_balance.csv \
  --mapping examples/account_mapping.csv \
  --subledger examples/subledger_balances.csv \
  --absolute-threshold 10000 \
  --percentage-threshold 0.10 \
  --reconciliation-tolerance 0.01 \
  --review-note examples/review_note.json \
  --output ../../../close-control-demo
```

The demo exits `2` because its deliberately fabricated exceptions need human review. It writes the 4 pack files described above.

`--output` points outside this checkout, and it has to: the command refuses an
output directory inside a version-control checkout, exiting `1` without
creating anything. A review pack names a client's accounts, balances and
unexplained movements, and inside a checkout it is one `git add -A` away from a
history that every clone copies. A `.gitignore` entry is a convention the next
commit can waive, and it does nothing about the copy sitting in the working
tree meanwhile. A checkout is any directory at or above the output holding
`.git`, `.hg`, `.svn` or `.bzr`, because the harm is the pack going under
version control and every one of those copies a committed pack to each clone.
A directory inside the work tree named by `GIT_WORK_TREE` counts too, since
Git can hold its metadata elsewhere and leave a tracked tree carrying no
marker to find. A work tree selected some other way, by `--work-tree` on
another process's git invocation or by `core.worktree` in a repository this
command never opens, cannot be discovered from here: the check is a backstop
for the location you chose, not a proof that a directory is untracked.
An output the command cannot examine, such as one behind a symbolic-link loop
or an unreadable parent, is refused on the same grounds rather than assumed
safe. The refusal applies to writing only: `close-control view` still opens a
pack wherever it already is.

Use exit code `0` only for an all-`PASS` pack, `2` for `REVIEW` or `BLOCKED`, and `1` for a malformed file, an invalid command configuration, an `--output` path inside a version-control checkout, or an `--output` path that cannot be written.

To run the check on a schedule in CI, copy [examples/github-actions-close-check.yml](examples/github-actions-close-check.yml) into `.github/workflows/`.
It runs against a repo-stored synthetic trial balance and fails the job when the pack is `BLOCKED`.

To run this pack against the co-located gateway's same-financial-year sample CSVs (not the Varrock June/July demo pair, which crosses the 1 July reset), see [examples/close-loop.md](examples/close-loop.md). That loop is local files only; it does not connect to Xero.

## Local close workbench

`close-control workbench` is a local façade over the same validation, control
engine, and 4-file writer used by `close-control review`. It is useful when
the close process starts with 2 already-created canonical exports in an
access-controlled directory outside this repository:

```bash
close-control workbench \
  --current C:\close-data\current.csv \
  --prior C:\close-data\prior.csv \
  --mapping C:\close-data\account-mapping.csv \
  --subledger C:\close-data\subledger.csv \
  --output C:\close-data\review-pack
```

It writes exactly `close-summary.md`, `exceptions.csv`, `client-queries.csv`
and `close-review-pack.json`. Open or import `exceptions.csv` in Excel or Power
Query if useful, then investigate and document conclusions through your normal
workpaper process. The command never starts Excel, creates a workbook, calls
Xero, reads OAuth credentials or tokens, calls an AI service, posts anything,
or copies the supplied sources. It records review evidence only; it does not
approve or close a period.

Keep inputs and output outside the checkout: repository fixtures remain
fabricated, and the existing `.gitignore` rules deliberately prevent ordinary
exports and generated packs becoming repository content.

## Viewing an existing pack

`close-control view` is the read-only half of the workbench: it loads a
generated pack, proves the 4 files still agree with each other, and prints
a review sheet. It never writes, renames or deletes anything, and it cannot
change what the engine computed.

```bash
close-control view --pack-dir C:\\close-data\\review-pack
```

Before displaying anything it fails closed on: a missing artefact; JSON that is
not valid UTF-8, not valid JSON, or carries unknown, missing or duplicated
top-level members; a threshold or source digest that no longer parses as the
writer rendered it; an acknowledgement whose members are not the writer's or
whose fixed effect statement differs from the writer's text (the sheet states
that effect itself, so the JSON member is compared, never displayed); a
`close-summary.md` whose overall status, source-evidence
digests or review-boundary statement disagree with the JSON (including a second,
conflicting status line, or a missing client-query boundary statement); and an
`exceptions.csv` or `client-queries.csv` whose header, row count or any cell
disagrees with the JSON member it projects, honouring the writer's
formula-injection guard exactly; and a `calculation_evidence` block whose
members, required list, effect text, provenance digest, figures, relied-on
flags or per-entry digest disagree with the summary's own calculation-evidence
section. The summary row carries a SHA-256 of each entry's canonical JSON, so
every member of the entry is witnessed, the printed ones and the advisory notes
and rate tables alike, and editing any of them in the JSON alone is refused
rather than displayed. Removing the block and its section together is refused
too, because `source_sha256` still names the evidence file. A data row holding more or fewer cells than
the header declares fails closed as well: a surplus cell would otherwise sit
outside every named column, unguarded against formula prefixes and compared
with nothing.

The summary's client-query section is checked against the JSON register too,
not only for its boundary sentence: the count line, every `query_id` and every
question must agree. That file is what a preparer reads and copies a question
out of, so a question edited only there is the divergence worth catching. A
duplicated `query_id` fails closed for the same reason: one identifier against
2 questions leaves a firm unable to tell which one a client answered.

A pack written before the client-query register existed still opens. Its 3
files verify and display as before, and the sheet says the pack predates the
register rather than leaving a reviewer to wonder. A pack holding half a
register, the file without the JSON member or the member without the file, was
assembled from 2 runs or edited, and is refused rather than read as an older
one. On success the sheet ends with the SHA-256 of
each artefact's exact bytes, so the displayed evidence can itself be archived.
Exit code is 0 when a pack was verified and shown, 1 when verification failed.

## Mapping compatibility policy

The development source adds optional `--mapping-policy` to `review` and
`workbench`. It requires `--mapping` and accepts a local UTF-8 CSV with exactly
`Section,ReviewGroup` columns, in either order. Each row permits one source
section for one review group. A group may allow several sections. Blank fields,
duplicate pairs, an empty policy and malformed rows fail with exit code 1.

For example, `Assets,Receivables` allows accounts whose source `Section` is
`Assets` to use the supplied `ReviewGroup` of `Receivables`. Matching trims outer
whitespace and preserves case. The policy contains the firm's explicit choices;
the tool does not infer allowed pairs from account names or balances.

An incompatible mapped account, or a mapped group with no policy entry, raises
a `REVIEW` exception under `mapping_compatibility`. The exception records the
original section and group and the permitted sections. It leaves the account
and mapping unchanged. Unmapped accounts remain covered by `account_mapping`.
The policy applies to current-period accounts and is reviewed by the firm, so
these exceptions do not draft client questions.

Add `--mapping-policy examples/mapping_policy.csv` to the quick demo to use the
fabricated policy that matches `examples/account_mapping.csv`. Keep real policy
files outside the checkout. The pack's `source_sha256.mapping_policy` records
the exact bytes read by the control. Runs without this option retain their
existing behaviour. This option ships from 0.1.5.

## Balance policy

The development source adds optional `--balance-policy` to `review` and
`workbench`. It accepts a local UTF-8 CSV with exactly
`AccountID,ExpectedBalance,ExpectMovement` columns. `ExpectedBalance` is
`debit`, `credit`, `nil` or `any`; `ExpectMovement` is `yes` or `no`. Blank
fields, other values, a repeated `AccountID`, an empty policy, any quote
character and malformed rows fail with exit code 1.

Each listed account is compared with the current trial balance, and each of
these raises a `REVIEW` exception under `balance_policy`:

- a `YTDDebit - YTDCredit` balance on the other side from a declared `debit` or
  `credit` (an overdrawn bank account, a debit balance in creditors);
- any balance in an account declared `nil` (suspense, clearing, rounding);
- no movement in the current period (`Debit` and `Credit` both nil) for an
  account declared `ExpectMovement` `yes`, such as accruals, payroll
  liabilities or GST. Xero leaves an account with no balance and no movement
  out of the trial balance, so an absent account counts as nil with no movement.

The policy is the firm's explicit list; the tool infers nothing from `Section`
or account names. Declare a contra account, such as accumulated depreciation,
with the side it really carries. Accounts the policy does not list are not
checked. The exceptions are the firm's to trace, so they draft no client
questions.

Add `--balance-policy examples/balance_policy.csv` to the quick demo to use the
fabricated policy; it raises one exception, for a payroll liability `210` that
has no movement. Keep real policy files outside the checkout. The pack's
`source_sha256.balance_policy` records the exact bytes read, and a run without
the option lists `balance_policy` under `controls_not_run`.

## Worked example

Running the quick-demo command above against the fabricated fixtures in `examples/` prints:

```text
close-control: REVIEW; 8 exception(s); 6 client query(ies) drafted
  json: /home/you/close-control-demo/close-review-pack.json
  summary: /home/you/close-control-demo/close-summary.md
  exceptions: /home/you/close-control-demo/exceptions.csv
  client_queries: /home/you/close-control-demo/client-queries.csv
```

The paths are absolute and have their symlinks resolved, whatever `--output`
was written as. The writer builds every destination from the directory the
checkout guard approved rather than from the argument, so what is printed is
where the pack actually went.

`close-summary.md` opens with the status, scope, and source digests, then lists every exception (abridged here to 4 of the eight rows):

```markdown
# Monthly Close Review Pack

**Overall status: REVIEW**

This pack is a review aid. It does not approve a close, post a journal, make a payment, lodge a return, or lock a period.

## Scope

- Current report date(s): 2026-07-31
- Prior report date(s): 2026-06-30
- Material variance thresholds: $10000.00 and 10.00%
- Reconciliation tolerance: $0.01
- Exceptions: 8 total; 0 blocked; 8 requiring review.

## Exceptions

| Status | Control | Tenant | Account | Difference | Reason |
| --- | --- | --- | --- | ---: | --- |
| REVIEW | account_mapping | Varrock Ventures Pty Ltd | 6000 / Operating Expenses | n/a | Current account has no supplied review-group mapping. |
| REVIEW | financial_year_reset | n/a | n/a | n/a | Current ReportDate 2026-07-31 and prior ReportDate 2026-06-30 fall in different Australian financial years (1 July to 30 June). YTD figures reset on 1 July, so this YTD-vs-YTD comparison crosses a year reset and the period_variance verdicts for profit-and-loss-style rows are not meaningful. |
| REVIEW | period_variance | Varrock Ventures Pty Ltd | 1000 / Operating Bank | 15000.00 | YTD net balance moved beyond both configured materiality thresholds. |
| REVIEW | subledger_reconciliation | Varrock Ventures Pty Ltd | 2000 / Trade Creditors | -250.00 | Current trial-balance balance differs from the supplied subledger beyond tolerance. |
```

`exceptions.csv` carries the same exceptions with full numeric detail. The Operating Bank variance row (wrapped here for readability):

```csv
control,status,tenant,account_id,account_code,account_name,review_group,current_value,prior_value,difference,threshold,percentage_change,reason,reviewer_action
period_variance,REVIEW,Varrock Ventures Pty Ltd,100,1000,Operating Bank,Cash and cash equivalents,120000.00,105000.00,15000.00,10000.00,14.29%,
  YTD net balance moved beyond both configured materiality thresholds.,
  "Investigate the driver, retain supporting evidence, and document the reviewer conclusion."
```

The reviewer reads this as: the Operating Bank YTD balance moved from $105,000.00 to $120,000.00, a $15,000.00 (14.29%) change that clears both the $10,000.00 absolute threshold and the 10% threshold, so a human must investigate the driver and document a conclusion.

`close-review-pack.json` records the same exception as structured evidence next to the thresholds and the source digests (abridged):

```json
{
  "exceptions": [
    {
      "account_code": "1000",
      "account_name": "Operating Bank",
      "review_group": "Cash and cash equivalents",
      "control": "period_variance",
      "current_value": "120000.00",
      "prior_value": "105000.00",
      "difference": "15000.00",
      "percentage_change": "14.29%",
      "threshold": "10000.00",
      "status": "REVIEW",
      "tenant": "Varrock Ventures Pty Ltd"
    }
  ],
  "overall_status": "REVIEW",
  "thresholds": {
    "absolute_variance": "10000.00",
    "percentage_variance": "10.00%",
    "reconciliation_tolerance": "0.01"
  }
}
```

## Client queries

An exception says what a control found. A query says what somebody has to ask,
and of whom. They are not the same list, so `client-queries.csv` is the second
one: the exceptions a client can settle, each turned into a question and the
evidence that would answer it.

| Column | Meaning |
|---|---|
| `query_id` | Derived from the control and the account, so the same open question keeps its identifier next month rather than being renumbered by an unrelated exception appearing earlier in the pack. |
| `control` | The control that raised the underlying exception. |
| `tenant`, `account_id`, `account_code`, `account_name`, `review_group` | The account the question is about, matching `exceptions.csv`. |
| `difference` | The amount at issue, rendered with the same exact-decimal arithmetic as everywhere else. |
| `question` | What to ask, as a question rather than a finding. |
| `evidence_requested` | What would answer it. |

Three controls never produce a query, because nobody should send them to a
client: `trial_balance_integrity` is an export the firm re-runs,
`financial_year_reset` is a comparison the firm chose, and `account_mapping` is
the firm's own reporting file. A `subledger_reconciliation` exception raised
because a subledger balance has no matching trial-balance account is held back
for the same reason: it is a mapping or source fault, and asking a client to
explain the difference between two figures that are not yet comparable produces
an answer nobody can use. A control this package gains later asks nothing until
somebody decides it should.

When the pack also carries `financial_year_reset`, every `period_variance`
query says so in its evidence column. Year-to-date figures for
profit-and-loss accounts restart at nil on 1 July, so a comparison across the
reset can show a movement that is an artefact of the two dates. The engine will
not guess which rows reset without section rules it does not have, and neither
does the register: it flags the rows that could be affected rather than asking
a client to explain arithmetic the firm chose.

The file carries no answer or status column, deliberately. A pack is evidence
of one run and `close-control view` proves its files still agree, so an answer
written back into the file would invalidate the pack that raised the question.
Copy the queries into the firm's own tracker and leave the pack as written.

Nothing is sent. These are draft questions for a preparer to read and edit
before they reach a client, through the firm's channel and under the firm's
own review. Answering every query does not close the period, and an empty
register is not evidence that nothing needs asking.

## Canonical trial-balance contract

The initial input is the 10-column, normalised trial-balance schema from `xero-trial-balance-export`:

```text
ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit
```

`Tenant` plus `AccountID` is the control key. `AccountCode` and `AccountName` are display attributes, not stable identifiers. The loader rejects unknown/missing columns, duplicate control keys, malformed ISO dates, empty identifiers, and malformed monetary values.

The current-period `Debit`/`Credit` pair represents movement. `YTDDebit`/`YTDCredit` represents the position used for variance comparison. All values are read as exact decimals.

## Optional mapping and reconciliation inputs

An account mapping is a 2-column CSV:

```text
AccountID,ReviewGroup
```

Any current TB account that is missing from a supplied mapping remains in the pack as a `REVIEW` exception. The mapping is a review label; it does not transform source numbers.

An optional subledger CSV must have:

```text
Tenant,AccountID,SubledgerBalance
```

`SubledgerBalance` must use the same signed convention as `YTDDebit - YTDCredit`: debit balances positive; credit balances negative. Each supplied subledger row is compared only with the matching current TB account. A missing GL account, or a difference beyond `--reconciliation-tolerance`, requires review.

Each of these controls runs only when its input is supplied, and so does the
calculation-evidence control. The pack therefore names the ones that did not run:
the summary's scope block carries a `Controls not run` line, and
`close-review-pack.json` carries the same names under `controls_not_run`. A
`PASS` or `REVIEW` covers the controls that ran and says nothing about the rest,
which is the difference between a mapping that found no exception and a mapping
nobody supplied.

## Calculation evidence, optional

A close sometimes depends on a figure a calculator produced: a payroll levy, a
minimum yearly repayment, a category taxable value. This tool does not make
that call. Something else does, writes an evidence file, and you hand the file
to the close.

```bash
close-control review   --current examples/current_trial_balance.csv   --prior examples/prior_trial_balance.csv   --calculation-evidence examples/calculation-evidence-coal-lsl-levy.json   --require-calculation coal-lsl-levy   --output outputs/demo
```

The control is off unless you use one of those two flags, and a pack produced
without them carries exactly the fields it always did: the
`calculation_evidence` key appears only when the control ran.

`--require-calculation` is the important half. It names a calculation this
close needs. If no evidence file carries that label, the pack raises an
exception rather than passing quietly, because a close that silently omits a
figure it was configured to carry is the failure worth preventing.

**Nothing here touches a network.** The file is read from disk, and this
package has no HTTP client, no credentials and no calculator. The evidence file
is treated as untrusted input: it is size-bounded, its money must be decimal
strings, its text is rejected if it carries a control character, a zero-width or
directional mark, an embedding, override or isolate, and everything rendered
into the pack is escaped like any other untrusted cell. A label is a slug, so
it cannot carry any of that either.

### What is checked, and what each failure earns

| Check | Status |
| --- | --- |
| The file is unreadable, or its `calculation_sha256` does not match its own calculation block | `BLOCKED` |
| The producer recorded a validation finding, or a computed figure carries no manifest, advisory or figure | `BLOCKED` |
| A `--require-calculation` label has no evidence file | `REVIEW` |
| The evidence records a refusal, an outage or a contract failure rather than a figure | `REVIEW` |
| The evidence period does not cover the current report date, or cannot be read as a period | `REVIEW` |
| A computed figure names no rate table | `REVIEW` |

No new status exists and none of these can become `PASS`. An acknowledgement
does not clear them, the same way it clears nothing else.

A refusal recorded in an evidence file stays a refusal. It never becomes a nil
amount: `examples/calculation-evidence-refused.json` is a fabricated example of
one, and the pack reports that no figure was produced.

### What the pack records

`close-review-pack.json` gains a `calculation_evidence` block listing what was
required and what was supplied: each file's label, provider, calculator,
period, status, engine, its own digest and the digest of the bytes read, the
rate tables it names, its advisory notes, its normalised figures and whether
the pack may rely on it. `usable` is true only where the file hangs together,
carries a figure and covers this close's period, so it cannot say yes while an
exception in the same pack says otherwise. `close-summary.md` gains a
`## Calculation evidence` section stating the required list and, per
calculation, the same label, status, period, figures and relied-on flag plus a
digest of the whole JSON entry, which is what lets `view` prove the JSON was
not edited after the pack was written. The review sheet `view` renders shows
the section it verified. A pack built without the control carries
neither the block nor the section and is byte-identical to one produced before
the control existed. The file's SHA-256 also joins `source_sha256` under
`calculation_evidence:<label>`, so the pack records the bytes rather than a
path a later reader cannot check.

## Human acknowledgement

If a reviewer wants the pack to record that it was read, supply a separate JSON file:

```json
{
  "reviewer_initials": "RD",
  "reviewed_on": "2026-08-08",
  "comment": "Reviewed demo exceptions; no client close was approved by this example."
}
```

`reviewed_on` must not be earlier than the current `ReportDate`. A note dated before the period it claims to review is rejected as a malformed input: the run stops with exit `1` and writes no pack.

An acknowledgement is evidence of a human action only. It **never** changes `REVIEW` or `BLOCKED` to `PASS`, and it never asserts that a period has been closed.

## Design and integrity

A close can be technically balanced and still need review. This tool keeps the evidence visible:

- Exact `Decimal` arithmetic for money controls, never binary floating point. Totals are summed under a fixed 28-digit context with rounding trapped, so a file whose amounts would round a control total is refused as malformed input rather than compared inexactly.
- Schema, duplicate-key, date, and numeric gates fail closed.
- Current-period and YTD debits must exactly equal credits.
- Material YTD variances, new/missing accounts, account metadata changes, unmapped accounts, and supplied subledger differences become explicit exceptions.
- A YTD variance is raised only when it clears both the absolute and the percentage threshold, with one carve-out: an account whose prior YTD balance is nil has no percentage change to compute, so the absolute threshold decides alone. Those exceptions name the absolute threshold only and render `percentage_change` as `n/a (prior period zero)`, rather than reporting that a percentage test passed that never ran. The sentinel is used instead of a blank cell because a blank reads as 'no change', while no consumer can read `n/a (prior period zero)` as a zero percentage.
- Output has only `PASS`, `REVIEW`, and `BLOCKED` states. A reviewer, not the tool, decides whether a close is acceptable.
- Source SHA-256 digests travel with the generated review pack so its source files can be identified later. Each digest is calculated from the same immutable byte snapshot the loader parses, so a file replaced during a run cannot be misidentified as the source of the calculations.
- Spreadsheet-facing source text whose first non-whitespace character is `=`, `+`, `-` or `@` is neutralised with a leading apostrophe. This includes identifier- and number-shaped text such as `+unsafe`, `@123` and `-1000`; the guard does not try to decide which formula-looking values a particular spreadsheet may evaluate. Every exception table cell rendered into `close-summary.md` is flattened onto one line, and its backslashes are escaped before its pipes so that neither a pipe nor a backslash shielding one can add a cell and shift the columns a reviewer reads. A reviewer-note comment keeps its line breaks: a multi-line comment renders as an indented blockquote under the acknowledgement item, with each line escaped the same way and a leading `#` escaped so quoted text cannot forge a document heading.
- `exceptions.csv` is written with a UTF-8 byte-order mark, matching the canonical input files, so a spreadsheet reads non-ASCII entity and account names correctly.
- The 4 pack files are staged beside their destinations and moved into place only once all 4 have been written. If one cannot be replaced (a reviewer holding `exceptions.csv` open is the usual cause), the files already moved are rolled back to the content they replaced, so the previous pack survives whole instead of half describing one trial balance and half describing another. A failed run never deletes a pack file it did not write. Run one export at a time into a given `--output` directory; concurrent runs are not serialised.
- Amounts are rendered with at least 2 decimal places and never fewer than the value carries. A percentage is rendered with at least 2 places and always enough to show its leading significant digit, so neither a tolerance finer than one cent nor a threshold finer than a hundredth of a per cent is flattened to `0.00`.

### What formula neutralisation covers, exactly

The escaping in `exceptions.csv` applies to the 5 source-controlled text fields: `tenant`, `account_id`, `account_code`, `account_name`, and `review_group`. `client-queries.csv` guards the same 5 plus `question` and `evidence_requested`: those 2 are project text rather than client text, but a template reworded to start with a dash would otherwise become a formula the day somebody edits one. For those fields:

Neutralised (prefixed with an apostrophe so a spreadsheet reads them as text):

- Any value whose first non-whitespace character is `=`, `+`, `-` or `@`. For example, `=SUM(A1)` becomes `'=SUM(A1)`, `@123` becomes `'@123`, and a leading tab cannot bypass the check.
- Identifier- and number-shaped source text receives the same treatment. Account codes such as `-1000` remain visually recognisable in a spreadsheet but are explicitly stored as text.

Passed through unchanged:

- Values that do not start with a formula trigger after leading whitespace.
- Values already prefixed with an apostrophe; the guard does not add a second one.
- Numeric fields rendered by the tool (`current_value`, `difference`, thresholds and percentages) do not pass through this text guard and retain their ordinary numeric representation.

## Data and operational boundaries

- Use a separate, access-controlled working directory for client source files and outputs.
- A generated pack cannot be written into a version-control checkout at all. `write_review_pack` walks up from the resolved `--output` directory and refuses if any level holds `.git`, `.hg`, `.svn` or `.bzr`, before it creates anything; a `.git` file counts as well as a directory, so a worktree and a submodule are checkouts too. A level it cannot examine is refused rather than read as an absence. The library enforces this too, so a caller that bypasses the CLI does not bypass the rule.
- Keep this checkout limited to fabricated fixtures. Its `.gitignore` blocks CSVs outside `examples/` and `schemas/`, names `close-review-pack.json` and `close-summary.md`, and names `exceptions.csv` and `client-queries.csv` inside those two re-included directories, so all 4 generated pack files are covered. That stays as a second line: it catches a pack copied in by hand, which no guard on the writer can see.
- Produce the source CSV through a read-only export workflow. Live Xero OAuth, token storage, and client authorisation are deliberately outside this MVP.
- Do not use this as tax, financial, audit, or legal advice. It is a configurable review aid that requires professional judgement.

## Development

```bash
python -m pip install -e ".[dev]"
pytest
python -m build
```

The test suite covers schema gates, exact balancing, variance and metadata exceptions, mapping and subledger checks, deterministic pack generation, acknowledgement parsing, and the command-line exit contract.

Continuous integration verifies the committed `uv.lock`, runs the test suite on Python 3.10, 3.11, 3.12, and 3.13, then builds and smoke-tests the wheel with the fabricated demo. CodeQL scans the Python source, and Dependabot is configured to propose updates for `uv` dependencies and pinned GitHub Actions. See [CONTRIBUTING.md](CONTRIBUTING.md) for the local verification and data-handling requirements.

## Related

The next layers remain separate components. This project stays a local review-pack generator; it does not grow a Xero client or a tax-advice engine.

- [Workpaper Review Gate](https://github.com/ryanduguid/accounting-review-pipeline/tree/main/packages/review-ready-gate) - the upstream pack-readiness gate. It answers whether a BAS, month-end, or year-end folder is allowed onto the review desk. This tool still answers a later question: what material exceptions exist on these trial balances. Run the gate first.
- [Xero Ledger Review Gate](https://github.com/ryanduguid/accounting-review-pipeline/tree/main/packages/elizabeth-anne-alexander) - a fixed-policy, synthetic-data review boundary for AI-assisted trial-balance analysis. No OAuth, no mutation tools.
- [Tax Radar AU](https://github.com/ryanduguid/au-tax-legislation-corpus/blob/main/RADAR.md) - a provenance-first monitor that turns source-version metadata into a technical-review queue, now maintained inside the Australian tax legislation corpus.

[examples/close-loop.md](examples/close-loop.md) runs close-control and the co-located gateway on local files: `close-control` reviews the gateway's May/June sample CSVs, then the gateway evaluates its own bundled context.

## Roadmap

The next layers are deliberately separated from the control engine. The pipeline components above are co-located without sharing runtime authority, and Tax Radar AU remains separate. The close-loop example runs the gateway on local files only.


MIT licensed. Boundary statement: [DISCLAIMER.md](DISCLAIMER.md).
