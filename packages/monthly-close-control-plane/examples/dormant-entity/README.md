# Dormant entity, August 2026

Fabricated Banksia Dormant Pty Ltd has no linked accounting organisation. The
reviewer has 2 manually prepared trial balances and one independent cash
confirmation. From this directory, run the commands below with a source build
of `close-control`. Choose a fresh output directory.

```powershell
close-control review --current current.csv --prior prior.csv --subledger bank-confirmation.csv --require-control subledger_reconciliation --require-control equity_reconciliation --output august-review
close-control view --pack-dir august-review
```

The first command returns `REVIEW` (exit 2). The second verifies and displays
the written pack. In `close-review-pack.json`, `required_controls` records
`subledger_reconciliation` as `ran` and `equity_reconciliation` as `skipped`.

| Workpaper total | Source | Review evidence |
| --- | --- | --- |
| August cash, $100.00 debit | `current.csv`, account A1, YTDDebit | The independent cash confirmation in `bank-confirmation.csv`, A1, agrees to the cent. |
| August issued capital, $100.00 credit | `current.csv`, account E1, YTDCredit | The trial balance balances. A share register and an equity movement schedule are still outstanding. |
| Period movement, $0.00 | Debit and Credit on A1 and E1 in both trial balances, with the same YTD balances at 31 July and 31 August | The supplied accounts show no movement. Establishing that all accounts are present requires additional evidence. |

The reviewer can trace both balances back to the supplied rows and request
the missing equity support before signing their own workpaper.
