# Grant acquittal workpapers

Prepare a local workpaper from a reviewed agreement summary, ledger expenses, explicit allocations and funding receipts. It uses Python's standard library and makes no network requests.

```powershell
python grant_workpaper.py --input examples/two-grants --output ../grant-workpaper-demo
python -m unittest discover -s tests -v
```

Run these commands from the grant checkout. The fabricated example returns
REVIEW (exit 2) and saves 5 review items. RECONCILED exits 0 and malformed input
exits 1. These describe the supplied arithmetic and evidence, never grant
eligibility or acquittal approval. Choose a new output folder for each run.

For a locked setup, use `uv sync --locked` and run the same commands through
`uv run --locked`. The package has no runtime dependencies.

## Run all joined accounting examples

From this checkout, with Python 3.11 or later, Git and uv on PATH:

```powershell
python setup_utility.py --workspace ../accounting-utility-demo
```

This optional development command clones the public Accounting Review Pipeline,
au-fpa-pack and australian-accounting repositories at `main`. It runs the close,
quarter, WIP cash and grant cash examples with fabricated inputs and this grant
checkout. Git and uv download source and dependencies; the accounting engines
continue to use local files. No new credential or access token is needed.

Choose a new workspace outside every Git checkout. Setup never updates existing
checkouts or reuses an output directory. An inconclusive Git check stops setup
before it creates the workspace. Results are in `results/`, cloned sources
in `sources/` and isolated environments in `environments/`. The results manifest
records all 19 workflow commands, four runtime probes, source revisions, locks
and output hashes. A failed run retains its workspace and any workflow diagnostics;
correct the cause and use a new workspace path to retry.

The `Joined accounting examples` workflow runs the same command on Linux and
Windows for PRs, pushes to `main`, manual runs and daily at 19:23 UTC. The daily
schedule starts after this workflow reaches `main`. Each run follows the latest
public companion `main` revisions, so their changes are checked by the next
scheduled run rather than on each companion PR. Logs and seven-day fabricated
result artefacts remain in this private repository. Existing component tests
remain separate from the joined integration run.

The pipeline runner checks the fixed examples' expected periods, cash balances,
command coverage and retained review findings before it writes a success
manifest. `results/summary.md` reports those checks, cash results and source
revisions. The workflow appends this summary to its private Actions job page,
including the failed command when available. Detailed subprocess diagnostics
stay in the retained files. A passing fixture check never approves accounting.

To repeat the source revisions from a previous complete run:

```powershell
python setup_utility.py --workspace ../accounting-utility-replay --replay-manifest ../accounting-utility-demo/results/manifest.json
```

Replay accepts only full commit IDs from an all-route `utility-workflows.v2`
manifest whose four source checkouts were clean. It fetches the three public
commits from the fixed repositories and the grant commit from this local
checkout, each into a new directory. It does not change this checkout. The grant
commit must still be available locally; unavailable commits fail explicitly.
The runner also saves `results/replay.json` before running commands. Use it with
`--replay-manifest` when a failed all-route run has no success manifest. Failures
before source evidence is collected do not have a replay record.
Manifest URLs and command arguments are never used to choose a download or
execute a command. Only replay manifests from runs and revisions you trust.

Both normal setup and replay require a pipeline revision with the fixture
result checks. An older runner without `fixture_validation: passed` is refused.
Replay pins source commits and their locks, not the host operating system,
Python patch release or external tool availability. It does not resume a failed
run or reconstruct uncommitted source edits. Without `--replay-manifest`, setup
continues to use the public companions' latest `main` branches.

For a local summary file, add `--summary ../accounting-summary.md`. It appends to
that file, which must sit outside the grant checkout and new workspace.
Failure before the runner starts produces a setup-failure summary without
claiming that results were verified.
If detailed summary text is missing after successful verification, the appended
fallback identifies the manifest as the result record. A summary read or write
error returns exit 1 and reports the setup outcome separately, so a completed
run and its results remain identifiable.

The au-fpa-pack source example `examples/restricted-cash/grant_cash.py`
consumes the hash-bound JSON workpaper and a separate reviewed cash plan.
It retains source findings, unplanned commitments and the distinction between
cash allocations and unspent funding. The Monthly Close Controls
`examples/utility_workflows.py --workflow grant-cash` recipe runs both owners
without importing one repository into the other.

## Run grant cash with an existing checkout

After running the workpaper example above, use an existing au-fpa-pack checkout
to forecast its cash. Set `$fpa` to that checkout's path; the example below assumes
it sits beside this repository. These scripts use Python's standard library and
need no downloads. Run the commands from the grant checkout:

```powershell
$fpa = "../au-fpa-pack"
$workpaper = "../grant-workpaper-demo/workpaper.json"
$digest = (Get-FileHash -LiteralPath $workpaper -Algorithm SHA256).Hash.ToLowerInvariant()
python "$fpa/examples/restricted-cash/grant_cash.py" --workpaper $workpaper --workpaper-sha256 $digest
```

The command prints JSON using the bundled fabricated cash plan. With the supplied
fixtures, closing bank cash is $34,800, the restricted allocation is $26,700 and
cash available under the assumptions is $8,100. It retains all 5 source review
items and shows $3,000 of unresolved unpaid allocations outside planned cash.
A RECONCILED cash forecast does not clear the workpaper's REVIEW status. The
digest binds the forecast to the file you supplied; it does not approve its data.

## Inputs and review boundary

The 4 files under `examples/two-grants/` are the exact version 1 contracts. Use one entity and AUD. Ledger expense amounts and cash allocations are GST-inclusive under an explicit supplied agreement basis. Positive expenditure and funding receipts are supported; refunds, recoverable GST adjustments and foreign exchange require a later contract. Missing amounts fail. Nil cash paid is valid.

Agreements carry their own start/end dates, opening cash allocation, opening unspent funding and reviewed budget lines. Those opening balances are supplied independently at the ledger period's start. Different grant periods do not change the ledger's financial-year scope. An allocation outside its agreement period remains in the tie-out and is flagged separately.

Each allocation must name its ledger row, grant and budget line. Expenditure and cash allocations cannot exceed their source amounts across all grants. Unallocated amounts, missing evidence, unapproved allocations, over-budget amounts and funding shortfalls remain visible. Approval is an input recording a review decision; the program never grants it.

Optionally add `contributions.csv` with the headings
`contribution_id,grant_id,date,kind,amount,evidence`, where `kind` is
`interest`, `cash_contribution` or `in_kind`. Each row must name a declared
grant, fall inside the ledger period and carry a positive amount; blank evidence
is a review finding. Contributions are reported per grant and never added to
the grant's cash or unspent funding: whether interest must be spent on the
activity or returned, and how a co-contribution or in-kind value counts, are
terms of the agreement for the reviewer to apply. Without the file, every
contribution column is nil. An unreadable path or dangling link is an input
error; no workpaper is published. Readable file links remain supported.

## Outputs

- `workpaper.md` and `workpaper.json`: reconciliation, review items and source SHA-256 values.
- `allocation_evidence.csv`: agreement-to-expense references and supplied decisions.
- `budget_lines.csv`: total allocation, within-period allocation and approved/evidenced allocation, kept separate.
- `ledger_tieout.csv`: every source row, allocation and remainder.
- `funding_movements.csv`: opening funding, receipts, expenditure and cash movements.
- `contributions.csv`: interest, cash co-contributions and in-kind contributions per grant.
- `findings.csv`: items requiring review.

CSV text that could start a spreadsheet formula is escaped. JSON retains the supplied values. Keep all real workpapers and inputs outside this repository.

Required identities and evidence must contain visible text. Unicode letters and
combining accents are supported; control and format characters, line separators
and combining marks without a base character are refused. Blank optional CSV
evidence stays an explicit review finding. Text validation cannot establish that
a referenced agreement exists or that a supplied review decision is justified.

Outputs are staged and published with an exclusive directory rename. An existing
or concurrently created destination is preserved, including an empty directory.
Windows, Linux and macOS are supported. A filesystem or operating system without
exclusive rename support fails without publishing a partial workpaper. Linux uses
[RENAME_NOREPLACE](https://man7.org/linux/man-pages/man2/rename.2.html); macOS uses
[RENAME_EXCL](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/stdio.h).

The fabricated ledger totals $6,500. Allocations total $6,300 and $200 remains unallocated. Grant A closes with $14,220 cash allocated to it but $9,100 unspent funding on the expenditure workpaper. Grant B closes at $7,680 cash and $7,600 unspent funding. The differences arise from unpaid expenditure and independently supplied opening balances. Neither schedule determines recognised income or a liability.

## Verification and development

Run the standard-library unittest suite above. No external service, credential, model judgement or real client data is required. The tests include excessive shared allocations, partly paid expenditure, different grant periods, budget and agreement-date boundaries, cash-only and unspent-only shortfalls, missing evidence, duplicate source IDs, CSV injection (a negative figure stays a number) and refusing to overwrite a prior run. No live agreement or independent practitioner evaluation is claimed.

GitHub Actions runs the suite with Python 3.10 and 3.14, including a competing
destination created immediately before publication. Pull requests run on Linux;
pushes to `main` and manual runs also test Windows. It does not run macOS, so the
`RENAME_EXCL` path is untested there.
It also runs Ruff with a pinned version. Local lint uses
`uv run --with ruff==0.16.7 ruff check .`.
