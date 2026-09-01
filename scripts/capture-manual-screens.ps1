#requires -Version 5.1

<#
.SYNOPSIS
Attaches to the one already-running bundled ORIGAMI3 app and captures the manual screen manifest.

.DESCRIPTION
This wrapper never builds, starts, stops, or restarts desktop.exe or a frontend server.
The coordinator must start exactly one bundled app with WebView2 CDP enabled beforehand.
#>

[CmdletBinding()]
param(
    [string]$Only,
    [string]$Resume,
    [string]$From,
    [string]$StagingRoot = "verification\manual-capture",
    [ValidateRange(1, 65535)]
    [int]$Port = 9222,
    [switch]$List
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$entry = Join-Path $PSScriptRoot "capture-manual-screens.mjs"
$manifest = Join-Path $PSScriptRoot "manual-screenshot-manifest.json"
$scenarioRegistry = Join-Path $PSScriptRoot "manual-capture\scenarios.mjs"
$captureOwnerLockPath = Join-Path $repositoryRoot "verification\manual-capture\.capture-owner-lock"

function Resolve-RepositoryPath {
    param([Parameter(Mandatory = $true)] [string]$Value)
    if ([System.IO.Path]::IsPathRooted($Value)) {
        return [System.IO.Path]::GetFullPath($Value)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot $Value))
}

function Test-ProcessDescendsFrom {
    param(
        [Parameter(Mandatory = $true)] [int]$CandidateProcessId,
        [Parameter(Mandatory = $true)] [int]$AncestorProcessId
    )
    $seen = New-Object 'System.Collections.Generic.HashSet[int]'
    $currentId = $CandidateProcessId
    while ($currentId -gt 0 -and $seen.Add($currentId)) {
        if ($currentId -eq $AncestorProcessId) { return $true }
        $process = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $currentId" -ErrorAction Stop
        if ($null -eq $process) { return $false }
        $currentId = [int]$process.ParentProcessId
    }
    return $false
}

function Read-CaptureOwnerReceipt {
    param([Parameter(Mandatory = $true)] [string]$LockPath)

    $expectedLockPath = [System.IO.Path]::GetFullPath($captureOwnerLockPath)
    $resolvedLockPath = [System.IO.Path]::GetFullPath($LockPath)
    if (-not [string]::Equals($resolvedLockPath, $expectedLockPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Manual capture owner recovery is restricted to the fixed path: $expectedLockPath"
    }
    $item = Get-Item -LiteralPath $resolvedLockPath -Force -ErrorAction Stop
    if (-not ($item -is [System.IO.FileInfo]) -or
        (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) -or
        -not [string]::Equals($item.FullName, $expectedLockPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "The fixed manual capture owner lock is not an ordinary file: $expectedLockPath"
    }
    $raw = Get-Content -LiteralPath $expectedLockPath -Raw -Encoding UTF8 -ErrorAction Stop
    try {
        $descriptor = $raw | ConvertFrom-Json
    }
    catch {
        throw "The fixed manual capture owner lock is not valid JSON: $expectedLockPath. $($_.Exception.Message)"
    }
    $descriptorKeys = @($descriptor.PSObject.Properties.Name | Sort-Object)
    $ownerPid = 0
    $schemaType = ""
    $pidType = ""
    if ($descriptorKeys -contains "schemaVersion" -and $null -ne $descriptor.schemaVersion) {
        $schemaType = $descriptor.schemaVersion.GetType().Name
    }
    if ($descriptorKeys -contains "pid" -and $null -ne $descriptor.pid) {
        $pidType = $descriptor.pid.GetType().Name
    }
    if (($descriptorKeys -join ",") -cne "acquiredAt,ownerToken,pid,schemaVersion" -or
        $schemaType -notin @("Int32", "Int64") -or
        [int64]$descriptor.schemaVersion -ne 1 -or
        -not ($descriptor.ownerToken -is [string]) -or
        ([string]$descriptor.ownerToken) -cnotmatch '^[a-f0-9]{32}$' -or
        $pidType -notin @("Int32", "Int64") -or
        -not [int]::TryParse([string]$descriptor.pid, [ref]$ownerPid) -or
        $ownerPid -le 0 -or
        -not ($descriptor.acquiredAt -is [string]) -or
        ([string]$descriptor.acquiredAt) -cnotmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$') {
        throw "The fixed manual capture owner lock descriptor is invalid: $expectedLockPath"
    }
    try {
        [void][DateTimeOffset]::ParseExact(
            [string]$descriptor.acquiredAt,
            "yyyy-MM-dd'T'HH:mm:ss.fff'Z'",
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::AssumeUniversal
        )
    }
    catch {
        throw "The fixed manual capture owner timestamp is invalid: $expectedLockPath"
    }
    return [pscustomobject]@{
        Path = $expectedLockPath
        Raw = $raw
        Length = [int64]$item.Length
        CreationTimeUtcTicks = [int64]$item.CreationTimeUtc.Ticks
        LastWriteTimeUtcTicks = [int64]$item.LastWriteTimeUtc.Ticks
        OwnerToken = [string]$descriptor.ownerToken
        OwnerPid = $ownerPid
    }
}

function Remove-StaleCaptureOwner {
    param(
        [Parameter(Mandatory = $true)] [string]$LockPath,
        [AllowNull()] [string]$ExpectedOwnerToken
    )

    $first = Read-CaptureOwnerReceipt -LockPath $LockPath
    if (-not [string]::IsNullOrEmpty($ExpectedOwnerToken) -and $first.OwnerToken -cne $ExpectedOwnerToken) {
        throw "The fixed owner token does not match the requested resume run."
    }
    if ($null -ne (Get-Process -Id $first.OwnerPid -ErrorAction SilentlyContinue)) {
        throw "The fixed manual capture owner PID $($first.OwnerPid) is still alive; refusing recovery."
    }

    $second = Read-CaptureOwnerReceipt -LockPath $LockPath
    if ($second.Raw -cne $first.Raw -or
        $second.Length -ne $first.Length -or
        $second.CreationTimeUtcTicks -ne $first.CreationTimeUtcTicks -or
        $second.LastWriteTimeUtcTicks -ne $first.LastWriteTimeUtcTicks) {
        throw "The fixed manual capture owner changed during recovery; leaving it untouched."
    }
    if ($null -ne (Get-Process -Id $second.OwnerPid -ErrorAction SilentlyContinue)) {
        throw "The fixed manual capture owner PID $($second.OwnerPid) became live during recovery."
    }

    Remove-Item -LiteralPath $captureOwnerLockPath -Force -ErrorAction Stop
    if (Test-Path -LiteralPath $captureOwnerLockPath) {
        throw "The fixed manual capture owner lock still exists after recovery."
    }
    return $first.OwnerToken
}

if (-not (Test-Path -LiteralPath $entry -PathType Leaf)) {
    throw "Manual capture entry script is missing: $entry"
}
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
    throw "Manual screenshot manifest is missing: $manifest"
}
if (-not (Test-Path -LiteralPath $scenarioRegistry -PathType Leaf)) {
    throw "Manual capture scenario registry is missing: $scenarioRegistry"
}
if (-not [string]::IsNullOrWhiteSpace($Only) -and
    (-not [string]::IsNullOrWhiteSpace($Resume) -or -not [string]::IsNullOrWhiteSpace($From))) {
    throw "-Only cannot be combined with -Resume or -From"
}
if (-not [string]::IsNullOrWhiteSpace($From) -and [string]::IsNullOrWhiteSpace($Resume)) {
    throw "-From is valid only together with -Resume"
}
if ($List -and
    (-not [string]::IsNullOrWhiteSpace($Only) -or
     -not [string]::IsNullOrWhiteSpace($Resume) -or
     -not [string]::IsNullOrWhiteSpace($From))) {
    throw "-List cannot be combined with capture or resume selection options"
}

$captureMutex = $null
$captureMutexHeld = $false
$nativeExitCode = 1
try {
    if (-not $List) {
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        try {
            $rootBytes = [System.Text.Encoding]::UTF8.GetBytes($repositoryRoot.ToLowerInvariant())
            $rootHash = [System.BitConverter]::ToString($sha256.ComputeHash($rootBytes)).Replace("-", "").ToLowerInvariant()
        }
        finally {
            $sha256.Dispose()
        }
        # Global scope serializes wrappers across Windows sessions.  The Node
        # runner additionally owns a repository-fixed filesystem lock for the
        # whole capture, so this mutex is not the sole exclusion boundary.
        $captureMutex = New-Object System.Threading.Mutex($false, "Global\ORIGAMI3-ManualCapture-$rootHash")
        try {
            $captureMutexHeld = $captureMutex.WaitOne(0)
        }
        catch [System.Threading.AbandonedMutexException] {
            # The previous wrapper died. Windows transferred ownership to this
            # process; the durable promotion journal decides how to recover.
            $captureMutexHeld = $true
        }
        if (-not $captureMutexHeld) {
            throw "Another manual screenshot capture or resume is already running for this repository."
        }
        $orphanedRunners = @(
            Get-CimInstance -ClassName Win32_Process -Filter "Name = 'node.exe'" -ErrorAction Stop |
                Where-Object {
                    -not [string]::IsNullOrWhiteSpace($_.CommandLine) -and
                    $_.CommandLine.IndexOf($entry, [System.StringComparison]::OrdinalIgnoreCase) -ge 0
                }
        )
        if ($orphanedRunners.Count -ne 0) {
            $orphanedIds = ($orphanedRunners | Select-Object -ExpandProperty ProcessId) -join ", "
            throw "A manual screenshot Node runner is already active or orphaned (PID: $orphanedIds). Stop or inspect it before retrying."
        }
    }

    $node = Get-Command node.exe -ErrorAction Stop
    $arguments = @(
        $entry,
        "--endpoint", "http://127.0.0.1:$Port",
        "--staging-root", (Resolve-RepositoryPath $StagingRoot)
    )
    if ($List) {
        $arguments += "--list"
    }
    else {
        $resolvedResume = $null
        $resumeRequested = -not [string]::IsNullOrWhiteSpace($Resume)
        if (-not $resumeRequested) {
            $ownerToken = [Guid]::NewGuid().ToString("N").ToLowerInvariant()
        }
        else {
            $resolvedResume = Resolve-RepositoryPath $Resume
            if (Test-Path -LiteralPath $resolvedResume -PathType Container) {
                $resumeStatePath = Join-Path $resolvedResume "run.json"
            }
            elseif ((Split-Path -Leaf $resolvedResume) -eq "run.json") {
                $resumeStatePath = $resolvedResume
            }
            else {
                throw "-Resume must name a run directory or its run.json: $resolvedResume"
            }
            if (-not (Test-Path -LiteralPath $resumeStatePath -PathType Leaf)) {
                $previousStatePath = "$resumeStatePath.previous"
                if (Test-Path -LiteralPath $previousStatePath -PathType Leaf) {
                    $resumeStatePath = $previousStatePath
                }
                else {
                    throw "Resume state is missing: $resumeStatePath"
                }
            }
            try {
                $resumeState = Get-Content -LiteralPath $resumeStatePath -Raw -Encoding UTF8 | ConvertFrom-Json
            }
            catch {
                throw "Resume state is not valid JSON: $resumeStatePath. $($_.Exception.Message)"
            }
            if (-not ($resumeState.PSObject.Properties.Name -contains "ownerToken")) {
                throw "Resume state has no wrapper owner token: $resumeStatePath"
            }
            if (-not ($resumeState.ownerToken -is [string])) {
                throw "Resume state wrapper owner token is not a string: $resumeStatePath"
            }
            $ownerToken = [string]$resumeState.ownerToken
            if ($ownerToken -cnotmatch '^[a-f0-9]{32}$') {
                throw "Resume state has an invalid wrapper owner token: $resumeStatePath"
            }
        }

        if (Test-Path -LiteralPath $captureOwnerLockPath) {
            $expectedOwnerToken = if ($resumeRequested) { $ownerToken } else { $null }
            $recoveredOwnerToken = Remove-StaleCaptureOwner `
                -LockPath $captureOwnerLockPath `
                -ExpectedOwnerToken $expectedOwnerToken
            if (-not $resumeRequested) {
                # A fresh invocation may be recovering a process that died
                # before run.json existed, so it adopts the validated token.
                $ownerToken = $recoveredOwnerToken
            }
        }

        # Resume is checked just as strictly as a fresh run. Even a
        # promotion-only resume must prove that the same bundled app is active.
        $desktopProcesses = @(Get-Process -Name "desktop" -ErrorAction SilentlyContinue)
        if ($desktopProcesses.Count -ne 1) {
            throw "Expected exactly one already-running desktop.exe; found $($desktopProcesses.Count). This script will not start or stop it."
        }
        $listeners = @(
            Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue |
                Where-Object { $_.OwningProcess -gt 0 }
        )
        $listenerOwners = @($listeners | Select-Object -ExpandProperty OwningProcess -Unique)
        if ($listenerOwners.Count -ne 1) {
            throw "Expected one owner for the CDP listener on port $Port; found $($listenerOwners.Count)."
        }
        if (-not (Test-ProcessDescendsFrom `
            -CandidateProcessId ([int]$listenerOwners[0]) `
            -AncestorProcessId ([int]$desktopProcesses[0].Id))) {
            throw "The CDP listener on port $Port is not owned by the one running desktop.exe process."
        }
        $appExecutable = $desktopProcesses[0].Path
        if ([string]::IsNullOrWhiteSpace($appExecutable) -or
            -not (Test-Path -LiteralPath $appExecutable -PathType Leaf)) {
            throw "Could not resolve the executable path of the one running desktop.exe process."
        }
        $appExecutable = [System.IO.Path]::GetFullPath($appExecutable)
        $appSha256 = (Get-FileHash -LiteralPath $appExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
        $arguments += @(
            "--app-exe", $appExecutable,
            "--app-sha256", $appSha256,
            "--owner-token", $ownerToken
        )

        if (-not [string]::IsNullOrWhiteSpace($Only)) {
            $arguments += @("--only", $Only)
        }
        if (-not [string]::IsNullOrWhiteSpace($Resume)) {
            $arguments += @("--resume", $resolvedResume)
        }
        if (-not [string]::IsNullOrWhiteSpace($From)) {
            $arguments += @("--from", $From)
        }
    }

    & $node.Source @arguments
    $nativeExitCode = $LASTEXITCODE
    if ($null -eq $nativeExitCode) { $nativeExitCode = 1 }
}
finally {
    if ($captureMutexHeld -and $null -ne $captureMutex) {
        $captureMutex.ReleaseMutex()
    }
    if ($null -ne $captureMutex) {
        $captureMutex.Dispose()
    }
}
exit ([int]$nativeExitCode)
