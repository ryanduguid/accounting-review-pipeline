# Joined conformance vectors

Fabricated trial balances for what the [contract's 3 fixtures](../../../contracts/xero-trial-balance-v1/README.md) leave open: quoted CSV, leading-zero account codes, Unicode text, numeric spellings and 8 rejection cases. They use the same 10-column header. They sit here because `contracts/xero-trial-balance-v1/` stays byte-identical: its digests are cited in exporter release evidence.

`expected_results.json` records what each consumer does with each file, and the debit, credit and year-to-date amounts each accepted row must parse to. `src/vectors.rs` in this crate holds Monthly Close Controls and the Workpaper Review Gate to it, and `test_gateway_satisfies_the_joined_vectors` holds the Xero Ledger Review Gate to it. `SHA256SUMS` pins the files, and the joined conformance workflow checks it.

| Vector | Monthly close and readiness gate | Ledger review gate |
| --- | --- | --- |
| `quoted_csv.csv` | Accepted | Accepted |
| `leading_zero_codes.csv` | Accepted, codes kept as text | Accepted, codes kept as text |
| `unicode_text.csv` | Accepted | Accepted |
| `plain_spellings.csv` | Accepted | Accepted |
| `grouped_spellings.csv` (`$1,200.00`) | Accepted | Refused |
| `exponent_spelling.csv` (`1.2E+3`) | Refused | Accepted |
| `failing_header_order.csv` (debit and credit columns swapped) | Accepted, read by name | Refused |
| `failing_duplicate_account.csv` | Refused | Refused |
| `failing_number.csv` (`12O0.0`) | Refused | Refused |
| `failing_date.csv` (`2026-06-31`) | Refused | Refused |
| `failing_missing_column.csv` (no `AccountCode` column) | Refused | Refused |
| `failing_extra_cell.csv` (an 11th cell on each row) | Refused | Refused |
| `failing_blank_account_id.csv` | Refused | Refused |
| `failing_mixed_tenants.csv` (2 tenants in one file) | Refused | Refused |

The `exponent_spelling.csv`, `grouped_spellings.csv` and `failing_header_order.csv` rows are a real difference between the consumers, recorded here rather than changed: each loader keeps its own released behaviour, although the contract calls its header ordered. The exporter writes none of these spellings. Excel and Power BI read these files through Power Query, which needs native Excel or Power BI, so no vector covers them:

- **Excel adapter:** `Xero.TrialBalance.pq` finds a header whose first field is `Account`, so these 10-column files are not its input. No outcome is recorded.
- **Power BI:** its contract test holds the contract's 3 files to the exact header, stable identity and decimal amounts. It does not read these files, so no outcome is recorded.
