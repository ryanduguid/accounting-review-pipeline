param(
    [Parameter(Mandatory = $true)][ValidatePattern('^localhost:[0-9]+$')][string]$Server,
    [Parameter(Mandatory = $true)][string]$SampleFolder,
    [string]$PowerBIBin = 'C:\Program Files\Microsoft Power BI Desktop\bin'
)

# This gate mutates only marked disposable fabricated inputs and restores exact bytes.
$ErrorActionPreference = 'Stop'
$folder = (Resolve-Path -LiteralPath $SampleFolder).Path
if (-not (Test-Path -LiteralPath (Join-Path $folder '.native-test-copy'))) { throw 'Disposable sample marker is missing.' }
if ($folder -eq (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../samples')).Path) { throw 'Refusing committed samples.' }
$evidencePath = Join-Path $folder 'sample-review-evidence.csv'
$findingPath = Join-Path $folder 'sample-review-exceptions.csv'
$originalEvidence = [IO.File]::ReadAllBytes($evidencePath)
$originalFindings = [IO.File]::ReadAllBytes($findingPath)
$evidenceHeader = [Text.Encoding]::UTF8.GetString($originalEvidence).Split("`n")[0].TrimEnd("`r")
[Reflection.Assembly]::LoadFrom((Join-Path $PowerBIBin 'Microsoft.PowerBI.Tabular.dll')) | Out-Null
$engine = New-Object Microsoft.AnalysisServices.Tabular.Server
$engine.Connect($Server)

function Write-Evidence($Rows) {
    if (@($Rows).Count -eq 0) {
        [IO.File]::WriteAllText($evidencePath, $evidenceHeader + "`n", [Text.UTF8Encoding]::new($false))
    } else {
        $Rows | Export-Csv -LiteralPath $evidencePath -NoTypeInformation -Encoding UTF8
    }
}

try {
    if ($engine.Databases.Count -ne 1) { throw 'Expected one disposable database.' }
    $model = $engine.Databases[0].Model
    if (-not $model.Expressions['SampleFolder'].Expression.Contains($folder.Replace('\', '/'))) { throw 'Disposable source binding differs.' }
    $cases = @(
        @{ Name = 'coordinated finding and account reassignment'; Error = 'Review evidence journal population or detail differs' },
        @{ Name = 'missing one journal row'; Error = 'Review evidence journal population or detail differs' },
        @{ Name = 'missing one finding population'; Error = 'Review evidence journal population or detail differs' },
        @{ Name = 'empty evidence population'; Error = 'Review evidence journal population or detail differs' },
        @{ Name = 'changed transaction identifier'; Error = 'Review evidence journal population or detail differs' },
        @{ Name = 'changed amount'; Error = 'Review evidence journal population or detail differs' },
        @{ Name = 'changed date'; Error = 'Review evidence journal population or detail differs' },
        @{ Name = 'changed reference'; Error = 'Review evidence journal population or detail differs' },
        @{ Name = 'changed description'; Error = 'Review evidence journal population or detail differs' },
        @{ Name = 'offsetting evidence amount corruption'; Error = 'Review evidence journal population or detail differs' },
        @{ Name = 'extra journal row'; Error = 'Review evidence journal population or detail differs' },
        @{ Name = 'evidence for unavailable finding'; Error = 'Unavailable findings cannot carry journal evidence' },
        @{ Name = 'reordered evidence'; Accept = $true },
        @{ Name = 'reordered findings'; Accept = $true },
        @{ Name = 'one unavailable finding'; Accept = $true },
        @{ Name = 'all unavailable findings'; Accept = $true },
        @{ Name = 'repeated account finding with complete evidence'; Accept = $true }
    )
    foreach ($case in $cases) {
        try {
            $evidence = @(Import-Csv -LiteralPath $evidencePath)
            $findings = @(Import-Csv -LiteralPath $findingPath)
            $key = $evidence[0].ExceptionKey
            $finding = $findings | Where-Object ExceptionKey -eq $key | Select-Object -First 1
            switch ($case.Name) {
                'coordinated finding and account reassignment' {
                    $other = $findings | Where-Object AccountID -ne $evidence[0].AccountID | Select-Object -First 1
                    if ($null -eq $other) { throw 'Expected a different finding account.' }
                    $evidence[0].ExceptionKey = $other.ExceptionKey
                    $evidence[0].AccountID = $other.AccountID
                }
                'missing one journal row' { $evidence = @($evidence | Select-Object -Skip 1) }
                'missing one finding population' { $evidence = @($evidence | Where-Object ExceptionKey -ne $key) }
                'empty evidence population' { $evidence = @() }
                'changed transaction identifier' { $evidence[0].TransactionID = 'NONEXISTENT:1' }
                'changed amount' { $evidence[0].Amount = ([decimal]$evidence[0].Amount + [decimal]0.01).ToString('0.00', [Globalization.CultureInfo]::InvariantCulture) }
                'changed date' { $evidence[0].Date = '2024-09-01' }
                'changed reference' { $evidence[0].Reference = 'Changed fixture reference' }
                'changed description' { $evidence[0].Description = 'Changed fixture description' }
                'offsetting evidence amount corruption' {
                    $pair = $evidence | Group-Object ExceptionKey | Where-Object Count -ge 2 | Select-Object -First 1
                    if ($null -eq $pair) { throw 'Expected two evidence rows in one finding.' }
                    $pair.Group[0].Amount = ([decimal]$pair.Group[0].Amount + 5).ToString('0.00', [Globalization.CultureInfo]::InvariantCulture)
                    $pair.Group[1].Amount = ([decimal]$pair.Group[1].Amount - 5).ToString('0.00', [Globalization.CultureInfo]::InvariantCulture)
                }
                'extra journal row' {
                    $extra = $evidence[0].PSObject.Copy()
                    $extra.TransactionID = 'EXTRA:1'
                    $evidence += $extra
                }
                'evidence for unavailable finding' { $finding.EvidenceState = 'Line-level evidence is unavailable under this control contract' }
                'reordered evidence' { [Array]::Reverse($evidence) }
                'reordered findings' { [Array]::Reverse($findings) }
                'one unavailable finding' {
                    $finding.EvidenceState = 'Line-level evidence is unavailable under this control contract'
                    $finding.AccountID = ''
                    $evidence = @($evidence | Where-Object ExceptionKey -ne $key)
                }
                'all unavailable findings' {
                    foreach ($item in $findings) { $item.EvidenceState = 'Line-level evidence is unavailable under this control contract' }
                    $evidence = @()
                }
                'repeated account finding with complete evidence' {
                    $duplicate = $finding.PSObject.Copy()
                    $duplicate.ExceptionKey = $key + ':repeat'
                    $findings += $duplicate
                    $duplicates = @($evidence | Where-Object ExceptionKey -eq $key | ForEach-Object {
                        $row = $_.PSObject.Copy()
                        $row.ExceptionKey = $duplicate.ExceptionKey
                        $row
                    })
                    $evidence += $duplicates
                }
            }
            Write-Evidence $evidence
            $findings | Export-Csv -LiteralPath $findingPath -NoTypeInformation -Encoding UTF8
            $failure = $null
            try {
                $model.RequestRefresh([Microsoft.AnalysisServices.Tabular.RefreshType]::Full)
                $model.SaveChanges() | Out-Null
            } catch { $failure = $_.Exception.ToString() }
            if ($case.Accept) {
                if ($null -ne $failure) { throw "Positive control failed for $($case.Name): $failure" }
                Write-Output "PASS accepted $($case.Name)."
            } else {
                if ($null -eq $failure -or -not $failure.Contains($case.Error)) { throw "Expected refusal absent for $($case.Name): $failure" }
                Write-Output "PASS rejected $($case.Name): $($case.Error)"
            }
        } finally {
            [IO.File]::WriteAllBytes($evidencePath, $originalEvidence)
            [IO.File]::WriteAllBytes($findingPath, $originalFindings)
            $engine.Refresh()
            $model = $engine.Databases[0].Model
            $model.RequestRefresh([Microsoft.AnalysisServices.Tabular.RefreshType]::Full)
            $model.SaveChanges() | Out-Null
        }
    }
    if ((Get-FileHash -LiteralPath $evidencePath).Hash -ne (Get-FileHash -InputStream ([IO.MemoryStream]::new($originalEvidence))).Hash -or
        (Get-FileHash -LiteralPath $findingPath).Hash -ne (Get-FileHash -InputStream ([IO.MemoryStream]::new($originalFindings))).Hash) { throw 'Original fabricated CSV bytes differ.' }
    Write-Output 'Twelve evidence refusals and five positive controls passed; exact original CSVs restored and refreshed.'
} finally {
    $engine.Disconnect()
}
