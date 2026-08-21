# Local-file monthly close loop. No Xero OAuth.
#
# 1. close-control review of the sibling xero-ai-review-gateway same-FY sample TBs
#    into outputs/gateway-tb-loop (relative to this repository).
# 2. xero-ai-review-gateway evaluate against that package's bundled samples/
#    context, if the CLI is on PATH or importable. Context is never this repo's
#    examples/ (the Acme June/July pair crosses the 1 July FY reset).
#
# Set XERO_AI_REVIEW_GATEWAY_ROOT when the gateway checkout is not a sibling.

[CmdletBinding()]
param(
    [string]$GatewayRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
# close-control exits 2 for a REVIEW pack. PowerShell 7+ would otherwise treat
# that native exit as a terminating error when this preference is on.
if (Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
    $PSNativeCommandUseErrorActionPreference = $false
}

if (-not $PSScriptRoot) {
    throw "Run examples/close-loop.ps1 as a file."
}

$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

if (-not $GatewayRoot) {
    $GatewayRoot = $env:XERO_AI_REVIEW_GATEWAY_ROOT
}
if (-not $GatewayRoot) {
    $GatewayRoot = Join-Path (Split-Path -Parent $RepoRoot) "xero-ai-review-gateway"
}

$SampleDir = Join-Path $GatewayRoot "xero_ai_review_gateway"
$SampleDir = Join-Path $SampleDir "samples"
$SampleDir = Join-Path $SampleDir "inputs"
$CurrentCsv = Join-Path $SampleDir "sample-tb-2026-06-30.csv"
$PriorCsv = Join-Path $SampleDir "sample-tb-2026-05-31.csv"

if (-not ((Test-Path -LiteralPath $CurrentCsv) -and (Test-Path -LiteralPath $PriorCsv))) {
    throw @"
Cannot find the gateway same-FY sample TBs:
  $CurrentCsv
  $PriorCsv
Clone xero-ai-review-gateway as a sibling of this repository, or set XERO_AI_REVIEW_GATEWAY_ROOT.
Do not substitute examples/current_trial_balance.csv and examples/prior_trial_balance.csv: that Acme June/July pair crosses the 1 July financial-year reset and cannot feed the gateway.
"@
}

function Get-Python {
    $python = Get-Command python -ErrorAction SilentlyContinue
    if ($null -ne $python) {
        return $python.Source
    }
    throw "python is not on PATH. Install Python 3.10+ and retry."
}

function Invoke-NamedOrModuleCli {
    param(
        [Parameter(Mandatory = $true)][string]$CommandName,
        [Parameter(Mandatory = $true)][string]$ModuleName,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )
    # Do not return $LASTEXITCODE: native stdout would then mix with the
    # integer and the caller could not test the exit code.
    $cmd = Get-Command $CommandName -ErrorAction SilentlyContinue
    if ($null -ne $cmd) {
        & $cmd.Source @Arguments
        return
    }
    $python = Get-Python
    & $python -m $ModuleName @Arguments
}

$OutputDir = Join-Path $RepoRoot "outputs"
$OutputDir = Join-Path $OutputDir "gateway-tb-loop"
$ReviewArgs = @(
    "review",
    "--current", $CurrentCsv,
    "--prior", $PriorCsv,
    "--output", $OutputDir
)

Write-Host "close-control review -> $OutputDir"
Invoke-NamedOrModuleCli -CommandName "close-control" -ModuleName "closecontrol.cli" -Arguments $ReviewArgs
# 0 = PASS, 2 = REVIEW/BLOCKED pack. 1 is a malformed input or unusable --output.
if ($LASTEXITCODE -notin 0, 2) {
    throw "close-control failed with exit code $LASTEXITCODE."
}

$EvaluateArgs = @(
    "evaluate",
    "--context", "samples/contexts/sample-monthly-variance.context.json",
    "--request", "samples/requests/sample-revenue-variance.request.json",
    "--policy", "policy/demo-policy-v1.json",
    "--out", "build/gateway-tb-loop"
)

$gatewayCmd = Get-Command xero-ai-review-gateway -ErrorAction SilentlyContinue
$gatewayModule = $false
if ($null -eq $gatewayCmd) {
    $python = Get-Command python -ErrorAction SilentlyContinue
    if ($null -ne $python) {
        & $python.Source -c "import xero_ai_review_gateway.cli" 2>$null
        $gatewayModule = ($LASTEXITCODE -eq 0)
    }
}

if ($null -ne $gatewayCmd -or $gatewayModule) {
    Write-Host "xero-ai-review-gateway evaluate (bundled package samples/, not close-control examples/)"
    Invoke-NamedOrModuleCli -CommandName "xero-ai-review-gateway" -ModuleName "xero_ai_review_gateway.cli" -Arguments $EvaluateArgs
    if ($LASTEXITCODE -ne 0) {
        throw "xero-ai-review-gateway evaluate failed with exit code $LASTEXITCODE."
    }
}
else {
    Write-Host "Skipping gateway evaluate: xero-ai-review-gateway is not on PATH and the package is not importable."
}
