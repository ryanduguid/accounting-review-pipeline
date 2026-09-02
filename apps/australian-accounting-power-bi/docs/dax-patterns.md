# DAX Patterns & Calculation Groups Reference

This model uses Calculation Groups for time intelligence and multi-entity consolidation so core measures stay in one place (DRY: Don't Repeat Yourself).

---

## 1. Time Intelligence Calculation Group (`CalcGroup_TimeIntelligence`)

Precedence: `10`

Instead of creating separate MTD, QTD, FYTD, and PY measures for every financial line item, a single calculation group dynamically modifies any base measure.

### Calculation Items

#### `Current Period` (Ordinal 0)
```dax
SELECTEDMEASURE()
```

#### `Month to Date (MTD)` (Ordinal 1)
```dax
CALCULATE(
    SELECTEDMEASURE(),
    DATESMTD(Dim_Date[Date])
)
```

#### `Quarter to Date (QTD)` (Ordinal 2)
```dax
CALCULATE(
    SELECTEDMEASURE(),
    DATESQTD(Dim_Date[Date])
)
```

#### `Financial Year to Date (FYTD)` (Ordinal 3)
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

#### `Prior Year (PY)` (Ordinal 4)
```dax
CALCULATE(
    SELECTEDMEASURE(),
    SAMEPERIODLASTYEAR(Dim_Date[Date])
)
```

#### `Year-on-Year Variance ($)` (Ordinal 5)
```dax
SELECTEDMEASURE() - CALCULATE(SELECTEDMEASURE(), SAMEPERIODLASTYEAR(Dim_Date[Date]))
```

#### `Year-on-Year Variance (%)` (Ordinal 6)
```dax
VAR CurrentVal = SELECTEDMEASURE()
VAR PriorVal = CALCULATE(SELECTEDMEASURE(), SAMEPERIODLASTYEAR(Dim_Date[Date]))
RETURN
    DIVIDE(CurrentVal - PriorVal, PriorVal, 0)
```
*Format string definition*: `0.0%`

---

## 2. Multi-Entity Consolidation Calculation Group (`CalcGroup_Consolidation`)

Precedence: `20`

Enables simultaneous reporting of individual legal entity performance, intercompany elimination journals, and the consolidated group net total in matrix visuals.

### Calculation Items

#### `Gross Group Total` (Ordinal 0)
```dax
SELECTEDMEASURE()
```

#### `Intercompany Eliminations` (Ordinal 1)
Filters down to intercompany management fees, internal rent, and intra-group logistics transactions:
```dax
CALCULATE(
    SELECTEDMEASURE(),
    Fact_GeneralLedger[IsIntercompany] = TRUE()
)
```

#### `Consolidated Group Net` (Ordinal 2)
Presents the true third-party consolidated financial position:
```dax
CALCULATE(
    SELECTEDMEASURE(),
    Fact_GeneralLedger[IsIntercompany] = FALSE()
)
```

---

## 3. ATO Small Business Compliance Risk Diagnostic

Evaluates gross profit and operating expense variances against ATO benchmark bands to assign an automated risk rating:

```dax
measure 'ATO Compliance Risk Profile' =
    VAR GP_Diff = ABS([Gross Profit Margin %] - [ATO Benchmark Gross Profit %])
    VAR Exp_Diff = ABS([Actual Total Expense Ratio %] - [ATO Benchmark Expense Ratio %])
    RETURN
        IF(
            ISBLANK([Revenue]) || [Revenue] == 0,
            "No Data",
            IF(
                GP_Diff > 0.08 || Exp_Diff > 0.08,
                "High Audit Risk (Red Zone)",
                IF(
                    GP_Diff > 0.04 || Exp_Diff > 0.04,
                    "Moderate Variance (Amber Zone)",
                    "Within Benchmark Range (Green Zone)"
                )
            )
        )
```
