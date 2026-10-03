param(
    [Parameter(Mandatory = $true)][string]$InstanceFile,
    [Parameter(Mandatory = $true)][string]$SourceManifest,
    [Parameter(Mandatory = $true)][string]$EvidenceDirectory,
    [string]$SourceRoot = '',
    [string]$PowerBIBin = 'C:\Program Files\Microsoft Power BI Desktop\bin'
)

# Compose the existing checks. Refresh and input-rejection checks remain explicit.
$ErrorActionPreference = 'Stop'
if (-not $SourceRoot) { $SourceRoot = Join-Path $PSScriptRoot '..' }
$SourceRoot = (Resolve-Path -LiteralPath $SourceRoot).Path
New-Item -ItemType Directory -Path $EvidenceDirectory -Force | Out-Null
$EvidenceDirectory = (Resolve-Path -LiteralPath $EvidenceDirectory).Path
$binding = @{
    InstanceFile = $InstanceFile; SourceManifest = $SourceManifest
    EvidencePath = (Join-Path $EvidenceDirectory 'model-binding.json')
    SourceRoot = $SourceRoot; PowerBIBin = $PowerBIBin
}
& (Join-Path $PSScriptRoot 'verify_native_model.ps1') @binding
$instance = Get-Content -LiteralPath $InstanceFile -Raw | ConvertFrom-Json
foreach ($name in @('test_financial_filters', 'test_benchmark_measures', 'test_review_controls', 'test_review_projection', 'test_review_presentation')) {
    $arguments = @('-NoProfile', '-File', (Join-Path $SourceRoot "tools/$name.ps1"), '-Server', $instance.server, '-PowerBIBin', $PowerBIBin)
    if ($name -eq 'test_review_presentation') { $arguments += @('-Samples', (Join-Path $instance.project 'samples')) }
    $output = & powershell @arguments 2>&1
    $result = $LASTEXITCODE
    $output | Set-Content -LiteralPath (Join-Path $EvidenceDirectory "$name.log") -Encoding utf8
    if ($result -ne 0) { throw "$name failed with exit $result; see its retained log." }
    Write-Output ($output | Select-Object -Last 1)
}
$timings = & powershell -NoProfile -File (Join-Path $SourceRoot 'tools/measure_report_queries.ps1') -Server $instance.server -PowerBIBin $PowerBIBin 2>&1
$result = $LASTEXITCODE
$timings | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'query-timings.json') -Encoding utf8
if ($result -ne 0) { throw "Query timing failed with exit $result; see its retained log." }
& (Join-Path $PSScriptRoot 'verify_native_model.ps1') @binding
Write-Output 'Native project checks passed.'
