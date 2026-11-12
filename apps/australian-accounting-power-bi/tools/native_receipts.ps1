# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

# Read-only receipts for a recorded Power BI Desktop instance. Dot-source this file.
Add-Type -Namespace PowerBINative -Name Kernel32 -MemberDefinition @'
[DllImport("kernel32.dll", SetLastError = true)]
public static extern IntPtr OpenProcess(uint access, bool inheritHandle, int processId);
[DllImport("kernel32.dll")]
public static extern bool CloseHandle(IntPtr handle);
[DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
public static extern int GetPackageFamilyName(IntPtr process, ref uint length, System.Text.StringBuilder name);
'@

# The package identity Windows reports for a process, or 'none'.
function Get-PackageFamily([int]$ProcessId) {
    $handle = [PowerBINative.Kernel32]::OpenProcess(0x1000, $false, $ProcessId) # PROCESS_QUERY_LIMITED_INFORMATION
    if ($handle -eq [IntPtr]::Zero) { throw "Cannot query process $ProcessId." }
    try {
        $length = [uint32]65 # PACKAGE_FAMILY_NAME_MAX_LENGTH and its terminator
        $name = [Text.StringBuilder]::new(65)
        $status = [PowerBINative.Kernel32]::GetPackageFamilyName($handle, [ref]$length, $name)
        if ($status -eq 15700) { return 'none' } # APPMODEL_ERROR_NO_PACKAGE
        if ($status -ne 0) { throw "GetPackageFamilyName failed for process ${ProcessId}: $status." }
        $name.ToString()
    } finally { [void][PowerBINative.Kernel32]::CloseHandle($handle) }
}

# Desktop keeps its settings, including the privacy rules, in User.zip beside its Traces folder. A
# packaged app's descendants write that folder into the package's own copy, even when they report
# no package identity, so the copy holding this process's own trace is the one it uses. Package
# copies are read first: inside a package, the unpackaged path also shows the package's files.
function Find-SettingsCopy([int]$ProcessId, [datetime]$StartedAt, [string]$LocalAppData = [Environment]::GetFolderPath('LocalApplicationData')) {
    $packages = Join-Path $LocalAppData 'Packages'
    $packaged = @(if (Test-Path -LiteralPath $packages) {
        Get-ChildItem -LiteralPath $packages -Directory | ForEach-Object { Join-Path $_.FullName 'LocalCache\Local\Microsoft\Power BI Desktop' }
    })
    foreach ($roots in @($packaged, @(Join-Path $LocalAppData 'Microsoft\Power BI Desktop'))) {
        # A trace last written before this process started belongs to an earlier process with the same ID.
        $traces = @(foreach ($root in $roots) {
            $folder = Join-Path $root 'Traces\Performance'
            if (Test-Path -LiteralPath $folder) {
                Get-ChildItem -LiteralPath $folder -Filter "PBIDesktop.$ProcessId.*.log" -File |
                    Where-Object { $_.LastWriteTimeUtc -ge $StartedAt.ToUniversalTime() } |
                    ForEach-Object { [ordered]@{ settings_trace = $_.FullName; user_zip = Join-Path $root 'User.zip' } }
            }
        })
        $copies = @($traces | ForEach-Object { $_.user_zip } | Sort-Object -Unique)
        if ($copies.Count -eq 1) { return $traces[0] }
        if ($copies.Count -gt 1) { break }
    }
    [ordered]@{ settings_trace = $null; user_zip = $null }
}

# The executable behind a recorded process, after checking that the ID still names that process.
function Get-ExecutableReceipt([int]$ProcessId, [datetime]$StartedAt) {
    $process = Get-CimInstance Win32_Process -Filter "ProcessId = $ProcessId"
    if ($null -eq $process -or [Math]::Abs(($process.CreationDate.ToUniversalTime() - $StartedAt.ToUniversalTime()).TotalMilliseconds) -gt 1) {
        throw "Process $ProcessId is not the recorded instance."
    }
    if (-not $process.ExecutablePath) { throw "Cannot read the executable of process $ProcessId; run at the same elevation as Desktop." }
    $version = (Get-Item -LiteralPath $process.ExecutablePath).VersionInfo
    [ordered]@{
        pid = $ProcessId
        path = $process.ExecutablePath
        file_version = $version.FileVersion
        product_version = $version.ProductVersion
        sha256 = (Get-FileHash -LiteralPath $process.ExecutablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

# Receipts for an instance record's desktop_pid, started_at, engine_pid and engine_started_at.
function Get-InstanceReceipt($Instance) {
    $desktop = Get-ExecutableReceipt $Instance.desktop_pid $Instance.started_at
    $desktop.package_family = Get-PackageFamily $Instance.desktop_pid
    $copy = Find-SettingsCopy $Instance.desktop_pid $Instance.started_at
    $desktop.settings_trace = $copy.settings_trace
    $desktop.user_zip = $copy.user_zip
    [ordered]@{ desktop = $desktop; engine = Get-ExecutableReceipt $Instance.engine_pid $Instance.engine_started_at }
}
