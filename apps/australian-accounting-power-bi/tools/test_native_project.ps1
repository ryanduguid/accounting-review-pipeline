# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

param(
    [Parameter(Mandatory = $true)][string]$InstanceFile,
    [Parameter(Mandatory = $true)][string]$SourceManifest,
    [Parameter(Mandatory = $true)][string]$EvidenceDirectory,
    [string]$SourceRoot = '',
    [string]$PowerBIBin = 'C:\Program Files\Microsoft Power BI Desktop\bin'
)

# Compose the existing checks. Refresh and input-rejection checks remain explicit.
# run-summary.json records each step and the recorded processes, whether the run passes or fails.
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Path $EvidenceDirectory -Force | Out-Null
$EvidenceDirectory = (Resolve-Path -LiteralPath $EvidenceDirectory).Path
$summary = [ordered]@{
    started_at = Get-Date -Format o; ended_at = $null; result = 'failed'; error = $null
    processes = $null; steps = [Collections.Generic.List[object]]::new()
}
# Appends a step record. The caller sets its end time and, for a child process, its raw exit code.
function Add-Step([string]$Name) {
    $step = [ordered]@{ name = $Name; started_at = Get-Date -Format o; ended_at = $null; exit_code = $null }
    $summary.steps.Add($step)
    $step
}
function Test-Binding {
    $step = Add-Step 'verify_native_model'
    try { & (Join-Path $PSScriptRoot 'verify_native_model.ps1') @binding } finally { $step.ended_at = Get-Date -Format o }
}
try {
    if (-not $SourceRoot) { $SourceRoot = Join-Path $PSScriptRoot '..' }
    $SourceRoot = (Resolve-Path -LiteralPath $SourceRoot).Path
    $binding = @{
        InstanceFile = $InstanceFile; SourceManifest = $SourceManifest
        EvidencePath = (Join-Path $EvidenceDirectory 'model-binding.json')
        SourceRoot = $SourceRoot; PowerBIBin = $PowerBIBin
    }
    . (Join-Path $PSScriptRoot 'native_receipts.ps1')
    $instance = Get-Content -LiteralPath $InstanceFile -Raw | ConvertFrom-Json
    $summary.processes = Get-InstanceReceipt $instance
    Test-Binding
    foreach ($name in @('test_financial_filters', 'test_benchmark_measures', 'test_review_controls', 'test_review_projection', 'test_review_presentation')) {
        $arguments = @('-NoProfile', '-File', (Join-Path $SourceRoot "tools/$name.ps1"), '-Server', $instance.server, '-PowerBIBin', $PowerBIBin)
        if ($name -eq 'test_review_presentation') { $arguments += @('-Samples', (Join-Path $instance.project 'samples')) }
        $step = Add-Step $name
        # Under 'Stop', Windows PowerShell would end the run at a suite's first stderr line, losing its exit code.
        $output = & { $ErrorActionPreference = 'Continue'; & powershell @arguments 2>&1 }
        $result = $LASTEXITCODE
        $step.ended_at = Get-Date -Format o
        $step.exit_code = $result
        $output | Set-Content -LiteralPath (Join-Path $EvidenceDirectory "$name.log") -Encoding utf8
        if ($result -ne 0) { throw "$name failed with exit $result; see its retained log." }
        Write-Output ($output | Select-Object -Last 1)
    }
    $step = Add-Step 'measure_report_queries'
    $timings = & { $ErrorActionPreference = 'Continue'; & powershell -NoProfile -File (Join-Path $SourceRoot 'tools/measure_report_queries.ps1') -Server $instance.server -PowerBIBin $PowerBIBin 2>&1 }
    $result = $LASTEXITCODE
    $step.ended_at = Get-Date -Format o
    $step.exit_code = $result
    $timings | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'query-timings.json') -Encoding utf8
    if ($result -ne 0) { throw "Query timing failed with exit $result; see its retained log." }
    Test-Binding
    $summary.result = 'passed'
} catch {
    $summary.error = $_.Exception.Message
    throw
} finally {
    $summary.ended_at = Get-Date -Format o
    $summary | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'run-summary.json') -Encoding utf8
}
Write-Output 'Native project checks passed.'
