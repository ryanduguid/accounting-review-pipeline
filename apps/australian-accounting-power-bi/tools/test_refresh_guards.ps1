param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^localhost:[0-9]+$')]
    [string]$Server,
    [Parameter(Mandatory = $true)]
    [string]$SampleFolder,
    [string]$PowerBIBin = 'C:\Program Files\Microsoft Power BI Desktop\bin'
)

# This gate deliberately corrupts fabricated inputs. Use a disposable copy only.
$ErrorActionPreference = 'Stop'
$folder = (Resolve-Path -LiteralPath $SampleFolder).Path
if (-not (Test-Path -LiteralPath (Join-Path $folder '.native-test-copy'))) {
    throw 'Create .native-test-copy inside a disposable copy of samples before running this gate.'
}
$committedSamples = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../samples')).Path
if ($folder -eq $committedSamples) { throw 'Refusing to mutate the committed samples directory.' }
[Reflection.Assembly]::LoadFrom((Join-Path $PowerBIBin 'Microsoft.PowerBI.Tabular.dll')) | Out-Null
$engine = New-Object Microsoft.AnalysisServices.Tabular.Server
$engine.Connect($Server)
try {
    if ($engine.Databases.Count -ne 1) { throw 'Expected one disposable database.' }
    $model = $engine.Databases[0].Model
    $configured = $model.Expressions['SampleFolder'].Expression
    if (-not $configured.Contains($folder.Replace('\', '/')) -and -not $configured.Contains($folder)) {
        throw 'The open model does not point to the supplied disposable samples directory.'
    }
    $cases = @(
        @{ Name = 'offsetting amount corruption'; File = 'general-ledger'; Table = 'Fact_GeneralLedger'; Error = 'Invalid ledger line' },
        @{ Name = 'duplicate journal line'; File = 'general-ledger'; Table = 'Fact_GeneralLedger'; Error = 'Duplicate key' },
        @{ Name = 'missing journal identifier'; File = 'general-ledger'; Table = 'Fact_GeneralLedger'; Error = 'Missing required value' },
        @{ Name = 'unknown account'; File = 'general-ledger'; Table = 'Fact_GeneralLedger'; Error = 'Invalid ledger line' },
        @{ Name = 'unknown entity'; File = 'general-ledger'; Table = 'Fact_GeneralLedger'; Error = 'Invalid ledger line' },
        @{ Name = 'date beyond model'; File = 'general-ledger'; Table = 'Fact_GeneralLedger'; Error = 'Invalid ledger line' },
        @{ Name = 'missing balanced counterparty'; File = 'general-ledger'; Table = 'Fact_GeneralLedger'; Error = 'Unmatched intercompany pair' },
        @{ Name = 'unresolved group counterparty'; File = 'general-ledger'; Table = 'Fact_GeneralLedger'; Error = 'Invalid ledger line' },
        @{ Name = 'unknown budget account'; File = 'budgets'; Table = 'Fact_Budget'; Error = 'Invalid budget key or period' },
        @{ Name = 'duplicate entity'; File = 'entities'; Table = 'Dim_Entity'; Error = 'Duplicate key' },
        @{ Name = 'foreign currency'; File = 'entities'; Table = 'Dim_Entity'; Error = 'Unsupported group' },
        @{ Name = 'minority ownership'; File = 'entities'; Table = 'Dim_Entity'; Error = 'Unsupported group' },
        @{ Name = 'missing fund receipt'; File = 'payroll-super'; Table = 'Fact_PayrollSuper'; Error = 'Unsupported payroll scenario' },
        @{ Name = 'incorrect receipt status'; File = 'payroll-super'; Table = 'Fact_PayrollSuper'; Error = 'Unsupported payroll scenario' },
        @{ Name = 'duplicate payroll event'; File = 'payroll-super'; Table = 'Fact_PayrollSuper'; Error = 'Duplicate key' },
        @{ Name = 'invalid amount type'; File = 'general-ledger'; Table = 'Fact_GeneralLedger'; Error = 'Invalid source values' },
        @{ Name = 'unknown industry'; File = 'entities'; Table = 'Dim_Entity'; Error = 'Unknown industry' },
        @{ Name = 'unmapped expense category'; File = 'chart-of-accounts'; Table = 'Dim_Account'; Error = 'Unknown account classification' },
        @{ Name = 'reversed benchmark range'; File = 'ato-benchmarks'; Table = 'Fact_ATOBenchmark'; Error = 'Invalid sample reference' },
        @{ Name = 'wrong review basis'; File = 'review-run'; Table = 'Review_Run'; Error = 'Review fixture context differs' },
        @{ Name = 'duplicate exception'; File = 'review-exceptions'; Table = 'Review_Exception'; Error = 'Duplicate key' },
        @{ Name = 'duplicate evidence line'; File = 'review-evidence'; Table = 'Review_Evidence'; Error = 'Duplicate key' },
        @{ Name = 'missing exception key'; File = 'review-evidence'; Table = 'Review_Evidence'; Error = 'Missing required value' },
        @{ Name = 'invalid review difference display'; File = 'review-exceptions'; Table = 'Review_Exception'; Error = 'Invalid decimal display value' },
        @{ Name = 'invalid evidence amount display'; File = 'review-evidence'; Table = 'Review_Evidence'; Error = 'Invalid decimal display value' },
        @{ Name = 'invalid threshold display'; File = 'review-run'; Table = 'Review_Run'; Error = 'Invalid decimal display value' },
        @{ Name = 'evidence attached to another finding'; File = 'review-evidence'; FullModel = $true; Error = 'Review evidence projection differs' },
        @{ Name = 'orphan evidence'; File = 'review-evidence'; FullModel = $true; Error = 'Review evidence projection differs' },
        @{ Name = 'different authoritative run'; File = 'review-run'; FullModel = $true; Error = 'Review exception projection differs' },
        @{ Name = 'exception from another run'; File = 'review-exceptions'; FullModel = $true; Error = 'Review exception projection differs' },
        @{ Name = 'evidence from another run'; File = 'review-evidence'; FullModel = $true; Error = 'Review evidence projection differs' },
        @{ Name = 'missing evidence account'; File = 'review-evidence'; FullModel = $true; Error = 'Missing required value' },
        @{ Name = 'evidence account differs'; File = 'review-evidence'; FullModel = $true; Error = 'Review evidence projection differs' },
        @{ Name = 'evidence tenant differs'; File = 'review-evidence'; FullModel = $true; Error = 'Review evidence projection differs' },
        @{ Name = 'uppercase run digest'; File = 'review-run'; FullModel = $true; Error = 'Review fixture context differs' },
        @{ Name = 'nonhex run digest'; File = 'review-run'; FullModel = $true; Error = 'Review fixture context differs' },
        @{ Name = 'second distinct run'; File = 'review-run'; FullModel = $true; Error = 'Review fixture context differs' },
        @{ Name = 'missing run tenant'; File = 'review-run'; FullModel = $true; Error = 'Missing required value' },
        @{ Name = 'unsupported run tenant'; File = 'review-run'; FullModel = $true; Error = 'Review fixture context differs' },
        @{ Name = 'exception tenant differs'; File = 'review-exceptions'; FullModel = $true; Error = 'Review exception projection differs' },
        @{ Name = 'missing evidence tenant'; File = 'review-evidence'; FullModel = $true; Error = 'Missing required value' }
    )
    foreach ($case in $cases) {
        $path = Join-Path $folder ("sample-$($case.File).csv")
        $original = [IO.File]::ReadAllBytes($path)
        try {
            $rows = @(Import-Csv -LiteralPath $path)
            switch ($case.Name) {
                'evidence attached to another finding' {
                    $other = Import-Csv -LiteralPath (Join-Path $folder 'sample-review-exceptions.csv') |
                        Where-Object AccountID -ne $rows[0].AccountID | Select-Object -First 1
                    if ($null -eq $other) { throw 'The fixture needs a finding for a different account.' }
                    $rows[0].ExceptionKey = $other.ExceptionKey
                }
                'orphan evidence' { $rows[0].ExceptionKey = 'NONEXISTENT' }
                'different authoritative run' { $rows[0].RunID = 'a' * 64 }
                'exception from another run' { $rows[0].RunID = 'a' * 64 }
                'evidence from another run' { $rows[0].RunID = 'a' * 64 }
                'missing evidence account' { $rows[0].AccountID = '' }
                'evidence account differs' {
                    $other = Import-Csv -LiteralPath (Join-Path $folder 'sample-review-exceptions.csv') |
                        Where-Object AccountID -ne $rows[0].AccountID | Select-Object -First 1
                    if ($null -eq $other) { throw 'The fixture needs a finding for a different account.' }
                    $rows[0].AccountID = $other.AccountID
                }
                'evidence tenant differs' { $rows[0].Tenant = 'Different fabricated tenant' }
                'uppercase run digest' { $rows[0].RunID = $rows[0].RunID.ToUpperInvariant() }
                'nonhex run digest' { $rows[0].RunID = 'g' + $rows[0].RunID.Substring(1) }
                'second distinct run' {
                    $second = $rows[0].PSObject.Copy()
                    $second.RunID = 'a' * 64
                    $rows += $second
                }
                'missing run tenant' { $rows[0].Tenant = '' }
                'unsupported run tenant' { $rows[0].Tenant = 'Different fabricated tenant' }
                'exception tenant differs' { $rows[0].Tenant = 'Different fabricated tenant' }
                'missing evidence tenant' { $rows[0].Tenant = '' }
                'invalid review difference display' { $rows[0].Difference = 'invalid' }
                'invalid evidence amount display' { $rows[0].Amount = 'invalid' }
                'invalid threshold display' { $rows[0].AbsoluteThreshold = 'invalid' }
                'offsetting amount corruption' {
                    $rows[0].Amount = ([decimal]$rows[0].Amount + 100).ToString([Globalization.CultureInfo]::InvariantCulture)
                    $rows[1].Amount = ([decimal]$rows[1].Amount - 100).ToString([Globalization.CultureInfo]::InvariantCulture)
                }
                'duplicate journal line' { $rows += $rows[0] }
                'missing journal identifier' { $rows[0].JournalID = '' }
                'unknown account' { $rows[0].AccountCode = 'UNKNOWN' }
                'unknown entity' { $rows[0].EntityID = 'UNKNOWN' }
                'date beyond model' { $rows[0].PostingDate = '2027-07-01' }
                'missing balanced counterparty' {
                    $target = ($rows | Where-Object { $_.IsIntercompany -eq 'TRUE' -and $_.EntityID -eq 'ENT002' } | Select-Object -First 1).JournalID
                    $rows = @($rows | Where-Object { $_.JournalID -ne $target })
                }
                'unresolved group counterparty' { ($rows | Where-Object IsIntercompany -eq 'TRUE' | Select-Object -First 1).IntercompanyEntityID = 'GROUP' }
                'unknown budget account' { $rows[0].AccountCode = 'UNKNOWN' }
                'duplicate entity' { $rows += $rows[0] }
                'foreign currency' { $rows[0].Currency = 'USD' }
                'minority ownership' { $rows[0].ConsolidationWeight = '0.8' }
                'missing fund receipt' { $rows[0].FundReceiptDate = '' }
                'incorrect receipt status' { $rows[0].ComplianceStatus = 'LATE_BREACH' }
                'duplicate payroll event' { $rows += $rows[0] }
                'invalid amount type' { $rows[0].Amount = 'invalid' }
                'unknown industry' { $rows[0].ANZSIC_Code = '9999' }
                'unmapped expense category' { ($rows | Where-Object Class -eq 'Expense' | Select-Object -First 1).SubClass = 'Unknown' }
                'reversed benchmark range' { $rows[0].GrossProfitPct_Low = '99.0' }
                'wrong review basis' { $rows[0].Basis = 'Cash' }
                'duplicate exception' { $rows += $rows[0] }
                'duplicate evidence line' { $rows += $rows[0] }
                'missing exception key' { $rows[0].ExceptionKey = '' }
            }
            $rows | Export-Csv -LiteralPath $path -NoTypeInformation -Encoding UTF8
            $failure = $null
            try {
                if ($case.FullModel) {
                    $model.RequestRefresh([Microsoft.AnalysisServices.Tabular.RefreshType]::Full)
                } else {
                    $model.Tables[$case.Table].RequestRefresh([Microsoft.AnalysisServices.Tabular.RefreshType]::Full)
                }
                $model.SaveChanges() | Out-Null
            } catch { $failure = $_.Exception.ToString() }
            if ($null -eq $failure) { throw "Refresh accepted $($case.Name)." }
            if (-not $failure.Contains($case.Error)) { throw "Unexpected failure for $($case.Name): $failure" }
            Write-Output "PASS rejected $($case.Name): $($case.Error)"
        } finally {
            [IO.File]::WriteAllBytes($path, $original)
            # Discard failed pending refresh requests before the next mutation.
            $engine.Refresh()
            $model = $engine.Databases[0].Model
        }
    }
    $model.RequestRefresh([Microsoft.AnalysisServices.Tabular.RefreshType]::Full)
    $model.SaveChanges() | Out-Null
    Write-Output "$($cases.Count) refresh rejection cases passed; restored fabricated inputs refreshed successfully."
} finally {
    $engine.Disconnect()
}
