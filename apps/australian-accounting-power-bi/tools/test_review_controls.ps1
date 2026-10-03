param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^localhost:[0-9]+$')]
    [string]$Server,
    [string]$PowerBIBin = 'C:\Program Files\Microsoft Power BI Desktop\bin'
)

# Independent CSV expectations and stored DAX measures. No model writes.
$ErrorActionPreference = 'Stop'
$culture = [Globalization.CultureInfo]::InvariantCulture
$samples = Join-Path $PSScriptRoot '../samples'
$accounts = @{}
Import-Csv (Join-Path $samples 'sample-chart-of-accounts.csv') | ForEach-Object { $accounts[$_.AccountCode] = $_ }
$ledger = @(Import-Csv (Join-Path $samples 'sample-general-ledger.csv'))
$budgets = @(Import-Csv (Join-Path $samples 'sample-budgets.csv'))
[Reflection.Assembly]::LoadFrom((Join-Path $PowerBIBin 'Microsoft.PowerBI.AdomdClient.dll')) | Out-Null
$connection = New-Object Microsoft.AnalysisServices.AdomdClient.AdomdConnection
$connection.ConnectionString = "Data Source=$Server"
$connection.Open()
$script:assertions = 0

function Assert-Native([string]$Name, [string]$Expression, $Expected, [string]$Filters) {
    $command = $connection.CreateCommand()
    $command.CommandText = "EVALUATE CALCULATETABLE(ROW(`"Value`", $Expression), $Filters)"
    $reader = $command.ExecuteReader()
    try {
        if (-not $reader.Read()) { throw "No result: $Name" }
        $actual = if ($reader.IsDBNull(0)) { $null } else { $reader.GetValue(0) }
        if ($null -eq $Expected) {
            if ($null -ne $actual) { throw "${Name}: expected blank, got $actual" }
        } elseif ($Expected -is [string]) {
            if ($actual -ne $Expected) { throw "${Name}: expected '$Expected', got '$actual'" }
        } elseif ($null -eq $actual -or [Math]::Abs([decimal]$actual - [decimal]$Expected) -gt [decimal]'0.000001') {
            throw "${Name}: independent $Expected, native $actual"
        }
        $script:assertions++
    } finally {
        $reader.Close()
        $command.Dispose()
    }
}

try {
    $year = 'TREATAS({2026}, Dim_Date[FinancialYearNumber])'
    foreach ($basis in @('Gross Group Total', 'Intercompany Eliminations', 'Consolidated Group Net')) {
        [decimal]$revenue = 0; [decimal]$cogs = 0; [decimal]$expenses = 0; [decimal]$addbacks = 0
        [decimal]$assets = 0; [decimal]$liabilities = 0; [decimal]$equity = 0
        [decimal]$currentAssets = 0; [decimal]$currentLiabilities = 0; [decimal]$earnings = 0
        foreach ($row in $ledger) {
            if ($basis -eq 'Intercompany Eliminations' -and $row.IsIntercompany -ne 'TRUE') { continue }
            if ($basis -eq 'Consolidated Group Net' -and $row.IsIntercompany -eq 'TRUE') { continue }
            if ($row.PostingDate -gt '2026-06-30') { continue }
            $account = $accounts[$row.AccountCode]
            $net = [decimal]::Parse($row.Debit, $culture) - [decimal]::Parse($row.Credit, $culture)
            switch ($account.Class) {
                'Asset' { $assets += $net }
                'Liability' { $liabilities -= $net }
                'Equity' { $equity -= $net }
                'Revenue' { $earnings -= $net }
                'Expense' { $earnings -= $net }
            }
            if ($account.SubClass -eq 'Current Assets') { $currentAssets += $net }
            if ($account.SubClass -eq 'Current Liabilities') { $currentLiabilities -= $net }
            if ($row.PostingDate -lt '2025-07-01') { continue }
            if ($account.Class -eq 'Revenue') { $revenue -= $net }
            if ($account.SubClass -eq 'Cost of Sales') { $cogs += $net }
            if ($account.SubClass -eq 'Operating Expenses') { $expenses += $net }
            if ($row.AccountCode -in @('890','895')) { $addbacks += $net }
        }
        $expected = @{
            'Revenue' = $revenue; 'COGS' = $cogs; 'Operating Expenses' = $expenses
            'Gross Profit' = $revenue - $cogs; 'Net Profit Before Tax' = $revenue - $cogs - $expenses
            'EBITDA' = $revenue - $cogs - $expenses + $addbacks
            'Total Assets' = $assets; 'Total Liabilities' = $liabilities; 'Total Equity' = $equity
            'Net Assets' = $assets - $liabilities; 'Working Capital' = $currentAssets - $currentLiabilities
            'Balance Sheet Check' = $assets - $liabilities - $equity - $earnings
        }
        $filters = "$year, TREATAS({`"$basis`"}, CalcGroup_Consolidation[Consolidation View])"
        $gross = $basis -eq 'Gross Group Total'
        $comparisonTitle = if ($gross) { 'Revenue against budget | gross basis' } else { 'Budget unavailable | select gross basis' }
        $monthlyTitle = if ($gross) { 'Monthly revenue against budget | AUD' } else { 'Monthly revenue | AUD | budget unavailable' }
        Assert-Native "$basis / revenue comparison title" '[Revenue Budget Title]' $comparisonTitle $filters
        Assert-Native "$basis / monthly revenue title" '[Monthly Revenue Title]' $monthlyTitle $filters
        foreach ($name in $expected.Keys) {
            Assert-Native "$basis / $name" "COALESCE([$name], 0)" $expected[$name] $filters
        }
        if ($basis -ne 'Intercompany Eliminations') {
            Assert-Native "$basis / gross margin" '[Gross Profit Margin %]' (($revenue - $cogs) / $revenue) $filters
        }
        Write-Output "PASS ${basis}: 12 monetary measures reconciled to independent CSV sums."
    }

    $netBasis = 'TREATAS({"Consolidated Group Net"}, CalcGroup_Consolidation[Consolidation View])'
    $eliminations = 'TREATAS({"Intercompany Eliminations"}, CalcGroup_Consolidation[Consolidation View])'
    Assert-Native 'subset net title explains recovery' '[Revenue Budget Title]' 'Budget unavailable | select gross basis' "$year, $netBasis, TREATAS({`"ENT001`"}, Dim_Entity[EntityID])"
    Assert-Native 'unfiltered basis uses the gross title' '[Revenue Budget Title]' 'Revenue against budget | gross basis' $year
    Assert-Native 'subset net is unavailable' '[Revenue]' $null "$year, $netBasis, TREATAS({`"ENT001`"}, Dim_Entity[EntityID])"
    Assert-Native 'subset eliminations unavailable' '[Revenue]' $null "$year, $eliminations, TREATAS({`"ENT001`",`"ENT002`"}, Dim_Entity[EntityID])"
    Assert-Native 'budget is not an elimination' '[Budget Revenue]' $null "$year, $eliminations"
    Assert-Native 'budget net is unavailable' '[Budget Revenue]' $null "$year, $netBasis"
    Assert-Native 'margin elimination is unavailable' '[Gross Profit Margin %]' $null "$year, $eliminations"
    Assert-Native 'payroll is outside consolidation' '[Super Liability Accrued (Code L)]' $null "$year, $netBasis"
    Assert-Native 'margin dollar variance unavailable' '[Gross Profit Margin %]' $null "$year, TREATAS({`"Year-on-Year Variance (`$)`"}, CalcGroup_TimeIntelligence[Time Calculation])"
    Assert-Native 'missing prior year has no dollar change' '[Revenue]' $null 'TREATAS({2025}, Dim_Date[FinancialYearNumber]), TREATAS({"Year-on-Year Variance ($)"}, CalcGroup_TimeIntelligence[Time Calculation])'
    Assert-Native 'no turnover leaves ratio blank' '[Actual Total Expense Ratio %]' $null "$year, TREATAS({`"UNKNOWN`"}, Dim_Entity[EntityID])"
    Assert-Native 'balance sheet grand total withheld' '[Balance Sheet Amount]' $null $year

    $entity = 'TREATAS({"ENT002"}, Dim_Entity[EntityID])'
    [decimal]$entityRevenue = 0; [decimal]$entityCosts = 0
    foreach ($row in $ledger) {
        if ($row.EntityID -ne 'ENT002' -or $row.PostingDate -lt '2025-07-01' -or $row.PostingDate -gt '2026-06-30') { continue }
        $net = [decimal]::Parse($row.Debit, $culture) - [decimal]::Parse($row.Credit, $culture)
        if ($accounts[$row.AccountCode].Class -eq 'Revenue') { $entityRevenue -= $net }
        if ($accounts[$row.AccountCode].Class -eq 'Expense') { $entityCosts += $net }
    }
    Assert-Native 'total expenses include cost of sales' '[Actual Total Expense Ratio %]' ($entityCosts/$entityRevenue) "$year, $entity"
    [decimal]$budgetRevenue = 0
    foreach ($row in $budgets) {
        if ($row.EntityID -eq 'ENT002' -and $row.PeriodDate -ge '2025-07-01' -and $row.PeriodDate -le '2026-06-30' -and $accounts[$row.AccountCode].Class -eq 'Revenue') {
            $budgetRevenue += [decimal]::Parse($row.BudgetAmount, $culture)
        }
    }
    Assert-Native 'revenue budget variance' '[Budget Variance $]' ($entityRevenue-$budgetRevenue) "$year, $entity"
    Assert-Native 'revenue budget percentage' '[Budget Variance %]' (($entityRevenue-$budgetRevenue)/$budgetRevenue) "$year, $entity"

    [decimal]$priorNetRevenue = 0; [decimal]$currentNetRevenue = 0
    foreach ($row in $ledger) {
        if ($row.IsIntercompany -eq 'TRUE' -or $accounts[$row.AccountCode].Class -ne 'Revenue') { continue }
        $amount = [decimal]::Parse($row.Credit, $culture) - [decimal]::Parse($row.Debit, $culture)
        if ($row.PostingDate -ge '2024-07-01' -and $row.PostingDate -le '2025-06-30') { $priorNetRevenue += $amount }
        if ($row.PostingDate -ge '2025-07-01' -and $row.PostingDate -le '2026-06-30') { $currentNetRevenue += $amount }
    }
    Assert-Native 'net prior year composition' '[Revenue]' $priorNetRevenue "$year, $netBasis, TREATAS({`"Prior Year (PY)`"}, CalcGroup_TimeIntelligence[Time Calculation])"
    Assert-Native 'net YoY composition' '[Revenue]' ($currentNetRevenue-$priorNetRevenue) "$year, $netBasis, TREATAS({`"Year-on-Year Variance (`$)`"}, CalcGroup_TimeIntelligence[Time Calculation])"
    Assert-Native 'net YoY percentage composition' '[Revenue]' (($currentNetRevenue-$priorNetRevenue)/$priorNetRevenue) "$year, $netBasis, TREATAS({`"Year-on-Year Variance (%)`"}, CalcGroup_TimeIntelligence[Time Calculation])"
    Assert-Native 'part-day calendar review withholds charge' '[Total SGC Exposure]' $null 'TREATAS({2027}, Dim_Date[FinancialYearNumber])'
    Assert-Native 'part-day calendar review withholds interest' '[Total GIC Nominal Interest]' $null 'TREATAS({2027}, Dim_Date[FinancialYearNumber])'
    Assert-Native 'no calendar review events displays zero' '[Calendar Review Events]' 0 'TREATAS({2027}, Dim_Date[FinancialYearNumber]), TREATAS({"LATE_BREACH"}, Fact_PayrollSuper[ComplianceStatus])'
    Assert-Native 'quarterly sample has no charge assessment' '[Total SGC Exposure]' $null $year
    Assert-Native 'late-only selection has zero on-time share' '[Payday Super Compliance Rate %]' 0 'TREATAS({2027}, Dim_Date[FinancialYearNumber]), TREATAS({"LATE_BREACH"}, Fact_PayrollSuper[ComplianceStatus])'
    Assert-Native 'on-time selection has full recorded share' '[Payday Super Compliance Rate %]' 1 'TREATAS({2027}, Dim_Date[FinancialYearNumber]), TREATAS({"ON_TIME"}, Fact_PayrollSuper[ComplianceStatus])'
    Assert-Native 'readable late label retains the receipt filter' '[Payday Super Compliance Rate %]' 0 'TREATAS({2027}, Dim_Date[FinancialYearNumber]), TREATAS({"Late"}, Fact_PayrollSuper[ReceiptStatus])'
    Assert-Native 'readable on-time label retains the receipt filter' '[Payday Super Compliance Rate %]' 1 'TREATAS({2027}, Dim_Date[FinancialYearNumber]), TREATAS({"On time"}, Fact_PayrollSuper[ReceiptStatus])'
    Assert-Native 'empty receipt selection explains recovery' 'IF(CONTAINSSTRING([Payroll Review Note], "No payroll receipts"), 1, 0)' 1 "$year, TREATAS({`"UNKNOWN`"}, Dim_Entity[EntityID])"
    Assert-Native 'calendar warning explains withheld estimates' 'IF(CONTAINSSTRING([Payroll Review Note], "withheld"), 1, 0)' 1 'TREATAS({2027}, Dim_Date[FinancialYearNumber])'
    Assert-Native 'quarterly note explains estimate scope' 'IF(CONTAINSSTRING([Payroll Review Note], "Quarterly receipt sample"), 1, 0)' 1 $year
    Assert-Native 'context survives consolidated selection' 'IF(CONTAINSSTRING([Report Selection], "Fabricated data"), 1, 0)' 1 "$year, $netBasis"
    Assert-Native 'discontinuous dates are labelled honestly' 'IF(CONTAINSSTRING([Report Selection], "Selected dates within"), 1, 0)' 1 "$year, TREATAS({7,9}, Dim_Date[MonthNumber])"
    Assert-Native 'complete-year context is a continuous range' 'IF(CONTAINSSTRING([Report Selection], "Selected dates within"), 1, 0)' 0 $year
    Assert-Native 'empty date selection explains recovery' 'IF(CONTAINSSTRING([Report Selection], "No matching dates"), 1, 0)' 1 'TREATAS({2099}, Dim_Date[FinancialYearNumber])'
    Write-Output "$script:assertions native review-control assertions passed."
} finally {
    $connection.Close()
    $connection.Dispose()
}
