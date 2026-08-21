# Local-file monthly close loop

This checkout does not connect to Xero. There is no OAuth step, token file, or live ledger call.

The loop is two local commands over fabricated trial-balance CSVs that share the ten-column contract:

```text
ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit
```

1. `close-control review` in this repository, pointed at the sibling gateway's same-financial-year sample pair.
2. `elizabeth-anne-alexander evaluate` in [`ElizabethAnneAlexander`](https://github.com/ryanduguid/ElizabethAnneAlexander), still pointed at **that package's bundled** `samples/` context. Do not pass this repo's `examples/` as `--context`.

A Windows driver for the same steps is [close-loop.ps1](close-loop.ps1). Set `ELIZABETH_ANNE_ALEXANDER_ROOT` if the gateway checkout is not a sibling of this repository.

## Why the Acme June/July files cannot feed the gateway

This repo's quick-demo pair is the wrong pair for the gateway step:

- `examples/prior_trial_balance.csv` is Acme Demo Pty Ltd at 2026-06-30
- `examples/current_trial_balance.csv` is Acme Demo Pty Ltd at 2026-07-31

Those dates straddle the Australian financial-year reset on 1 July. Close-control will still write a pack, and it will raise `financial_year_reset` because YTD figures restart. The gateway **refuses** a current/prior pair in different financial years (unless they are the same calendar day and month, for a year-on-year comparison). Feeding it the Acme files would not be a variance review; it would be a blocked run.

The gateway also does not take arbitrary CSV paths as `--context`. `evaluate` resolves `--context`, `--request`, and `--policy` against data bundled inside the installed package. Those bundled manifests already name the May/June Demo Entity samples. Pointing `--context` at close-control `examples/` is unsupported.

Use the gateway's same-FY pair instead:

| Role | File | ReportDate |
| --- | --- | --- |
| Current | `elizabeth_anne_alexander/samples/inputs/sample-tb-2026-06-30.csv` | 2026-06-30 |
| Prior | `elizabeth_anne_alexander/samples/inputs/sample-tb-2026-05-31.csv` | 2026-05-31 |

Both dates sit in FY2025 (1 July 2025 to 30 June 2026). Close-control should report `REVIEW` with no `financial_year_reset` exception.

## Command 1: close-control review

From this repository root, after `python -m pip install -e ".[dev]"`:

```bash
close-control review \
  --current ../ElizabethAnneAlexander/elizabeth_anne_alexander/samples/inputs/sample-tb-2026-06-30.csv \
  --prior ../ElizabethAnneAlexander/elizabeth_anne_alexander/samples/inputs/sample-tb-2026-05-31.csv \
  --output outputs/gateway-tb-loop
```

Leave the default materiality thresholds ($1,000 absolute and 10%). The command exits `2` because the pack is `REVIEW`, not because the files are invalid. It writes `close-summary.md`, `exceptions.csv`, and `close-review-pack.json` under `outputs/gateway-tb-loop`.

Do not pass this repo's Acme mapping, subledger, or review note: those fixtures belong to a different tenant and a different date pair.

## Command 2: gateway evaluate (bundled context)

Install the sibling package, then run its documented evaluate. Relative `samples/` and `policy/` paths resolve against the bundled package data, not against this checkout's `examples/`:

```bash
elizabeth-anne-alexander evaluate \
  --context samples/contexts/sample-monthly-variance.context.json \
  --request samples/requests/sample-revenue-variance.request.json \
  --policy policy/demo-policy-v1.json \
  --out build/gateway-tb-loop
```

`--out` is created under `build/` in the working directory. That is the gateway's output contract; it is not this package's review pack.
