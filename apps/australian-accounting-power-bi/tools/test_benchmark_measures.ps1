param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^localhost:[0-9]+$')]
    [string]$Server,
    [string]$PowerBIBin = 'C:\Program Files\Microsoft Power BI Desktop\bin'
)

# Query a disposable, refreshed copy of the sample model. Query-scoped measure
# definitions do not change the open model.
$ErrorActionPreference = 'Stop'
[Reflection.Assembly]::LoadFrom((Join-Path $PowerBIBin 'Microsoft.PowerBI.AdomdClient.dll')) | Out-Null
$connection = New-Object Microsoft.AnalysisServices.AdomdClient.AdomdConnection
$connection.ConnectionString = "Data Source=$Server"
$connection.Open()

# Rebind the production expressions in query scope so their references resolve
# to the synthetic input measures below, rather than the stored model inputs.
$tablePath = Join-Path $PSScriptRoot '../australian-accounting-power-bi.SemanticModel/definition/tables/Fact_ATOBenchmark.tmdl'
$tableSource = [IO.File]::ReadAllText($tablePath)
$definitions = foreach ($match in [regex]::Matches($tableSource, '(?ms)^\tmeasure ''([^'']+)'' =\s*(.*?)(?=^\t\t(?:formatString|lineageTag):)')) {
    if ($match.Groups[1].Value -eq 'Actual Total Expense Ratio %') { continue }
    "MEASURE Fact_ATOBenchmark[$($match.Groups[1].Value)] = $($match.Groups[2].Value.Trim())"
}
if (-not $definitions) { throw 'No benchmark measure expressions found' }

$cases = @(
    @{ Name = 'lower margin endpoint'; Turnover = '750000'; Margin = '0.72'; Status = 'Within gross profit range'; Benchmark = 0.78 },
    @{ Name = 'upper margin endpoint'; Turnover = '750000'; Margin = '0.84'; Status = 'Within gross profit range'; Benchmark = 0.78 },
    @{ Name = 'below margin range'; Turnover = '750000'; Margin = '0.71'; Status = 'Below gross profit range'; Benchmark = 0.78 },
    @{ Name = 'above margin range'; Turnover = '750000'; Margin = '0.85'; Status = 'Above gross profit range'; Benchmark = 0.78 },
    @{ Name = 'turnover upper endpoint'; Turnover = '1000000'; Margin = '0.84'; Status = 'Within gross profit range'; Benchmark = 0.78 },
    @{ Name = 'next turnover band'; Turnover = '1000001'; Margin = '0.84'; Status = 'Above gross profit range'; Benchmark = 0.765 },
    @{ Name = 'outside available bands'; Turnover = '6000000'; Margin = '0.84'; Status = 'No matching benchmark'; Benchmark = $null },
    @{ Name = 'no turnover'; Turnover = '0'; Margin = '0'; Status = 'No turnover'; Benchmark = $null },
    @{ Name = 'multiple entities'; Turnover = '750000'; Margin = '0.84'; Entities = '"ENT002", "ENT003"'; Status = 'Select one entity and financial year'; Benchmark = $null },
    @{ Name = 'multiple financial years'; Turnover = '750000'; Margin = '0.84'; Years = '2025, 2026'; Status = 'Select one entity and financial year'; Benchmark = $null },
    @{ Name = 'subperiod does not change annual band'; Turnover = 'IF(ISFILTERED(Dim_Date[MonthNumber]), 750000, 1500000)'; Margin = '0.75'; ExtraFilter = ', TREATAS({7}, Dim_Date[MonthNumber])'; Status = 'Within gross profit range'; Benchmark = 0.765 },
    @{ Name = 'account filter does not change annual band'; Turnover = 'IF(ISFILTERED(Dim_Account[AccountCode]), 750000, 1500000)'; Margin = '0.75'; ExtraFilter = ', TREATAS({"800"}, Dim_Account[AccountCode])'; Status = 'Within gross profit range'; Benchmark = 0.765 },
    @{ Name = 'benchmark value filter does not remove selected band'; Turnover = '1500000'; Margin = '0.75'; ExtraFilter = ', TREATAS({78.0}, Fact_ATOBenchmark[GrossProfitPct_Avg])'; Status = 'Within gross profit range'; Benchmark = 0.765 }
)

$failed = 0
try {
    foreach ($case in $cases) {
        $entities = if ($case.ContainsKey('Entities')) { $case.Entities } else { '"ENT003"' }
        $years = if ($case.ContainsKey('Years')) { $case.Years } else { '2026' }
        $command = $connection.CreateCommand()
        $command.CommandText = @"
DEFINE
    $($definitions -join "`n")
    MEASURE Fact_GeneralLedger[Revenue] = $($case.Turnover)
    MEASURE Fact_GeneralLedger[Gross Profit Margin %] = $($case.Margin)
    MEASURE Fact_ATOBenchmark[Actual Total Expense Ratio %] = 0.6575
EVALUATE
    CALCULATETABLE(
        ROW("Status", [ATO Compliance Risk Profile], "Benchmark", [ATO Benchmark Gross Profit %]),
        TREATAS({ $entities }, Dim_Entity[EntityID]),
        TREATAS({ $years }, Dim_Date[FinancialYearNumber]) $($case.ExtraFilter)
    )
"@
        $reader = $command.ExecuteReader()
        try {
            if (-not $reader.Read()) { throw "No result for $($case.Name)" }
            $status = $reader.GetString(0)
            $benchmark = if ($reader.IsDBNull(1)) { $null } else { [double]$reader.GetValue(1) }
            $matches = if ($null -eq $case.Benchmark) {
                $null -eq $benchmark
            } else {
                $null -ne $benchmark -and [Math]::Abs($benchmark - $case.Benchmark) -lt 0.00000001
            }
            if ($status -ne $case.Status -or -not $matches) {
                $failed++
                Write-Output "FAIL $($case.Name): status='$status', benchmark='$benchmark'; expected '$($case.Status)', '$($case.Benchmark)'"
            } else {
                Write-Output "PASS $($case.Name)"
            }
        } finally {
            $reader.Close()
            $command.Dispose()
        }
    }
} finally {
    $connection.Close()
    $connection.Dispose()
}
if ($failed) { throw "$failed native benchmark checks failed" }
Write-Output "$($cases.Count) native benchmark checks passed"
