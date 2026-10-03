param(
    [Parameter(Mandatory = $true)][string]$InstanceFile,
    [Parameter(Mandatory = $true)][string]$SourceManifest,
    [Parameter(Mandatory = $true)][string]$EvidencePath,
    [string]$SourceRoot = '',
    [string]$PowerBIBin = 'C:\Program Files\Microsoft Power BI Desktop\bin'
)

# Read-only comparison of a recorded disposable instance with pinned source.
$ErrorActionPreference = 'Stop'
if (-not $SourceRoot) { $SourceRoot = Join-Path $PSScriptRoot '..' }
[Reflection.Assembly]::LoadFrom((Join-Path $PowerBIBin 'Microsoft.PowerBI.Tabular.dll')) | Out-Null
$SourceRoot = (Resolve-Path -LiteralPath $SourceRoot).Path
$instance = Get-Content -LiteralPath $InstanceFile -Raw | ConvertFrom-Json
if ($instance.server -notmatch '^localhost:[0-9]+$') { throw 'Expected a localhost model endpoint.' }
if ((Resolve-Path -LiteralPath $instance.project).Path -eq $SourceRoot) { throw 'Expected a separate disposable project.' }
$expected = [Microsoft.AnalysisServices.Tabular.TmdlSerializer]::DeserializeDatabaseFromFolder((Join-Path $SourceRoot 'australian-accounting-power-bi.SemanticModel/definition'))
$expectedFolder = (Join-Path $instance.project 'samples').Replace('\', '/')
$expected.Model.Expressions['SampleFolder'].Expression = $expected.Model.Expressions['SampleFolder'].Expression.Replace('""', ('"' + $expectedFolder + '"'))
$engine = New-Object Microsoft.AnalysisServices.Tabular.Server
$engine.Connect($instance.server)
function Normalise-Expression([string]$value) { $value.Replace("`r`n", "`n").Trim() }
try {
    if ($engine.Databases.Count -ne 1) { throw 'Expected one disposable database.' }
    $actual = $engine.Databases[0].Model
    if ($actual.Tables.Count -ne $expected.Model.Tables.Count) { throw 'Table inventory mismatch.' }
    $measures = 0
    $partitions = 0
    $items = 0
    foreach ($table in $expected.Model.Tables) {
        $loaded = $actual.Tables[$table.Name]
        # Desktop adds an internal row-number column when it loads each table.
        $declaredColumns = @($loaded.Columns | Where-Object { $_ -isnot [Microsoft.AnalysisServices.Tabular.RowNumberColumn] })
        if ($null -eq $loaded -or $loaded.Measures.Count -ne $table.Measures.Count -or $declaredColumns.Count -ne $table.Columns.Count) { throw "Field inventory mismatch: $($table.Name)" }
        foreach ($column in $table.Columns) {
            if ($null -eq $loaded.Columns[$column.Name] -or $loaded.Columns[$column.Name].DataType -ne $column.DataType) { throw "Column identity differs: $($table.Name), $($column.Name)" }
            foreach ($property in @('SourceColumn', 'Expression')) {
                if ($null -ne $column.PSObject.Properties[$property] -and (Normalise-Expression ([string]$column.$property)) -ne (Normalise-Expression ([string]$loaded.Columns[$column.Name].$property))) { throw "Column definition differs: $($table.Name), $($column.Name), $property" }
            }
        }
        foreach ($measure in $table.Measures) {
            if ((Normalise-Expression $measure.Expression) -ne (Normalise-Expression $loaded.Measures[$measure.Name].Expression)) { throw "Loaded measure differs: $($measure.Name)" }
            $measures++
        }
        if ($loaded.Partitions.Count -ne $table.Partitions.Count) { throw "Partition inventory mismatch: $($table.Name)" }
        foreach ($partition in $table.Partitions) {
            $loadedPartition = $loaded.Partitions[$partition.Name]
            if ((Normalise-Expression $partition.Source.Expression) -ne (Normalise-Expression $loadedPartition.Source.Expression)) { throw "Partition differs: $($table.Name)" }
            $partitions++
        }
        if ($null -ne $table.CalculationGroup) {
            if ($loaded.CalculationGroup.CalculationItems.Count -ne $table.CalculationGroup.CalculationItems.Count) { throw 'Calculation item inventory differs.' }
            foreach ($item in $table.CalculationGroup.CalculationItems) {
                if ((Normalise-Expression $item.Expression) -ne (Normalise-Expression $loaded.CalculationGroup.CalculationItems[$item.Name].Expression)) { throw "Calculation item differs: $($item.Name)" }
                $items++
            }
        }
    }
    if ($actual.Expressions.Count -ne $expected.Model.Expressions.Count) { throw 'Expression inventory differs.' }
    foreach ($expression in $expected.Model.Expressions) {
        if ((Normalise-Expression $expression.Expression) -ne (Normalise-Expression $actual.Expressions[$expression.Name].Expression)) { throw "Expression differs: $($expression.Name)" }
    }
    if ($actual.Relationships.Count -ne $expected.Model.Relationships.Count) { throw 'Relationship inventory differs.' }
    foreach ($relationship in $expected.Model.Relationships) {
        $loaded = $actual.Relationships[$relationship.Name]
        if ($null -eq $loaded) { throw "Missing relationship: $($relationship.Name)" }
        foreach ($endpoint in @('FromColumn','ToColumn')) {
            if ($loaded.$endpoint.Table.Name -ne $relationship.$endpoint.Table.Name -or $loaded.$endpoint.Name -ne $relationship.$endpoint.Name) { throw "Relationship endpoint differs: $($relationship.Name), $endpoint" }
        }
        foreach ($property in @('FromCardinality','ToCardinality','IsActive','CrossFilteringBehavior')) {
            if ([string]$loaded.$property -ne [string]$relationship.$property) { throw "Relationship differs: $($relationship.Name), $property" }
        }
    }
    $desktop = Get-CimInstance Win32_Process -Filter "ProcessId = $($instance.desktop_pid)"
    $child = Get-CimInstance Win32_Process -Filter "ProcessId = $($instance.engine_pid)"
    $port = [int]($instance.server.Split(':')[1])
    $listener = Get-NetTCPConnection -LocalPort $port -State Listen
    if ($desktop.Name -ne 'PBIDesktop.exe' -or $child.Name -ne 'msmdsrv.exe' -or
        $child.ParentProcessId -ne $instance.desktop_pid -or
        $listener.OwningProcess -notcontains $instance.engine_pid -or
        [Math]::Abs(($desktop.CreationDate.ToUniversalTime() - ([datetime]$instance.started_at).ToUniversalTime()).TotalMilliseconds) -gt 1 -or
        [Math]::Abs(($child.CreationDate.ToUniversalTime() - ([datetime]$instance.engine_started_at).ToUniversalTime()).TotalMilliseconds) -gt 1) {
        throw 'Desktop and engine identity differs from the recorded instance.'
    }
    $prior = Get-Content -LiteralPath $SourceManifest -Raw | ConvertFrom-Json
    if (-not $prior.source.file_inventory -or $prior.source.file_inventory.Count -lt 1) { throw 'Source manifest has no file inventory.' }
    $sampleCount = 0
    $paths = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($file in $prior.source.file_inventory) {
        if ($file.path -notmatch '^(australian-accounting-power-bi\.pbip|australian-accounting-power-bi\.Report/(definition\.pbir|\.platform|(definition|StaticResources)/[a-zA-Z0-9_./-]+\.json)|australian-accounting-power-bi\.SemanticModel/(definition\.pbism|\.platform|definition/[a-zA-Z0-9_./-]+\.tmdl)|samples/sample-[a-z-]+\.csv)$' -or $file.path.Split('/') -contains '..') {
            throw 'Source manifest contains a path outside the project source whitelist.'
        }
        if (-not $paths.Add($file.path)) { throw 'Source manifest contains a duplicate path.' }
        if ((Get-FileHash -LiteralPath (Join-Path $sourceRoot $file.path)).Hash.ToLowerInvariant() -ne $file.source_sha256) {
            throw "Pinned source hash changed: $($file.path)"
        }
        $copyHash = $file.source_sha256
        if ($file.path -eq 'australian-accounting-power-bi.SemanticModel/definition/expressions.tmdl') {
            $text = [IO.File]::ReadAllText((Join-Path $SourceRoot $file.path))
            $declaration = 'expression SampleFolder = ""'
            if ([regex]::Matches($text, [regex]::Escape($declaration)).Count -ne 1) { throw 'Expected one unset SampleFolder declaration.' }
            $samplePath = $expectedFolder.Replace('"', '""')
            $bytes = [Text.UTF8Encoding]::new($false).GetBytes($text.Replace($declaration, ('expression SampleFolder = "' + $samplePath + '"')))
            $stream = [IO.MemoryStream]::new($bytes)
            try { $copyHash = (Get-FileHash -InputStream $stream).Hash.ToLowerInvariant() } finally { $stream.Dispose() }
        }
        if ((Get-FileHash -LiteralPath (Join-Path $instance.project $file.path)).Hash.ToLowerInvariant() -ne $copyHash) {
            throw "Disposable project hash changed: $($file.path)"
        }
        if ($file.path.StartsWith('samples/')) { $sampleCount++ }
    }
    if ($sampleCount -ne 9) { throw 'Expected six original and three review synthetic CSVs.' }
    foreach ($sample in Get-ChildItem -LiteralPath (Join-Path $SourceRoot 'samples') -Filter 'sample-*.csv' -File) {
        if (-not $paths.Contains("samples/$($sample.Name)")) { throw 'Source manifest omits a synthetic CSV.' }
    }
    $sourceFiles = @(
        Get-Item -LiteralPath (Join-Path $SourceRoot 'australian-accounting-power-bi.pbip')
        foreach ($folder in @('australian-accounting-power-bi.Report', 'australian-accounting-power-bi.SemanticModel')) {
            Get-ChildItem -LiteralPath (Join-Path $SourceRoot $folder) -Recurse -File | Where-Object {
                $_.FullName -notmatch '[\\/]\.pbi[\\/]' -and ($_.Extension -in @('.json', '.tmdl', '.pbir', '.pbism') -or $_.Name -eq '.platform')
            }
        }
        Get-ChildItem -LiteralPath (Join-Path $SourceRoot 'samples') -Filter 'sample-*.csv' -File
    )
    if ($sourceFiles.Count -ne $paths.Count) { throw 'Source manifest omits or adds project source.' }
    foreach ($file in $sourceFiles) {
        if (-not $paths.Contains($file.FullName.Substring($SourceRoot.Length + 1).Replace('\', '/'))) { throw 'Source manifest omits project source.' }
    }
    $copyFiles = @(
        Get-Item -LiteralPath (Join-Path $instance.project 'australian-accounting-power-bi.pbip')
        foreach ($folder in @('australian-accounting-power-bi.Report', 'australian-accounting-power-bi.SemanticModel')) {
            Get-ChildItem -LiteralPath (Join-Path $instance.project $folder) -Recurse -File | Where-Object {
                $_.FullName -notmatch '[\\/]\.pbi[\\/]' -and ($_.Extension -in @('.json', '.tmdl', '.pbir', '.pbism') -or $_.Name -eq '.platform')
            }
        }
        Get-ChildItem -LiteralPath (Join-Path $instance.project 'samples') -Filter 'sample-*.csv' -File
    )
    if ($copyFiles.Count -ne $paths.Count) { throw 'Disposable project inventory differs.' }
    foreach ($file in $copyFiles) {
        if (-not $paths.Contains($file.FullName.Substring($instance.project.Length + 1).Replace('\', '/'))) { throw 'Disposable project adds project source.' }
    }
    [ordered]@{
        checked_at = Get-Date -Format o
        revision = $prior.revision
        server = $instance.server
        desktop_pid = $instance.desktop_pid
        desktop_started = $instance.started_at
        engine_pid = $instance.engine_pid
        engine_started = $instance.engine_started_at
        database_id = $engine.Databases[0].ID
        tables = $actual.Tables.Count
        measures = $measures
        partitions = $partitions
        calculation_items = $items
        relationship_endpoints = $actual.Relationships.Count * 2
        matching_synthetic_csvs = $sampleCount
        matching_project_files = $paths.Count
    } | ConvertTo-Json | Set-Content -LiteralPath $EvidencePath -Encoding utf8
    Write-Output "Native source and instance verified: $measures measures, $partitions partitions, $items calculation items, $sampleCount synthetic CSVs."
} finally { $engine.Disconnect() }
