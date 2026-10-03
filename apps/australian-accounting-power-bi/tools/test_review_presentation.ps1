param(
    [Parameter(Mandatory = $true)][ValidatePattern('^localhost:[0-9]+$')][string]$Server,
    [string]$Samples = '',
    [string]$PowerBIBin = 'C:\Program Files\Microsoft Power BI Desktop\bin'
)
$ErrorActionPreference = 'Stop'
if (-not $Samples) { $Samples = Join-Path $PSScriptRoot '../samples' }
[Reflection.Assembly]::LoadFrom((Join-Path $PowerBIBin 'Microsoft.PowerBI.AdomdClient.dll')) | Out-Null
[Reflection.Assembly]::LoadFrom((Join-Path $PowerBIBin 'Microsoft.PowerBI.Tabular.dll')) | Out-Null
$connection = [Microsoft.AnalysisServices.AdomdClient.AdomdConnection]::new("Data Source=$Server")
$connection.Open()
$engine = [Microsoft.AnalysisServices.Tabular.Server]::new()
$engine.Connect($Server)
$assertions = 0
$temporaryName = '__ReviewDisplayTest'
$addedTemporaryTable = $false
function Query([string]$dax) {
    $command = $connection.CreateCommand()
    $command.CommandText = $dax
    $reader = $command.ExecuteReader()
    $result = @()
    try {
        while ($reader.Read()) {
            $row = [ordered]@{}
            for ($index = 0; $index -lt $reader.FieldCount; $index++) {
                $row[$reader.GetName($index)] = if ($reader.IsDBNull($index)) { $null } else { [string]$reader.GetValue($index) }
            }
            $result += [pscustomobject]$row
        }
    } finally { $reader.Close(); $command.Dispose() }
    return $result
}
function Literal([string]$value) { '"' + $value.Replace('"','""') + '"' }
function Require([bool]$condition, [string]$message) {
    if (-not $condition) { throw $message }
    $script:assertions++
}
try {
    if ($engine.Databases.Count -ne 1) { throw 'Expected one disposable database.' }
    $model = $engine.Databases[0].Model
    $configured = $model.Expressions['SampleFolder'].Expression
    $folder = (Resolve-Path -LiteralPath $Samples).Path
    # The formatter probe creates a temporary table, so require the disposable marker.
    if (-not (Test-Path -LiteralPath (Join-Path $folder '.native-test-copy')) -or
        (-not $configured.Contains($folder.Replace('\','/')) -and -not $configured.Contains($folder))) {
        throw 'Presentation checks require the configured disposable fabricated samples.'
    }
    if ($model.Tables.Contains($temporaryName)) { throw 'Temporary formatter table already exists.' }
    $cases = @(
        @('0.00','0.00'), @('-0.00','(0.00)'), @('9.99','9.99'), @('999.00','999.00'),
        @('1000.00','1,000.00'), @('-92750.00','(92,750.00)'), @('-0.01','(0.01)'),
        @('123456789012345678901234567890.99','123,456,789,012,345,678,901,234,567,890.99'),
        @('00.00','00.00'), @($null,$null), @('12.3','#ERROR'), @('1e3','#ERROR'),
        @('1,000.00','#ERROR'), @('1.2.3','#ERROR'), @('abc','#ERROR'), @('','#ERROR')
    )
    $caseRows = for ($index=0; $index -lt $cases.Count; $index++) {
        $raw = if ($null -eq $cases[$index][0]) { 'null' } else { Literal $cases[$index][0] }
        '{' + (Literal ([string]$index)) + ', ' + $raw + '}'
    }
    $table = [Microsoft.AnalysisServices.Tabular.Table]::new()
    $table.Name = $temporaryName
    foreach ($name in @('ID','Raw','Display')) {
        $column = [Microsoft.AnalysisServices.Tabular.DataColumn]::new()
        $column.Name = $name; $column.SourceColumn = $name
        $column.DataType = [Microsoft.AnalysisServices.Tabular.DataType]::String
        $table.Columns.Add($column)
    }
    $partition = [Microsoft.AnalysisServices.Tabular.Partition]::new()
    $partition.Name = $temporaryName
    $partition.Mode = [Microsoft.AnalysisServices.Tabular.ModeType]::Import
    $source = [Microsoft.AnalysisServices.Tabular.MPartitionSource]::new()
    $source.Expression = 'let Rows = #table(type table [ID=text, Raw=nullable text], {' + ($caseRows -join ',') + '}), Display = Table.AddColumn(Rows, "Display", each let Result = try Fx_MoneyDisplay([Raw]) in if Result[HasError] then "#ERROR" else Result[Value], type nullable text) in Display'
    $partition.Source = $source; $table.Partitions.Add($partition)
    $model.Tables.Add($table); $addedTemporaryTable = $true
    $table.RequestRefresh([Microsoft.AnalysisServices.Tabular.RefreshType]::Full)
    $model.SaveChanges() | Out-Null
    $actual = @(Query "EVALUATE '$temporaryName'")
    Require ($actual.Count -eq $cases.Count) 'Formatter case count differs.'
    foreach ($row in $actual) {
        $index = [int]$row."$temporaryName[ID]"
        Require ($row."$temporaryName[Raw]" -ceq $cases[$index][0]) 'Formatter changed the original string.'
        Require ($row."$temporaryName[Display]" -ceq $cases[$index][1]) "Formatter case $index differs."
    }
    $model.Tables.Remove($temporaryName); $model.SaveChanges() | Out-Null
    $addedTemporaryTable = $false

    foreach ($pair in @(@('Review_Exception','sample-review-exceptions.csv',@('Current','Prior','Difference')), @('Review_Evidence','sample-review-evidence.csv',@('Amount')), @('Review_Run','sample-review-run.csv',@('AbsoluteThreshold')))) {
        $actual = @(Query "EVALUATE $($pair[0])")
        foreach ($row in $actual) {
            foreach ($field in $pair[2]) {
                $raw = $row."$($pair[0])[$field]"
                $parts = $raw.TrimStart('-').Split('.')
                $expected = [regex]::Replace($parts[0], '(?<=\d)(?=(\d{3})+$)', ',') + '.' + $parts[1]
                if ($raw.StartsWith('-')) { $expected = '(' + $expected + ')' }
                Require ($row."$($pair[0])[$($field)Display]" -ceq $expected) "Imported display differs: $($pair[0]).$field"
            }
        }
    }
    $base = @(Query 'EVALUATE ROW("Selected", [Finding selection present], "Question", [Finding Question])')[0]
    Require ($base.'[Selected]' -eq '0' -and $null -eq $base.'[Question]') 'Finding detail leaked without a selection.'
    $exceptions = @(Import-Csv -LiteralPath (Join-Path $folder 'sample-review-exceptions.csv'))
    foreach ($exception in $exceptions) {
        foreach ($selector in @('Account','ExceptionKey')) {
            $filter = "TREATAS({$(Literal $exception.$selector)}, Review_Exception[$selector])"
            $row = @(Query ('EVALUATE CALCULATETABLE(ROW("Selected", [Finding selection present], "Action", [Finding Action], "Question", [Finding Question], "Reason", [Finding Reason], "Request", [Finding Evidence requested], "State", [Finding Evidence state]), ' + $filter + ')'))[0]
            Require ($row.'[Selected]' -eq '1') 'A single selected finding did not populate details.'
            foreach ($mapping in @(@('Action','Action'),@('Question','Question'),@('Reason','Reason'),@('Request','EvidenceRequested'),@('State','EvidenceState'))) {
                Require ($row.('['+$mapping[0]+']') -ceq $exception.($mapping[1])) 'Selected finding details differ from the raw record.'
            }
        }
    }
    $multiple = (Literal $exceptions[0].ExceptionKey) + ', ' + (Literal $exceptions[1].ExceptionKey)
    $row = @(Query ('EVALUATE CALCULATETABLE(ROW("Selected", [Finding selection present], "Question", [Finding Question]), TREATAS({' + $multiple + '}, Review_Exception[ExceptionKey]))'))[0]
    Require ($row.'[Selected]' -eq '0' -and $null -eq $row.'[Question]') 'Multiple findings leaked a single detail.'
    $row = @(Query 'EVALUATE ROW("Display", SELECTEDVALUE(Review_Run[ControlsNotRunDisplay]))')[0]
    Require ($row.'[Display]' -ceq 'Account mapping; Balance policy; Subledger; Calculation evidence') 'Readable control labels differ.'
    Write-Output "Review presentation native checks passed: $assertions assertions."
} finally {
    try {
        if ($addedTemporaryTable) { $model.Tables.Remove($temporaryName); $model.SaveChanges() | Out-Null }
    } finally {
        $connection.Close(); $connection.Dispose(); $engine.Disconnect(); $engine.Dispose()
    }
}
