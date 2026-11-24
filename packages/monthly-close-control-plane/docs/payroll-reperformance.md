# Payroll reperformance

`close-control payroll` reads one tenant's payroll register and reperforms 2
amounts on each pay line: the PAYG withholding the ATO's Schedule 1 formulas
give for the line's earnings, and the super guarantee due on its qualifying
earnings. It lists each line worth a reviewer's look. It reads a local file,
writes JSON to standard output and changes nothing. It exits `0` for `PASS`,
`2` for `REVIEW` and `1` for malformed input or options.

From the component's source:

```text
cargo run --locked --release --bin close-control -- payroll --register examples/payroll.csv
```

The fabricated register holds 4 July 2026 pay lines. The command exits `2` and
lists a fortnightly line withheld $6 under the formula, and a weekly line whose
super is $10 short and reached the fund 12 days after payday. The monthly line
declares tax offsets, so its withholding is not reperformed.

## Supply the register

```text
Tenant,EmployeeID,PayDate,Frequency,Scale,Adjusted,Earnings,Withheld,QualifyingEarnings,Super,FundReceived
```

- `PayDate` is from 1 July 2026 to 30 June 2027. The command holds only the
  2026-27 rates it was built from and refuses any other pay date, so review
  other years by hand.
- `Frequency` is `WEEKLY`, `FORTNIGHTLY`, `MONTHLY` or `QUARTERLY`.
- `Scale` is the Schedule 1 scale: `1` (tax-free threshold not claimed), `2`
  (claimed), `3` (foreign resident), `4R` and `4F` (no TFN, resident and
  foreign resident), `5` (full Medicare levy exemption) or `6` (half
  exemption).
- `Adjusted` is `Y` when the payee claims tax offsets, a Medicare levy
  adjustment, has a study and training support loan, or has another
  variation the formulas leave out. Its withholding is
  then not reperformed and is counted in `payg_not_reperformed`.
- `Earnings` is the payment and any allowances subject to withholding.
  `Withheld`, `QualifyingEarnings` and `Super` are the amounts recorded. Every
  amount is non-negative with at most 2 decimal places.
- `FundReceived` is the date the fund received the contribution, or blank.

## What it lists

| Code | When |
| --- | --- |
| `PAYG_DIFFERENCE` | `Withheld` differs by more than $1 from the Schedule 1 formula's amount. The ATO accepts the tax table's footnote method too, which can differ slightly above $3,400 a week. |
| `SG_SHORTFALL` | `Super` is less than 12% of the line's qualifying earnings, rounded to the cent, counted up to the 2026-27 maximum contribution base of $270,830 per employee. |
| `SG_NOT_RECEIVED` | Super is due on the line and `FundReceived` is blank. |
| `SG_TIMING_REVIEW` | The fund received the contribution more than 7 calendar days after payday. |

Withholding follows Schedule 1 (published 17 June 2026, for payments made from
1 July 2026): weekly earnings in whole dollars plus 99 cents, `y = a x - b`
for the scale's bracket, rounded to the nearest dollar, then converted back to
the pay period. Fortnightly, monthly and quarterly earnings use the weekly
equivalent the schedule sets out. Scale 4 withholds 47% or 45% of whole-dollar
earnings, cents ignored.

The super guarantee is 12.00% of qualifying earnings for 1 July 2026 to 30 June
2027, and a contribution is on time if the fund receives it within 7 business
days of payday, or 20 business days for some new employees and funds (ATO
super guarantee and payday super pages, read 24 November 2026).

## Limits

- The maximum contribution base is applied over the lines supplied, in pay
  date order. A register that starts after 1 July can overstate the super due
  once an employee has passed the base earlier in the year.
- Timing is checked in calendar days. A receipt within 7 calendar days is
  always within 7 business days; a later one is listed for review, because
  whether it is late depends on public holidays and the employee's history,
  which the command does not read. A listed line is not a finding of a late
  payment, and a line not listed is not shown to be on time.
- Tax offsets, Medicare levy adjustments, study and training support loans,
  the 53rd-pay addition and other variations are not reperformed.
- The command does not decide who is an employee, what counts as qualifying
  earnings or whether the super guarantee charge applies.
