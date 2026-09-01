[CmdletBinding(DefaultParameterSetName = "Hook")]
param(
    [Parameter(ParameterSetName = "Hook")]
    [string]$StateRoot = "",

    [Parameter(Mandatory = $true, ParameterSetName = "StopWatcher")]
    [switch]$StopRecordedWatcher,

    [Parameter(Mandatory = $true, ParameterSetName = "WaitReport")]
    [switch]$WaitForReportUpdate,

    [Parameter(Mandatory = $true, ParameterSetName = "WaitReport")]
    [ValidateNotNullOrEmpty()]
    [string]$DefinitionPath,

    [Parameter(Mandatory = $true, ParameterSetName = "WaitReport")]
    [ValidateNotNullOrEmpty()]
    [string]$ReportPath,

    [Parameter(Mandatory = $true, ParameterSetName = "WaitReport")]
    [ValidateRange(1, 3600)]
    [int]$TimeoutSeconds,

    [Parameter(Mandatory = $true, ParameterSetName = "StopWatcher")]
    [Parameter(Mandatory = $true, ParameterSetName = "WaitReport")]
    [ValidateNotNullOrEmpty()]
    [string]$RepositoryRoot
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)

$script:SchemaVersion = 1
$script:ForbiddenDocument = "docs/competitive-review-2026-08-20.md"
$script:Utf8NoBom = New-Object Text.UTF8Encoding($false)
$script:Mutex = $null
$script:MutexHeld = $false
$script:BoundaryScriptPath = [IO.Path]::GetFullPath([string]$MyInvocation.MyCommand.Path)

function Get-ObjectPropertyValue {
    param(
        [AllowNull()]$Object,
        [Parameter(Mandatory = $true)][string]$Name
    )

    if ($null -eq $Object) { return $null }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    return $property.Value
}

function Get-Sha256Hex {
    param([Parameter(Mandatory = $true)][string]$Text)

    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return -join ($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text)) | ForEach-Object { $_.ToString("x2") })
    }
    finally {
        $sha.Dispose()
    }
}

function Get-RepositoryKey {
    param([Parameter(Mandatory = $true)][string]$RepositoryRoot)

    $normalized = [IO.Path]::GetFullPath($RepositoryRoot).Replace("\", "/").TrimEnd("/").ToLowerInvariant()
    return Get-Sha256Hex $normalized
}

function Get-StatePaths {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$Root
    )

    $repositoryKey = Get-RepositoryKey $RepositoryRoot
    $directory = Join-Path ([IO.Path]::GetFullPath($Root)) "ori3-coordinator-boundary"
    $statePath = Join-Path $directory ("{0}.json" -f $repositoryKey)
    return [PSCustomObject]@{
        RepositoryKey = $repositoryKey
        Directory = $directory
        State = $statePath
        Acknowledgement = $statePath + ".block"
        Audit = Join-Path $directory ("{0}.audit.jsonl" -f $repositoryKey)
    }
}

function Enter-StateLock {
    param([Parameter(Mandatory = $true)][string]$RepositoryKey)

    $script:Mutex = New-Object Threading.Mutex($false, ("Local\ORI3CoordinatorBoundary-{0}" -f $RepositoryKey))
    try {
        $script:MutexHeld = $script:Mutex.WaitOne([TimeSpan]::FromSeconds(5))
    }
    catch [Threading.AbandonedMutexException] {
        $script:MutexHeld = $true
    }
    if (-not $script:MutexHeld) {
        throw "coordinator boundary state lock timed out"
    }
}

function Exit-StateLock {
    if ($script:MutexHeld -and $null -ne $script:Mutex) {
        $script:Mutex.ReleaseMutex()
    }
    $script:MutexHeld = $false
    if ($null -ne $script:Mutex) {
        $script:Mutex.Dispose()
        $script:Mutex = $null
    }
}

function Write-JsonAtomically {
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    [void][IO.Directory]::CreateDirectory($Directory)
    $temporary = Join-Path $Directory (".{0}.{1}.{2}.tmp" -f [IO.Path]::GetFileName($Path), $PID, [Guid]::NewGuid().ToString("N"))
    $backup = Join-Path $Directory (".{0}.{1}.{2}.bak" -f [IO.Path]::GetFileName($Path), $PID, [Guid]::NewGuid().ToString("N"))
    try {
        [IO.File]::WriteAllText($temporary, ($Value | ConvertTo-Json -Depth 8 -Compress), $script:Utf8NoBom)
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            [IO.File]::Replace($temporary, $Path, $backup, $true)
        }
        else {
            [IO.File]::Move($temporary, $Path)
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporary -PathType Leaf) {
            Remove-Item -LiteralPath $temporary -Force
        }
        if (Test-Path -LiteralPath $backup -PathType Leaf) {
            Remove-Item -LiteralPath $backup -Force
        }
    }
}

function Write-AuditEvent {
    param(
        [Parameter(Mandatory = $true)]$Paths,
        [Parameter(Mandatory = $true)][string]$Event,
        [string]$ToolName = "",
        [string]$CommandHash = "",
        [string]$ToolUseId = "",
        [string]$Detail = ""
    )

    [void][IO.Directory]::CreateDirectory($Paths.Directory)
    $entry = [ordered]@{
        schemaVersion = $script:SchemaVersion
        atUtc = [DateTime]::UtcNow.ToString("o")
        event = $Event
        repositoryKey = $Paths.RepositoryKey
        toolName = $ToolName
        commandHash = $CommandHash
        toolUseId = $ToolUseId
        detail = $Detail
    }
    $line = ($entry | ConvertTo-Json -Depth 5 -Compress) + [Environment]::NewLine
    $stream = New-Object IO.FileStream($Paths.Audit, [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try {
        $bytes = $script:Utf8NoBom.GetBytes($line)
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    }
    finally {
        $stream.Dispose()
    }
}

function Read-BoundaryState {
    param(
        [Parameter(Mandatory = $true)]$Paths
    )

    if (-not (Test-Path -LiteralPath $Paths.State -PathType Leaf)) {
        return [PSCustomObject]@{ Exists = $false; Valid = $true; Value = $null; Error = "" }
    }
    try {
        $state = [IO.File]::ReadAllText($Paths.State, $script:Utf8NoBom) | ConvertFrom-Json
        $schema = Get-ObjectPropertyValue $state "schemaVersion"
        $repositoryKey = [string](Get-ObjectPropertyValue $state "repositoryKey")
        $status = [string](Get-ObjectPropertyValue $state "status")
        $toolName = [string](Get-ObjectPropertyValue $state "toolName")
        $commandHash = [string](Get-ObjectPropertyValue $state "commandHash")
        if ([int]$schema -ne $script:SchemaVersion -or
            $repositoryKey -ne $Paths.RepositoryKey -or
            @("blocked", "released", "in-flight") -notcontains $status -or
            [string]::IsNullOrWhiteSpace($toolName) -or
            $commandHash -notmatch "^[0-9a-f]{64}$") {
            throw "required state fields are missing or invalid"
        }
        return [PSCustomObject]@{ Exists = $true; Valid = $true; Value = $state; Error = "" }
    }
    catch {
        return [PSCustomObject]@{ Exists = $true; Valid = $false; Value = $null; Error = $_.Exception.Message }
    }
}

function Write-AcknowledgementReceipt {
    param(
        [Parameter(Mandatory = $true)]$Paths,
        [Parameter(Mandatory = $true)]$State
    )

    $receipt = [ordered]@{
        schemaVersion = $script:SchemaVersion
        repositoryKey = $Paths.RepositoryKey
        issuedAtUtc = $State.issuedAtUtc
        toolName = $State.toolName
        commandHash = $State.commandHash
    }
    Write-JsonAtomically -Directory $Paths.Directory -Path $Paths.Acknowledgement -Value $receipt
}

function New-BlockedState {
    param(
        [Parameter(Mandatory = $true)]$Paths,
        [Parameter(Mandatory = $true)][string]$ToolName,
        [Parameter(Mandatory = $true)][string]$CommandHash,
        [Parameter(Mandatory = $true)][string]$Reason,
        [string]$ToolUseId = ""
    )

    $now = [DateTime]::UtcNow.ToString("o")
    $state = [ordered]@{
        schemaVersion = $script:SchemaVersion
        repositoryKey = $Paths.RepositoryKey
        status = "blocked"
        toolName = $ToolName
        commandHash = $CommandHash
        issuedAtUtc = $now
        deniedToolUseId = $ToolUseId
        denialReason = $Reason
        acknowledgedAtUtc = $null
        releasedAtUtc = $null
        releaseToolUseId = $null
        lastFailureAtUtc = $null
        lastFailure = $null
    }
    Write-JsonAtomically -Directory $Paths.Directory -Path $Paths.State -Value $state
    Write-AcknowledgementReceipt -Paths $Paths -State $state
    Write-AuditEvent -Paths $Paths -Event "deny" -ToolName $ToolName -CommandHash $CommandHash -ToolUseId $ToolUseId -Detail $Reason
    return $state
}

function Remove-ActiveState {
    param([Parameter(Mandatory = $true)]$Paths)

    if (Test-Path -LiteralPath $Paths.State -PathType Leaf) {
        Remove-Item -LiteralPath $Paths.State -Force
    }
    if (Test-Path -LiteralPath $Paths.Acknowledgement -PathType Leaf) {
        Remove-Item -LiteralPath $Paths.Acknowledgement -Force
    }
}

function Write-PreToolDeny {
    param([Parameter(Mandatory = $true)][string]$Reason)

    $response = [ordered]@{
        hookSpecificOutput = [ordered]@{
            hookEventName = "PreToolUse"
            permissionDecision = "deny"
            permissionDecisionReason = $Reason
        }
    }
    [Console]::Out.WriteLine(($response | ConvertTo-Json -Depth 5 -Compress))
    exit 0
}

function Get-DenialReason {
    param(
        [Parameter(Mandatory = $true)][string]$Detail,
        [string]$CommandHash = "unknown",
        [string]$AcknowledgementPath = ""
    )

    $allowed = @(
        "ALLOW-1: literal-path git add, commit/push/tag, origin fetch, read-only state/diff, and the exact snapshot-worktrees.ps1 normal/-Check refs/wip snapshot plumbing",
        "ALLOW-2: exact scripts/check.ps1, check-ci.ps1, and check-release-ready.ps1 quality gates; plus scripts/check-receipt.ps1 -RepairSigningKey -RepoRoot <this repository> because the coordinator identity must create its own Windows DPAPI signing key",
        "ALLOW-3: literal file reads; rg requires the prohibited-document exclusion glob",
        "ALLOW-4: read-only process and free-capacity inspection; exact local report-time read by Get-Date -Format 'yyyy-MM-dd HH:mm'",
        "ALLOW-5: literal desktop.exe Start-Process and desktop CloseMainWindow()",
        "ALLOW-6: exact hidden detached Start-Process launcher for continuous 10-minute scripts/watch-agents.ps1 monitoring, plus the exact boundary-script StopRecordedWatcher mode that can stop only the process identified by the fixed runtime state; -Once and direct Stop-Process are denied"
    ) -join "; "
    $escape = ""
    if (-not [string]::IsNullOrWhiteSpace($AcknowledgementPath)) {
        $escaped = $AcknowledgementPath.Replace("'", "''")
        $escape = " Escape receipt: acknowledgement=$AcknowledgementPath. Only Remove-Item -LiteralPath '$escaped' acknowledges it, and only the same tool/command hash is released once."
    }
    return (
        "ORIGAMI3_COORDINATOR_BOUNDARY_DENY: $Detail commandHash=$CommandHash. " +
        "$allowed. Delegate implementation, builds, individual checks, diagnosis, and inventory work to a worker agent.$escape"
    )
}

function New-PolicyDecision {
    param(
        [Parameter(Mandatory = $true)][bool]$Allowed,
        [Parameter(Mandatory = $true)][string]$Category,
        [Parameter(Mandatory = $true)][string]$Reason,
        [bool]$PipelineSource = $false,
        [bool]$PipelineTransform = $false
    )

    return [PSCustomObject]@{
        Allowed = $Allowed
        Category = $Category
        Reason = $Reason
        PipelineSource = $PipelineSource
        PipelineTransform = $PipelineTransform
    }
}

function Resolve-PolicyPath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    if ([IO.Path]::IsPathRooted($Path)) {
        return [IO.Path]::GetFullPath($Path).TrimEnd([char[]]"\/")
    }
    return [IO.Path]::GetFullPath((Join-Path $RepositoryRoot $Path)).TrimEnd([char[]]"\/")
}

function Get-RequiredWatcherStateValue {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Name
    )

    if (@($State.PSObject.Properties.Name) -notcontains $Name) {
        throw "watcher runtime state is missing required field '$Name'"
    }
    return $State.$Name
}

function Test-SameWatcherPath {
    param(
        [Parameter(Mandatory = $true)][string]$Actual,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    try {
        $actualFull = [IO.Path]::GetFullPath($Actual).TrimEnd([char[]]"\/")
        $expectedFull = [IO.Path]::GetFullPath($Expected).TrimEnd([char[]]"\/")
    }
    catch {
        return $false
    }
    return [string]::Equals($actualFull, $expectedFull, [StringComparison]::OrdinalIgnoreCase)
}

function Resolve-RegisteredWatchReportPath {
    param(
        [Parameter(Mandatory = $true)][string]$Definition,
        [Parameter(Mandatory = $true)][string]$Report,
        [Parameter(Mandatory = $true)][string]$Root
    )

    if (-not [IO.Path]::IsPathRooted($Definition) -or
        -not [IO.Path]::IsPathRooted($Report) -or
        -not [IO.Path]::IsPathRooted($Root)) {
        throw "DefinitionPath, ReportPath, and RepositoryRoot must be absolute literal paths"
    }
    $resolvedRoot = [IO.Path]::GetFullPath($Root).TrimEnd([char[]]"\/")
    if (-not (Test-Path -LiteralPath $resolvedRoot -PathType Container)) {
        throw "RepositoryRoot does not exist: $resolvedRoot"
    }
    $resolvedDefinition = Resolve-PolicyPath $Definition $resolvedRoot
    $definitionPrefix = $resolvedRoot + [IO.Path]::DirectorySeparatorChar
    if (-not $resolvedDefinition.StartsWith($definitionPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($resolvedDefinition) -notlike "watch-agents-*.json" -or
        -not (Test-Path -LiteralPath $resolvedDefinition -PathType Leaf)) {
        throw "DefinitionPath must be an existing watch-agents-*.json file below this repository"
    }
    $definitionItem = Get-Item -LiteralPath $resolvedDefinition -Force -ErrorAction Stop
    if (($definitionItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "watch definition reparse points are not allowed"
    }

    try {
        $strictUtf8 = New-Object Text.UTF8Encoding($false, $true)
        $definitionValue = [IO.File]::ReadAllText($resolvedDefinition, $strictUtf8) | ConvertFrom-Json
    }
    catch {
        throw "watch definition could not be read: $($_.Exception.Message)"
    }
    if ($null -eq $definitionValue -or @($definitionValue.PSObject.Properties.Name) -notcontains "agents") {
        throw "watch definition is missing agents"
    }
    $agents = @($definitionValue.agents)
    if ($agents.Count -eq 0) {
        throw "watch definition agents must not be empty"
    }

    $resolvedRequestedReport = Resolve-PolicyPath $Report $resolvedRoot
    $matchedReport = $null
    $knownNames = @{}
    foreach ($agent in $agents) {
        if ($null -eq $agent) {
            throw "watch definition contains a null agent"
        }
        $propertyNames = @($agent.PSObject.Properties.Name)
        if ($propertyNames -notcontains "name" -or
            [string]::IsNullOrWhiteSpace([string]$agent.name) -or
            $propertyNames -notcontains "reportPath" -or
            [string]::IsNullOrWhiteSpace([string]$agent.reportPath) -or
            $propertyNames -notcontains "sourcePaths" -or
            @($agent.sourcePaths).Count -eq 0) {
            throw "watch definition contains an invalid name/reportPath/sourcePaths entry"
        }
        $agentName = [string]$agent.name
        if ($knownNames.ContainsKey($agentName)) {
            throw "watch definition contains a duplicate agent name: $agentName"
        }
        $knownNames[$agentName] = $true
        foreach ($sourcePath in @($agent.sourcePaths)) {
            if ([string]::IsNullOrWhiteSpace([string]$sourcePath)) {
                throw "watch definition contains an empty sourcePath"
            }
        }

        $resolvedConfiguredReport = Resolve-PolicyPath ([string]$agent.reportPath) $resolvedRoot
        if (Test-SameWatcherPath -Actual $resolvedConfiguredReport -Expected $resolvedRequestedReport) {
            $matchedReport = $resolvedConfiguredReport
        }
    }
    if ([string]::IsNullOrWhiteSpace([string]$matchedReport)) {
        throw "ReportPath is not registered as a reportPath in the watch definition"
    }
    if (-not (Test-Path -LiteralPath $matchedReport -PathType Leaf)) {
        throw "registered ReportPath does not identify an existing file"
    }
    $reportItem = Get-Item -LiteralPath $matchedReport -Force -ErrorAction Stop
    if (($reportItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "registered ReportPath reparse points are not allowed"
    }
    return $matchedReport
}

function Invoke-WaitForReportUpdate {
    param(
        [Parameter(Mandatory = $true)][string]$Definition,
        [Parameter(Mandatory = $true)][string]$Report,
        [Parameter(Mandatory = $true)][int]$Timeout,
        [Parameter(Mandatory = $true)][string]$Root
    )

    if ($Timeout -lt 1 -or $Timeout -gt 3600) {
        throw "TimeoutSeconds must be between 1 and 3600"
    }
    $resolvedReport = Resolve-RegisteredWatchReportPath -Definition $Definition -Report $Report -Root $Root
    $initialItem = Get-Item -LiteralPath $resolvedReport -Force -ErrorAction Stop
    $initialTicks = $initialItem.LastWriteTimeUtc.Ticks
    $timer = [Diagnostics.Stopwatch]::StartNew()
    try {
        while ($timer.Elapsed.TotalSeconds -lt $Timeout) {
            [Threading.Thread]::Sleep(100)
            $currentItem = Get-Item -LiteralPath $resolvedReport -Force -ErrorAction Stop
            if ($currentItem.LastWriteTimeUtc.Ticks -ne $initialTicks) {
                Write-Host ("REPORT_UPDATE_DETECTED path={0} initialTicks={1} currentTicks={2} elapsedMilliseconds={3}" -f
                    $resolvedReport, $initialTicks, $currentItem.LastWriteTimeUtc.Ticks, $timer.ElapsedMilliseconds)
                return
            }
        }
        $finalItem = Get-Item -LiteralPath $resolvedReport -Force -ErrorAction Stop
        if ($finalItem.LastWriteTimeUtc.Ticks -ne $initialTicks) {
            Write-Host ("REPORT_UPDATE_DETECTED path={0} initialTicks={1} currentTicks={2} elapsedMilliseconds={3}" -f
                $resolvedReport, $initialTicks, $finalItem.LastWriteTimeUtc.Ticks, $timer.ElapsedMilliseconds)
            return
        }
        Write-Host ("REPORT_UPDATE_TIMEOUT path={0} ticks={1} timeoutSeconds={2} elapsedMilliseconds={3}" -f
            $resolvedReport, $initialTicks, $Timeout, $timer.ElapsedMilliseconds)
    }
    finally {
        $timer.Stop()
    }
}

function Assert-WatcherPathIsNotReparsePoint {
    param([Parameter(Mandatory = $true)][string]$Path)

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "watcher stop path may not be a reparse point: $Path"
    }
}

function Assert-WatcherLockHeld {
    param([Parameter(Mandatory = $true)][string]$LockPath)

    $probe = $null
    try {
        $probe = New-Object IO.FileStream(
            $LockPath,
            [IO.FileMode]::Open,
            [IO.FileAccess]::ReadWrite,
            [IO.FileShare]::None
        )
    }
    catch [IO.IOException] {
        $nativeCode = ($_.Exception.GetBaseException().HResult -band 0xFFFF)
        if ($nativeCode -eq 32 -or $nativeCode -eq 33) {
            return
        }
        throw "watcher singleton lock could not be verified (Win32=$nativeCode): $LockPath"
    }
    finally {
        if ($null -ne $probe) {
            $probe.Dispose()
        }
    }
    throw "watcher singleton lock is not held: $LockPath"
}

function Wait-WatcherLockReleased {
    param([Parameter(Mandatory = $true)][string]$LockPath)

    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        $probe = $null
        try {
            $probe = New-Object IO.FileStream(
                $LockPath,
                [IO.FileMode]::Open,
                [IO.FileAccess]::ReadWrite,
                [IO.FileShare]::None
            )
            return
        }
        catch [IO.IOException] {
            $nativeCode = ($_.Exception.GetBaseException().HResult -band 0xFFFF)
            if ($nativeCode -ne 32 -and $nativeCode -ne 33) {
                throw "watcher singleton lock release could not be verified (Win32=$nativeCode): $LockPath"
            }
        }
        finally {
            if ($null -ne $probe) {
                $probe.Dispose()
            }
        }
        [Threading.Thread]::Sleep(100)
    }
    throw "watcher singleton lock remained held after the recorded process exited: $LockPath"
}

function Invoke-StopRecordedWatcher {
    param([Parameter(Mandatory = $true)][string]$Root)

    if (-not [IO.Path]::IsPathRooted($Root)) {
        throw "RepositoryRoot must be an absolute path"
    }
    $resolvedRoot = [IO.Path]::GetFullPath($Root).TrimEnd([char[]]"\/")
    if (-not (Test-Path -LiteralPath $resolvedRoot -PathType Container)) {
        throw "RepositoryRoot does not exist: $resolvedRoot"
    }

    $expectedBoundaryPath = [IO.Path]::GetFullPath((Join-Path $resolvedRoot "scripts\hooks\enforce-coordinator-boundary.ps1"))
    if ([string]::IsNullOrWhiteSpace($script:BoundaryScriptPath) -or
        -not (Test-SameWatcherPath -Actual $script:BoundaryScriptPath -Expected $expectedBoundaryPath)) {
        throw "StopRecordedWatcher must run from this repository's boundary script"
    }

    $watcherPath = [IO.Path]::GetFullPath((Join-Path $resolvedRoot "scripts\watch-agents.ps1"))
    $runtimePath = [IO.Path]::GetFullPath((Join-Path $resolvedRoot "scratchpad\watch-agents.runtime.json"))
    $outputPath = [IO.Path]::GetFullPath((Join-Path $resolvedRoot "scratchpad\watch-agents.latest.log"))
    $lockPath = [IO.Path]::GetFullPath((Join-Path $resolvedRoot "scratchpad\watch-agents.lock"))
    foreach ($requiredDirectory in @(
        $resolvedRoot,
        (Join-Path $resolvedRoot "scripts"),
        (Join-Path $resolvedRoot "scripts\hooks"),
        (Join-Path $resolvedRoot "scratchpad")
    )) {
        if (-not (Test-Path -LiteralPath $requiredDirectory -PathType Container)) {
            throw "recorded watcher stop requires existing fixed directory: $requiredDirectory"
        }
        Assert-WatcherPathIsNotReparsePoint -Path $requiredDirectory
    }
    foreach ($requiredPath in @($expectedBoundaryPath, $watcherPath, $runtimePath, $lockPath)) {
        if (-not (Test-Path -LiteralPath $requiredPath -PathType Leaf)) {
            throw "recorded watcher stop requires existing fixed path: $requiredPath"
        }
        Assert-WatcherPathIsNotReparsePoint -Path $requiredPath
    }

    $strictUtf8 = New-Object Text.UTF8Encoding($false, $true)
    try {
        $stateText = $strictUtf8.GetString([IO.File]::ReadAllBytes($runtimePath))
        $state = $stateText | ConvertFrom-Json
    }
    catch {
        throw "watcher runtime state could not be parsed as strict UTF-8 JSON: $($_.Exception.Message)"
    }

    $schemaVersion = [int](Get-RequiredWatcherStateValue -State $state -Name "schemaVersion")
    $instanceId = [string](Get-RequiredWatcherStateValue -State $state -Name "instanceId")
    $watchPid = [int](Get-RequiredWatcherStateValue -State $state -Name "pid")
    $processStartText = [string](Get-RequiredWatcherStateValue -State $state -Name "processStartUtc")
    $processExecutablePath = [string](Get-RequiredWatcherStateValue -State $state -Name "processExecutablePath")
    $stateScriptPath = [string](Get-RequiredWatcherStateValue -State $state -Name "scriptPath")
    $stateScriptSha256 = [string](Get-RequiredWatcherStateValue -State $state -Name "scriptSha256")
    $stateRepositoryRoot = [string](Get-RequiredWatcherStateValue -State $state -Name "repositoryRoot")
    $stateRuntimePath = [string](Get-RequiredWatcherStateValue -State $state -Name "runtimePath")
    $stateOutputPath = [string](Get-RequiredWatcherStateValue -State $state -Name "outputPath")
    $stateLockPath = [string](Get-RequiredWatcherStateValue -State $state -Name "lockPath")
    $mode = [string](Get-RequiredWatcherStateValue -State $state -Name "mode")

    if ($schemaVersion -ne 2 -or $watchPid -le 0 -or $watchPid -eq $PID -or $mode -cne "continuous") {
        throw "watcher runtime identity fields are invalid"
    }
    $parsedInstanceId = [Guid]::Empty
    if (-not [Guid]::TryParse($instanceId, [ref]$parsedInstanceId) -or $parsedInstanceId -eq [Guid]::Empty) {
        throw "watcher runtime instanceId is invalid"
    }
    if ($stateScriptSha256 -notmatch '^[0-9a-f]{64}$') {
        throw "watcher runtime scriptSha256 is invalid"
    }
    try {
        $processStartUtc = [DateTime]::ParseExact(
            $processStartText,
            "o",
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::RoundtripKind
        ).ToUniversalTime()
    }
    catch {
        throw "watcher runtime processStartUtc is invalid: $processStartText"
    }

    $pathChecks = @(
        @("repositoryRoot", $stateRepositoryRoot, $resolvedRoot),
        @("scriptPath", $stateScriptPath, $watcherPath),
        @("runtimePath", $stateRuntimePath, $runtimePath),
        @("outputPath", $stateOutputPath, $outputPath),
        @("lockPath", $stateLockPath, $lockPath)
    )
    foreach ($pathCheck in $pathChecks) {
        if (-not (Test-SameWatcherPath -Actual ([string]$pathCheck[1]) -Expected ([string]$pathCheck[2]))) {
            throw "watcher runtime $($pathCheck[0]) does not match its fixed path"
        }
    }
    if (-not [IO.Path]::IsPathRooted($processExecutablePath) -or
        [IO.Path]::GetFileName($processExecutablePath).ToLowerInvariant() -notin @("powershell.exe", "pwsh.exe")) {
        throw "watcher runtime processExecutablePath is not an absolute PowerShell host path"
    }

    Assert-WatcherLockHeld -LockPath $lockPath
    $process = $null
    try {
        $process = Get-Process -Id $watchPid -ErrorAction Stop
        # Windows PowerShell can expose StartTime/Path for an existing process while
        # leaving Process.SafeHandle null. PID、開始時刻、実行ファイル、singleton lockを
        # 既に照合しているので、handle表示の有無で正当な記録済みwatcherを拒否しない。
        # Kill()はその時点で終了権限を取得できなければ失敗し、成功後もlock解放を確認する。
        $actualStartUtc = $null
        $actualProcessPath = ""
        try {
            $actualStartUtc = $process.StartTime.ToUniversalTime()
            $actualProcessPath = [IO.Path]::GetFullPath([string]$process.Path)
        }
        catch {
            throw "recorded watcher process identity could not be read: PID=$watchPid ($($_.Exception.Message))"
        }
        if ($null -eq $actualStartUtc -or [string]::IsNullOrWhiteSpace($actualProcessPath)) {
            throw "recorded watcher process identity is unavailable: PID=$watchPid"
        }
        if ($actualStartUtc.Ticks -ne $processStartUtc.Ticks) {
            throw "recorded watcher PID start time does not match runtime state: PID=$watchPid"
        }
        if (-not (Test-SameWatcherPath -Actual $actualProcessPath -Expected $processExecutablePath)) {
            throw "recorded watcher executable does not match runtime state: PID=$watchPid"
        }

        if (-not $process.HasExited) {
            $process.Kill()
        }
        if (-not $process.WaitForExit(10000)) {
            throw "recorded watcher did not exit within 10 seconds: PID=$watchPid"
        }
        Wait-WatcherLockReleased -LockPath $lockPath
        Write-Host "RECORDED_WATCHER_STOPPED pid=$watchPid startUtc=$($processStartUtc.ToString('o'))"
    }
    finally {
        if ($null -ne $process) {
            $process.Dispose()
        }
    }
}

function Test-ForbiddenDocumentArgument {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)

    foreach ($argument in $Arguments) {
        $normalized = ([string]$argument).Replace("\", "/").ToLowerInvariant()
        if ($normalized.Contains($script:ForbiddenDocument) -and -not $normalized.Contains("!" + $script:ForbiddenDocument)) {
            return $true
        }
    }
    return $false
}

function Get-StaticCommandParts {
    param([Parameter(Mandatory = $true)][Management.Automation.Language.CommandAst]$CommandAst)

    if ($CommandAst.InvocationOperator -ne [Management.Automation.Language.TokenKind]::Unknown -or
        @($CommandAst.Redirections).Count -ne 0) {
        return [PSCustomObject]@{ Valid = $false; Parts = @(); Error = "dynamic invocation or redirection" }
    }

    $parts = New-Object "Collections.Generic.List[string]"
    foreach ($element in @($CommandAst.CommandElements)) {
        if ($element -is [Management.Automation.Language.StringConstantExpressionAst]) {
            $parts.Add([string]$element.Value)
            continue
        }
        if ($element -is [Management.Automation.Language.ExpandableStringExpressionAst]) {
            if (@($element.NestedExpressions).Count -ne 0 -or $element.Extent.Text.Contains('$') -or $element.Extent.Text.Contains('`')) {
                return [PSCustomObject]@{ Valid = $false; Parts = @(); Error = "dynamic expandable string" }
            }
            $parts.Add([string]$element.Value)
            continue
        }
        if ($element -is [Management.Automation.Language.ConstantExpressionAst]) {
            $parts.Add([string]$element.Value)
            continue
        }
        if ($element -is [Management.Automation.Language.CommandParameterAst]) {
            if ($null -ne $element.Argument) {
                return [PSCustomObject]@{ Valid = $false; Parts = @(); Error = "inline dynamic parameter argument" }
            }
            $parts.Add("-" + [string]$element.ParameterName)
            continue
        }
        return [PSCustomObject]@{ Valid = $false; Parts = @(); Error = "non-literal command element: $($element.GetType().Name)" }
    }
    return [PSCustomObject]@{ Valid = $true; Parts = $parts.ToArray(); Error = "" }
}

function Test-GitInvocation {
    param(
        [Parameter(Mandatory = $true)][string[]]$Parts,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    if ($Parts.Count -lt 2) {
        return New-PolicyDecision $false "git" "git subcommand is missing"
    }
    $index = 1
    if ($Parts[$index].ToLowerInvariant() -eq "--no-pager") { $index++ }
    if ($index -lt $Parts.Count -and $Parts[$index] -ceq "-C") {
        if ($Parts.Count - $index -ne 4) {
            return New-PolicyDecision $false "git-read" "git -C is limited to an absolute registered worktree path and one exact read-only command"
        }

        $worktreePath = $Parts[$index + 1]
        $gitCSubcommand = $Parts[$index + 2]
        $gitCArgument = $Parts[$index + 3]
        $allowedGitCRead =
            ($gitCSubcommand -ceq "status" -and $gitCArgument -ceq "--short") -or
            ($gitCSubcommand -ceq "rev-parse" -and $gitCArgument -ceq "HEAD") -or
            ($gitCSubcommand -ceq "diff" -and $gitCArgument -ceq "--numstat")
        if (-not $allowedGitCRead) {
            return New-PolicyDecision $false "git-read" "git -C permits only status --short, rev-parse HEAD, or diff --numstat"
        }

        $isAbsoluteWorktreePath = [IO.Path]::IsPathRooted($worktreePath)
        if ($isAbsoluteWorktreePath -and [IO.Path]::DirectorySeparatorChar -eq [char]92) {
            $isAbsoluteWorktreePath = $worktreePath -match '^(?:[A-Za-z]:[\\/]|[\\/]{2}[^\\/]+[\\/][^\\/]+(?:[\\/]|$))'
        }
        if (-not $isAbsoluteWorktreePath) {
            return New-PolicyDecision $false "git-read" "git -C requires an absolute worktree path"
        }

        try {
            $normalizedWorktreePath = [IO.Path]::GetFullPath($worktreePath).TrimEnd([char[]]"\/")
        }
        catch {
            return New-PolicyDecision $false "git-read" ("git -C worktree path could not be normalized: " + $_.Exception.Message)
        }
        if (-not (Test-Path -LiteralPath $normalizedWorktreePath -PathType Container)) {
            return New-PolicyDecision $false "git-read" "git -C worktree path must be an existing directory"
        }

        try {
            $worktreeListOutput = @(& git -C $RepositoryRoot worktree list --porcelain -z 2>$null)
            $worktreeListExitCode = $LASTEXITCODE
        }
        catch {
            return New-PolicyDecision $false "git-read" ("git worktree list could not be read: " + $_.Exception.Message)
        }
        if ($worktreeListExitCode -ne 0) {
            return New-PolicyDecision $false "git-read" "git worktree list failed; git -C is denied"
        }

        $registeredWorktrees = @()
        $worktreeListRaw = $worktreeListOutput -join "`n"
        foreach ($field in [regex]::Split($worktreeListRaw, [string][char]0)) {
            if (-not $field.StartsWith("worktree ", [StringComparison]::Ordinal)) { continue }
            $listedPath = $field.Substring("worktree ".Length)
            if ([string]::IsNullOrWhiteSpace($listedPath) -or -not [IO.Path]::IsPathRooted($listedPath)) {
                return New-PolicyDecision $false "git-read" "git worktree list returned an invalid worktree path"
            }
            try {
                $registeredWorktrees += [IO.Path]::GetFullPath($listedPath).TrimEnd([char[]]"\/")
            }
            catch {
                return New-PolicyDecision $false "git-read" "git worktree list returned a worktree path that could not be normalized"
            }
        }
        if ($registeredWorktrees.Count -eq 0) {
            return New-PolicyDecision $false "git-read" "git worktree list returned no registered worktrees"
        }

        $pathComparison = [StringComparison]::Ordinal
        if ([IO.Path]::DirectorySeparatorChar -eq [char]92) {
            $pathComparison = [StringComparison]::OrdinalIgnoreCase
        }
        $isRegisteredWorktree = $false
        foreach ($registeredWorktree in $registeredWorktrees) {
            if ([string]::Equals($normalizedWorktreePath, $registeredWorktree, $pathComparison)) {
                $isRegisteredWorktree = $true
                break
            }
        }
        if (-not $isRegisteredWorktree) {
            return New-PolicyDecision $false "git-read" "git -C path is not present in git worktree list"
        }

        return New-PolicyDecision $true "git-read" "allowed exact read-only git -C for a registered worktree" $true $false
    }
    if ($index -ge $Parts.Count -or $Parts[$index].StartsWith("-")) {
        return New-PolicyDecision $false "git" "git global options other than --no-pager are not allowed"
    }
    $subcommand = $Parts[$index].ToLowerInvariant()
    $arguments = @()
    if ($index + 1 -lt $Parts.Count) { $arguments = @($Parts[($index + 1)..($Parts.Count - 1)]) }
    $lowerArguments = @($arguments | ForEach-Object { $_.ToLowerInvariant() })
    if (@($lowerArguments | Where-Object {
        $_ -in @("--ext-diff", "--textconv", "--output") -or $_.StartsWith("--output=")
    }).Count -gt 0) {
        return New-PolicyDecision $false "git" "external diff/text conversion and file output are outside the allowlist"
    }

    if ($subcommand -eq "add") {
        $pathArguments = @($arguments)
        if ($pathArguments.Count -gt 0 -and $pathArguments[0] -eq "--") {
            if ($pathArguments.Count -eq 1) {
                return New-PolicyDecision $false "git-write" "git add requires at least one literal repository path"
            }
            $pathArguments = @($pathArguments[1..($pathArguments.Count - 1)])
        }
        if ($pathArguments.Count -eq 0) {
            return New-PolicyDecision $false "git-write" "git add requires at least one literal repository path"
        }
        $repositoryPath = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd([char[]]"\/")
        foreach ($pathArgument in $pathArguments) {
            if ([string]::IsNullOrWhiteSpace($pathArgument) -or $pathArgument.StartsWith("-") -or
                $pathArgument.StartsWith(":") -or [Management.Automation.WildcardPattern]::ContainsWildcardCharacters($pathArgument)) {
                return New-PolicyDecision $false "git-write" "git add permits only literal paths; options, pathspec magic, and wildcards are denied"
            }
            try { $resolvedPath = Resolve-PolicyPath $pathArgument $repositoryPath }
            catch { return New-PolicyDecision $false "git-write" ("git add path could not be resolved: " + $_.Exception.Message) }
            if ([string]::Equals($resolvedPath, $repositoryPath, [StringComparison]::OrdinalIgnoreCase) -or
                -not $resolvedPath.StartsWith($repositoryPath + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
                return New-PolicyDecision $false "git-write" "git add paths must be below this repository and cannot select the repository root"
            }
            $relativePath = $resolvedPath.Substring($repositoryPath.Length).TrimStart([char[]]"\/")
            if ($relativePath -match '^(?i:\.git)(?:[\\/]|$)') {
                return New-PolicyDecision $false "git-write" "git add cannot target git metadata"
            }
        }
        return New-PolicyDecision $true "git-write" "allowed literal-path git add"
    }

    if ($subcommand -eq "fetch") {
        if ($arguments.Count -eq 1 -and $arguments[0] -ceq "origin") {
            return New-PolicyDecision $true "git-write" "allowed fixed origin fetch"
        }
        return New-PolicyDecision $false "git-write" "git fetch is limited to the exact form: git fetch origin"
    }

    if (@("commit", "push", "tag") -contains $subcommand) {
        if ($subcommand -eq "commit" -and @($arguments | Where-Object {
            $_ -cmatch '^-[^-]*n' -or $_.ToLowerInvariant() -eq "--no-verify" -or
            $_.ToLowerInvariant().StartsWith("--no-verify=")
        }).Count -gt 0) {
            return New-PolicyDecision $false "git-write" "commit hook bypass (-n/--no-verify) is outside the allowlist; use the recorded escape receipt for a genuine exception"
        }
        if ($subcommand -eq "commit") {
            for ($argumentIndex = 0; $argumentIndex -lt $arguments.Count; $argumentIndex++) {
                $argument = $arguments[$argumentIndex]
                if ($argument -ceq "-F" -or $argument -ceq "--file") {
                    if ($argumentIndex + 1 -ge $arguments.Count) {
                        return New-PolicyDecision $false "git-write" "git commit -F/--file requires a literal message file"
                    }
                    $messagePath = $arguments[++$argumentIndex]
                    if ($messagePath -eq "-" -or $messagePath.StartsWith("-") -or
                        [Management.Automation.WildcardPattern]::ContainsWildcardCharacters($messagePath)) {
                        return New-PolicyDecision $false "git-write" "git commit -F/--file requires a literal file path, not stdin, an option, or a wildcard"
                    }
                    try { $resolvedMessagePath = Resolve-PolicyPath $messagePath $RepositoryRoot }
                    catch { return New-PolicyDecision $false "git-write" ("commit message path could not be resolved: " + $_.Exception.Message) }
                    $repositoryPath = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd([char[]]"\/")
                    if (-not $resolvedMessagePath.StartsWith($repositoryPath + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
                        -not (Test-Path -LiteralPath $resolvedMessagePath -PathType Leaf)) {
                        return New-PolicyDecision $false "git-write" "git commit message file must be an existing literal file below this repository"
                    }
                    continue
                }
                if ($argument -clike "-F?*" -or $argument.ToLowerInvariant() -eq "--file" -or
                    $argument.ToLowerInvariant().StartsWith("--file=")) {
                    return New-PolicyDecision $false "git-write" "attached git commit message-file options are denied; use -F followed by one literal repository path"
                }
            }
        }
        if ($subcommand -eq "push" -and @($lowerArguments | Where-Object {
            $_ -in @("-f", "--force", "--force-with-lease", "-d", "--delete", "--mirror") -or
            $_ -eq "--no-verify" -or $_.StartsWith("--receive-pack") -or $_.StartsWith("--exec")
        }).Count -gt 0) {
            return New-PolicyDecision $false "git-write" "force/delete/exec push is outside the allowlist"
        }
        if ($subcommand -eq "push") {
            $remote = $null
            for ($argumentIndex = 0; $argumentIndex -lt $arguments.Count; $argumentIndex++) {
                $candidate = $arguments[$argumentIndex]
                if ($candidate -in @("-u", "--set-upstream")) { continue }
                if ($candidate.StartsWith("-")) { continue }
                $remote = $candidate
                break
            }
            if ($null -eq $remote -or $remote -notin @("origin", "https://github.com/oltotlo79-rgb/ORIGAMI3.git")) {
                return New-PolicyDecision $false "git-write" "push remote must be literal origin or the fixed ORIGAMI3 remote"
            }
        }
        if ($subcommand -eq "tag" -and @($lowerArguments | Where-Object { $_ -in @("-d", "--delete") }).Count -gt 0) {
            return New-PolicyDecision $false "git-write" "tag deletion is outside the allowlist"
        }
        return New-PolicyDecision $true "git-write" "allowed git write"
    }

    $readSubcommands = @(
        "status", "diff", "show", "log", "rev-parse", "ls-files",
        "diff-files", "diff-index", "diff-tree", "describe", "name-rev", "rev-list"
    )
    if ($readSubcommands -contains $subcommand) {
        if ($subcommand -eq "diff" -and @($lowerArguments | Where-Object { $_ -eq "--output" }).Count -gt 0) {
            return New-PolicyDecision $false "git-read" "git diff external execution/output is not allowed"
        }
        return New-PolicyDecision $true "git-read" "allowed git state/diff read" $true $false
    }
    if ($subcommand -eq "branch") {
        $mutating = @("-d", "-D", "-m", "-M", "-c", "-C", "--delete", "--move", "--copy", "--edit-description", "--set-upstream-to", "--unset-upstream")
        if (@($arguments | Where-Object { $mutating -contains $_ }).Count -eq 0) {
            return New-PolicyDecision $true "git-read" "allowed git branch read" $true $false
        }
    }
    if ($subcommand -eq "worktree" -and
        (($arguments.Count -eq 1 -and $arguments[0].ToLowerInvariant() -eq "list") -or
         ($arguments.Count -eq 2 -and $arguments[0].ToLowerInvariant() -eq "list" -and $arguments[1].ToLowerInvariant() -eq "--porcelain"))) {
        return New-PolicyDecision $true "git-read" "allowed exact git worktree list" $true $false
    }
    if ($subcommand -eq "for-each-ref") {
        if ($arguments.Count -eq 1 -and $arguments[0] -ceq "refs/wip") {
            return New-PolicyDecision $true "git-read" "allowed exact refs/wip inventory" $true $false
        }
        return New-PolicyDecision $false "git-read" "git for-each-ref is limited to the exact refs/wip prefix"
    }
    if ($subcommand -eq "write-tree") {
        if ($arguments.Count -eq 0) {
            return New-PolicyDecision $true "git-write" "allowed snapshot tree creation"
        }
        return New-PolicyDecision $false "git-write" "git write-tree takes no arguments in the coordinator allowlist"
    }
    if ($subcommand -eq "commit-tree") {
        if ($arguments.Count -lt 3 -or $arguments[0] -notmatch '^(?:[0-9A-Fa-f]{40}|[0-9A-Fa-f]{64})$') {
            return New-PolicyDecision $false "git-write" "git commit-tree requires a literal 40/64-hex tree id and an explicit -m or -F message"
        }
        $hasMessage = $false
        for ($argumentIndex = 1; $argumentIndex -lt $arguments.Count; $argumentIndex++) {
            $argument = $arguments[$argumentIndex]
            if ($argument -ceq "-p") {
                if (++$argumentIndex -ge $arguments.Count -or $arguments[$argumentIndex] -notmatch '^(?:HEAD|[0-9A-Fa-f]{40}|[0-9A-Fa-f]{64})$') {
                    return New-PolicyDecision $false "git-write" "git commit-tree -p requires literal HEAD or a 40/64-hex parent id"
                }
                continue
            }
            if ($argument -ceq "-m") {
                if (++$argumentIndex -ge $arguments.Count -or [string]::IsNullOrWhiteSpace($arguments[$argumentIndex])) {
                    return New-PolicyDecision $false "git-write" "git commit-tree -m requires a nonempty literal message"
                }
                $hasMessage = $true
                continue
            }
            if ($argument -ceq "-F") {
                if (++$argumentIndex -ge $arguments.Count) {
                    return New-PolicyDecision $false "git-write" "git commit-tree -F requires a literal repository file"
                }
                $messagePath = $arguments[$argumentIndex]
                try { $resolvedMessagePath = Resolve-PolicyPath $messagePath $RepositoryRoot }
                catch { return New-PolicyDecision $false "git-write" ("commit-tree message path could not be resolved: " + $_.Exception.Message) }
                $repositoryPath = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd([char[]]"\/")
                if ($messagePath -eq "-" -or $messagePath.StartsWith("-") -or
                    -not $resolvedMessagePath.StartsWith($repositoryPath + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
                    -not (Test-Path -LiteralPath $resolvedMessagePath -PathType Leaf)) {
                    return New-PolicyDecision $false "git-write" "git commit-tree -F requires an existing literal message file below this repository"
                }
                $hasMessage = $true
                continue
            }
            return New-PolicyDecision $false "git-write" "git commit-tree permits only literal -p, -m, and -F operands"
        }
        if (-not $hasMessage) {
            return New-PolicyDecision $false "git-write" "git commit-tree requires an explicit literal -m or -F message"
        }
        return New-PolicyDecision $true "git-write" "allowed refs/wip snapshot commit creation"
    }
    if ($subcommand -eq "update-ref") {
        if ($arguments.Count -eq 2 -and $arguments[0] -match '^refs/wip/[A-Za-z0-9][A-Za-z0-9._-]*$' -and
            $arguments[1] -match '^(?:[0-9A-Fa-f]{40}|[0-9A-Fa-f]{64})$') {
            return New-PolicyDecision $true "git-write" "allowed refs/wip snapshot ref update"
        }
        return New-PolicyDecision $false "git-write" "git update-ref is limited to refs/wip/<safe-name> and one literal 40/64-hex object id"
    }
    if ($subcommand -eq "remote") {
        if ($arguments.Count -eq 0 -or $arguments[0].ToLowerInvariant() -in @("-v", "--verbose", "get-url")) {
            return New-PolicyDecision $true "git-read" "allowed git remote read" $true $false
        }
    }
    return New-PolicyDecision $false "git" "git subcommand is outside commit/push/tag and state/diff reads"
}

function Test-RgInvocation {
    param([Parameter(Mandatory = $true)][string[]]$Parts)

    $arguments = @()
    if ($Parts.Count -gt 1) { $arguments = @($Parts[1..($Parts.Count - 1)]) }
    $lower = @($arguments | ForEach-Object { $_.ToLowerInvariant().Replace("\", "/") })
    foreach ($argument in $lower) {
        if ($argument -eq "--pre" -or $argument.StartsWith("--pre=") -or
            $argument -eq "--pre-glob" -or $argument.StartsWith("--pre-glob=") -or
            $argument -eq "--search-zip" -or $argument -eq "-z") {
            return New-PolicyDecision $false "file-read" "rg external preprocessing/archive execution flags are not allowed"
        }
        if ($argument.Contains($script:ForbiddenDocument) -and $argument -ne ("!" + $script:ForbiddenDocument) -and
            $argument -ne ("--glob=!" + $script:ForbiddenDocument)) {
            return New-PolicyDecision $false "file-read" "rg arguments must not re-include or directly name the prohibited document"
        }
    }
    $hasExclusion = $false
    for ($i = 0; $i -lt $lower.Count; $i++) {
        if ($lower[$i] -eq "--glob" -and $i + 1 -lt $lower.Count -and $lower[$i + 1] -eq ("!" + $script:ForbiddenDocument)) {
            $hasExclusion = $true
        }
        if ($lower[$i] -eq ("--glob=!" + $script:ForbiddenDocument)) {
            $hasExclusion = $true
        }
    }
    if (-not $hasExclusion) {
        return New-PolicyDecision $false "file-read" "rg is missing the mandatory prohibited-document exclusion glob"
    }
    return New-PolicyDecision $true "file-read" "allowed rg file search" $true $false
}

function Test-FileReadInvocation {
    param([Parameter(Mandatory = $true)][string[]]$Parts)

    $arguments = @()
    if ($Parts.Count -gt 1) { $arguments = @($Parts[1..($Parts.Count - 1)]) }
    if (Test-ForbiddenDocumentArgument $arguments) {
        return New-PolicyDecision $false "file-read" "the prohibited document cannot be read"
    }
    foreach ($argument in $arguments) {
        if (-not $argument.StartsWith("-") -and $argument -match '^[A-Za-z][A-Za-z0-9.-]*:' -and
            $argument -notmatch '^[A-Za-z]:[\\/]') {
            return New-PolicyDecision $false "file-read" "non-filesystem PowerShell providers are outside literal file viewing"
        }
    }
    return New-PolicyDecision $true "file-read" "allowed literal file read" $true $false
}

function Test-CurrentTimeInvocation {
    param(
        [Parameter(Mandatory = $true)][string[]]$Parts,
        [Parameter(Mandatory = $true)][string]$ToolName
    )

    if ($ToolName -ne "PowerShell") {
        return New-PolicyDecision $false "current-time" "the local report-time command is allowed only through the PowerShell tool"
    }
    if ($Parts.Count -ne 3 -or
        $Parts[0].ToLowerInvariant() -ne "get-date" -or
        $Parts[1].ToLowerInvariant() -ne "-format" -or
        $Parts[2] -cne "yyyy-MM-dd HH:mm") {
        return New-PolicyDecision $false "current-time" "only exact Get-Date -Format 'yyyy-MM-dd HH:mm' is allowed for local report-time reads"
    }
    return New-PolicyDecision $true "current-time" "allowed exact read-only local report-time read"
}

function Test-WatchStartInvocation {
    param(
        [Parameter(Mandatory = $true)][string[]]$Parts,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    if ($Parts.Count -ne 7 -or
        $Parts[1].ToLowerInvariant() -ne "-filepath" -or
        $Parts[3].ToLowerInvariant() -ne "-argumentlist" -or
        $Parts[5].ToLowerInvariant() -ne "-windowstyle" -or
        $Parts[6].ToLowerInvariant() -ne "hidden") {
        return New-PolicyDecision $false "watch" "watch launcher requires exact Start-Process -FilePath <absolute current PowerShell host> -ArgumentList <complete literal arguments> -WindowStyle Hidden shape"
    }

    if (-not [IO.Path]::IsPathRooted($Parts[2])) {
        return New-PolicyDecision $false "watch" "watch launcher FilePath must be the literal absolute path of the current hook PowerShell host; PATH lookup is denied"
    }
    try {
        $requestedHostPath = [IO.Path]::GetFullPath($Parts[2])
        $currentHostValue = [string](Get-Process -Id $PID -ErrorAction Stop).Path
        if ([string]::IsNullOrWhiteSpace($currentHostValue) -or -not [IO.Path]::IsPathRooted($currentHostValue)) {
            return New-PolicyDecision $false "watch" "current hook PowerShell host path is unavailable or non-absolute"
        }
        $currentHostPath = [IO.Path]::GetFullPath($currentHostValue)
        if (-not [string]::Equals($requestedHostPath, $currentHostPath, [StringComparison]::OrdinalIgnoreCase)) {
            return New-PolicyDecision $false "watch" "watch launcher FilePath must equal the current hook PowerShell host path: $currentHostPath"
        }
    }
    catch {
        return New-PolicyDecision $false "watch" ("watch launcher PowerShell host path could not be verified: " + $_.Exception.Message)
    }

    $argumentList = $Parts[4]
    $argumentPattern = '^-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "(?<script>[^"\r\n]+)" -DefinitionPath "(?<definition>[^"\r\n]+)" -RepositoryRoot "(?<root>[^"\r\n]+)" -IntervalMinutes 10 -StaleAfterMinutes 40$'
    $argumentMatch = [regex]::Match(
        $argumentList,
        $argumentPattern,
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if (-not $argumentMatch.Success) {
        return New-PolicyDecision $false "watch" "watch launcher arguments must be the complete fixed noninteractive continuous argument list; missing, extra, reordered, -Once, and dynamic arguments are denied"
    }

    $scriptPath = $argumentMatch.Groups["script"].Value
    $definitionPath = $argumentMatch.Groups["definition"].Value
    $rootPath = $argumentMatch.Groups["root"].Value
    foreach ($literalPath in @($scriptPath, $definitionPath, $rootPath)) {
        if (-not [IO.Path]::IsPathRooted($literalPath)) {
            return New-PolicyDecision $false "watch" "watch launcher script, DefinitionPath, and RepositoryRoot must be absolute literal paths"
        }
    }

    try {
        $resolvedScript = Resolve-PolicyPath $scriptPath $RepositoryRoot
        $expectedScript = Resolve-PolicyPath (Join-Path $RepositoryRoot "scripts\watch-agents.ps1") $RepositoryRoot
        if (-not [string]::Equals($resolvedScript, $expectedScript, [StringComparison]::OrdinalIgnoreCase) -or
            -not (Test-Path -LiteralPath $resolvedScript -PathType Leaf)) {
            return New-PolicyDecision $false "watch" "watch launcher -File must identify this repository's existing scripts/watch-agents.ps1"
        }
        $scriptItem = Get-Item -LiteralPath $resolvedScript -Force
        if (($scriptItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            return New-PolicyDecision $false "watch" "watch-agents.ps1 reparse points are not allowed"
        }

        $resolvedRoot = Resolve-PolicyPath $rootPath $RepositoryRoot
        if (-not [string]::Equals($resolvedRoot, $RepositoryRoot, [StringComparison]::OrdinalIgnoreCase)) {
            return New-PolicyDecision $false "watch" "watch launcher RepositoryRoot must be this repository"
        }

        $resolvedDefinition = Resolve-PolicyPath $definitionPath $RepositoryRoot
        if (-not (Test-Path -LiteralPath $resolvedDefinition -PathType Leaf) -or
            (Test-ForbiddenDocumentArgument @($resolvedDefinition))) {
            return New-PolicyDecision $false "watch" "watch launcher DefinitionPath must be an existing allowed file"
        }
        $definitionItem = Get-Item -LiteralPath $resolvedDefinition -Force
        if (($definitionItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            return New-PolicyDecision $false "watch" "watch DefinitionPath reparse points are not allowed"
        }
    }
    catch {
        return New-PolicyDecision $false "watch" ("watch launcher paths could not be verified: " + $_.Exception.Message)
    }

    return New-PolicyDecision $true "watch" "allowed exact hidden detached continuous watch-agents launcher"
}

function Test-DesktopStartInvocation {
    param(
        [Parameter(Mandatory = $true)][string[]]$Parts,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $filePath = $null
    $workingDirectory = $null
    $index = 1
    while ($index -lt $Parts.Count) {
        $argument = $Parts[$index]
        $lower = $argument.ToLowerInvariant()
        if ($lower -eq "-filepath") {
            if ($null -ne $filePath -or $index + 1 -ge $Parts.Count) { return New-PolicyDecision $false "desktop" "invalid Start-Process FilePath" }
            $filePath = $Parts[$index + 1]
            $index += 2
            continue
        }
        if ($lower -eq "-workingdirectory") {
            if ($null -ne $workingDirectory -or $index + 1 -ge $Parts.Count) { return New-PolicyDecision $false "desktop" "invalid Start-Process WorkingDirectory" }
            $workingDirectory = $Parts[$index + 1]
            $index += 2
            continue
        }
        if ($lower -eq "-passthru") {
            $index++
            continue
        }
        if (-not $argument.StartsWith("-") -and $null -eq $filePath) {
            $filePath = $argument
            $index++
            continue
        }
        return New-PolicyDecision $false "desktop" "Start-Process only permits desktop.exe FilePath, matching WorkingDirectory, and PassThru"
    }
    if ([string]::IsNullOrWhiteSpace([string]$filePath) -or -not [IO.Path]::IsPathRooted($filePath)) {
        return New-PolicyDecision $false "desktop" "desktop.exe must be an absolute literal path"
    }
    try {
        $resolved = Resolve-PolicyPath $filePath $RepositoryRoot
        if ([IO.Path]::GetFileName($resolved).ToLowerInvariant() -ne "desktop.exe" -or
            -not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
            return New-PolicyDecision $false "desktop" "desktop.exe literal path does not identify an existing file"
        }
        $item = Get-Item -LiteralPath $resolved -Force
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            return New-PolicyDecision $false "desktop" "desktop.exe reparse points are not allowed"
        }
        if ($null -ne $workingDirectory) {
            $resolvedWorkingDirectory = Resolve-PolicyPath $workingDirectory $RepositoryRoot
            if (-not [string]::Equals($resolvedWorkingDirectory, [IO.Path]::GetDirectoryName($resolved), [StringComparison]::OrdinalIgnoreCase)) {
                return New-PolicyDecision $false "desktop" "desktop WorkingDirectory must equal the executable directory"
            }
        }
        return New-PolicyDecision $true "desktop" "allowed desktop.exe start"
    }
    catch {
        return New-PolicyDecision $false "desktop" ("desktop.exe path could not be verified: " + $_.Exception.Message)
    }
}

function Test-ReportUpdateWaitInvocation {
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    if ($Arguments.Count -ne 9 -or
        $Arguments[0].ToLowerInvariant() -ne "-waitforreportupdate" -or
        $Arguments[1].ToLowerInvariant() -ne "-definitionpath" -or
        $Arguments[3].ToLowerInvariant() -ne "-reportpath" -or
        $Arguments[5].ToLowerInvariant() -ne "-timeoutseconds" -or
        $Arguments[7].ToLowerInvariant() -ne "-repositoryroot") {
        return New-PolicyDecision $false "report-wait" "report update wait requires exact ordered -WaitForReportUpdate -DefinitionPath <absolute watch-agents-*.json> -ReportPath <absolute registered report> -TimeoutSeconds <1..3600> -RepositoryRoot <this repository> arguments"
    }
    foreach ($pathArgument in @($Arguments[2], $Arguments[4], $Arguments[8])) {
        if (-not [IO.Path]::IsPathRooted($pathArgument)) {
            return New-PolicyDecision $false "report-wait" "DefinitionPath, ReportPath, and RepositoryRoot must be absolute literal paths"
        }
    }
    if ($Arguments[6] -notmatch '^[1-9][0-9]{0,3}$') {
        return New-PolicyDecision $false "report-wait" "TimeoutSeconds must be a literal integer between 1 and 3600"
    }
    $timeout = [int]$Arguments[6]
    if ($timeout -lt 1 -or $timeout -gt 3600) {
        return New-PolicyDecision $false "report-wait" "TimeoutSeconds must be a literal integer between 1 and 3600"
    }
    try {
        $resolvedArgumentRoot = Resolve-PolicyPath $Arguments[8] $RepositoryRoot
        if (-not [string]::Equals($resolvedArgumentRoot, $RepositoryRoot, [StringComparison]::OrdinalIgnoreCase)) {
            return New-PolicyDecision $false "report-wait" "RepositoryRoot must be this repository"
        }
        [void](Resolve-RegisteredWatchReportPath -Definition $Arguments[2] -Report $Arguments[4] -Root $RepositoryRoot)
    }
    catch {
        return New-PolicyDecision $false "report-wait" ("report update wait could not verify its read-only scope: " + $_.Exception.Message)
    }
    return New-PolicyDecision $true "report-wait" "allowed bounded read-only wait for one registered reportPath update"
}

function Test-ScriptInvocation {
    param(
        [Parameter(Mandatory = $true)][string]$ScriptPath,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    try {
        $resolvedScript = Resolve-PolicyPath $ScriptPath $RepositoryRoot
    }
    catch {
        return New-PolicyDecision $false "script" "script path could not be resolved"
    }
    $scriptsRoot = Join-Path $RepositoryRoot "scripts"
    $gatePaths = @{
        (Resolve-PolicyPath (Join-Path $scriptsRoot "check.ps1") $RepositoryRoot).ToLowerInvariant() = "check"
        (Resolve-PolicyPath (Join-Path $scriptsRoot "check-ci.ps1") $RepositoryRoot).ToLowerInvariant() = "check-ci"
        (Resolve-PolicyPath (Join-Path $scriptsRoot "check-release-ready.ps1") $RepositoryRoot).ToLowerInvariant() = "check-release-ready"
    }
    $resolvedKey = $resolvedScript.ToLowerInvariant()
    $receiptRepairPath = (Resolve-PolicyPath (Join-Path $scriptsRoot "check-receipt.ps1") $RepositoryRoot).ToLowerInvariant()
    $boundaryStopPath = (Resolve-PolicyPath (Join-Path $scriptsRoot "hooks\enforce-coordinator-boundary.ps1") $RepositoryRoot).ToLowerInvariant()
    $snapshotWorktreesPath = (Resolve-PolicyPath (Join-Path $scriptsRoot "snapshot-worktrees.ps1") $RepositoryRoot).ToLowerInvariant()
    if ($resolvedKey -eq $snapshotWorktreesPath) {
        try {
            $snapshotItem = Get-Item -LiteralPath $resolvedScript -Force -ErrorAction Stop
        }
        catch {
            return New-PolicyDecision $false "snapshot" "snapshot-worktrees script could not be verified"
        }
        if (($snapshotItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            return New-PolicyDecision $false "snapshot" "snapshot-worktrees script may not be a reparse point"
        }
        if ($Arguments.Count -eq 0) {
            return New-PolicyDecision $true "snapshot" "allowed exact refs/wip snapshot creation; branches and working indexes are not selected"
        }
        if ($Arguments.Count -eq 1 -and $Arguments[0].ToLowerInvariant() -eq "-check") {
            return New-PolicyDecision $true "snapshot" "allowed exact refs/wip snapshot freshness check"
        }
        return New-PolicyDecision $false "snapshot" "snapshot-worktrees permits only normal execution or the sole literal -Check argument"
    }
    if ($resolvedKey -eq $boundaryStopPath) {
        try {
            $boundaryItem = Get-Item -LiteralPath $resolvedScript -Force -ErrorAction Stop
        }
        catch {
            return New-PolicyDecision $false "boundary-control" "boundary script could not be verified"
        }
        if (($boundaryItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            return New-PolicyDecision $false "boundary-control" "boundary script may not be a reparse point"
        }
        if ($Arguments.Count -gt 0 -and $Arguments[0].ToLowerInvariant() -eq "-waitforreportupdate") {
            if (-not [IO.Path]::IsPathRooted($ScriptPath)) {
                return New-PolicyDecision $false "report-wait" "report update wait requires the absolute boundary script path"
            }
            return Test-ReportUpdateWaitInvocation -Arguments $Arguments -RepositoryRoot $RepositoryRoot
        }
        if (-not [IO.Path]::IsPathRooted($ScriptPath) -or
            $Arguments.Count -ne 3 -or
            $Arguments[0].ToLowerInvariant() -ne "-stoprecordedwatcher" -or
            $Arguments[1].ToLowerInvariant() -ne "-repositoryroot" -or
            -not [IO.Path]::IsPathRooted($Arguments[2])) {
            return New-PolicyDecision $false "watch-stop" "watcher stop requires the absolute boundary script and exact -StopRecordedWatcher -RepositoryRoot <this repository> arguments"
        }
        try { $resolvedArgumentRoot = Resolve-PolicyPath $Arguments[2] $RepositoryRoot }
        catch {
            return New-PolicyDecision $false "watch-stop" "watcher stop script or RepositoryRoot could not be verified"
        }
        if (-not [string]::Equals($resolvedArgumentRoot, $RepositoryRoot, [StringComparison]::OrdinalIgnoreCase) -or
            ($boundaryItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            return New-PolicyDecision $false "watch-stop" "watcher stop RepositoryRoot must be this repository and the boundary script may not be a reparse point"
        }
        return New-PolicyDecision $true "watch-stop" "allowed exact fixed-runtime watcher stop"
    }
    if ($resolvedKey -eq $receiptRepairPath) {
        if ($Arguments.Count -ne 3 -or
            $Arguments[0].ToLowerInvariant() -ne "-repairsigningkey" -or
            $Arguments[1].ToLowerInvariant() -ne "-reporoot") {
            return New-PolicyDecision $false "signing-key-repair" "check-receipt only permits -RepairSigningKey -RepoRoot <this repository>; missing, reordered, additional, or other mode arguments are denied"
        }
        if (-not [IO.Path]::IsPathRooted($Arguments[2])) {
            return New-PolicyDecision $false "signing-key-repair" "check-receipt -RepoRoot must be the absolute path of this repository"
        }
        try {
            $resolvedArgumentRoot = Resolve-PolicyPath $Arguments[2] $RepositoryRoot
        }
        catch {
            return New-PolicyDecision $false "signing-key-repair" "check-receipt -RepoRoot could not be resolved"
        }
        if (-not [string]::Equals($resolvedArgumentRoot.TrimEnd([char[]]'\/'), $RepositoryRoot.TrimEnd([char[]]'\/'), [StringComparison]::OrdinalIgnoreCase)) {
            return New-PolicyDecision $false "signing-key-repair" "check-receipt -RepoRoot must be this repository"
        }
        return New-PolicyDecision $true "signing-key-repair" "allowed exact coordinator-owned Windows DPAPI signing-key repair"
    }
    if (-not $gatePaths.ContainsKey($resolvedKey)) {
        return New-PolicyDecision $false "script" "only the three quality gates, exact signing-key repair, and exact fixed-runtime watcher stop are allowed as direct PowerShell script invocations; watch-agents start must use the exact hidden detached Start-Process launcher"
    }
    $kind = [string]$gatePaths[$resolvedKey]
    if ($kind -in @("check", "check-ci") -and $Arguments.Count -ne 0) {
        return New-PolicyDecision $false "quality-gate" "$kind must be invoked without narrowing/test-only arguments"
    }
    if ($kind -eq "check-release-ready") {
        if ($Arguments.Count -ne 0) {
            if ($Arguments.Count -ne 2 -or $Arguments[0].ToLowerInvariant() -ne "-tag" -or
                $Arguments[1] -notmatch "^v?[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$") {
                return New-PolicyDecision $false "quality-gate" "check-release-ready only permits a literal semantic -Tag"
            }
        }
    }
    return New-PolicyDecision $true "quality-gate" "allowed exact quality gate"
}

function Test-PowerShellWrapper {
    param(
        [Parameter(Mandatory = $true)][string[]]$Parts,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $wrapperCommand = $Parts[0]
    if ([IO.Path]::IsPathRooted($wrapperCommand) -or $wrapperCommand.Contains("\") -or
        $wrapperCommand.Contains("/") -or $wrapperCommand.Contains(":")) {
        try {
            $requestedHostPath = Resolve-PolicyPath $wrapperCommand $RepositoryRoot
            $currentHostValue = [string](Get-Process -Id $PID -ErrorAction Stop).Path
            if ([string]::IsNullOrWhiteSpace($currentHostValue) -or -not [IO.Path]::IsPathRooted($currentHostValue)) {
                return New-PolicyDecision $false "wrapper" "current hook PowerShell host path is unavailable or non-absolute"
            }
            $currentHostPath = [IO.Path]::GetFullPath($currentHostValue)
            if (-not [string]::Equals($requestedHostPath, $currentHostPath, [StringComparison]::OrdinalIgnoreCase)) {
                return New-PolicyDecision $false "wrapper" "a path-qualified PowerShell wrapper must equal the current hook PowerShell host path: $currentHostPath"
            }
        }
        catch {
            return New-PolicyDecision $false "wrapper" ("PowerShell wrapper host path could not be verified: " + $_.Exception.Message)
        }
    }

    $fileIndex = -1
    $index = 1
    while ($index -lt $Parts.Count) {
        $lower = $Parts[$index].ToLowerInvariant()
        if ($lower -in @("-nologo", "-noprofile", "-noninteractive")) { $index++; continue }
        if ($lower -eq "-executionpolicy") {
            if ($index + 1 -ge $Parts.Count -or $Parts[$index + 1].ToLowerInvariant() -ne "bypass") {
                return New-PolicyDecision $false "wrapper" "PowerShell wrapper only permits ExecutionPolicy Bypass"
            }
            $index += 2
            continue
        }
        if ($lower -eq "-file") { $fileIndex = $index; break }
        return New-PolicyDecision $false "wrapper" "PowerShell -Command/encoded/dynamic wrappers are not allowed"
    }
    if ($fileIndex -lt 0 -or $fileIndex + 1 -ge $Parts.Count) {
        return New-PolicyDecision $false "wrapper" "PowerShell wrapper must use -File with an exact allowed script"
    }
    $scriptArguments = @()
    if ($fileIndex + 2 -lt $Parts.Count) { $scriptArguments = @($Parts[($fileIndex + 2)..($Parts.Count - 1)]) }
    return Test-ScriptInvocation -ScriptPath $Parts[$fileIndex + 1] -Arguments $scriptArguments -RepositoryRoot $RepositoryRoot
}

function Test-StaticCommandInvocation {
    param(
        [Parameter(Mandatory = $true)][string[]]$Parts,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$ToolName
    )

    if ($Parts.Count -eq 0 -or [string]::IsNullOrWhiteSpace($Parts[0])) {
        return New-PolicyDecision $false "unknown" "command name is not static"
    }
    $leaf = [IO.Path]::GetFileName($Parts[0].Replace("/", "\")).ToLowerInvariant()
    if ($leaf -in @("powershell", "powershell.exe", "pwsh", "pwsh.exe")) {
        return Test-PowerShellWrapper -Parts $Parts -RepositoryRoot $RepositoryRoot
    }
    if ($leaf -eq "git" -or $leaf -eq "git.exe") {
        return Test-GitInvocation -Parts $Parts -RepositoryRoot $RepositoryRoot
    }
    if ($leaf -eq "rg" -or $leaf -eq "rg.exe") {
        return Test-RgInvocation -Parts $Parts
    }
    if ($leaf -in @("get-content", "get-item", "get-childitem", "test-path", "resolve-path", "get-filehash", "select-string")) {
        return Test-FileReadInvocation -Parts $Parts
    }
    if ($leaf -in @("get-process", "get-ciminstance", "get-nettcpconnection", "get-psdrive", "get-volume")) {
        return New-PolicyDecision $true "process-capacity" "allowed process/capacity read" $true $false
    }
    if ($leaf -eq "get-date") {
        return Test-CurrentTimeInvocation -Parts $Parts -ToolName $ToolName
    }
    if ($leaf -in @("where-object", "select-object", "sort-object", "measure-object", "format-table", "format-list", "out-string")) {
        return New-PolicyDecision $true "transform" "allowed literal output transform" $false $true
    }
    if ($leaf -eq "start-process") {
        if ($Parts.Count -ge 4 -and
            $Parts[1].ToLowerInvariant() -eq "-filepath" -and
            $Parts[3].ToLowerInvariant() -eq "-argumentlist") {
            return Test-WatchStartInvocation -Parts $Parts -RepositoryRoot $RepositoryRoot
        }
        return Test-DesktopStartInvocation -Parts $Parts -RepositoryRoot $RepositoryRoot
    }
    if ($leaf.EndsWith(".ps1")) {
        $arguments = @()
        if ($Parts.Count -gt 1) { $arguments = @($Parts[1..($Parts.Count - 1)]) }
        return Test-ScriptInvocation -ScriptPath $Parts[0] -Arguments $arguments -RepositoryRoot $RepositoryRoot
    }
    return New-PolicyDecision $false "unknown" "command/wrapper is not in the coordinator allowlist"
}

function Test-PolicyCommand {
    param(
        [Parameter(Mandatory = $true)][string]$ToolName,
        [Parameter(Mandatory = $true)][string]$CommandText,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    if ([string]::IsNullOrWhiteSpace($CommandText)) {
        return New-PolicyDecision $false "parse" "command is missing or empty"
    }
    if ($ToolName -eq "Bash" -and $CommandText -match '[;&|<>`$(){}\[\]\r\n\\*?~]') {
        return New-PolicyDecision $false "parse" "Bash metacharacters, expansion, chaining, and escaping are not supported"
    }
    $tokens = $null
    $parseErrors = $null
    $ast = [Management.Automation.Language.Parser]::ParseInput($CommandText, [ref]$tokens, [ref]$parseErrors)
    if (@($parseErrors).Count -ne 0) {
        return New-PolicyDecision $false "parse" ("shell parse error: " + [string]$parseErrors[0].Message)
    }
    $statements = @($ast.EndBlock.Statements)
    if ($statements.Count -ne 1 -or -not ($statements[0] -is [Management.Automation.Language.PipelineAst])) {
        return New-PolicyDecision $false "parse" "exactly one top-level pipeline is required; assignments/control flow are denied"
    }
    $pipeline = $statements[0]
    $backgroundProperty = $pipeline.PSObject.Properties["Background"]
    if ($null -ne $backgroundProperty -and [bool]$backgroundProperty.Value) {
        return New-PolicyDecision $false "parse" "shell background operators are denied"
    }
    $elements = @($pipeline.PipelineElements)
    if ($elements.Count -eq 1 -and $elements[0] -is [Management.Automation.Language.CommandExpressionAst]) {
        $normalized = [regex]::Replace($CommandText.Trim(), '\s+', ' ')
        if ($normalized -match '^\(Get-Process (?:-Name )?["'']?desktop["'']?(?: -ErrorAction Stop)?\)\.CloseMainWindow\(\)$') {
            return New-PolicyDecision $true "desktop" "allowed desktop CloseMainWindow"
        }
        return New-PolicyDecision $false "member" "member/static .NET invocation is denied except exact desktop CloseMainWindow()"
    }
    if ($elements.Count -eq 0) {
        return New-PolicyDecision $false "parse" "empty pipeline"
    }
    $decisions = New-Object "Collections.Generic.List[object]"
    foreach ($element in $elements) {
        if (-not ($element -is [Management.Automation.Language.CommandAst])) {
            return New-PolicyDecision $false "parse" "pipeline expressions are denied"
        }
        $shape = Get-StaticCommandParts $element
        if (-not $shape.Valid) {
            return New-PolicyDecision $false "parse" $shape.Error
        }
        $decision = Test-StaticCommandInvocation -Parts $shape.Parts -RepositoryRoot $RepositoryRoot -ToolName $ToolName
        if (-not $decision.Allowed) { return $decision }
        $decisions.Add($decision)
    }
    if ($decisions.Count -gt 1) {
        if (-not $decisions[0].PipelineSource) {
            return New-PolicyDecision $false "pipeline" "the first command is not an allowed read-only pipeline source"
        }
        for ($i = 1; $i -lt $decisions.Count; $i++) {
            if (-not $decisions[$i].PipelineTransform) {
                return New-PolicyDecision $false "pipeline" "only literal output transforms may follow a read-only source"
            }
        }
    }
    return $decisions[0]
}

function Test-AcknowledgementDeleteControl {
    param(
        [Parameter(Mandatory = $true)][string]$ToolName,
        [Parameter(Mandatory = $true)][string]$CommandText,
        [Parameter(Mandatory = $true)][string]$AcknowledgementPath
    )

    if ($ToolName -ne "PowerShell") { return $false }
    $tokens = $null
    $parseErrors = $null
    $ast = [Management.Automation.Language.Parser]::ParseInput($CommandText, [ref]$tokens, [ref]$parseErrors)
    if (@($parseErrors).Count -ne 0) { return $false }
    $statements = @($ast.EndBlock.Statements)
    if ($statements.Count -ne 1 -or -not ($statements[0] -is [Management.Automation.Language.PipelineAst])) { return $false }
    $elements = @($statements[0].PipelineElements)
    if ($elements.Count -ne 1 -or -not ($elements[0] -is [Management.Automation.Language.CommandAst])) { return $false }
    $shape = Get-StaticCommandParts $elements[0]
    if (-not $shape.Valid -or $shape.Parts.Count -ne 3) { return $false }
    if ($shape.Parts[0].ToLowerInvariant() -ne "remove-item" -or $shape.Parts[1].ToLowerInvariant() -ne "-literalpath") { return $false }
    try {
        $actual = [IO.Path]::GetFullPath($shape.Parts[2])
        $expected = [IO.Path]::GetFullPath($AcknowledgementPath)
        return [string]::Equals($actual, $expected, [StringComparison]::OrdinalIgnoreCase)
    }
    catch { return $false }
}

function Read-HookInputText {
    $stream = [Console]::OpenStandardInput()
    $reader = New-Object IO.StreamReader($stream, (New-Object Text.UTF8Encoding($false, $true)), $false)
    try { $text = $reader.ReadToEnd() }
    finally { $reader.Dispose() }
    $normalized = [string]$text
    while ($normalized.Length -gt 0) {
        $normalized = $normalized.TrimStart()
        if ($normalized.Length -eq 0 -or $normalized[0] -ne [char]0xFEFF) { break }
        $normalized = $normalized.Substring(1)
    }
    return $normalized
}

function Get-RepositoryRootFromPayload {
    param([Parameter(Mandatory = $true)]$Payload)

    $root = [string]$env:CLAUDE_PROJECT_DIR
    if ([string]::IsNullOrWhiteSpace($root)) {
        $root = [string](Get-ObjectPropertyValue $Payload "cwd")
    }
    if ([string]::IsNullOrWhiteSpace($root)) {
        throw "CLAUDE_PROJECT_DIR and payload cwd are both missing"
    }
    return [IO.Path]::GetFullPath($root).TrimEnd([char[]]"\/")
}

function Invoke-PostEvent {
    param(
        [Parameter(Mandatory = $true)]$Payload,
        [Parameter(Mandatory = $true)][string]$EventName,
        [Parameter(Mandatory = $true)]$Paths
    )

    $read = Read-BoundaryState -Paths $Paths
    if (-not $read.Exists) { return }
    if (-not $read.Valid) {
        Write-Warning "ORIGAMI3_COORDINATOR_BOUNDARY_STATE_BROKEN: post event cannot update state: $($read.Error)"
        return
    }
    $state = $read.Value
    if ([string](Get-ObjectPropertyValue $state "status") -ne "in-flight") { return }
    $toolUseId = [string](Get-ObjectPropertyValue $Payload "tool_use_id")
    $expected = [string](Get-ObjectPropertyValue $state "releaseToolUseId")
    if ([string]::IsNullOrWhiteSpace($toolUseId) -or $toolUseId -ne $expected) {
        Write-AuditEvent -Paths $Paths -Event "post-mismatch" -ToolName ([string]$state.toolName) -CommandHash ([string]$state.commandHash) -ToolUseId $toolUseId -Detail "expected=$expected event=$EventName"
        Write-Warning "HOOK_HEALTH_RELEASED check=coordinator-boundary status=awaiting-success reason=tool-use-id-mismatch"
        return
    }
    if ($EventName -eq "PostToolUse") {
        Write-AuditEvent -Paths $Paths -Event "release-success" -ToolName ([string]$state.toolName) -CommandHash ([string]$state.commandHash) -ToolUseId $toolUseId -Detail "PostToolUse"
        Remove-ActiveState -Paths $Paths
        return
    }

    $state.status = "blocked"
    $state.releaseToolUseId = $null
    $state.lastFailureAtUtc = [DateTime]::UtcNow.ToString("o")
    $state.lastFailure = [string](Get-ObjectPropertyValue $Payload "error")
    Write-JsonAtomically -Directory $Paths.Directory -Path $Paths.State -Value $state
    Write-AcknowledgementReceipt -Paths $Paths -State $state
    Write-AuditEvent -Paths $Paths -Event "release-failure" -ToolName ([string]$state.toolName) -CommandHash ([string]$state.commandHash) -ToolUseId $toolUseId -Detail ([string]$state.lastFailure)
    Write-Warning "HOOK_HEALTH_DEGRADED check=coordinator-boundary status=blocked reason=PostToolUseFailure acknowledgement=$($Paths.Acknowledgement)"
}

if ($WaitForReportUpdate.IsPresent) {
    try {
        Invoke-WaitForReportUpdate `
            -Definition $DefinitionPath `
            -Report $ReportPath `
            -Timeout $TimeoutSeconds `
            -Root $RepositoryRoot
        exit 0
    }
    catch {
        Write-Error "REPORT_UPDATE_WAIT_FAILED: $($_.Exception.Message)"
        exit 1
    }
}

if ($StopRecordedWatcher.IsPresent) {
    try {
        Invoke-StopRecordedWatcher -Root $RepositoryRoot
        exit 0
    }
    catch {
        Write-Error "RECORDED_WATCHER_STOP_FAILED: $($_.Exception.Message)"
        exit 1
    }
}

$rawInput = ""
try {
    $rawInput = Read-HookInputText
    if ([string]::IsNullOrWhiteSpace($rawInput)) {
        Write-PreToolDeny (Get-DenialReason -Detail "hook input is empty")
    }
    try {
        $payload = ConvertFrom-Json -InputObject $rawInput
    }
    catch {
        Write-PreToolDeny (Get-DenialReason -Detail ("hook JSON is invalid: " + $_.Exception.Message) -CommandHash (Get-Sha256Hex $rawInput))
    }

    $agentIdValue = Get-ObjectPropertyValue $payload "agent_id"
    if ($agentIdValue -is [string] -and -not [string]::IsNullOrWhiteSpace([string]$agentIdValue)) {
        exit 0
    }

    $eventName = [string](Get-ObjectPropertyValue $payload "hook_event_name")
    if ($eventName -notin @("PreToolUse", "PostToolUse", "PostToolUseFailure")) {
        Write-PreToolDeny (Get-DenialReason -Detail "unsupported or missing hook event")
    }
    $toolName = [string](Get-ObjectPropertyValue $payload "tool_name")
    if ($toolName -notin @("PowerShell", "Bash")) {
        if ($eventName -eq "PreToolUse") {
            Write-PreToolDeny (Get-DenialReason -Detail "main-thread tool is outside the shell allowlist")
        }
        exit 0
    }

    $repositoryRoot = Get-RepositoryRootFromPayload $payload
    if ([string]::IsNullOrWhiteSpace($StateRoot)) { $StateRoot = [IO.Path]::GetTempPath() }
    $paths = Get-StatePaths -RepositoryRoot $repositoryRoot -Root $StateRoot
    Enter-StateLock -RepositoryKey $paths.RepositoryKey

    if ($eventName -in @("PostToolUse", "PostToolUseFailure")) {
        Invoke-PostEvent -Payload $payload -EventName $eventName -Paths $paths
        exit 0
    }

    $toolInput = Get-ObjectPropertyValue $payload "tool_input"
    $command = [string](Get-ObjectPropertyValue $toolInput "command")
    $toolUseId = [string](Get-ObjectPropertyValue $payload "tool_use_id")
    $commandHash = Get-Sha256Hex ("$toolName`n$command")
    $stateRead = Read-BoundaryState -Paths $paths
    if ($stateRead.Exists -and -not $stateRead.Valid) {
        $detail = "boundary state is unreadable: $($stateRead.Error)"
        [void](New-BlockedState -Paths $paths -ToolName $toolName -CommandHash $commandHash -Reason $detail -ToolUseId $toolUseId)
        Write-PreToolDeny (Get-DenialReason -Detail $detail -CommandHash $commandHash -AcknowledgementPath $paths.Acknowledgement)
    }

    $state = $stateRead.Value
    if ($null -ne $state -and [string]$state.status -eq "blocked" -and
        (Test-Path -LiteralPath $paths.Acknowledgement -PathType Leaf) -and
        (Test-AcknowledgementDeleteControl -ToolName $toolName -CommandText $command -AcknowledgementPath $paths.Acknowledgement)) {
        Write-AuditEvent -Paths $paths -Event "ack-delete-control" -ToolName $toolName -CommandHash $commandHash -ToolUseId $toolUseId -Detail $paths.Acknowledgement
        exit 0
    }

    if ($null -ne $state -and [string]$state.status -eq "blocked" -and
        -not (Test-Path -LiteralPath $paths.Acknowledgement -PathType Leaf)) {
        $state.status = "released"
        $state.acknowledgedAtUtc = [DateTime]::UtcNow.ToString("o")
        Write-JsonAtomically -Directory $paths.Directory -Path $paths.State -Value $state
        Write-AuditEvent -Paths $paths -Event "acknowledged" -ToolName ([string]$state.toolName) -CommandHash ([string]$state.commandHash) -ToolUseId $toolUseId -Detail "acknowledgement file removed"
    }

    if ($null -ne $state -and [string]$state.status -eq "released" -and
        [string]$state.toolName -eq $toolName -and [string]$state.commandHash -eq $commandHash) {
        if ([string]::IsNullOrWhiteSpace($toolUseId)) {
            Write-PreToolDeny (Get-DenialReason -Detail "release requires a nonempty tool_use_id" -CommandHash $commandHash -AcknowledgementPath $paths.Acknowledgement)
        }
        $state.status = "in-flight"
        $state.releasedAtUtc = [DateTime]::UtcNow.ToString("o")
        $state.releaseToolUseId = $toolUseId
        Write-JsonAtomically -Directory $paths.Directory -Path $paths.State -Value $state
        Write-AuditEvent -Paths $paths -Event "release-used" -ToolName $toolName -CommandHash $commandHash -ToolUseId $toolUseId -Detail "one exact command allowed"
        Write-Warning "HOOK_HEALTH_RELEASED check=coordinator-boundary commandHash=$commandHash toolUseId=$toolUseId status=awaiting-success"
        exit 0
    }

    $decision = Test-PolicyCommand -ToolName $toolName -CommandText $command -RepositoryRoot $repositoryRoot
    if ($decision.Allowed) {
        if ($null -ne $state -and [string]$state.status -in @("released", "in-flight")) {
            Write-Warning "HOOK_HEALTH_RELEASED check=coordinator-boundary commandHash=$($state.commandHash) toolUseId=$($state.releaseToolUseId) status=awaiting-success"
        }
        exit 0
    }

    if ($null -ne $state -and [string]$state.status -eq "in-flight") {
        Write-PreToolDeny (Get-DenialReason -Detail "a one-time release is already awaiting its matching PostToolUse" -CommandHash $commandHash -AcknowledgementPath $paths.Acknowledgement)
    }

    $detail = [string]$decision.Reason
    [void](New-BlockedState -Paths $paths -ToolName $toolName -CommandHash $commandHash -Reason $detail -ToolUseId $toolUseId)
    Write-PreToolDeny (Get-DenialReason -Detail $detail -CommandHash $commandHash -AcknowledgementPath $paths.Acknowledgement)
}
catch {
    $detail = "boundary hook failed closed: $($_.Exception.Message)"
    Write-PreToolDeny (Get-DenialReason -Detail $detail -CommandHash (Get-Sha256Hex ([string]$rawInput)))
}
finally {
    Exit-StateLock
}
