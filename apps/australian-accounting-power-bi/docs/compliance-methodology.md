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
3. **National Business Day Calendar**: A business day excludes Saturdays, Sundays, and any public holiday gazetted for the **whole of any State or Territory**. Regional holidays (for example, Brisbane Ekka, Melbourne Cup regional gazettals) do not stop the national clock.

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
- **Modelled GIC Rate**: GIC is set quarterly, not fixed, so the fixtures hold a rate per calendar quarter and apply **each accrual day's own rate**. An accrual that crosses a quarter or year boundary is not flattened onto a single rate. `GIC_PUBLISHED_RATES` in `tools/generate_fixtures.py` carries the ATO's published annual rate for the **July to September 2026 quarter, 11.43%**, the first quarter of the Payday Super regime. Quarters the ATO has not yet published fall back to `GIC_PROJECTED_RATE`, which carries that rate forward. Those later figures are a **stated assumption, not published data**; move each rate into `GIC_PUBLISHED_RATES` as the ATO releases it.

### What the modelled exposure represents

Every late event in `samples/sample-payroll-super.csv` records a fund receipt date, so each one models a contribution that **reached the fund before any assessment**. Under **SGAA s 18D** that reduces the individual final SG shortfall to nil, leaving only notional earnings and the administrative uplift on them. The modelled `SGC_Shortfall` is therefore small relative to the underlying liability, and that is the correct result rather than an understatement.

The offset applies only where the fund receipt date is **strictly after** the due day. A contribution that arrived by the due day was never a shortfall and must not be allowed to offset a real one. The fixtures do not model an employer who never pays, so they do not exercise the case where the full shortfall stands at assessment.
- **Tax Deductibility**: SG charge relating to QE days from 1 July 2026 is deductible under the amended regime. Schedule 1 item 80 of the [*Treasury Laws Amendment (Payday Superannuation) Act 2025*](https://www.legislation.gov.au/C2025A00057/asmade/2025-11-06/text/original/epub/OEBPS/document_1/document_1.html) repealed ITAA 1997 s 26-95. The transitional provisions preserve the earlier law for pre-commencement quarters; this statement does not extend to those charges or separate penalties.

---

## 2. ATO small business benchmarks

### ANZSIC benchmarking
The ATO publishes financial performance benchmarks across turnover bands for small businesses based on income tax returns and activity statements. The model selects one sample band for the entity's industry and full selected financial-year revenue. Turnover must be greater than the lower bound and at most the upper bound. Multiple entities, multiple financial years, missing bands or overlapping bands leave the comparison unavailable. A subperiod or account filter does not change the annual turnover used to select the band.

### Key ratios monitored
1. **Gross Profit Margin %**: `(Sales - Cost of Goods Sold) / Sales`
2. **Total Expenses Ratio %**: `Operating Expenses / Sales`
3. **Labour Cost Ratio %**: `(Salaries + Superannuation) / Sales`
4. **Rent Ratio %**: `Rent Expense / Sales`
5. **Motor Vehicle Expense Ratio %**: `Motor Vehicle Running Costs / Sales`

### Gross profit comparison

The card reports whether the displayed gross profit margin falls below, within or above the selected sample band's inclusive bounds. The signed variance is zero inside the range and the percentage-point distance to the nearest bound outside it. It does not estimate the likelihood of an ATO audit.

The sample file supplies lower and upper bounds for gross profit only. Expense, rent, motor vehicle and labour reference values remain descriptive comparisons; their averages do not establish an acceptable range. No missing bounds are inferred. The report does not establish that these sample reference values are current ATO authority.

Use one complete financial year for an annual comparison. A monthly view retains the year's turnover band but displays that month's ratios. The legacy DAX name `ATO Compliance Risk Profile` remains for existing report bindings; its output and visible labels describe only the gross profit comparison.

---

## 3. Multi-entity consolidation (AASB 10)

### Intercompany elimination principle
Where entities within a corporate group trade with each other (for example, parent company charging subsidiary management fees, or logistics entity hauling goods for retail entity), intra-group transactions must be eliminated to present the true third-party consolidated group financial position.

- **P&L Eliminations**: Intra-group revenue (Account 650) debited against intra-group expense (Account 880 / 750).
- **Balance Sheet Eliminations**: Intercompany loan assets (Account 180) credited against intercompany loan liabilities (Account 380).
- The net effect of all elimination journals across a balanced group is exactly **$0.00**.
