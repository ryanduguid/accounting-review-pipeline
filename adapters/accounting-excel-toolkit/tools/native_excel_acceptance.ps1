#requires -Version 5.1
<#
.SYNOPSIS
Runs the repository's Power Query acceptance checks in desktop Excel.

.DESCRIPTION
Evaluates 72 checks in Excel's real Power Query engine. The default All mode
isolates the 46 core checks and 26 Payday Super checks in fresh child
PowerShell and Excel processes. The Payday child uses 20 independent
single-source queries across 19 fabricated files so Excel's
cross-source privacy/firewall composition state cannot mask adapter behaviour.
The checks cover both fabricated Xero trial-balance layouts, financial-year
boundaries, ABN validation, header promotion, and adverse and lazy-evaluation
branches. Payday scaling checks materialise 500, 5,000 and 10,000 contribution
rows and report their measured refresh times.

This runner requires Windows, Windows PowerShell 5.1 or newer, desktop
Microsoft Excel, Power Query, and the Microsoft.Mashup.OleDb.1 provider. It
does not import or execute VBA. Repository sources and sample files are read
only. Generated fixtures, workbooks and private child results live in
GUID-named directories under the operating system's temporary directory and
are removed in finally.

.PARAMETER RepositoryRoot
Path to the repository checkout to test. By default this is the
repository containing this script.

.PARAMETER CheckSet
Internal isolation mode. The default All mode launches fresh child PowerShell
processes for the 46 core checks and 26 Payday Super checks. Core and Payday
are child modes so Excel's Mashup host cannot carry file-source state from one
group into the other.

.PARAMETER ResultPath
Private child-process result path. All mode creates this path beneath a
GUID-named temporary directory; direct callers should not set it.

.EXAMPLE
powershell -NoProfile -File .\tools\native_excel_acceptance.ps1

.EXAMPLE
powershell -NoProfile -File .\tools\native_excel_acceptance.ps1 -RepositoryRoot C:\src\accounting-excel-toolkit
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateNotNullOrEmpty()]
    [string]$RepositoryRoot,
    [ValidateSet('All', 'Core', 'Payday')]
    [string]$CheckSet = 'All',
    [string]$ResultPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $scriptPath = $MyInvocation.MyCommand.Path
    if ([string]::IsNullOrWhiteSpace($scriptPath)) {
        throw 'Could not determine the native acceptance script path.'
    }
    $scriptDirectory = Split-Path -Parent $scriptPath
    $RepositoryRoot = Split-Path -Parent $scriptDirectory
}

$repository = (Resolve-Path -LiteralPath $RepositoryRoot -ErrorAction Stop).ProviderPath

$script:cleanupFailed = $false

function ConvertTo-MText {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Value
    )

    # Quotes are doubled in M string literals; backslashes are literal.
    return '"' + ($Value -replace '"', '""') + '"'
}

function Write-PaydayScaleFixture {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [int]$RowCount,
        [Parameter(Mandatory = $true)]
        [string]$HeaderLine,
        [Parameter(Mandatory = $true)]
        [string]$ProvenanceLine
    )

    $encoding = New-Object System.Text.UTF8Encoding($true)
    $writer = New-Object System.IO.StreamWriter($Path, $false, $encoding)
    try {
        $writer.NewLine = "`r`n"
        $writer.WriteLine($HeaderLine)
        for ($row = 1; $row -le $RowCount; $row++) {
            $writer.WriteLine((
                '{0},E{0:D6},2026-06-30,quarterly,2026-07-28,ON_TIME,' +
                '0,fund receipt,100.00,0.00,0.00,0.00,0.00,0.00,0.00,' +
                'Fabricated scale fixture,Fabricated scale contribution,' -f $row
            ))
        }
        $writer.WriteLine($ProvenanceLine)
    }
    finally {
        $writer.Dispose()
    }
}

function Test-SafeTemporaryDirectory {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Candidate,
        [Parameter(Mandatory = $true)]
        [string]$SystemTemporaryRoot
    )

    $candidateFullPath = [IO.Path]::GetFullPath($Candidate)
    $expectedParent = [IO.Path]::GetFullPath($SystemTemporaryRoot).TrimEnd(
        [IO.Path]::DirectorySeparatorChar
    )
    $actualParent = [IO.Path]::GetDirectoryName($candidateFullPath).TrimEnd(
        [IO.Path]::DirectorySeparatorChar
    )
    $leaf = [IO.Path]::GetFileName($candidateFullPath)

    return [string]::Equals(
        $actualParent,
        $expectedParent,
        [StringComparison]::OrdinalIgnoreCase
    ) -and $leaf -match (
        '^sir-alexander-fitzgerald-native-' +
        '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
    )
}

function Release-ComReference {
    param(
        [AllowNull()]
        [object]$Reference,
        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    if ($null -eq $Reference) {
        return
    }

    if (-not [Runtime.InteropServices.Marshal]::IsComObject($Reference)) {
        [Console]::Error.WriteLine("CLEANUP ERROR: $Label is not a COM object.")
        $script:cleanupFailed = $true
        return
    }

    try {
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Reference)
    }
    catch {
        [Console]::Error.WriteLine(
            "CLEANUP ERROR: could not release $Label. $($_.Exception.Message)"
        )
        $script:cleanupFailed = $true
    }
}

function ConvertFrom-NativeCheckValues {
    param(
        [Parameter(Mandatory = $true)]
        [array]$Values
    )

    $rows = @()
    $rowLower = $Values.GetLowerBound(0)
    $rowUpper = $Values.GetUpperBound(0)
    $columnLower = $Values.GetLowerBound(1)
    for ($row = $rowLower; $row -le $rowUpper; $row++) {
        $passValue = $Values[$row, ($columnLower + 3)]
        $rows += [pscustomobject]@{
            Check = [string]$Values[$row, $columnLower]
            Expected = [string]$Values[$row, ($columnLower + 1)]
            Actual = [string]$Values[$row, ($columnLower + 2)]
            Pass = (($passValue -eq $true) -or ([string]$passValue -eq 'TRUE'))
        }
    }
    return $rows
}

function Write-NativeCheckSummary {
    param(
        [Parameter(Mandatory = $true)]
        [object[]]$Rows,
        [Parameter(Mandatory = $true)]
        [string]$ExcelVersion,
        [Parameter(Mandatory = $true)]
        [string]$ExcelBuild
    )

    Write-Host ''
    Write-Host (
        'Excel {0} build {1}; locale {2}' -f
            $ExcelVersion,
            $ExcelBuild,
            (Get-Culture).Name
    )
    Write-Host ('-' * 78)
    foreach ($row in $Rows) {
        Write-Host ('{0}  {1}' -f $(if ($row.Pass) { 'PASS' } else { 'FAIL' }), $row.Check)
        if (-not $row.Pass) {
            Write-Host ('        expected [{0}]  actual [{1}]' -f $row.Expected, $row.Actual)
        }
    }
    $failedChecks = @($Rows | Where-Object { -not $_.Pass }).Count
    Write-Host ('-' * 78)
    Write-Host ('{0} checks, {1} failed' -f $Rows.Count, $failedChecks)
    return $failedChecks
}

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'Native Excel acceptance requires Windows and desktop Microsoft Excel.'
}

$powerQueryDirectory = Join-Path $repository 'powerquery'
$combinedFixture = Join-Path $repository 'samples\sample-xero-trial-balance.csv'
$columnsFixture = Join-Path $repository 'samples\sample-xero-trial-balance-columns.csv'
$paydaySuperFixture = Join-Path $repository 'samples\sample-payday-super-report.csv'

foreach ($requiredPath in @(
    $powerQueryDirectory,
    $combinedFixture,
    $columnsFixture,
    $paydaySuperFixture
)) {
    if (-not (Test-Path -LiteralPath $requiredPath)) {
        throw "Required repository path is missing: $requiredPath"
    }
}

$powerQueryFiles = @(
    Get-ChildItem -LiteralPath $powerQueryDirectory -Filter '*.pq' -File |
        Sort-Object Name
)
if ($powerQueryFiles.Count -eq 0) {
    throw "No Power Query source files were found under $powerQueryDirectory."
}

$systemTemporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())

if ($CheckSet -eq 'All') {
    if (-not [string]::IsNullOrWhiteSpace($ResultPath)) {
        throw 'ResultPath is private to Core and Payday child processes.'
    }

    $parentTemporaryDirectory = Join-Path $systemTemporaryRoot (
        'sir-alexander-fitzgerald-native-' + [guid]::NewGuid().ToString('D')
    )
    $parentTemporaryDirectory = [IO.Path]::GetFullPath($parentTemporaryDirectory)
    if (-not (Test-SafeTemporaryDirectory $parentTemporaryDirectory $systemTemporaryRoot)) {
        throw "Refusing to use unexpected parent temporary path: $parentTemporaryDirectory"
    }

    $parentExitCode = 1
    $parentDirectoryCreated = $false
    try {
        New-Item -ItemType Directory -Path $parentTemporaryDirectory | Out-Null
        $parentDirectoryCreated = $true

        $childPowerShell = Join-Path $PSHOME 'powershell.exe'
        if (-not (Test-Path -LiteralPath $childPowerShell -PathType Leaf)) {
            throw "Windows PowerShell child executable is missing: $childPowerShell"
        }
        $scriptFile = $MyInvocation.MyCommand.Path
        if ([string]::IsNullOrWhiteSpace($scriptFile)) {
            throw 'Could not determine the native acceptance script path for child execution.'
        }

        # Run the Payday file-source group first. Desktop Excel's Mashup host
        # can retain the larger core group's source environment briefly after
        # that child exits; the reverse order has no dependency and keeps both
        # groups isolated without weakening either check set.
        $childPayloads = @{}
        foreach ($childSet in @('Payday', 'Core')) {
            $resultFile = Join-Path $parentTemporaryDirectory (
                $childSet.ToLowerInvariant() + '-result.json'
            )
            & $childPowerShell `
                -NoLogo `
                -NoProfile `
                -NonInteractive `
                -File $scriptFile `
                -RepositoryRoot $repository `
                -CheckSet $childSet `
                -ResultPath $resultFile
            $childExitCode = $LASTEXITCODE
            if ($childExitCode -ne 0) {
                throw "$childSet native acceptance child exited $childExitCode."
            }
            if (-not (Test-Path -LiteralPath $resultFile -PathType Leaf)) {
                throw "$childSet native acceptance child wrote no result file."
            }
            try {
                $payload = Get-Content -LiteralPath $resultFile -Raw -Encoding UTF8 |
                    ConvertFrom-Json -ErrorAction Stop
            }
            catch {
                throw "$childSet native acceptance result is not valid JSON. $($_.Exception.Message)"
            }
            if ($payload.SchemaVersion -ne 1 -or $payload.CheckSet -ne $childSet) {
                throw "$childSet native acceptance result has the wrong schema or check-set identity."
            }
            $expectedChildCount = if ($childSet -eq 'Core') { 46 } else { 26 }
            $childRows = @($payload.Rows)
            if ($childRows.Count -ne $expectedChildCount) {
                throw (
                    "$childSet native acceptance returned $($childRows.Count) rows; " +
                    "expected exactly $expectedChildCount."
                )
            }
            foreach ($childRow in $childRows) {
                if (
                    [string]::IsNullOrWhiteSpace([string]$childRow.Check) -or
                    $childRow.Pass -isnot [bool]
                ) {
                    throw "$childSet native acceptance returned a malformed check row."
                }
            }
            if (
                [string]::IsNullOrWhiteSpace([string]$payload.ExcelVersion) -or
                [string]::IsNullOrWhiteSpace([string]$payload.ExcelBuild)
            ) {
                throw "$childSet native acceptance returned no Excel version/build evidence."
            }
            if ($childSet -eq 'Payday') {
                $timingRows = @($payload.Timings)
                if ($timingRows.Count -ne 3) {
                    throw "Payday native acceptance returned $($timingRows.Count) scale timings; expected 3."
                }
                $expectedScaleRows = @(500, 5000, 10000)
                foreach ($scaleRows in $expectedScaleRows) {
                    $matchingTimings = @(
                        $timingRows | Where-Object { [int]$_.ScaleRows -eq $scaleRows }
                    )
                    if (
                        $matchingTimings.Count -ne 1 -or
                        [long]$matchingTimings[0].ElapsedMilliseconds -lt 0
                    ) {
                        throw "Payday native acceptance returned malformed timing evidence for $scaleRows rows."
                    }
                }
            }
            $childPayloads[$childSet] = $payload
        }

        if (
            $childPayloads['Core'].ExcelVersion -ne $childPayloads['Payday'].ExcelVersion -or
            $childPayloads['Core'].ExcelBuild -ne $childPayloads['Payday'].ExcelBuild
        ) {
            throw 'Core and Payday children ran against different Excel versions or builds.'
        }
        $allRows = @($childPayloads['Core'].Rows) + @($childPayloads['Payday'].Rows)
        $rowCount = $allRows.Count
        if ($rowCount -ne 72) {
            throw "Combined native acceptance count was $rowCount; expected exactly 72."
        }
        $failedChecks = Write-NativeCheckSummary `
            -Rows $allRows `
            -ExcelVersion ([string]$childPayloads['Core'].ExcelVersion) `
            -ExcelBuild ([string]$childPayloads['Core'].ExcelBuild)
        foreach ($timing in @($childPayloads['Payday'].Timings) | Sort-Object ScaleRows) {
            Write-Host (
                'TIMING  Payday Super: {0} rows materialised in {1} ms' -f
                    $timing.ScaleRows,
                    $timing.ElapsedMilliseconds
            )
        }
        if ($failedChecks -eq 0) {
            $parentExitCode = 0
        }
    }
    catch {
        [Console]::Error.WriteLine('HARNESS ERROR: ' + $_.Exception.Message)
    }
    finally {
        if (
            $parentDirectoryCreated -and
            (Test-Path -LiteralPath $parentTemporaryDirectory)
        ) {
            if (Test-SafeTemporaryDirectory $parentTemporaryDirectory $systemTemporaryRoot) {
                try {
                    Remove-Item -LiteralPath $parentTemporaryDirectory -Recurse -Force
                }
                catch {
                    [Console]::Error.WriteLine(
                        'CLEANUP ERROR: could not remove the parent result directory. ' +
                        $_.Exception.Message
                    )
                    $parentExitCode = 1
                }
            }
            else {
                [Console]::Error.WriteLine(
                    "CLEANUP ERROR: refusing to remove unexpected path: " +
                    $parentTemporaryDirectory
                )
                $parentExitCode = 1
            }
        }
    }
    exit $parentExitCode
}

if ([string]::IsNullOrWhiteSpace($ResultPath)) {
    throw 'Core and Payday modes require the private ResultPath argument.'
}
$ResultPath = [IO.Path]::GetFullPath($ResultPath)
$resultDirectory = [IO.Path]::GetDirectoryName($ResultPath)
$expectedResultName = $CheckSet.ToLowerInvariant() + '-result.json'
if (
    -not (Test-SafeTemporaryDirectory $resultDirectory $systemTemporaryRoot) -or
    -not [string]::Equals(
        [IO.Path]::GetFileName($ResultPath),
        $expectedResultName,
        [StringComparison]::Ordinal
    )
) {
    throw "Refusing to write an unexpected child result path: $ResultPath"
}

$temporaryDirectory = Join-Path $systemTemporaryRoot (
    'sir-alexander-fitzgerald-native-' + [guid]::NewGuid().ToString('D')
)
$temporaryDirectory = [IO.Path]::GetFullPath($temporaryDirectory)
if (-not (Test-SafeTemporaryDirectory $temporaryDirectory $systemTemporaryRoot)) {
    throw "Refusing to use unexpected temporary path: $temporaryDirectory"
}

$temporaryWorkbook = Join-Path $temporaryDirectory 'native-excel-acceptance.xlsx'
$periodOnlyFixture = Join-Path $temporaryDirectory 'period-only.csv'
$notTrialBalanceFixture = Join-Path $temporaryDirectory 'not-a-tb.csv'
$decoyEntityFixture = Join-Path $temporaryDirectory 'decoy-entity.csv'
$badAmountFixture = Join-Path $temporaryDirectory 'bad-amount.csv'
$paydayExtraFixture = Join-Path $temporaryDirectory 'payday-extra-column.csv'
$paydayMissingHeaderFixture = Join-Path $temporaryDirectory 'payday-missing-header.csv'
$paydayDuplicateHeaderFixture = Join-Path $temporaryDirectory 'payday-duplicate-header.csv'
$paydayMalformedFixture = Join-Path $temporaryDirectory 'payday-malformed.csv'
$paydayBadAmountFixture = Join-Path $temporaryDirectory 'payday-bad-amount.csv'
$paydayNoProvenanceFixture = Join-Path $temporaryDirectory 'payday-no-provenance.csv'
$paydayEmployeeNoteFixture = Join-Path $temporaryDirectory 'payday-employee-note.csv'
$paydayDuplicateProvenanceFixture = Join-Path $temporaryDirectory 'payday-duplicate-provenance.csv'
$paydayNonTerminalProvenanceFixture = Join-Path $temporaryDirectory 'payday-non-terminal-provenance.csv'
$paydayEmptyProvenanceFixture = Join-Path $temporaryDirectory 'payday-empty-provenance.csv'
$paydayShortRowFixture = Join-Path $temporaryDirectory 'payday-short-row.csv'
$paydayNoContributionsFixture = Join-Path $temporaryDirectory 'payday-no-contributions.csv'
$paydayUnterminatedQuoteFixture = Join-Path $temporaryDirectory 'payday-unterminated-quote.csv'
$paydayUnterminatedExtraQuoteFixture = Join-Path $temporaryDirectory 'payday-unterminated-extra-quote.csv'
$paydayQuotedMultilineFixture = Join-Path $temporaryDirectory 'payday-quoted-multiline.csv'
$paydayScaleFixtures = @{}
foreach ($scaleRows in @(500, 5000, 10000)) {
    $paydayScaleFixtures[$scaleRows] = Join-Path $temporaryDirectory (
        'payday-scale-{0}.csv' -f $scaleRows
    )
}

$temporaryDirectoryCreated = $false
$exitCode = 1
$excelVersion = $null
$excelBuild = $null
$previousDisplayAlerts = $null
$previousAutomationSecurity = $null

# COM references are declared once and released in exact reverse creation order.
$excel = $null
$workbooks = $null
$workbook = $null
$worksheets = $null
$worksheet = $null
$queries = $null
$createdQueries = New-Object System.Collections.ArrayList
$currentQuery = $null
$targetRange = $null
$listObjects = $null
$listObject = $null
$queryTable = $null
$dataBodyRange = $null

try {
    New-Item -ItemType Directory -Path $temporaryDirectory | Out-Null
    $temporaryDirectoryCreated = $true

    # A period-movement-only export: no YTD pair at all.
    @'
Trial Balance
Yanille Trading Pty Ltd
For the month ended 30 June 2026

Account,Account Type,Debit,Credit
Business Bank Account (090),Bank,415.00,
Accounts Receivable (610),Current Asset,1830.00,
Wages and Salaries (477),Expense,3750.00,
Rent (469),Expense,1000.00,
Accounts Payable (800),Current Liability,,415.00
GST (820),Current Liability,,380.00
Sales (200),Revenue,,6200.00
Total,,6995.00,6995.00
'@ | Set-Content -LiteralPath $periodOnlyFixture -Encoding UTF8

    # This is deliberately not a trial balance.
    @'
Contact,Invoice Number,Due Date,Amount
Acme Pty Ltd,INV-001,30/06/2026,1100.00
'@ | Set-Content -LiteralPath $notTrialBalanceFixture -Encoding UTF8

    # An entity name beginning with "Account" must not be mistaken for a header.
    $decoyEntityContents = (
        Get-Content -LiteralPath $combinedFixture -Raw -Encoding UTF8
    ) -replace 'Yanille Trading Pty Ltd', 'Accountable Plumbing Pty Ltd'
    $decoyEntityContents |
        Set-Content -LiteralPath $decoyEntityFixture -Encoding UTF8

    # A non-numeric Debit value must raise once M's lazy column is forced.
    $badAmountContents = (
        Get-Content -LiteralPath $combinedFixture -Raw -Encoding UTF8
    ) -replace '15234\.50', 'TBC'
    $badAmountContents |
        Set-Content -LiteralPath $badAmountFixture -Encoding UTF8

    # Payday Super report variants exercise the named producer contract. They
    # derive solely from the fabricated checked-in fixture and stay under the
    # GUID-named temporary directory.
    $paydayLines = @(Get-Content -LiteralPath $paydaySuperFixture -Encoding UTF8)
    ($paydayLines | ForEach-Object { $_ + ',ignored' }) |
        Set-Content -LiteralPath $paydayExtraFixture -Encoding UTF8

    $paydayMissingHeaderLines = @($paydayLines)
    $paydayMissingHeaderLines[0] = $paydayMissingHeaderLines[0] -replace 'verdict', 'decision'
    $paydayMissingHeaderLines |
        Set-Content -LiteralPath $paydayMissingHeaderFixture -Encoding UTF8

    $paydayDuplicateHeaderLines = @($paydayLines)
    $paydayDuplicateHeaderLines[0] += ',verdict'
    for ($index = 1; $index -lt $paydayDuplicateHeaderLines.Count; $index++) {
        $paydayDuplicateHeaderLines[$index] += ',ignored'
    }
    $paydayDuplicateHeaderLines |
        Set-Content -LiteralPath $paydayDuplicateHeaderFixture -Encoding UTF8

    $paydayMalformedLines = @($paydayLines)
    $paydayMalformedLines[1] = $paydayMalformedLines[1] -replace ',000123,', ',,'
    $paydayMalformedLines |
        Set-Content -LiteralPath $paydayMalformedFixture -Encoding UTF8

    $paydayBadAmountLines = @($paydayLines)
    $paydayBadAmountLines[1] = $paydayBadAmountLines[1] -replace '780\.00', 'TBC'
    $paydayBadAmountLines |
        Set-Content -LiteralPath $paydayBadAmountFixture -Encoding UTF8

    $paydayLines[0..($paydayLines.Count - 2)] |
        Set-Content -LiteralPath $paydayNoProvenanceFixture -Encoding UTF8

    $paydayEmployeeNoteLines = @($paydayLines)
    $paydayEmployeeNoteLines[1] = $paydayEmployeeNoteLines[1] -replace ',000123,', ',NOTE,'
    $paydayEmployeeNoteLines |
        Set-Content -LiteralPath $paydayEmployeeNoteFixture -Encoding UTF8

    @($paydayLines + $paydayLines[$paydayLines.Count - 1]) |
        Set-Content -LiteralPath $paydayDuplicateProvenanceFixture -Encoding UTF8

    @(
        $paydayLines[0]
        $paydayLines[$paydayLines.Count - 1]
        $paydayLines[1..($paydayLines.Count - 2)]
    ) | Set-Content -LiteralPath $paydayNonTerminalProvenanceFixture -Encoding UTF8

    $paydayEmptyProvenanceLines = @($paydayLines)
    $paydayEmptyProvenanceFields = @($paydayEmptyProvenanceLines[-1].Split(','))
    if ($paydayEmptyProvenanceFields.Count -ne 18) {
        throw 'The fabricated terminal NOTE fixture no longer has exactly 18 fields.'
    }
    $paydayEmptyProvenanceFields[16] = ''
    $paydayEmptyProvenanceLines[-1] = [string]::Join(',', $paydayEmptyProvenanceFields)
    $paydayEmptyProvenanceLines |
        Set-Content -LiteralPath $paydayEmptyProvenanceFixture -Encoding UTF8

    $paydayShortRowLines = @($paydayLines)
    $paydayShortRowLines[2] = $paydayShortRowLines[2] -replace ',LATE or ON_TIME$', ''
    $paydayShortRowLines |
        Set-Content -LiteralPath $paydayShortRowFixture -Encoding UTF8

    @($paydayLines[0], $paydayLines[-1]) |
        Set-Content -LiteralPath $paydayNoContributionsFixture -Encoding UTF8

    $utf8WithBom = New-Object System.Text.UTF8Encoding($true)
    [IO.File]::WriteAllText(
        $paydayUnterminatedQuoteFixture,
        $paydayLines[0] + "`r`n" + '1,"unterminated',
        $utf8WithBom
    )
    $paydayExtraLines = @($paydayLines | ForEach-Object { $_ + ',ignored' })
    $paydayUnterminatedExtraText = (
        [string]::Join("`r`n", $paydayExtraLines[0..($paydayExtraLines.Count - 2)]) +
        "`r`n" + $paydayLines[-1] + ',"unterminated'
    )
    [IO.File]::WriteAllText(
        $paydayUnterminatedExtraQuoteFixture,
        $paydayUnterminatedExtraText,
        $utf8WithBom
    )
    $paydayMultilineLines = @($paydayLines)
    $paydayMultilineLines[1] = $paydayMultilineLines[1].Replace(
        '"Fabricated, ""late"" contribution"',
        '"Fabricated multiline' + "`r`n" + 'contribution"'
    )
    [IO.File]::WriteAllText(
        $paydayQuotedMultilineFixture,
        [string]::Join("`r`n", $paydayMultilineLines) + "`r`n",
        $utf8WithBom
    )

    foreach ($scaleRows in @(500, 5000, 10000)) {
        Write-PaydayScaleFixture `
            -Path $paydayScaleFixtures[$scaleRows] `
            -RowCount $scaleRows `
            -HeaderLine $paydayLines[0] `
            -ProvenanceLine $paydayLines[-1]
    }

    $mCombined = ConvertTo-MText $combinedFixture
    $mColumns = ConvertTo-MText $columnsFixture
    $mPeriodOnly = ConvertTo-MText $periodOnlyFixture
    $mNotTrialBalance = ConvertTo-MText $notTrialBalanceFixture
    $mDecoyEntity = ConvertTo-MText $decoyEntityFixture
    $mBadAmount = ConvertTo-MText $badAmountFixture
    $mPayday = ConvertTo-MText $paydaySuperFixture
    $mPaydayExtra = ConvertTo-MText $paydayExtraFixture
    $mPaydayMissingHeader = ConvertTo-MText $paydayMissingHeaderFixture
    $mPaydayDuplicateHeader = ConvertTo-MText $paydayDuplicateHeaderFixture
    $mPaydayMalformed = ConvertTo-MText $paydayMalformedFixture
    $mPaydayBadAmount = ConvertTo-MText $paydayBadAmountFixture
    $mPaydayNoProvenance = ConvertTo-MText $paydayNoProvenanceFixture
    $mPaydayEmployeeNote = ConvertTo-MText $paydayEmployeeNoteFixture
    $mPaydayDuplicateProvenance = ConvertTo-MText $paydayDuplicateProvenanceFixture
    $mPaydayNonTerminalProvenance = ConvertTo-MText $paydayNonTerminalProvenanceFixture
    $mPaydayEmptyProvenance = ConvertTo-MText $paydayEmptyProvenanceFixture
    $mPaydayShortRow = ConvertTo-MText $paydayShortRowFixture
    $mPaydayNoContributions = ConvertTo-MText $paydayNoContributionsFixture
    $mPaydayUnterminatedQuote = ConvertTo-MText $paydayUnterminatedQuoteFixture
    $mPaydayUnterminatedExtraQuote = ConvertTo-MText $paydayUnterminatedExtraQuoteFixture
    $mPaydayQuotedMultiline = ConvertTo-MText $paydayQuotedMultilineFixture
    $mPaydayScale500 = ConvertTo-MText $paydayScaleFixtures[500]
    $mPaydayScale5000 = ConvertTo-MText $paydayScaleFixtures[5000]
    $mPaydayScale10000 = ConvertTo-MText $paydayScaleFixtures[10000]

    $coreChecksM = @"
let
    // --- helpers -------------------------------------------------------
    s = (v) => if v = null then "(null)" else Text.From(v),
    near = (a, b) => a <> null and b <> null and Number.Abs(a - b) < 0.005,
    chk = (name, expected, actual) =>
        [Check = name, Expected = s(expected), Actual = s(actual), Pass = (s(expected) = s(actual))],
    raises = (f) => (try f())[HasError],

    // --- Xero_TrialBalance: both layouts -------------------------------
    tbC    = Xero_TrialBalance($mCombined),
    tbX    = Xero_TrialBalance($mColumns),
    tbCper = Xero_TrialBalance($mCombined, false),
    tbCytd = Xero_TrialBalance($mCombined, true),

    rentC = Table.SelectRows(tbC, each Text.Contains([AccountName], "Sydney")),
    rentX = Table.SelectRows(tbX, each Text.Contains([AccountName], "Sydney")),
    bankC = Table.SelectRows(tbC, each [AccountName] = "Business Bank Account"),

    keys = {"AccountCode", "AccountName", "Debit", "Credit"},
    projC = Table.SelectColumns(tbC, keys),
    projX = Table.SelectColumns(tbX, keys),
    // Table.Difference does not exist in M. Compare sorted, pipe-joined row
    // text so nulls and numeric typing cannot mask a mismatch.
    rowText = (t) => List.Sort(List.Transform(Table.ToRecords(t),
        each Text.Combine(List.Transform(Record.FieldValues(_),
            each if _ = null then "~" else Text.From(_)), "|"))),

    checks = {
        // -- combined layout, as-at (YTD) pair is the default
        chk("combined: 12 data rows (Total + blanks dropped)", 12, Table.RowCount(tbC)),
        chk("combined: YTD debit total 129934.50", true, near(List.Sum(tbC[Debit]), 129934.50)),
        chk("combined: YTD credit total 129934.50", true, near(List.Sum(tbC[Credit]), 129934.50)),
        chk("combined: balances within 0.005", true,
            Number.Abs(List.Sum(tbC[Debit]) - List.Sum(tbC[Credit])) < 0.005),
        chk("combined: default pair == explicit useYTD=true", true,
            near(List.Sum(tbCytd[Debit]), List.Sum(tbC[Debit]))),

        // -- the period pair is different and also balances
        chk("combined: useYTD=false debit total 6995.00", true, near(List.Sum(tbCper[Debit]), 6995.00)),
        chk("combined: useYTD=false credit total 6995.00", true, near(List.Sum(tbCper[Credit]), 6995.00)),
        chk("combined: period pair differs from as-at pair", true,
            not near(List.Sum(tbCper[Debit]), List.Sum(tbC[Debit]))),

        // -- "Rent (Sydney)" has a parenthetical name, not an account code
        chk("combined: Rent (Sydney) code is null", "(null)", Table.FirstValue(Table.SelectColumns(rentC, {"AccountCode"}))),
        chk("combined: Rent (Sydney) name kept intact", "Rent (Sydney)", Table.FirstValue(Table.SelectColumns(rentC, {"AccountName"}))),
        chk("columns: Rent (Sydney) code is null", "(null)", Table.FirstValue(Table.SelectColumns(rentX, {"AccountCode"}))),
        chk("columns: Rent (Sydney) name kept intact", "Rent (Sydney)", Table.FirstValue(Table.SelectColumns(rentX, {"AccountName"}))),

        // -- leading zero survives as text, rather than being coerced to 90
        chk("combined: code 090 keeps its leading zero", "090", Table.FirstValue(Table.SelectColumns(bankC, {"AccountCode"}))),
        chk("combined: AccountCode is text", true, Value.Is(Table.FirstValue(Table.SelectColumns(bankC, {"AccountCode"})), type text)),

        // -- both layouts land on the same shape
        chk("columns: 12 data rows", 12, Table.RowCount(tbX)),
        chk("parity: combined and columns layouts agree on all 12 rows", true,
            rowText(projC) = rowText(projX)),

        // --- Fx_AUFinancialYear ----------------------------------------
        chk("FY: 30 Jun 2026 -> FY2026", "FY2026", Fx_AUFinancialYear(#date(2026, 6, 30))[Label]),
        chk("FY: 1 Jul 2026 -> FY2027 (boundary)", "FY2027", Fx_AUFinancialYear(#date(2026, 7, 1))[Label]),
        chk("FY: UTC 2026-06-30T14:30Z is 1 Jul in AEST -> FY2027", "FY2027",
            Fx_AUFinancialYear(#datetimezone(2026, 6, 30, 14, 30, 0, 0, 0))[Label]),
        chk("FY: 9am +10:00 on 1 Jul 2026 -> FY2027", "FY2027",
            Fx_AUFinancialYear(#datetimezone(2026, 7, 1, 9, 0, 0, 10, 0))[Label]),
        chk("FY: text 01/07/2026 parsed en-AU, not machine locale", "FY2027",
            Fx_AUFinancialYear("01/07/2026")[Label]),
        chk("FY: null date -> null label (blank cell, not an error)", "(null)",
            Fx_AUFinancialYear(null)[Label]),
        chk("FY: StartDate/EndDate bracket the year", true,
            Fx_AUFinancialYear(#date(2026, 7, 1))[StartDate] = #date(2026, 7, 1)
                and Fx_AUFinancialYear(#date(2026, 7, 1))[EndDate] = #date(2027, 6, 30)),

        // --- Fx_ABNIsValid ---------------------------------------------
        chk("ABN: ATO example 51 824 753 556 valid", true, Fx_ABNIsValid("51 824 753 556")),
        chk("ABN: unspaced form valid", true, Fx_ABNIsValid("51824753556")),
        chk("ABN: number-typed input valid", true, Fx_ABNIsValid(51824753556)),
        chk("ABN: leading zero rejected even though checksum passes", false, Fx_ABNIsValid("00000090000")),
        chk("ABN: null rejected", false, Fx_ABNIsValid(null)),
        chk("ABN: punctuation rejected (not silently stripped)", false, Fx_ABNIsValid("51-824-753-556")),
        chk("ABN: wrong length rejected", false, Fx_ABNIsValid("5182475355")),
        chk("ABN: bad checksum rejected", false, Fx_ABNIsValid("51 824 753 557")),
        chk("ABN: unconvertible value (record) returns false, does not break refresh", false,
            Fx_ABNIsValid([unexpected = "shape"])),
        chk("ABN: unconvertible value (list) returns false", false, Fx_ABNIsValid({1, 2, 3})),

        // --- Fx_PromoteHeaderAt ----------------------------------------
        chk("PromoteHeaderAt: promotes the located header row", "Account",
            List.First(Table.ColumnNames(Fx_PromoteHeaderAt(
                #table({"Column1", "Column2"},
                    {{"Trial Balance", null}, {"Yanille Trading Pty Ltd", null},
                     {"Account", "Debit"}, {"Sales (200)", "6200.00"}}),
                "Account")))),
        chk("PromoteHeaderAt: caller value is trimmed before matching", "Account",
            List.First(Table.ColumnNames(Fx_PromoteHeaderAt(
                #table({"Column1", "Column2"}, {{"Account", "Debit"}, {"Sales (200)", "1.00"}}),
                "  Account  ")))),
        chk("PromoteHeaderAt: zero-column input raises", true,
            raises(() => Fx_PromoteHeaderAt(#table({}, {}), "Account"))),
        chk("PromoteHeaderAt: blank header value raises", true,
            raises(() => Fx_PromoteHeaderAt(#table({"Column1"}, {{"Account"}}), "   "))),
        chk("PromoteHeaderAt: header not found raises", true,
            raises(() => Fx_PromoteHeaderAt(#table({"Column1"}, {{"Contact"}}), "Account"))),
        chk("PromoteHeaderAt: zero-ROW input still raises (lazy-guard trap)", true,
            raises(() => Fx_PromoteHeaderAt(Table.FirstN(#table({"Column1"}, {{"Account"}}), 0), "Account"))),

        // --- Xero_TrialBalance adverse branches ------------------------
        chk("TB: useYTD=true on a period-only export raises", true,
            raises(() => Xero_TrialBalance($mPeriodOnly, true))),
        chk("TB: useYTD=false still works on a period-only export", true,
            near(List.Sum(Xero_TrialBalance($mPeriodOnly, false)[Debit]), 6995.00)),
        chk("TB: default picks the period pair when there is no YTD pair", true,
            near(List.Sum(Xero_TrialBalance($mPeriodOnly)[Debit]), 6995.00)),
        chk("TB: a non-Xero CSV raises rather than loading garbage", true,
            raises(() => Xero_TrialBalance($mNotTrialBalance))),
        chk("TB: an entity name starting 'Account' does not hijack the header", 12,
            Table.RowCount(Xero_TrialBalance($mDecoyEntity))),
        // M is lazy: the sum must force the Debit column to expose bad input.
        chk("TB: unparseable amount raises once the column is forced", true,
            raises(() => List.Sum(Xero_TrialBalance($mBadAmount)[Debit]))),
        chk("TB: merely calling with a bad amount does NOT raise (lazy)", false,
            raises(() => Xero_TrialBalance($mBadAmount)))
    },
    Result = Table.FromRecords(
        checks,
        type table [Check = text, Expected = text, Actual = text, Pass = logical]
    )
in
    Result
"@

    # Each Payday query reads one fabricated file only. Combining the file
    # sources in one M expression triggers Excel's privacy/firewall host
    # bug (a spurious missing Source step) even though every predicate passes
    # independently. Separate query materialisations preserve all 26 checks.
    $paydayBaseChecksM = @"
let
    s = (v) => if v = null then "(null)" else Text.From(v),
    near = (a, b) => a <> null and b <> null and Number.Abs(a - b) < 0.005,
    chk = (name, expected, actual) =>
        [Check = name, Expected = s(expected), Actual = s(actual), Pass = (s(expected) = s(actual))],
    ps = PaydaySuper_Report($mPayday),
    psFirst = Table.SelectRows(ps, each [row] = "1"),
    psFormula = Table.SelectRows(ps, each [employee_id] = "'=FORMULA"),
    checks = {
        chk("Payday Super: terminal NOTE is excluded from two data rows", 2, Table.RowCount(ps)),
        chk("Payday Super: leading-zero and escaped formula IDs stay text", true,
            Table.FirstValue(Table.SelectColumns(ps, {"employee_id"})) = "000123"
                and Value.Is(Table.FirstValue(Table.SelectColumns(ps, {"employee_id"})), type text)
                and Table.RowCount(psFormula) = 1),
        chk("Payday Super: raw verdict, caveat and unassessable range survive", true,
            Table.FirstValue(Table.SelectColumns(psFormula, {"verdict"})) = "UNKNOWN"
                and Text.Contains(Table.FirstValue(Table.SelectColumns(psFormula, {"caveats"})), "calendar coverage")
                and Table.FirstValue(Table.SelectColumns(psFormula, {"unassessable_between"})) = "LATE or ON_TIME"),
        chk("Payday Super: producer amounts are numbers, not recalculated", true,
            Value.Is(Table.FirstValue(Table.SelectColumns(ps, {"sg_amount"})), type number)
                and near(Table.FirstValue(Table.SelectColumns(ps, {"sg_amount"})), 780.00)),
        chk("Payday Super: blank producer amount stays null", "(null)",
            Table.FirstValue(Table.SelectColumns(psFormula, {"final_shortfall"}))),
        chk("Payday Super: terminal NOTE provenance is table metadata", true,
            Text.Contains(Value.Metadata(ps)[PaydaySuperProvenance], "payday-super-checker")),
        chk("Payday Super: a present empty trailing field remains valid", true,
            let trailing = Table.FirstValue(Table.SelectColumns(psFirst, {"unassessable_between"}))
            in trailing = null or trailing = "")
    },
    Result = Table.FromRecords(
        checks,
        type table [Check = text, Expected = text, Actual = text, Pass = logical]
    )
in
    Result
"@

    $paydayExtraCheckM = @"
let
    actual = Table.RowCount(PaydaySuper_Report($mPaydayExtra)),
    Result = #table(
        type table [Check = text, Expected = text, Actual = text, Pass = logical],
        {{"Payday Super: extra producer columns are tolerated", "2", Text.From(actual), actual = 2}}
    )
in
    Result
"@

    function New-PaydayExpectedErrorCheckM {
        param(
            [Parameter(Mandatory = $true)]
            [string]$Name,
            [Parameter(Mandatory = $true)]
            [string]$MExpression
        )

        $mName = ConvertTo-MText $Name
        return @"
let
    actual = (try $MExpression)[HasError],
    Result = #table(
        type table [Check = text, Expected = text, Actual = text, Pass = logical],
        {{$mName, "true", Text.From(actual), actual = true}}
    )
in
    Result
"@
    }

    function New-PaydayMalformedRowErrorCheckM {
        param(
            [Parameter(Mandatory = $true)]
            [string]$Name,
            [Parameter(Mandatory = $true)]
            [string]$MExpression,
            [Parameter(Mandatory = $true)]
            [string]$DetailContains
        )

        $mName = ConvertTo-MText $Name
        $mDetailContains = ConvertTo-MText $DetailContains
        $mExpected = ConvertTo-MText (
            'PaydaySuper.Report / Malformed report row / ' + $DetailContains
        )
        return @"
let
    attempt = try $MExpression,
    errorRecord = if attempt[HasError] then attempt[Error] else [],
    reason = Text.From(Record.FieldOrDefault(errorRecord, "Reason", "")),
    message = Text.From(Record.FieldOrDefault(errorRecord, "Message", "")),
    detail = Text.From(Record.FieldOrDefault(errorRecord, "Detail", "")),
    actual = attempt[HasError]
        and reason = "PaydaySuper.Report"
        and message = "Malformed report row"
        and Text.Contains(detail, $mDetailContains),
    actualText = if attempt[HasError]
        then reason & " / " & message & " / " & detail
        else "no error",
    Result = #table(
        type table [Check = text, Expected = text, Actual = text, Pass = logical],
        {{$mName, $mExpected, actualText, actual = true}}
    )
in
    Result
"@
    }

    $paydayCheckQueries = @(
        [pscustomobject]@{ Name = 'ZZ_PaydayBaseChecks'; ExpectedRows = 7; Source = $paydayBaseChecksM },
        [pscustomobject]@{ Name = 'ZZ_PaydayExtraCheck'; ExpectedRows = 1; Source = $paydayExtraCheckM },
        [pscustomobject]@{
            Name = 'ZZ_PaydayEmployeeNoteCheck'
            ExpectedRows = 1
            Source = @"
let
    adapted = PaydaySuper_Report($mPaydayEmployeeNote),
    literalNote = Table.SelectRows(adapted, each [employee_id] = "NOTE"),
    actual = Table.RowCount(adapted) = 2
        and Table.RowCount(literalNote) = 1
        and literalNote{0}[row] = "1",
    Result = #table(
        type table [Check = text, Expected = text, Actual = text, Pass = logical],
        {{"Payday Super: literal NOTE employee identifier remains data", "true", Text.From(actual), actual = true}}
    )
in
    Result
"@
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayMissingHeaderCheck'
            ExpectedRows = 1
            Source = New-PaydayExpectedErrorCheckM `
                -Name 'Payday Super: renamed or missing required header raises' `
                -MExpression "Table.RowCount(PaydaySuper_Report($mPaydayMissingHeader))"
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayDuplicateHeaderCheck'
            ExpectedRows = 1
            Source = New-PaydayExpectedErrorCheckM `
                -Name 'Payday Super: duplicate header raises' `
                -MExpression "Table.RowCount(PaydaySuper_Report($mPaydayDuplicateHeader))"
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayMalformedCheck'
            ExpectedRows = 1
            Source = New-PaydayExpectedErrorCheckM `
                -Name 'Payday Super: malformed contribution row raises' `
                -MExpression "Table.RowCount(PaydaySuper_Report($mPaydayMalformed))"
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayBadAmountCheck'
            ExpectedRows = 1
            Source = New-PaydayExpectedErrorCheckM `
                -Name 'Payday Super: invalid producer amount raises' `
                -MExpression "List.Sum(PaydaySuper_Report($mPaydayBadAmount)[sg_amount])"
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayNoProvenanceCheck'
            ExpectedRows = 1
            Source = New-PaydayExpectedErrorCheckM `
                -Name 'Payday Super: no terminal NOTE provenance raises' `
                -MExpression "Value.Metadata(PaydaySuper_Report($mPaydayNoProvenance))[PaydaySuperProvenance]"
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayNoProvenanceMaterialisationCheck'
            ExpectedRows = 1
            Source = New-PaydayExpectedErrorCheckM `
                -Name 'Payday Super: ordinary table use rejects absent provenance' `
                -MExpression "Table.RowCount(PaydaySuper_Report($mPaydayNoProvenance))"
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayDuplicateProvenanceCheck'
            ExpectedRows = 1
            Source = New-PaydayExpectedErrorCheckM `
                -Name 'Payday Super: ordinary table use rejects duplicate provenance' `
                -MExpression "Table.RowCount(PaydaySuper_Report($mPaydayDuplicateProvenance))"
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayNonTerminalProvenanceCheck'
            ExpectedRows = 1
            Source = New-PaydayExpectedErrorCheckM `
                -Name 'Payday Super: ordinary table use rejects non-terminal provenance' `
                -MExpression "Table.RowCount(PaydaySuper_Report($mPaydayNonTerminalProvenance))"
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayEmptyProvenanceCheck'
            ExpectedRows = 1
            Source = New-PaydayExpectedErrorCheckM `
                -Name 'Payday Super: ordinary table use rejects empty provenance' `
                -MExpression "Table.RowCount(PaydaySuper_Report($mPaydayEmptyProvenance))"
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayShortRowCheck'
            ExpectedRows = 1
            Source = @"
let
    attempt = try Table.RowCount(PaydaySuper_Report($mPaydayShortRow)),
    detail = if attempt[HasError] then Text.From(attempt[Error][Detail]) else "",
    actual = attempt[HasError]
        and Text.Contains(detail, "CSV record 3")
        and Text.Contains(detail, "17 fields"),
    Result = #table(
        type table [Check = text, Expected = text, Actual = text, Pass = logical],
        {{"Payday Super: first short record is identified and raises", "true", Text.From(actual), actual = true}}
    )
in
    Result
"@
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayNoContributionsCheck'
            ExpectedRows = 1
            Source = New-PaydayExpectedErrorCheckM `
                -Name 'Payday Super: header plus provenance but no contributions raises' `
                -MExpression "Table.RowCount(PaydaySuper_Report($mPaydayNoContributions))"
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayUnterminatedQuoteCheck'
            ExpectedRows = 1
            Source = New-PaydayMalformedRowErrorCheckM `
                -Name 'Payday Super: EOF inside a quoted contract field raises' `
                -MExpression "Table.RowCount(PaydaySuper_Report($mPaydayUnterminatedQuote))" `
                -DetailContains 'unterminated quoted field'
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayUnterminatedExtraQuoteCheck'
            ExpectedRows = 1
            Source = New-PaydayMalformedRowErrorCheckM `
                -Name 'Payday Super: EOF inside an ignored extra quoted field raises' `
                -MExpression "Table.RowCount(PaydaySuper_Report($mPaydayUnterminatedExtraQuote))" `
                -DetailContains 'unterminated quoted field'
        },
        [pscustomobject]@{
            Name = 'ZZ_PaydayQuotedMultilineCheck'
            ExpectedRows = 1
            Source = @"
let
    adapted = PaydaySuper_Report($mPaydayQuotedMultiline),
    first = Table.SelectRows(adapted, each [row] = "1"){0},
    actual = Table.RowCount(adapted) = 2
        and Text.Contains(first[notes], "Fabricated multiline")
        and Text.Contains(first[notes], Character.FromNumber(10)),
    Result = #table(
        type table [Check = text, Expected = text, Actual = text, Pass = logical],
        {{"Payday Super: quoted multiline text remains one record", "true", Text.From(actual), actual = true}}
    )
in
    Result
"@
        }
    )

    $paydayScaleSpecifications = @(
        [pscustomobject]@{ Name = 'ZZ_PaydayScale500Check'; Rows = 500; MPath = $mPaydayScale500 },
        [pscustomobject]@{ Name = 'ZZ_PaydayScale5000Check'; Rows = 5000; MPath = $mPaydayScale5000 },
        [pscustomobject]@{ Name = 'ZZ_PaydayScale10000Check'; Rows = 10000; MPath = $mPaydayScale10000 }
    )
    foreach ($scaleSpecification in $paydayScaleSpecifications) {
        $scaleRows = [int]$scaleSpecification.Rows
        $scaleMPath = [string]$scaleSpecification.MPath
        $paydayCheckQueries += [pscustomobject]@{
            Name = [string]$scaleSpecification.Name
            ExpectedRows = 1
            ScaleRows = $scaleRows
            Source = @"
let
    actual = Table.RowCount(PaydaySuper_Report($scaleMPath)),
    Result = #table(
        type table [Check = text, Expected = text, Actual = text, Pass = logical],
        {{"Payday Super: $scaleRows-row report materialises", "$scaleRows", Text.From(actual), actual = $scaleRows}}
    )
in
    Result
"@
        }
    }

    try {
        $excel = New-Object -ComObject Excel.Application
    }
    catch {
        throw (
            'Desktop Microsoft Excel could not be started through COM. ' +
            'Install desktop Excel on Windows before running this acceptance test. ' +
            $_.Exception.Message
        )
    }

    $excelVersion = [string]$excel.Version
    $excelBuild = [string]$excel.Build
    $previousDisplayAlerts = $excel.DisplayAlerts
    $previousAutomationSecurity = $excel.AutomationSecurity
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    # msoAutomationSecurityForceDisable: the runner never imports VBA and no
    # workbook opened by it may execute embedded macros.
    $excel.AutomationSecurity = 3

    $workbooks = $excel.Workbooks
    $workbook = $workbooks.Add()
    # The $Workbook$ provider is more stable when the workbook has a path.
    $workbook.SaveAs($temporaryWorkbook, 51)
    $worksheets = $workbook.Worksheets
    $queries = $workbook.Queries

    $selectedPowerQueryFiles = @(
        if ($CheckSet -eq 'Core') {
            $powerQueryFiles
        }
        else {
            $powerQueryFiles | Where-Object { $_.Name -eq 'PaydaySuper.Report.pq' }
        }
    )
    if ($selectedPowerQueryFiles.Count -eq 0) {
        throw "$CheckSet child selected no Power Query source files."
    }
    foreach ($file in $selectedPowerQueryFiles) {
        # Queries.Add rejects dots in names, so Fx.ABNIsValid becomes
        # Fx_ABNIsValid. Only the query name changes; the M source is unaltered.
        $queryName = [IO.Path]::GetFileNameWithoutExtension($file.Name) -replace '\.', '_'
        $querySource = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8
        $currentQuery = $queries.Add($queryName, $querySource)
        [void]$createdQueries.Add($currentQuery)
        $currentQuery = $null
    }

    $querySpecifications = @(
        if ($CheckSet -eq 'Core') {
            [pscustomobject]@{
                Name = 'ZZ_CoreChecks'
                ExpectedRows = 46
                Source = $coreChecksM
            }
        }
        else {
            $paydayCheckQueries
        }
    )
    $checkRows = @()
    $timings = @()
    for ($queryIndex = 0; $queryIndex -lt $querySpecifications.Count; $queryIndex++) {
        $querySpecification = $querySpecifications[$queryIndex]
        $selectedQueryName = [string]$querySpecification.Name
        $currentQuery = $queries.Add(
            $selectedQueryName,
            [string]$querySpecification.Source
        )
        [void]$createdQueries.Add($currentQuery)
        $currentQuery = $null

        $worksheet = if ($queryIndex -eq 0) {
            $worksheets.Item(1)
        }
        else {
            $worksheets.Add()
        }
        $connectionString = (
            'OLEDB;Provider=Microsoft.Mashup.OleDb.1;Data Source=$Workbook$;' +
            "Location=$selectedQueryName;Extended Properties=`"`""
        )
        $targetRange = $worksheet.Range('A1')
        $listObjects = $worksheet.ListObjects
        $listObject = $listObjects.Add(0, $connectionString, $null, 1, $targetRange)
        $queryTable = $listObject.QueryTable
        $queryTable.CommandType = 2
        $queryTable.CommandText = @("SELECT * FROM [$selectedQueryName]")
        $queryTable.BackgroundQuery = $false

        $refreshStopwatch = [Diagnostics.Stopwatch]::StartNew()
        try {
            [void]$queryTable.Refresh($false)
            $excel.CalculateUntilAsyncQueriesDone()
        }
        catch {
            throw (
                "$selectedQueryName Power Query refresh failed. Desktop Excel must provide " +
                'Microsoft.Mashup.OleDb.1. ' + $_.Exception.Message
            )
        }
        finally {
            $refreshStopwatch.Stop()
        }

        if ($null -ne $querySpecification.PSObject.Properties['ScaleRows']) {
            $timings += [pscustomobject]@{
                Name = $selectedQueryName
                ScaleRows = [int]$querySpecification.ScaleRows
                ElapsedMilliseconds = [long]$refreshStopwatch.ElapsedMilliseconds
            }
        }

        $dataBodyRange = $listObject.DataBodyRange
        if ($null -eq $dataBodyRange) {
            throw "$selectedQueryName returned no rows; the M did not evaluate."
        }
        $values = $dataBodyRange.Value2
        if ($values -isnot [array] -or $values.Rank -ne 2) {
            throw "$selectedQueryName returned an unexpected result shape."
        }
        $rowCount = $values.GetUpperBound(0) - $values.GetLowerBound(0) + 1
        $columnCount = $values.GetUpperBound(1) - $values.GetLowerBound(1) + 1
        if ($rowCount -ne [int]$querySpecification.ExpectedRows) {
            throw (
                "$selectedQueryName returned $rowCount rows; expected exactly " +
                "$($querySpecification.ExpectedRows)."
            )
        }
        if ($columnCount -ne 4) {
            throw "$selectedQueryName returned $columnCount columns; expected exactly 4."
        }
        $checkRows += @(ConvertFrom-NativeCheckValues -Values $values)

        # The workbook owns the materialised table. Release this iteration's
        # automation references before creating the next independent query.
        Release-ComReference $dataBodyRange 'DataBodyRange'
        $dataBodyRange = $null
        Release-ComReference $queryTable 'QueryTable'
        $queryTable = $null
        Release-ComReference $listObject 'ListObject'
        $listObject = $null
        Release-ComReference $listObjects 'ListObjects collection'
        $listObjects = $null
        Release-ComReference $targetRange 'target Range'
        $targetRange = $null
        Release-ComReference $worksheet 'Worksheet'
        $worksheet = $null
    }
    $expectedRowCount = if ($CheckSet -eq 'Core') { 46 } else { 26 }
    if ($checkRows.Count -ne $expectedRowCount) {
        throw (
            "$CheckSet child aggregated $($checkRows.Count) rows; " +
            "expected exactly $expectedRowCount."
        )
    }
    $payload = [ordered]@{
        SchemaVersion = 1
        CheckSet = $CheckSet
        ExcelVersion = $excelVersion
        ExcelBuild = $excelBuild
        Rows = $checkRows
        Timings = $timings
    }
    $json = $payload | ConvertTo-Json -Depth 5
    $utf8WithoutBom = New-Object System.Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($ResultPath, $json, $utf8WithoutBom)

    # A child succeeded when it produced a complete, validated result set.
    # Failed predicates remain structured rows for the parent to print and
    # decide; a non-zero child exit is reserved for a harness/cleanup failure.
    $exitCode = 0
}
catch {
    [Console]::Error.WriteLine('HARNESS ERROR: ' + $_.Exception.Message)
    if ($null -ne $_.Exception.InnerException) {
        [Console]::Error.WriteLine('  inner: ' + $_.Exception.InnerException.Message)
    }
}
finally {
    # Close the workbook even when query creation or refresh fails.
    if ($null -ne $workbook) {
        try {
            $workbook.Close($false)
        }
        catch {
            [Console]::Error.WriteLine(
                'CLEANUP ERROR: could not close the workbook. ' + $_.Exception.Message
            )
            $script:cleanupFailed = $true
        }
    }

    if ($null -ne $excel) {
        if ($null -ne $previousAutomationSecurity) {
            try {
                $excel.AutomationSecurity = $previousAutomationSecurity
            }
            catch {
                [Console]::Error.WriteLine(
                    'CLEANUP ERROR: could not restore AutomationSecurity. ' +
                    $_.Exception.Message
                )
                $script:cleanupFailed = $true
            }
        }
        if ($null -ne $previousDisplayAlerts) {
            try {
                $excel.DisplayAlerts = $previousDisplayAlerts
            }
            catch {
                [Console]::Error.WriteLine(
                    'CLEANUP ERROR: could not restore DisplayAlerts. ' +
                    $_.Exception.Message
                )
                $script:cleanupFailed = $true
            }
        }
    }

    # Release every created COM reference in reverse creation order.
    Release-ComReference $dataBodyRange 'DataBodyRange'
    $dataBodyRange = $null
    Release-ComReference $queryTable 'QueryTable'
    $queryTable = $null
    Release-ComReference $listObject 'ListObject'
    $listObject = $null
    Release-ComReference $listObjects 'ListObjects collection'
    $listObjects = $null
    Release-ComReference $targetRange 'target Range'
    $targetRange = $null
    Release-ComReference $currentQuery 'partially registered WorkbookQuery'
    $currentQuery = $null

    for ($index = $createdQueries.Count - 1; $index -ge 0; $index--) {
        Release-ComReference $createdQueries[$index] "WorkbookQuery[$index]"
        $createdQueries[$index] = $null
    }

    Release-ComReference $queries 'Queries collection'
    $queries = $null
    Release-ComReference $worksheet 'Worksheet'
    $worksheet = $null
    Release-ComReference $worksheets 'Worksheets collection'
    $worksheets = $null
    Release-ComReference $workbook 'Workbook'
    $workbook = $null
    Release-ComReference $workbooks 'Workbooks collection'
    $workbooks = $null

    if ($null -ne $excel) {
        try {
            $excel.Quit()
        }
        catch {
            [Console]::Error.WriteLine(
                'CLEANUP ERROR: could not quit Excel. ' + $_.Exception.Message
            )
            $script:cleanupFailed = $true
        }
    }
    Release-ComReference $excel 'Excel Application'
    $excel = $null

    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()

    if ($temporaryDirectoryCreated -and (Test-Path -LiteralPath $temporaryDirectory)) {
        if (Test-SafeTemporaryDirectory $temporaryDirectory $systemTemporaryRoot) {
            try {
                Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force
            }
            catch {
                [Console]::Error.WriteLine(
                    'CLEANUP ERROR: could not remove the temporary fixture directory. ' +
                    $_.Exception.Message
                )
                $script:cleanupFailed = $true
            }
        }
        else {
            [Console]::Error.WriteLine(
                "CLEANUP ERROR: refusing to remove unexpected path: $temporaryDirectory"
            )
            $script:cleanupFailed = $true
        }
    }
}

if ($script:cleanupFailed) {
    $exitCode = 1
}
exit $exitCode
