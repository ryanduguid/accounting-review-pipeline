# DAX patterns and calculation groups reference

This model uses Calculation Groups for time intelligence and multi-entity consolidation so core measures stay in one place (DRY: Don't Repeat Yourself).

---

## 1. Time intelligence calculation group (`CalcGroup_TimeIntelligence`)

Precedence: `10`

Instead of creating separate MTD, QTD, FYTD, and PY measures for every financial line item, a single calculation group dynamically modifies any base measure.

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

#### `Year-on-Year Variance ($)` (ordinal 5)
```dax
SELECTEDMEASURE() - CALCULATE(SELECTEDMEASURE(), SAMEPERIODLASTYEAR(Dim_Date[Date]))
```

#### `Year-on-Year Variance (%)` (ordinal 6)
```dax
VAR CurrentVal = SELECTEDMEASURE()
VAR PriorVal = CALCULATE(SELECTEDMEASURE(), SAMEPERIODLASTYEAR(Dim_Date[Date]))
RETURN
    DIVIDE(CurrentVal - PriorVal, PriorVal, 0)
```
*Format string definition*: `0.0%`

---

## 2. Multi-entity consolidation calculation group (`CalcGroup_Consolidation`)

Precedence: `20`

Enables simultaneous reporting of individual legal entity performance, intercompany elimination journals, and the consolidated group net total in matrix visuals.

### Calculation items

#### `Gross Group Total` (ordinal 0)
```dax
SELECTEDMEASURE()
```

#### `Intercompany Eliminations` (ordinal 1)
Filters down to intercompany management fees, internal rent, and intra-group logistics transactions:
```dax
CALCULATE(
    SELECTEDMEASURE(),
    Fact_GeneralLedger[IsIntercompany] = TRUE()
)
```

#### `Consolidated Group Net` (ordinal 2)
Presents the true third-party consolidated financial position:
```dax
CALCULATE(
    SELECTEDMEASURE(),
    Fact_GeneralLedger[IsIntercompany] = FALSE()
)
```

---

## 3. Gross profit comparison

`Benchmark Annual Turnover` selects full financial-year revenue for one entity. `Benchmark Turnover Band` matches that revenue to exactly one industry band. All 5 reference measures use that same band and return blank when selection is ambiguous or unsupported.

`Gross Profit Variance to Benchmark %` compares the displayed margin with the band's inclusive bounds. It returns zero inside the range, the signed distance to the nearest bound outside it, and blank when no comparison can be made. The source expressions are in [Fact_ATOBenchmark.tmdl](../australian-accounting-power-bi.SemanticModel/definition/tables/Fact_ATOBenchmark.tmdl).

The `ATO Compliance Risk Profile` identifier is retained for existing bindings. Its visible output is a gross profit comparison, with no red/amber/green thresholds or audit-risk prediction. Expense reference averages are not treated as range boundaries.

The [native regression checks](benchmark-verification.md) exercise the source expressions in the Power BI engine, including the previously misclassified upper endpoint and turnover-band boundaries.
