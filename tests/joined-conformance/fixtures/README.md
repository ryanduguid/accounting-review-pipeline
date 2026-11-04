# Joined conformance vectors

Fabricated trial balances for what the [contract's 3 fixtures](../../../contracts/xero-trial-balance-v1/README.md) leave open: quoted CSV, leading-zero account codes, Unicode text and numeric spellings. They use the same 10-column header. They sit here because `contracts/xero-trial-balance-v1/` stays byte-identical: its digests are cited in exporter release evidence.

`expected_results.json` records what each consumer does with each file. `src/vectors.rs` in this crate holds Monthly Close Controls and the Workpaper Review Gate to it, and `test_gateway_satisfies_the_joined_vectors` holds the Xero Ledger Review Gate to it. `SHA256SUMS` pins the files, and the joined conformance workflow checks it.

| Vector | Monthly close and readiness gate | Ledger review gate |
| --- | --- | --- |
| `quoted_csv.csv` | Accepted | Accepted |
| `leading_zero_codes.csv` | Accepted, codes kept as text | Accepted, codes kept as text |
| `unicode_text.csv` | Accepted | Accepted |
| `plain_spellings.csv` | Accepted | Accepted |
| `grouped_spellings.csv` (`$1,200.00`) | Accepted | Refused |
| `exponent_spelling.csv` (`1.2E+3`) | Refused | Accepted |

The last 2 rows are a real difference between the consumers, recorded here rather than changed: each loader keeps its own released behaviour. The exporter writes neither spelling. Excel and Power BI read these files through Power Query, which needs native Excel or Power BI, so no vector covers them.
