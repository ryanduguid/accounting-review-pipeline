# Equity-movement reconciliation proposal

Status: implemented in the source checkout on 21 September 2026, not released.
This document retains the design rationale. The current input, output and
verification contract is in [Equity reconciliation](equity-reconciliation.md).

## Purpose and boundary

Check whether an independently prepared movement schedule explains the change in
selected ledger equity accounts between the supplied prior and current trial
balances. A balanced trial balance can still contain an unexplained equity entry.
This control would identify that difference for review.

The first version should support one entity, one declared currency and an explicit
set of ledger equity accounts. It should remain offline and use exact `Decimal`
arithmetic. It would not calculate tax, translate currencies, consolidate entities,
post adjustments or approve a close.

This is a **ledger equity** reconciliation. Include profit or loss only when a
transfer into the selected equity accounts is evidenced in the movement schedule.
Do not automatically add unclosed P&L to equity: the supplied ledger might already
include a current-earnings account, and adding it again would double count it.

## Proposed input

Add an optional `--equity-schedule PATH` argument to `review` and `workbench`,
with a separate `--equity-currency CODE` run declaration confirmed against the
source reporting currency of both trial balances. Record the supporting reference
with `--equity-currency-evidence REFERENCE`.
The schedule argument would read one local JSON document containing:

| Field | Contract |
| --- | --- |
| `schema_version` | Integer `1`; reject unsupported versions and unknown fields. |
| `tenant` | Exact match to the single entity in both trial balances. |
| `opening_date`, `closing_date` | ISO dates matching the prior and current report dates; closing must follow opening. |
| `currency` | Must match the separately confirmed trial-balance currency supplied for the run. Missing or mismatched currency produces `BLOCKED`. No FX conversion. |
| `basis` | Fixed value `ledger_equity`. |
| `prior_source_sha256`, `current_source_sha256` | Digests matching the exact trial-balance byte snapshots used in the run. |
| `equity_account_ids` | Non-empty, unique list of stable account IDs present in both trial balances. Do not infer accounts from their names or review-group labels. |
| `complete` | Boolean statement by the preparer that the schedule covers the whole interval and selected account set. |
| `movements` | Array of movement records. An empty array with `complete: true` explicitly declares no movements. |

Each movement would contain `movement_id`, `date`, `account_id`, `debit`, `credit`,
`description` and `evidence_reference`. Require unique movement IDs, a listed
account ID, a date after opening and on or before closing, and non-empty description
and evidence reference. Debit and credit must be finite, non-negative decimal
strings, with exactly one side positive. The records describe the equity legs;
their debits and credits need not balance against each other.

Prepare this schedule from separate supporting records. A schedule generated as
the difference between the same two trial balances cannot test that difference.
Evidence references are local review labels, not commands or URLs to fetch. The
tool would not authenticate those records or prove that the preparer's completeness
statement is true. The ten-column trial-balance format does not carry currency.
The preparer must confirm the reporting currency from source report metadata or
separate supporting evidence for both snapshots and record that reference with
the run declaration. Do not derive the run currency from the schedule itself.
Absent confirmation, absent evidence or a currency mismatch must produce
`BLOCKED`, even if the amounts reconcile. Include the run currency and its
evidence reference in the result. The tool can check agreement between these
inputs, but cannot authenticate the source evidence.

Account additions, removals, section changes and transfers requiring a changed
equity account set should produce `BLOCKED` in this first version. Supporting those
cases needs an explicit account-continuity contract; silent zero balances would
hide missing evidence. A section that explicitly identifies an account as an asset,
liability, revenue or expense must not be accepted as equity merely because its ID
appears in the schedule.

## Calculation and evidence

Use credit-positive amounts for the displayed reconciliation:

```text
opening_equity = sum(prior YTDCredit - prior YTDDebit for selected accounts)
movement      = credit - debit for each schedule record
expected_close = opening_equity + sum(movement)
actual_close   = sum(current YTDCredit - current YTDDebit for selected accounts)
unexplained    = actual_close - expected_close
```

Calculate each account separately before the aggregate. Equal and opposite errors
in different equity accounts must not cancel out of the findings. Report opening,
scheduled movement, expected closing, actual closing, unexplained difference and
tolerance for each account and for the total, with source and movement references.

For example, a fabricated account opens at $100,000. Its schedule contains a
$25,000 credit contribution, $10,000 debit withdrawal and a $15,000 credit transfer
from profit. Expected closing is $130,000. An actual closing balance of $129,750
leaves a negative $250 difference for review. Profit appears once, as an evidenced
transfer, rather than as a second figure inferred from the trial balance.

## Result and exit behaviour

| Condition | Proposed behaviour |
| --- | --- |
| No schedule argument | Preserve existing output and exit behaviour. Do not imply that this optional control ran. |
| Valid complete schedule, confirmed matching currency, each account and total within tolerance | Control result `PASS`; other controls still determine the overall pack state. |
| Any account or aggregate absolute difference exceeds tolerance | `REVIEW`, even when the aggregate difference is zero. |
| Incomplete schedule, missing or mismatched currency confirmation, mismatched snapshot/date/entity, or unsupported account continuity | `BLOCKED`; do not calculate a passing substitute. |
| Combined control status | Include the equity result in the existing `_overall_status` aggregation: `BLOCKED > REVIEW > PASS`. Any equity or other control result of `REVIEW` or `BLOCKED` prevents overall `PASS` and exit `0`. |
| Malformed JSON, unknown fields, duplicate identifiers, missing required fields, invalid dates or monetary strings | Input error, exit `1`, and no partial replacement of an existing pack. |

Reuse `--reconciliation-tolerance`, including its existing finite, non-negative
validation. Require `abs(unexplained) <= tolerance` for every account and for the
aggregate. A positive or negative difference outside that bound requires `REVIEW`;
equality at either bound passes.
Preserve exit `0` only for overall `PASS`, exit `2` for `REVIEW` or `BLOCKED`, and
exit `1` for malformed input or failed output. A reviewer acknowledgement must not
change a result or confer accounting approval.

## Implementation scope

- Extend `closecontrol/loader.py` with the strict schedule parser, reusing
  `SourceSnapshot` so parsing and the recorded hash use the same bytes.
- Add the account reconciliation in `closecontrol/engine.py` and its evidence
  model in `closecontrol/models.py`. Keep it separate from variance thresholds.
- Extend the shared review arguments in `closecontrol/cli.py` for explicit
  schedule selection and a separate currency declaration with its source evidence
  reference. Preserve behaviour when the schedule option is absent. The
  quarantined `openaccountants-au` entry point is outside this change.
- Update `closecontrol/report.py` and `closecontrol/viewer.py` together so the JSON,
  Markdown and CSV outputs expose and cross-check the new evidence, including
  client-query evidence in `client-queries.csv`. Follow the
  existing atomic output and spreadsheet-formula neutralisation rules.
- Agree and document the additive pack schema before implementing it. Successful
  reconciliations need visible evidence, rather than appearing only as an absence
  of exceptions. Check all pack consumers against that schema.
- Add fabricated examples, input-contract documentation and focused tests under
  this component. Do not change sibling packages' runtime dependencies.

## Acceptance checks for that implementation

1. Reproduce the worked example with independent decimal arithmetic.
2. Cover a matching schedule, a loss transfer, a debit equity balance, explicit
   zero movements, sub-cent values, and positive and negative differences at and
   just beyond tolerance for both individual accounts and the aggregate.
3. Detect equal and opposite differences across accounts, duplicate movements,
   excluded account IDs, wrong signs, non-finite numbers and incomplete evidence.
4. Block missing or mismatched currency confirmation, including numerically
   equal AUD and USD inputs. Reject stale input digests and mismatched entities
   or dates. Require reviewable evidence for a year-end transfer instead of assuming
   a P&L reset is a movement.
5. Show that an absent schedule preserves existing output. With all other controls
   passing, equity `REVIEW` must make the pack `REVIEW` and equity `BLOCKED` must
   make it `BLOCKED`, both with exit `2`. A supplied incomplete schedule cannot
   disappear from the pack or yield a passing result.
6. Verify all 4 pack files across the 3 output formats, viewer tamper detection,
   safe source text rendering,
   source snapshot integrity, and preservation of the old pack on a failed write.
7. Run this component's required lint, mypy, coverage, dependency audit, build and
   clean-wheel smoke checks from `AGENTS.md`. If the shared pack contract changes,
   also run the affected consumers and joined conformance tests.

## Basis for the proposal

This adopts the separate movement-check idea from
[Clawdog's equity roll-forward](https://github.com/lodgeit-labs/clawdog/blob/89566b76a967810255256dae1cfd213f6c071530/engine/audit.pl#L177).
The implementation should use this repository's existing Python, exact arithmetic,
local evidence and review-state contracts. No Clawdog code or Prolog dependency is
needed.
