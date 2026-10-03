param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^localhost:[0-9]+$')]
    [string]$Server,
    [ValidateRange(5, 50)]
    [int]$Repetitions = 10,
    [string]$PowerBIBin = 'C:\Program Files\Microsoft Power BI Desktop\bin'
)

# Read-only local timing. The first request is not guaranteed to have a cold cache.
# These measure engine round trips, including row reads, rather than visual rendering.
$ErrorActionPreference = 'Stop'
[Reflection.Assembly]::LoadFrom((Join-Path $PowerBIBin 'Microsoft.PowerBI.AdomdClient.dll')) | Out-Null
$connection = New-Object Microsoft.AnalysisServices.AdomdClient.AdomdConnection
$connection.ConnectionString = "Data Source=$Server"
$connection.Open()
$queries = [ordered]@{
    'Financial summary' = 'EVALUATE CALCULATETABLE(ROW("Revenue", [Revenue], "Margin", [Gross Profit Margin %], "EBITDA", [EBITDA], "Net assets", [Net Assets]), TREATAS({2026}, Dim_Date[FinancialYearNumber]))'
    'Monthly revenue and budget' = 'EVALUATE SUMMARIZECOLUMNS(Dim_Date[CalendarYearMonth], TREATAS({2026}, Dim_Date[FinancialYearNumber]), "Revenue", [Revenue], "Budget", [Budget Revenue])'
    'Profit and loss comparison' = 'EVALUATE SUMMARIZECOLUMNS(Dim_Account[SubClass], TREATAS({2026}, Dim_Date[FinancialYearNumber]), "Actual", [Net Profit Before Tax], "Prior year", [Prior Year Profit], "Change", [Profit Change $])'
    'Consolidated balances' = 'EVALUATE SUMMARIZECOLUMNS(Dim_Account[Class], CalcGroup_Consolidation[Consolidation View], TREATAS({2026}, Dim_Date[FinancialYearNumber]), "Balance", [Balance Sheet Amount])'
    'Payroll review' = 'EVALUATE CALCULATETABLE(ROW("Share", [Payday Super Compliance Rate %], "SGC", [Total SGC Exposure], "Interest", [Total GIC Nominal Interest], "Late", [Late Events Count], "Calendar review", [Calendar Review Events], "Note", [Payroll Review Note]), TREATAS({2027}, Dim_Date[FinancialYearNumber]))'
}
try {
    $results = foreach ($entry in $queries.GetEnumerator()) {
        $timings = @()
        $rows = 0
        for ($iteration = 0; $iteration -le $Repetitions; $iteration++) {
            $command = $connection.CreateCommand()
            $command.CommandText = $entry.Value
            $command.CommandTimeout = 30
            $timer = [Diagnostics.Stopwatch]::StartNew()
            $reader = $null
            try {
                $reader = $command.ExecuteReader()
                $rows = 0
                while ($reader.Read()) { $rows++ }
                $timer.Stop()
                $timings += $timer.Elapsed.TotalMilliseconds
            } finally {
                if ($reader) { $reader.Close(); $reader.Dispose() }
                $command.Dispose()
            }
        }
        $warm = @($timings[1..$Repetitions] | Sort-Object)
        $middle = [int][Math]::Floor($Repetitions / 2)
        $median = if ($Repetitions % 2 -eq 0) { ($warm[$middle - 1] + $warm[$middle]) / 2 } else { $warm[$middle] }
        [PSCustomObject]@{
            Query = $entry.Key
            Rows = $rows
            FirstObservedMs = [Math]::Round($timings[0], 2)
            WarmMedianMs = [Math]::Round($median, 2)
            WarmP95Ms = [Math]::Round($warm[[Math]::Ceiling($Repetitions * 0.95) - 1], 2)
            WarmRepetitions = $Repetitions
        }
    }
    $results | ConvertTo-Json
} finally {
    $connection.Close()
    $connection.Dispose()
}
