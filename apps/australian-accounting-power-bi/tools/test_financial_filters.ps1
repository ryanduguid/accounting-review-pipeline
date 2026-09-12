param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^localhost:[0-9]+$')]
    [string]$Server,
    [string]$PowerBIBin = 'C:\Program Files\Microsoft Power BI Desktop\bin'
)

# Compare stored DAX measures with independent sums of the fabricated CSVs.
# No query-scoped measure overrides, model writes or external data sources.
$ErrorActionPreference = 'Stop'
$culture = [Globalization.CultureInfo]::InvariantCulture
$samples = Join-Path $PSScriptRoot '../samples'
$accounts = @{}
Import-Csv (Join-Path $samples 'sample-chart-of-accounts.csv') | ForEach-Object {
    $accounts[$_.AccountCode] = $_
}
$ledger = @(Import-Csv (Join-Path $samples 'sample-general-ledger.csv') | ForEach-Object {
    if (-not $accounts.ContainsKey($_.AccountCode)) {
        throw "Fabricated ledger account '$($_.AccountCode)' is missing from the chart of accounts."
    }
    $account = $accounts[$_.AccountCode]
    [pscustomobject]@{
        Entity = $_.EntityID
        Date = [datetime]::ParseExact($_.PostingDate, 'yyyy-MM-dd', $culture)
        Account = $_.AccountCode
        Class = $account.Class
        SubClass = $account.SubClass
        Net = [decimal]::Parse($_.Debit, $culture) - [decimal]::Parse($_.Credit, $culture)
    }
})
if (-not $ledger.Count) { throw 'The fabricated ledger is empty' }

$cases = @(
    @{ Name = 'group FY2025'; Start = '2024-07-01'; End = '2025-06-30' },
    @{ Name = 'group FY2026'; Start = '2025-07-01'; End = '2026-06-30' },
    @{ Name = 'ENT001 FY2026'; Start = '2025-07-01'; End = '2026-06-30'; Entities = @('ENT001') },
    @{ Name = 'ENT002 FY2026'; Start = '2025-07-01'; End = '2026-06-30'; Entities = @('ENT002') },
    @{ Name = 'ENT003 FY2026'; Start = '2025-07-01'; End = '2026-06-30'; Entities = @('ENT003') },
    @{ Name = 'ENT004 FY2026'; Start = '2025-07-01'; End = '2026-06-30'; Entities = @('ENT004') },
    @{ Name = 'two entities June'; Start = '2025-06-01'; End = '2025-06-30'; Entities = @('ENT001', 'ENT002') },
    @{ Name = 'ENT001 June'; Start = '2025-06-01'; End = '2025-06-30'; Entities = @('ENT001') },
    @{ Name = 'ENT001 July'; Start = '2025-07-01'; End = '2025-07-31'; Entities = @('ENT001') },
    @{ Name = 'group July'; Start = '2025-07-01'; End = '2025-07-31' },
    @{ Name = 'group FYTD September'; Start = '2025-07-01'; End = '2025-09-30'; FYTD = $true },
    @{ Name = 'group two financial years'; Start = '2024-07-01'; End = '2026-06-30' },
    @{ Name = 'group financial year number'; Start = '2025-07-01'; End = '2026-06-30'; PeriodFilter = 'TREATAS({2026}, Dim_Date[FinancialYearNumber])' },
    @{ Name = 'group financial year label'; Start = '2024-07-01'; End = '2025-06-30'; PeriodFilter = 'TREATAS({"FY24-25"}, Dim_Date[FinancialYear])' },
    @{ Name = 'ENT001 financial June'; Start = '2025-06-01'; End = '2025-06-30'; Entities = @('ENT001'); PeriodFilter = 'TREATAS({"FY24-25-12"}, Dim_Date[FinancialYearMonth])' },
    @{ Name = 'ENT001 financial July'; Start = '2025-07-01'; End = '2025-07-31'; Entities = @('ENT001'); PeriodFilter = 'TREATAS({"FY25-26-01"}, Dim_Date[FinancialYearMonth])' }
)

[Reflection.Assembly]::LoadFrom((Join-Path $PowerBIBin 'Microsoft.PowerBI.AdomdClient.dll')) | Out-Null
$connection = New-Object Microsoft.AnalysisServices.AdomdClient.AdomdConnection
$connection.ConnectionString = "Data Source=$Server"
$connection.Open()
$assertions = 0
try {
    foreach ($case in $cases) {
        $start = [datetime]::ParseExact($case.Start, 'yyyy-MM-dd', $culture)
        $end = [datetime]::ParseExact($case.End, 'yyyy-MM-dd', $culture)
        [decimal]$revenue = 0
        [decimal]$cogs = 0
        [decimal]$expenses = 0
        [decimal]$addbacks = 0
        [decimal]$assets = 0
        [decimal]$liabilities = 0
        [decimal]$currentAssets = 0
        [decimal]$currentLiabilities = 0
        [decimal]$equity = 0
        [decimal]$earnings = 0
        $periodRows = 0
        foreach ($row in $ledger) {
            if ($case.Entities -and $row.Entity -notin $case.Entities) { continue }
            if ($row.Date -gt $end) { continue }
            if ($row.SubClass -eq 'Current Assets') { $currentAssets += $row.Net }
            if ($row.SubClass -eq 'Current Liabilities') { $currentLiabilities -= $row.Net }
            switch ($row.Class) {
                'Asset' { $assets += $row.Net }
                'Liability' { $liabilities -= $row.Net }
                'Equity' { $equity -= $row.Net }
                'Revenue' { $earnings -= $row.Net }
                'Expense' { $earnings -= $row.Net }
            }
            if ($row.Date -lt $start) { continue }
            $periodRows++
            if ($row.Class -eq 'Revenue') { $revenue -= $row.Net }
            if ($row.SubClass -eq 'Cost of Sales') { $cogs += $row.Net }
            if ($row.SubClass -eq 'Operating Expenses') { $expenses += $row.Net }
            if ($row.Account -in @('890', '895')) { $addbacks += $row.Net }
        }
        $expected = [ordered]@{
            'Rows' = $periodRows
            'Revenue' = $revenue
            'COGS' = $cogs
            'Gross Profit' = $revenue - $cogs
            'Operating Expenses' = $expenses
            'Net Profit Before Tax' = $revenue - $cogs - $expenses
            'EBITDA' = $revenue - $cogs - $expenses + $addbacks
            'Total Assets' = $assets
            'Total Liabilities' = $liabilities
            'Net Assets' = $assets - $liabilities
            'Working Capital' = $currentAssets - $currentLiabilities
            'Balance Sheet Check' = $assets - $liabilities - $equity - $earnings
        }
        $filterStart = if ($case.FYTD) { $end } else { $start }
        $filters = @(if ($case.PeriodFilter) {
            $case.PeriodFilter
        } else {
            "DATESBETWEEN(Dim_Date[Date], DATE($($filterStart.Year),$($filterStart.Month),$($filterStart.Day)), DATE($($end.Year),$($end.Month),$($end.Day)))"
        })
        if ($case.Entities) {
            $entities = ($case.Entities | ForEach-Object { '"' + $_ + '"' }) -join ','
            $filters += "TREATAS({$entities}, Dim_Entity[EntityID])"
        }
        if ($case.FYTD) {
            $filters += 'TREATAS({"Financial Year to Date (FYTD)"}, CalcGroup_TimeIntelligence[Time Calculation])'
        }
        $fields = @('"Rows", [Native Acceptance Rows]')
        foreach ($name in $expected.Keys) {
            if ($name -ne 'Rows') { $fields += ('"' + $name + '", [' + $name + ']') }
        }
        $command = $connection.CreateCommand()
        $command.CommandText = @"
DEFINE MEASURE Fact_GeneralLedger[Native Acceptance Rows] = COUNTROWS(Fact_GeneralLedger)
EVALUATE CALCULATETABLE(ROW($($fields -join ', ')), $($filters -join ', '))
"@
        $reader = $command.ExecuteReader()
        try {
            if (-not $reader.Read()) { throw "No native result: $($case.Name)" }
            $column = 0
            foreach ($name in $expected.Keys) {
                # SUM returns BLANK when an entity has no rows in a category.
                # Accept it only for a zero expectation; the row count must exist.
                if ($reader.IsDBNull($column)) {
                    if ($name -eq 'Rows' -or $expected[$name] -ne 0) {
                        throw "Blank native result: $($case.Name), $name"
                    }
                    $actual = [decimal]0
                } else {
                    $actual = [decimal]$reader.GetValue($column)
                }
                if ([Math]::Abs($actual - [decimal]$expected[$name]) -gt [decimal]'0.005') {
                    throw "$($case.Name), ${name}: native $actual, independent $($expected[$name])"
                }
                $column++
                $assertions++
            }
            Write-Output "PASS $($case.Name): revenue=$revenue; EBITDA=$($expected['EBITDA']); net assets=$($expected['Net Assets'])"
        } finally {
            $reader.Close()
            $command.Dispose()
        }
    }
} finally {
    $connection.Close()
    $connection.Dispose()
}
Write-Output "$($cases.Count) native filter cases passed ($assertions assertions)"
