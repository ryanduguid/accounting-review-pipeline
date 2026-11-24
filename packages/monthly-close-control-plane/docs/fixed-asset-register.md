# Fixed-asset register tie-out

`close-control fixed-assets` ties one tenant's fixed-asset register to the
trial balance at its `ReportDate`. For each cost account it compares the
closing balance (`YTDDebit - YTDCredit`) with the cost of the assets in service
that the register places there; for each accumulated depreciation account it
compares the credit balance with their accumulated depreciation. It reads local
files, writes JSON to standard output and changes nothing. It exits `0` for
`PASS`, `2` for `REVIEW` and `1` for malformed input or options.

From the component's source:

```text
cargo run --locked --release --bin close-control -- fixed-assets --register examples/fixed_asset_register.csv --trial-balance examples/fixed_asset_trial_balance.csv --currency AUD
```

The fabricated register holds 5 assets at 30 September 2026. The command exits
`2` and lists a van sold in August whose cost (28,000.00) and accumulated
depreciation (24,500.00) are still in the ledger, and laptops depreciated past
their cost.

## Supply the register at the trial balance's date

```text
Tenant,AssetID,Description,CostAccountID,DepreciationAccountID,Cost,AccumulatedDepreciation,AcquisitionDate,DisposalDate,Currency
```

The trial balance is the canonical CSV the `review` command reads. Each
`AssetID` appears once, `Cost` is positive and `AccumulatedDepreciation` zero
or more, each with at most 2 decimal places, and every row carries the
declared `--currency` and the trial balance's `Tenant`. `DisposalDate` is empty
for an asset still held. An asset acquired after the `ReportDate`, a disposal
before its acquisition, one account named as an asset's cost and depreciation
account, or one account used as a cost account by one asset and a depreciation
account by another, is refused.

An asset disposed of on or before the `ReportDate` leaves the register's
totals, so its account must have left the ledger too. An asset disposed of
after it is still in service.

## What is listed

| Code | Listed when |
| --- | --- |
| `COST_DIFFERENCE` | a cost account's closing balance is not the cost of the assets in service in it. |
| `ACCUMULATED_DEPRECIATION_DIFFERENCE` | an accumulated depreciation account's credit balance is not the accumulated depreciation of the assets in service in it. |
| `ACCOUNT_NOT_IN_TRIAL_BALANCE` | an account the register names has no trial-balance row. |
| `DEPRECIATION_EXCEEDS_COST` | an asset in service has more accumulated depreciation than cost. |

An account finding carries `account_id`, `account_name`, `role`,
`assets_in_service`, `register_total`, `ledger_balance` and `difference`
(ledger less register); an asset finding carries the asset's ID, description,
cost and accumulated depreciation. Balances must agree to the cent.

The result is `PASS` only when at least one asset is in service and nothing is
listed. A tie does not show that the register is complete, that any cost is
right or that depreciation was calculated correctly: an asset missing from both
the register and the ledger is not seen, and nothing is inferred from account
names or sections, so a fixed-asset account the register never names is not
compared.

## Output

The JSON carries `status`, `tenant`, `currency`, `report_date`, the `boundary`
statement, both files' SHA-256 under `source_sha256`, `asset_count`,
`assets_in_service`, `accounts` (every account the register names, tied or
not) and `findings`.
