param(
    [Parameter(Mandatory = $true)][string]$Destination,
    [string]$SourceRoot = '',
    [string]$Revision = 'working-copy',
    [string]$PowerBIBin = 'C:\Program Files\Microsoft Power BI Desktop\bin',
    [switch]$Launch
)

# Copy source only. Privacy classification and refresh are explicit Desktop actions.
$ErrorActionPreference = 'Stop'
if (-not $SourceRoot) { $SourceRoot = Join-Path $PSScriptRoot '..' }
if ($SourceRoot.StartsWith('\\') -or $SourceRoot.StartsWith('Z:', [StringComparison]::OrdinalIgnoreCase)) { throw 'Use a local source directory.' }
$SourceRoot = (Resolve-Path -LiteralPath $SourceRoot).Path
$Destination = [IO.Path]::GetFullPath($Destination)
if (Test-Path -LiteralPath $Destination) { throw 'Destination must be new.' }
if ($Destination.StartsWith($SourceRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or $Destination -eq $SourceRoot) { throw 'Destination must be outside the source project.' }
if ($Destination.StartsWith('\\') -or $Destination.StartsWith('Z:', [StringComparison]::OrdinalIgnoreCase)) { throw 'Use a local disposable directory.' }
$ancestor = [IO.DirectoryInfo]::new($Destination)
while ($null -ne $ancestor) {
    if (Test-Path -LiteralPath (Join-Path $ancestor.FullName '.git')) { throw 'Destination must be outside version control.' }
    if ($ancestor.Exists -and ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Linked destination paths are not admitted.' }
    $ancestor = $ancestor.Parent
}
$project = Join-Path $Destination 'project'
New-Item -ItemType Directory -Path $project | Out-Null
$files = @(
    Get-Item -LiteralPath (Join-Path $SourceRoot 'australian-accounting-power-bi.pbip')
    foreach ($folder in @('australian-accounting-power-bi.Report', 'australian-accounting-power-bi.SemanticModel')) {
        Get-ChildItem -LiteralPath (Join-Path $SourceRoot $folder) -Recurse -File | Where-Object {
            $_.FullName -notmatch '[\\/]\.pbi[\\/]' -and ($_.Extension -in @('.json', '.tmdl', '.pbir', '.pbism') -or $_.Name -eq '.platform')
        }
    }
    Get-ChildItem -LiteralPath (Join-Path $SourceRoot 'samples') -Filter 'sample-*.csv' -File
)
$sourceAncestor = [IO.DirectoryInfo]::new($SourceRoot)
while ($null -ne $sourceAncestor) {
    if ($sourceAncestor.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Linked source paths are not admitted.' }
    $sourceAncestor = $sourceAncestor.Parent
}
$inventory = @()
foreach ($file in $files | Sort-Object FullName) {
    if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Linked source files are not admitted.' }
    $fileAncestor = $file.Directory
    while ($fileAncestor -and $fileAncestor.FullName.Length -ge $SourceRoot.Length) {
        if ($fileAncestor.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Linked source paths are not admitted.' }
        $fileAncestor = $fileAncestor.Parent
    }
    $relative = $file.FullName.Substring($SourceRoot.Length + 1).Replace('\', '/')
    $target = Join-Path $project $relative
    New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
    $hash = (Get-FileHash -LiteralPath $file.FullName).Hash.ToLowerInvariant()
    Copy-Item -LiteralPath $file.FullName -Destination $target
    if ((Get-FileHash -LiteralPath $target).Hash.ToLowerInvariant() -ne $hash -or (Get-FileHash -LiteralPath $file.FullName).Hash.ToLowerInvariant() -ne $hash) { throw 'Source changed during preparation.' }
    $inventory += [ordered]@{ path = $relative; source_sha256 = $hash }
}
$manifest = Join-Path $Destination 'source-manifest.json'
[ordered]@{ revision = $Revision; source = @{ file_inventory = $inventory } } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $manifest -Encoding utf8
$expressions = Join-Path $project 'australian-accounting-power-bi.SemanticModel/definition/expressions.tmdl'
$text = [IO.File]::ReadAllText($expressions)
$declaration = 'expression SampleFolder = ""'
if ([regex]::Matches($text, [regex]::Escape($declaration)).Count -ne 1) { throw 'Expected one unset SampleFolder declaration.' }
$samplePath = (Join-Path $project 'samples').Replace('\', '/').Replace('"', '""')
[IO.File]::WriteAllText($expressions, $text.Replace($declaration, ('expression SampleFolder = "' + $samplePath + '"')), [Text.UTF8Encoding]::new($false))
Set-Content -LiteralPath (Join-Path $project 'samples/.native-test-copy') -Value 'Fabricated disposable samples only.' -Encoding utf8
if ($Launch) {
    $desktop = Start-Process -FilePath (Join-Path $PowerBIBin 'PBIDesktop.exe') -ArgumentList ('"' + (Join-Path $project 'australian-accounting-power-bi.pbip') + '"') -WindowStyle Hidden -PassThru
    $deadline = (Get-Date).AddMinutes(2)
    $server = $null
    do {
        Start-Sleep -Milliseconds 500
        $ownedEngine = @(Get-CimInstance Win32_Process -Filter "Name = 'msmdsrv.exe' AND ParentProcessId = $($desktop.Id)")
        if ($ownedEngine.Count -eq 1) {
            $ports = @(Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | Where-Object OwningProcess -eq $ownedEngine[0].ProcessId | Select-Object -ExpandProperty LocalPort -Unique)
            if ($ports.Count -eq 1) { $server = "localhost:$($ports[0])" }
        }
        if ($desktop.HasExited) { throw 'The launched Desktop exited before its engine was recorded.' }
    } while (-not $server -and (Get-Date) -lt $deadline)
    if (-not $server) { throw 'Owned engine did not become available; no other instance was adopted.' }
    $desktopRecord = Get-CimInstance Win32_Process -Filter "ProcessId = $($desktop.Id)"
    [ordered]@{
        project = $project; server = $server
        desktop_pid = $desktop.Id; started_at = $desktopRecord.CreationDate.ToString('o')
        engine_pid = $ownedEngine[0].ProcessId; engine_started_at = $ownedEngine[0].CreationDate.ToString('o')
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Destination 'instance.json') -Encoding utf8
}
Write-Output 'Native project prepared. Set the fabricated CSV privacy classifications and refresh in Desktop before running verification.'
