[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("RecordSuccess", "RecordFailure", "Check")]
    [string]$Action,

    [Parameter(Mandatory = $true)]
    [string]$RepositoryRoot,

    [string]$CheckId = "test-claim-scope-warning",

    [ValidateRange(1, 255)]
    [int]$FailureExitCode = 1,

    [string]$FailureKind = "scanner-exit",

    [ValidateRange(1, 99)]
    [int]$Threshold = 2
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

function Get-RepositoryKey {
    param([Parameter(Mandatory = $true)][string]$Root)

    $normalized = [IO.Path]::GetFullPath($Root).Replace("\", "/").TrimEnd("/").ToLowerInvariant()
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        return -join ($sha256.ComputeHash([Text.Encoding]::UTF8.GetBytes($normalized)) | ForEach-Object { $_.ToString("x2") })
    }
    finally {
        $sha256.Dispose()
    }
}

function Get-HealthStatePath {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Id
    )

    if ($Id -notmatch '^[a-z0-9][a-z0-9-]*$') {
        throw "CheckId must contain only lowercase letters, digits, and hyphens."
    }
    $healthRoot = Join-Path ([IO.Path]::GetTempPath()) "ori3-hook-health"
    $repositoryKey = Get-RepositoryKey $Root
    return [PSCustomObject]@{
        Directory = $healthRoot
        Path = Join-Path $healthRoot ("{0}-{1}.json" -f $repositoryKey, $Id)
        BlockPath = Join-Path $healthRoot ("{0}-{1}.json.block" -f $repositoryKey, $Id)
    }
}

function New-HealthState {
    param([Parameter(Mandatory = $true)][string]$Id)

    return [ordered]@{
        schemaVersion = 1
        checkId = $Id
        consecutiveFailures = 0
        lastAttemptAtUtc = $null
        lastSuccessAtUtc = $null
        lastFailureAtUtc = $null
        lastFailureExitCode = $null
        lastFailureKind = $null
        blockIssuedAtUtc = $null
        releaseAcknowledgedAtUtc = $null
        releaseFailureCount = $null
        releasePendingSuccess = $false
    }
}

function Read-HealthState {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $null
    }
    try {
        $state = [IO.File]::ReadAllText($Path, [Text.UTF8Encoding]::new($false)) | ConvertFrom-Json
        if ($null -eq $state -or
            [string]::IsNullOrWhiteSpace([string]$state.checkId) -or
            $null -eq $state.consecutiveFailures -or
            [int]$state.consecutiveFailures -lt 0) {
            throw "required health fields are missing or invalid"
        }
        return $state
    }
    catch {
        return [PSCustomObject]@{
            Invalid = $true
            Error = $_.Exception.Message
        }
    }
}

function Get-StateValue {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Name
    )

    $property = $State.PSObject.Properties[$Name]
    if ($null -eq $property -or $null -eq $property.Value) { return $null }
    return $property.Value
}

function Write-HealthStateAtomically {
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$State
    )

    [void][IO.Directory]::CreateDirectory($Directory)
    $temporaryPath = Join-Path $Directory (".{0}.{1}.{2}.tmp" -f [IO.Path]::GetFileName($Path), $PID, [Guid]::NewGuid().ToString("N"))
    $backupPath = Join-Path $Directory (".{0}.{1}.{2}.bak" -f [IO.Path]::GetFileName($Path), $PID, [Guid]::NewGuid().ToString("N"))
    try {
        [IO.File]::WriteAllText($temporaryPath, ($State | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            [IO.File]::Replace($temporaryPath, $Path, $backupPath, $true)
        }
        else {
            [IO.File]::Move($temporaryPath, $Path)
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporaryPath -PathType Leaf) {
            Remove-Item -LiteralPath $temporaryPath -Force
        }
        if (Test-Path -LiteralPath $backupPath -PathType Leaf) {
            Remove-Item -LiteralPath $backupPath -Force
        }
    }
}

function Write-HealthBlockAtomically {
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$State
    )

    [void][IO.Directory]::CreateDirectory($Directory)
    $temporaryPath = Join-Path $Directory (".{0}.{1}.{2}.tmp" -f [IO.Path]::GetFileName($Path), $PID, [Guid]::NewGuid().ToString("N"))
    $backupPath = Join-Path $Directory (".{0}.{1}.{2}.bak" -f [IO.Path]::GetFileName($Path), $PID, [Guid]::NewGuid().ToString("N"))
    $receipt = [ordered]@{
        schemaVersion = 1
        checkId = $State.checkId
        issuedAtUtc = $State.blockIssuedAtUtc
        consecutiveFailures = $State.consecutiveFailures
    }
    try {
        [IO.File]::WriteAllText($temporaryPath, ($receipt | ConvertTo-Json -Depth 3), [Text.UTF8Encoding]::new($false))
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            [IO.File]::Replace($temporaryPath, $Path, $backupPath, $true)
        }
        else {
            [IO.File]::Move($temporaryPath, $Path)
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporaryPath -PathType Leaf) {
            Remove-Item -LiteralPath $temporaryPath -Force
        }
        if (Test-Path -LiteralPath $backupPath -PathType Leaf) {
            Remove-Item -LiteralPath $backupPath -Force
        }
    }
}

function Format-OptionalTimestamp {
    param($Value)

    if ($null -eq $Value -or [string]::IsNullOrWhiteSpace([string]$Value)) { return "never" }
    return [string]$Value
}

$location = Get-HealthStatePath -Root $RepositoryRoot -Id $CheckId
$current = Read-HealthState -Path $location.Path

if ($Action -eq "Check") {
    if ($null -eq $current) {
        Write-Host "[OK] HOOK_HEALTH_OK check=$CheckId state=missing failures=0 threshold=$Threshold lastSuccess=never"
        exit 0
    }
    if ($current.PSObject.Properties["Invalid"] -and $current.Invalid) {
        Write-Host "[NG] HOOK_HEALTH_DEGRADED check=$CheckId failures=unknown threshold=$Threshold lastSuccess=unknown reason=state-unreadable"
        exit 2
    }
    $failures = [int](Get-StateValue $current "consecutiveFailures")
    $lastSuccess = Format-OptionalTimestamp (Get-StateValue $current "lastSuccessAtUtc")
    $lastFailure = Format-OptionalTimestamp (Get-StateValue $current "lastFailureAtUtc")
    $kind = [string](Get-StateValue $current "lastFailureKind")
    if ($failures -ge $Threshold) {
        $released = [bool](Get-StateValue $current "releasePendingSuccess")
        if ($released) {
            $releasedAt = Format-OptionalTimestamp (Get-StateValue $current "releaseAcknowledgedAtUtc")
            $releasedFailures = Get-StateValue $current "releaseFailureCount"
            Write-Host "[WARN] HOOK_HEALTH_RELEASED check=$CheckId releasedAt=$releasedAt releasedFailures=$releasedFailures lastSuccess=$lastSuccess status=awaiting-success"
            exit 0
        }

        $issuedAt = Get-StateValue $current "blockIssuedAtUtc"
        if ([string]::IsNullOrWhiteSpace([string]$issuedAt)) {
            Write-Host "[NG] HOOK_HEALTH_DEGRADED check=$CheckId failures=$failures threshold=$Threshold lastSuccess=$lastSuccess lastFailure=$lastFailure reason=$kind acknowledgement=not-issued"
            exit 2
        }
        if (Test-Path -LiteralPath $location.BlockPath -PathType Leaf) {
            Write-Host "[NG] HOOK_HEALTH_DEGRADED check=$CheckId failures=$failures threshold=$Threshold lastSuccess=$lastSuccess lastFailure=$lastFailure reason=$kind acknowledgement=$($location.BlockPath) release=delete-acknowledgement-file instruction=delete-file-at-$($location.BlockPath)"
            exit 2
        }

        $current.releaseAcknowledgedAtUtc = [DateTime]::UtcNow.ToString("o")
        $current.releaseFailureCount = $failures
        $current.releasePendingSuccess = $true
        Write-HealthStateAtomically -Directory $location.Directory -Path $location.Path -State $current
        $releasedAt = Format-OptionalTimestamp $current.releaseAcknowledgedAtUtc
        Write-Host "[WARN] HOOK_HEALTH_RELEASED check=$CheckId releasedAt=$releasedAt releasedFailures=$failures lastSuccess=$lastSuccess status=awaiting-success"
        exit 0
        exit 2
    }
    Write-Host "[OK] HOOK_HEALTH_OK check=$CheckId failures=$failures threshold=$Threshold lastSuccess=$lastSuccess lastFailure=$lastFailure"
    exit 0
}

$state = New-HealthState -Id $CheckId
if ($null -ne $current -and -not ($current.PSObject.Properties["Invalid"] -and $current.Invalid)) {
    foreach ($propertyName in @("lastSuccessAtUtc", "lastFailureAtUtc", "lastFailureExitCode", "lastFailureKind", "blockIssuedAtUtc", "releaseAcknowledgedAtUtc", "releaseFailureCount", "releasePendingSuccess")) {
        $value = Get-StateValue $current $propertyName
        if ($null -ne $value) { $state[$propertyName] = $value }
    }
    $state.consecutiveFailures = [int](Get-StateValue $current "consecutiveFailures")
}

$now = [DateTime]::UtcNow.ToString("o")
$state.lastAttemptAtUtc = $now
if ($Action -eq "RecordSuccess") {
    $state.consecutiveFailures = 0
    $state.lastSuccessAtUtc = $now
    $state.lastFailureAtUtc = $null
    $state.lastFailureExitCode = $null
    $state.lastFailureKind = $null
    $state.blockIssuedAtUtc = $null
    $state.releaseAcknowledgedAtUtc = $null
    $state.releaseFailureCount = $null
    $state.releasePendingSuccess = $false
    Write-HealthStateAtomically -Directory $location.Directory -Path $location.Path -State $state
    if (Test-Path -LiteralPath $location.BlockPath -PathType Leaf) {
        Remove-Item -LiteralPath $location.BlockPath -Force
    }
    Write-Host "[OK] HOOK_HEALTH_OK check=$CheckId failures=0 threshold=$Threshold lastSuccess=$now"
    exit 0
}

$state.consecutiveFailures = [int]$state.consecutiveFailures + 1
$state.lastFailureAtUtc = $now
$state.lastFailureExitCode = $FailureExitCode
$state.lastFailureKind = $FailureKind
$state.blockIssuedAtUtc = $null
$state.releaseAcknowledgedAtUtc = $null
$state.releaseFailureCount = $null
$state.releasePendingSuccess = $false
Write-HealthStateAtomically -Directory $location.Directory -Path $location.Path -State $state
$blockFailure = $state.consecutiveFailures -ge $Threshold
if ($blockFailure) {
    $state.blockIssuedAtUtc = $now
    try {
        Write-HealthBlockAtomically -Directory $location.Directory -Path $location.BlockPath -State $state
        Write-HealthStateAtomically -Directory $location.Directory -Path $location.Path -State $state
    }
    catch {
        $state.blockIssuedAtUtc = $null
        Write-HealthStateAtomically -Directory $location.Directory -Path $location.Path -State $state
        Write-Warning "HOOK_HEALTH_DEGRADED check=$CheckId failures=$($state.consecutiveFailures) threshold=$Threshold reason=acknowledgement-write-failed detail=$($_.Exception.Message)"
        exit 0
    }
}
$lastSuccess = Format-OptionalTimestamp $state.lastSuccessAtUtc
if ($blockFailure) {
    Write-Warning "HOOK_HEALTH_DEGRADED check=$CheckId failures=$($state.consecutiveFailures) threshold=$Threshold lastSuccess=$lastSuccess lastFailure=$now reason=$FailureKind acknowledgement=$($location.BlockPath)"
}
else {
    Write-Warning "HOOK_HEALTH_DEGRADED check=$CheckId failures=$($state.consecutiveFailures) threshold=$Threshold lastSuccess=$lastSuccess lastFailure=$now reason=$FailureKind"
}
exit 0
