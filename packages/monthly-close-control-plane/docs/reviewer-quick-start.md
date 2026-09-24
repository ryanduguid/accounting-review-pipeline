# Reviewer quick start

This guide is for a reviewer who wants to see what Monthly Close Controls
produces without cloning the repository or writing Python. It runs the
published 0.1.5 release on fabricated trial balances. Review aid, not
professional advice; the reviewer decides whether a close is acceptable.

## What you need

- [uv](https://docs.astral.sh/uv/getting-started/installation/). `uvx` runs
  the published release in a temporary environment and installs nothing else.
- An empty folder that is not inside a Git, Mercurial, Subversion or Bazaar
  checkout. The command refuses to write a review pack inside one, because a
  pack names accounts and balances.

## 1. Download the fabricated inputs

The four files come from the commit that published 0.1.5. They describe a
fictional company, Varrock Ventures Pty Ltd, at 30 June and 31 July 2026.

PowerShell:

```powershell
$base = "https://raw.githubusercontent.com/ryanduguid/accounting-review-pipeline/95030019c1d61aca21502956eadfb6f3678681a3/packages/monthly-close-control-plane/examples"
foreach ($f in "current_trial_balance.csv", "prior_trial_balance.csv", "account_mapping.csv", "subledger_balances.csv") { Invoke-WebRequest "$base/$f" -OutFile $f }
```

bash:

```bash
base=https://raw.githubusercontent.com/ryanduguid/accounting-review-pipeline/95030019c1d61aca21502956eadfb6f3678681a3/packages/monthly-close-control-plane/examples
for f in current_trial_balance.csv prior_trial_balance.csv account_mapping.csv subledger_balances.csv; do curl -fsSLO "$base/$f"; done
```

## 2. Run the review

```bash
uvx --from monthly-close-control-plane==0.1.5 close-control review --current current_trial_balance.csv --prior prior_trial_balance.csv --mapping account_mapping.csv --subledger subledger_balances.csv --absolute-threshold 10000 --percentage-threshold 0.10 --reconciliation-tolerance 0.01 --output close-pack
```

It prints `close-control: REVIEW; 8 exception(s); 6 client query(ies) drafted`
and exits 2. Exit 2 means the pack needs review; it does not mean the command
failed. Exit 0 is kept for a pack with no exceptions and exit 1 for bad input.

## 3. Read the result

`close-pack` now holds four files:

| File | Open it to answer |
| --- | --- |
| `close-summary.md` | What needs my attention this close? |
| `exceptions.csv` | Which accounts, by how much and why? Filter it in Excel. |
| `client-queries.csv` | What would I ask the client? Drafts only; nothing is sent. |
| `close-review-pack.json` | Exactly which files, thresholds and hashes did this run use? |

The 8 exceptions are one unmapped account, a comparison that crosses the
1 July financial-year reset, five movements above both thresholds, and a
$250.00 difference between Trade Creditors and the supplied subledger.
Eight exceptions are not eight errors. The
[manager case study](manager-case-study.md) works through what a reviewer
would do with each one.

## 4. Decide the next action

The pack does not decide anything. For each exception, the reviewer explains
it, returns it to the preparer or asks the client. Record that decision in
the firm's own workpaper, not in the pack.

## Keep the pack with the job

Store the four files together, unedited, as one folder in the job's document
store: a practice management system, document manager or file share. This is
a file handoff, not an integration with any practice management product.

To check a stored pack later, run:

```bash
uvx --from monthly-close-control-plane==0.1.5 close-control view --pack-dir close-pack
```

It prints the review sheet and exits 0 when the four files still agree. If
any file was edited after the run, it exits 1 with `verification failed` and
names the disagreement. The check compares the four files with each other; it
does not re-read the trial balances. To confirm the inputs themselves, compare
each input file's SHA-256 (`Get-FileHash` in PowerShell, `sha256sum` in bash)
with the source evidence listed in `close-summary.md`.

## Tell us what happened

If you try this, the most useful feedback is how long setup took, any
exception you think is wrong or missing, and whether you would run it on a
second close. Open an
[issue](https://github.com/ryanduguid/accounting-review-pipeline/issues).
Never attach client data.
