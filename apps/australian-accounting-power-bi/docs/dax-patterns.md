# DAX patterns and calculation groups reference

This model uses Calculation Groups for time intelligence and multi-entity consolidation so core measures stay in one place (DRY: Don't Repeat Yourself).

---

## 1. Time intelligence calculation group (`CalcGroup_TimeIntelligence`)

Precedence: `10`

Instead of creating separate MTD, QTD, FYTD, and PY measures for every financial line item, the calculation group modifies the selected measure. Dollar and percentage change items apply only to the listed monetary measures; ratio and text measures return blank for those items.

### Calculation items

#### `Current Period` (ordinal 0)
```dax
SELECTEDMEASURE()
```

#### `Month to Date (MTD)` (ordinal 1)
```dax
CALCULATE(
    SELECTEDMEASURE(),
    DATESMTD(Dim_Date[Date])
)
```

#### `Quarter to Date (QTD)` (ordinal 2)
```dax
CALCULATE(
    SELECTEDMEASURE(),
    DATESQTD(Dim_Date[Date])
)
```

#### `Financial Year to Date (FYTD)` (ordinal 3)
Calculates year-to-date across the Australian Financial Year (1 July to 30 June):
```dax
VAR MaxDate = MAX(Dim_Date[Date])
VAR FYStart = CALCULATE(MIN(Dim_Date[FYStartDate]), Dim_Date[Date] = MaxDate)
RETURN
    CALCULATE(
        SELECTEDMEASURE(),
        DATESBETWEEN(Dim_Date[Date], FYStart, MaxDate)
    )
```

#### `Prior Year (PY)` (ordinal 4)
```dax
CALCULATE(
    SELECTEDMEASURE(),
    SAMEPERIODLASTYEAR(Dim_Date[Date])
)
```

#### Year-on-year changes (ordinals 5 and 6)

The dollar item returns current less prior-year value only when a prior value exists. The percentage item divides that change by the prior-year value and uses a `0.0%` format. A missing or zero denominator returns blank.

Both items use an explicit `ISSELECTEDMEASURE` list of supported monetary measures. They do not subtract text or present a margin change as dollars. The full list and expressions live in [CalcGroup_TimeIntelligence.tmdl](../australian-accounting-power-bi.SemanticModel/definition/tables/CalcGroup_TimeIntelligence.tmdl).

---

## 2. Multi-entity consolidation calculation group (`CalcGroup_Consolidation`)

Precedence: `20`

`Gross Group Total` preserves the selected measure and entity scope. `Intercompany Eliminations` includes intercompany lines for supported additive financial measures. `Consolidated Group Net` excludes those lines and also supports recalculated financial ratios.

Net and elimination items require all entities in `Dim_Entity` to be selected. Refresh already requires AUD and an ownership weight of one. Budget, payroll, text and sample-reference measures return blank under those items. `Consolidation Scope` is the exception: its text explains unsupported selections. Budget data has no intercompany split, so a plausible unchanged budget must not be labelled as an elimination.

The Boolean filters use `KEEPFILTERS`. Gross less eliminations equals net for additive monetary measures; ratios must be recalculated on net inputs rather than subtracted. The [source calculation group](../australian-accounting-power-bi.SemanticModel/definition/tables/CalcGroup_Consolidation.tmdl) contains the supported-measure lists.

The report's balance-sheet measure is blank outside an account-class row, because a grand total combining assets, liabilities and equity has no useful accounting meaning. Existing total asset, liability and equity measures remain available separately.

---

## 3. Gross profit comparison

`Benchmark Annual Turnover` selects full financial-year revenue for one entity. `Benchmark Turnover Band` matches that revenue to exactly one industry band. All 5 reference measures use that same band and return blank when selection is ambiguous or unsupported.

`Gross Profit Variance to Benchmark %` compares the displayed margin with the band's inclusive bounds. It returns zero inside the range, the signed distance to the nearest bound outside it, and blank when no comparison can be made. The source expressions are in [Fact_ATOBenchmark.tmdl](../australian-accounting-power-bi.SemanticModel/definition/tables/Fact_ATOBenchmark.tmdl).

The `ATO Compliance Risk Profile` identifier is retained for existing bindings. Its visible output is a gross profit comparison, with no red/amber/green thresholds or audit-risk prediction. Expense reference averages are not treated as range boundaries.

The [native regression checks](benchmark-verification.md) exercise the source expressions in the Power BI engine, including the previously misclassified upper endpoint and turnover-band boundaries.

---

## 4. Account category predicates

Both financial matrices use `Dim_Account[Class]` or `Dim_Account[SubClass]` as row headings. A bare predicate inside `CALCULATE` replaces the filter on the column it names, so `Revenue` evaluated on the Asset row returned the whole group's revenue and each profit and loss subclass row repeated the full cost of sales and operating expenses. Every account category predicate in `Fact_GeneralLedger.tmdl` and `Fact_Budget.tmdl` is therefore wrapped in `KEEPFILTERS`, which intersects the category with the row selection instead of overwriting it:

```dax
CALCULATE(
    SUM(Fact_GeneralLedger[Credit]) - SUM(Fact_GeneralLedger[Debit]),
    KEEPFILTERS(Dim_Account[Class] = "Revenue")
)
```

The cumulative `DATESBETWEEN(Dim_Date[Date], BLANK(), MAX(Dim_Date[Date]))` argument in the balance sheet measures stays bare on purpose: those measures must reach past the period in context. `Benchmark Annual Turnover` still removes the account filter itself, so the turnover band is unaffected.

## Revenue budgets and receipt filters

The report compares actual revenue with revenue budget on the gross basis. Expense budgets are partial, so the page does not present their sum as a complete profit target. `Budget Variance $` is actual revenue less budget revenue and is blank without a budget. The percentage variance is blank without a non-zero revenue budget.

Payroll on-time and late measures intersect the selected receipt status using `KEEPFILTERS`. The on-time share uses the liability in the same selection. A late-only selection therefore has a zero on-time share, while a selection with no liability remains blank.
