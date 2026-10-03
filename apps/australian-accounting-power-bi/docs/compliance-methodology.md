# Statutory and compliance methodology

This document details the Australian statutory frameworks, tax laws, and accounting standards modelled in Australian Accounting Power BI.

---

## 1. Payday Superannuation regime (in force 1 July 2026)

### Enabling legislation
- *Treasury Laws Amendment (Payday Superannuation) Act 2025* (No 57 of 2025, Royal Assent 6 November 2025), amending the *Superannuation Guarantee (Administration) Act 1992* (SGAA).
- Live commencement: **1 July 2026**.
- **First-year compliance approach**: Practical Compliance Guideline **PCG 2026/1** *Payday Super - first year ATO compliance approach* sets out how the Commissioner will direct compliance resources at SG shortfalls for qualifying earnings days falling between 1 July 2026 and 30 June 2027. It ranks employer behaviour as low, medium or high risk for that first year only. It does not alter the due date or the composition of the SG charge.

### Statutory 7-business-day rule
1. **Due Date Test**: Contributions must be **received by the employee's superannuation fund** within **7 business days** after payday (20 business days for new employees or newly nominated funds).
2. **Transit Risk**: Remittance to a commercial clearing house on day 6 is non-compliant if the fund receives the money on day 8. Transit time is the employer's risk.
3. **National business-day calendar**: The ATO excludes weekends and public holidays applying to the whole of any Australian state or territory, even if the employer operates elsewhere. Regional show and regatta days are excluded from the sample holiday list. The generator carries a dated calendar for 1 January 2026 to 31 July 2027 and refuses calculations outside it. See the [ATO payment-deadline rule](https://www.ato.gov.au/businesses-and-organisations/super-for-employers/paying-super-on-payday/payment-deadlines-for-payday-super).

The reference snapshot was checked on **25 September 2026** against Fair Work's [2026](https://www.fairwork.gov.au/employment-conditions/public-holidays/2026-public-holidays) and [2027](https://www.fairwork.gov.au/employment-conditions/public-holidays/2027-public-holidays) calendars. It includes the AFL Grand Final holiday on 25 September 2026, WA Labour Day, May Day, ACT Reconciliation Day and WA Day. Holidays applying to only part of a state remain business days. That includes WA King's Birthday and Melbourne Cup Day, which named local areas may replace with another date.

**Part-day holiday assumption:** the generator excludes the whole of 24 and 31 December 2026 for the evening holidays recorded in the official calendars. The cited ATO page does not resolve their treatment for a date-only deadline. Events whose due-date window overlaps these dates are labelled `Calendar review: part-day holidays`; the report withholds total charge and interest estimates when they are selected. The raw source estimates remain visible as provisional inputs. Verify the treatment before using these dates for an actual deadline.

The 2027 AFL date was not confirmed in the source snapshot and falls outside the supported calendar. Adding later payroll periods requires a new source check and an explicit calendar extension.

### Base earnings and rates
- **Statutory SG Rate**: **12.0%** (effective since 1 July 2025).
- **Qualifying Earnings (Code Q)**: Replaces Ordinary Time Earnings (OTE) as the statutory SG base from 1 July 2026.
- **Single Touch Payroll (STP Phase 2)**:
  - `Code Q`: Qualifying earnings paid in the pay run.
  - `Code L`: Superannuation liability accrued in the pay run.

### Superannuation Guarantee Charge (SGC) & GIC
Where a contribution is not received in time, the SG charge for the qualifying earnings day is the sum of 4 components. The pre-2026 quarterly model (shortfall on total salary and wages, nominal interest from the first day of the quarter, and a \$20 per employee per quarter administration fee) no longer applies.

1. **Individual final SG shortfalls**: the SG that remains unpaid for each employee when the Commissioner assesses, measured on **qualifying earnings**, not total salary and wages.
2. **Individual notional earnings** (SGAA s 19A): interest on each individual base shortfall, compounding daily at the GIC rate from **the day after the due day**. Accrual stops on the earlier of the day a late eligible contribution clears the shortfall and the day before assessment.
3. **Administrative uplift** (SGAA s 19B): **60%** of total individual final shortfalls plus total individual notional earnings for the day. Reduced by 20 percentage points where no Commissioner-initiated SGC assessment was in force in the 24 months ending on that day, and reduced further where a voluntary disclosure statement is lodged before assessment.
4. **Choice loading** (SGAA s 20A): **25%** of the affected contributions where the choice of fund rules were not met. Nil throughout these fixtures, because every fabricated employee has a nominated fund.

- **Daily GIC Divisor**: Per Section 8AAD of the *Taxation Administration Act 1953*, the GIC rate for a day is the base interest rate plus 7 percentage points, divided by **the number of days in the calendar year** (366 in a leap year, 365 otherwise).
- **Modelled GIC rate**: Each accrual day uses its own calendar quarter's rate and year divisor. The [ATO rate table](https://www.ato.gov.au/tax-rates-and-codes/general-interest-charge-rates), checked on 25 September 2026, publishes **11.43% for July to September 2026** and **11.51% for October to December 2026**. Later quarters carry the latest published rate forward as an explicit projection. The model's `EstimateBasis` field identifies those records. Update the published-rate map, tests and this source date together when a later rate is released.

### What the modelled exposure represents

Every late event in `samples/sample-payroll-super.csv` records a fund receipt date, so each one models a contribution that **reached the fund before any assessment**. Under **SGAA s 18D** that reduces the individual final SG shortfall to nil, leaving only notional earnings and the administrative uplift on them. The modelled `SGC_Shortfall` is therefore small relative to the underlying liability, under those assumptions.

The offset applies only where the fund receipt date is **strictly after** the due day. A contribution that arrived by the due day was never a shortfall and must not be allowed to offset a real one. The fixtures do not model unpaid, partial or split contributions, exceptional deadlines, choice failures, assessment dates or uplift reductions. A receipt date alone does not prove the amount received. Refresh refuses missing receipt dates and inconsistent status; it cannot detect a partial payment that is described as full. Visible labels therefore describe recorded receipts and estimates, not statutory compliance. Quarterly sample records are separate and do not receive a Payday charge assessment.
- **Tax Deductibility**: SG charge relating to QE days from 1 July 2026 is deductible under the amended regime. Schedule 1 item 80 of the [*Treasury Laws Amendment (Payday Superannuation) Act 2025*](https://www.legislation.gov.au/C2025A00057/asmade/2025-11-06/text/original/epub/OEBPS/document_1/document_1.html) repealed ITAA 1997 s 26-95. The transitional provisions preserve the earlier law for pre-commencement quarters; this statement does not extend to those charges or separate penalties.

---

## 2. ATO small business benchmarks

### ANZSIC benchmarking
The ATO publishes financial performance benchmarks across turnover bands for small businesses based on income tax returns and activity statements. The model selects one sample band for the entity's industry and full selected financial-year revenue. Turnover must be greater than the lower bound and at most the upper bound. Multiple entities, multiple financial years, missing bands or overlapping bands leave the comparison unavailable. A subperiod or account filter does not change the annual turnover used to select the band.

### Key ratios monitored
1. **Gross Profit Margin %**: `(Sales - Cost of Goods Sold) / Sales`
2. **Total Expenses Ratio %**: `(Cost of Sales + Operating Expenses) / Sales`
3. **Labour Cost Ratio %**: `(Salaries + Superannuation) / Sales`
4. **Rent Ratio %**: `Rent Expense / Sales`
5. **Motor Vehicle Expense Ratio %**: `Motor Vehicle Running Costs / Sales`

### Gross profit comparison

The card reports whether the displayed gross profit margin falls below, within or above the selected sample band's inclusive bounds. The signed variance is zero inside the range and the percentage-point distance to the nearest bound outside it. It does not estimate the likelihood of an ATO audit.

The fabricated total-expense averages combine the existing operating-expense assumptions with `100 - GrossProfitPct_Avg`, so the actual and reference use the same definition. A zero denominator returns blank. The sample file supplies lower and upper bounds for gross profit only. Expense, rent, motor vehicle and labour reference values remain descriptive comparisons; their averages do not establish an acceptable range. No missing bounds are inferred. The report does not establish that these sample reference values are current ATO authority.

Use one complete financial year for an annual comparison. A monthly view retains the year's turnover band but displays that month's ratios. The legacy DAX name `ATO Compliance Risk Profile` remains for existing report bindings; its output and visible labels describe only the gross profit comparison.

---

## 3. Sample group eliminations

### Intercompany elimination principle
Where entities within a corporate group trade with each other (for example, parent company charging subsidiary management fees, or logistics entity hauling goods for retail entity), intra-group transactions must be eliminated to present the true third-party consolidated group financial position.

- **P&L Eliminations**: Intra-group revenue (Account 650) debited against intra-group expense (Account 880 / 750).
- **Balance Sheet Eliminations**: Intercompany loan assets (Account 180) credited against intercompany loan liabilities (Account 380).
Refresh checks each bilateral entity pair and posting date, separately for balance-sheet and profit-and-loss legs. Both entities must be present and each leg must sum to zero. A balanced journal alone does not prove the counterparty exists.

The model supports complete, wholly owned AUD groups. Other currencies and ownership weights are rejected at refresh; selecting only part of the group makes net and elimination measures blank. This does not implement foreign exchange, non-controlling interests, unrealised-profit elimination or all AASB 10 requirements. The daily pairing control also cannot identify two offsetting errors within the same pair and leg. A production consumer would need transaction-level matching evidence.
