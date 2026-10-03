param(
    [Parameter(Mandatory = $true)][string]$Server,
    [string]$Samples = '',
    [string]$PowerBIBin = 'C:\Program Files\Microsoft Power BI Desktop\bin'
)
$ErrorActionPreference = 'Stop'
if (-not $Samples) { $Samples = Join-Path $PSScriptRoot '../samples' }
[Reflection.Assembly]::LoadFrom((Join-Path $PowerBIBin 'Microsoft.PowerBI.AdomdClient.dll')) | Out-Null
$connection = [Microsoft.AnalysisServices.AdomdClient.AdomdConnection]::new("Data Source=$Server")
$connection.Open()
$assertions = 0
function Query([string]$dax) {
    $command = $connection.CreateCommand()
    $command.CommandText = $dax
    $reader = $command.ExecuteReader()
    $result = @()
    try {
        while ($reader.Read()) {
            $row = [ordered]@{}
            for ($index = 0; $index -lt $reader.FieldCount; $index++) { $row[$reader.GetName($index)] = [string]$reader.GetValue($index) }
            $result += [pscustomobject]$row
        }
    } finally { $reader.Close(); $command.Dispose() }
    return $result
}
function Literal([string]$value) { '"' + $value.Replace('"','""') + '"' }
try {
    foreach ($pair in @(@('Review_Run','sample-review-run.csv','RunID'), @('Review_Exception','sample-review-exceptions.csv','ExceptionKey'), @('Review_Evidence','sample-review-evidence.csv','TransactionID'))) {
        $expected = @(Import-Csv -LiteralPath (Join-Path $Samples $pair[1]))
        $actual = @(Query "EVALUATE $($pair[0])")
        if ($actual.Count -ne $expected.Count) { throw "Imported row count differs: $($pair[0])" }
        foreach ($row in $expected) {
            $key = "$($pair[0])[$($pair[2])]"
            $projectionMatches = @($actual | Where-Object { $_.$key -eq $row.($pair[2]) })
            if ($pair[0] -eq 'Review_Evidence') { $projectionMatches = @($projectionMatches | Where-Object { $_.'Review_Evidence[ExceptionKey]' -eq $row.ExceptionKey }) }
            if ($projectionMatches.Count -ne 1) { throw "Imported identity differs: $($pair[0])" }
            foreach ($property in $row.PSObject.Properties) {
                $field = "$($pair[0])[$($property.Name)]"
                if ($projectionMatches[0].$field -cne $property.Value) { throw "Imported text differs: $field" }
                $assertions++
            }
        }
    }
    $query = 'SUMMARIZECOLUMNS(Review_Evidence[TransactionID], "Selected", [Evidence selection present])'
    if (@(Query "EVALUATE $query").Count -ne 0) { throw 'Evidence leaked without an exception selection.' }
    $exceptions = @(Import-Csv -LiteralPath (Join-Path $Samples 'sample-review-exceptions.csv'))
    $evidence = @(Import-Csv -LiteralPath (Join-Path $Samples 'sample-review-evidence.csv'))
    foreach ($exception in $exceptions) {
        $filters = @('ExceptionKey','RunID','Entity','Period','Basis') | ForEach-Object { "TREATAS({$(Literal $exception.$_)}, Review_Exception[$_])" }
        $actual = @(Query ("EVALUATE CALCULATETABLE($query, " + ($filters -join ', ') + ')'))
        $expected = @($evidence | Where-Object ExceptionKey -eq $exception.ExceptionKey | Select-Object -ExpandProperty TransactionID | Sort-Object)
        $identifiers = @($actual | ForEach-Object { $_.'Review_Evidence[TransactionID]' } | Sort-Object)
        if (($expected -join '|') -cne ($identifiers -join '|')) { throw ("Selected exception differs: expected $($expected.Count), actual $($actual.Count); columns $($actual[0].PSObject.Properties.Name -join ','); identifiers $($identifiers -join '|')") }
        $assertions++
        foreach ($field in @('RunID','Entity','Period','Basis')) {
            $wrong = @('ExceptionKey','RunID','Entity','Period','Basis') | ForEach-Object { $value = if ($_ -eq $field) { 'wrong-context' } else { $exception.$_ }; "TREATAS({$(Literal $value)}, Review_Exception[$_])" }
            if (@(Query ("EVALUATE CALCULATETABLE($query, " + ($wrong -join ', ') + ')')).Count -ne 0) { throw 'Evidence fell back across a wrong context.' }
            $assertions++
        }
        $rows = @(Query ('EVALUATE ROW("Before", [Revenue], "Selected", CALCULATE([Revenue], ' + ($filters -join ', ') + '))'))
        if ($rows[0].'[Before]' -ne $rows[0].'[Selected]') { throw 'Review filters changed the core revenue measure.' }
        $assertions++
    }
    Write-Output "Review projection native checks passed: $assertions assertions."
} finally { $connection.Close(); $connection.Dispose() }
