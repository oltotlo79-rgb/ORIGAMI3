<#
.SYNOPSIS
委譲前に、10分間隔の停滞監視が実際に継続していることを検査します。

.DESCRIPTION
Check は終了コード 0=正常、1=監視policy不適合、2=検査不能を返します。
Hook は Claude Code の PreToolUse payloadを標準入力から受け、mainが呼ぶ委譲系toolだけを
同じ条件でfail-closedにします。非空文字列agent_idのsubagentは対象外です。
Hook接続時は `-Action Hook` を明示してください。
#>
[CmdletBinding()]
param(
    [ValidateSet("Check", "Hook")]
    [string]$Action = "Check",

    [string]$RepositoryRoot = ""
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
$script:Utf8NoBom = New-Object Text.UTF8Encoding($false, $true)
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
$script:RequiredIntervalMinutes = 10
$script:RequiredStaleAfterMinutes = 40
# 実機で観測した継続scan間隔の最大は600.253秒だった。12分はそこへ
# 119.747秒（約2分）のscheduler/走査猶予を加え、1周期を逃した監視は通さない境界である。
$script:FreshnessMinutes = 12.0
$script:FutureToleranceMinutes = 2.0
$script:RetryCount = 4
$script:RetryMilliseconds = 50

function New-AgentWatchResult {
    param(
        [Parameter(Mandatory = $true)][int]$ExitCode,
        [Parameter(Mandatory = $true)][string]$Code,
        [Parameter(Mandatory = $true)][string]$Message
    )

    return [pscustomobject]@{
        ExitCode = $ExitCode
        Code = $Code
        Message = $Message
    }
}

function Read-Utf8StandardInput {
    $stream = [Console]::OpenStandardInput()
    $reader = New-Object IO.StreamReader($stream, $script:Utf8NoBom, $false)
    try {
        $text = $reader.ReadToEnd()
    }
    finally {
        $reader.Dispose()
    }
    while ($text.Length -gt 0 -and $text[0] -eq [char]0xFEFF) {
        $text = $text.Substring(1)
    }
    return $text
}

function New-PolicyResult {
    param(
        [Parameter(Mandatory = $true)][string]$Code,
        [Parameter(Mandatory = $true)][string]$Message
    )
    return New-AgentWatchResult -ExitCode 1 -Code $Code -Message $Message
}

function New-CheckErrorResult {
    param(
        [Parameter(Mandatory = $true)][string]$Code,
        [Parameter(Mandatory = $true)][string]$Message
    )
    return New-AgentWatchResult -ExitCode 2 -Code $Code -Message $Message
}

function Get-ScriptFullPath {
    $path = [string]$MyInvocation.ScriptName
    if ([string]::IsNullOrWhiteSpace($path)) {
        $path = [string]$PSCommandPath
    }
    if ([string]::IsNullOrWhiteSpace($path)) {
        throw "検査scriptの実パスを取得できません"
    }
    return [IO.Path]::GetFullPath($path)
}

function Resolve-RepositoryRoot {
    param([string]$SuppliedRoot)

    $root = $SuppliedRoot
    if ([string]::IsNullOrWhiteSpace($root)) {
        $root = [string]$env:CLAUDE_PROJECT_DIR
    }
    if ([string]::IsNullOrWhiteSpace($root)) {
        $hookPath = Get-ScriptFullPath
        $hookDirectory = Split-Path -Parent $hookPath
        $scriptsDirectory = Split-Path -Parent $hookDirectory
        $root = Split-Path -Parent $scriptsDirectory
    }
    if ([string]::IsNullOrWhiteSpace($root)) {
        throw "RepositoryRootを解決できません"
    }
    $fullRoot = [IO.Path]::GetFullPath($root).TrimEnd([char[]]"\/")
    if (-not (Test-Path -LiteralPath $fullRoot -PathType Container)) {
        throw "RepositoryRootが存在しません: $fullRoot"
    }
    return $fullRoot
}

function Read-SharedFileBytes {
    param([Parameter(Mandatory = $true)][string]$Path)

    $stream = New-Object IO.FileStream(
        $Path,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
    )
    try {
        if ($stream.Length -gt [int]::MaxValue) {
            throw "検査対象が大きすぎます: $Path"
        }
        $bytes = New-Object byte[] ([int]$stream.Length)
        $offset = 0
        while ($offset -lt $bytes.Length) {
            $read = $stream.Read($bytes, $offset, $bytes.Length - $offset)
            if ($read -le 0) {
                throw "検査対象を最後まで読めません: $Path"
            }
            $offset += $read
        }
        return $bytes
    }
    finally {
        $stream.Dispose()
    }
}

function Get-Sha256HexFromBytes {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)

    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace("-", "").ToLowerInvariant()
    }
    finally {
        $sha.Dispose()
    }
}

function Get-SharedFileSha256Hex {
    param([Parameter(Mandatory = $true)][string]$Path)
    return Get-Sha256HexFromBytes -Bytes (Read-SharedFileBytes -Path $Path)
}

function Test-ReparsePoint {
    param([Parameter(Mandatory = $true)][string]$Path)

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    return (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)
}

function Test-SamePath {
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

function Get-RequiredStateValue {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Name
    )

    if (@($State.PSObject.Properties.Name) -notcontains $Name) {
        throw "runtime stateの必須fieldがありません: $Name"
    }
    return $State.$Name
}

function Parse-RoundtripUtc {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$FieldName
    )

    try {
        $parsed = [DateTime]::ParseExact(
            $Text,
            "o",
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::RoundtripKind
        )
    }
    catch {
        throw "runtime stateの日時fieldを読めません: $FieldName=$Text"
    }
    return $parsed.ToUniversalTime()
}

function Test-FreshTimestamp {
    param(
        [Parameter(Mandatory = $true)][DateTime]$TimestampUtc,
        [Parameter(Mandatory = $true)][DateTime]$NowUtc,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $ageMinutes = ($NowUtc - $TimestampUtc).TotalMinutes
    if ($ageMinutes -lt (-1.0 * $script:FutureToleranceMinutes)) {
        return New-CheckErrorResult -Code "FUTURE_TIMESTAMP" -Message (
            "{0} が現在より {1:N1} 分未来です。時計またはstateを確認してください。" -f $Label, (-1.0 * $ageMinutes)
        )
    }
    if ($ageMinutes -gt $script:FreshnessMinutes) {
        return New-PolicyResult -Code "STALE" -Message (
            "{0} が {1:N1} 分更新されていません（上限 {2:N0} 分）。" -f $Label, $ageMinutes, $script:FreshnessMinutes
        )
    }
    return $null
}

function Test-LockHeld {
    param([Parameter(Mandatory = $true)][string]$LockPath)

    try {
        $probe = New-Object IO.FileStream(
            $LockPath,
            [IO.FileMode]::Open,
            [IO.FileAccess]::ReadWrite,
            [IO.FileShare]::None
        )
        $probe.Dispose()
        return New-PolicyResult -Code "LOCK_NOT_HELD" -Message "singleton lockが保持されていません: $LockPath"
    }
    catch [IO.IOException] {
        $nativeCode = ($_.Exception.GetBaseException().HResult -band 0xFFFF)
        if ($nativeCode -eq 32 -or $nativeCode -eq 33) {
            return $null
        }
        return New-CheckErrorResult -Code "LOCK_CHECK_ERROR" -Message (
            "singleton lockを検査できません（Win32=$nativeCode）: $LockPath ($($_.Exception.Message))"
        )
    }
    catch {
        return New-CheckErrorResult -Code "LOCK_CHECK_ERROR" -Message (
            "singleton lockを検査できません: $LockPath ($($_.Exception.Message))"
        )
    }
}

function Test-AgentWatchSnapshot {
    param([Parameter(Mandatory = $true)][string]$Root)

    $expectedWatcherPath = [IO.Path]::GetFullPath((Join-Path $Root "scripts\watch-agents.ps1"))
    $scratchpadPath = [IO.Path]::GetFullPath((Join-Path $Root "scratchpad"))
    $expectedRuntimePath = [IO.Path]::GetFullPath((Join-Path $scratchpadPath "watch-agents.runtime.json"))
    $expectedOutputPath = [IO.Path]::GetFullPath((Join-Path $scratchpadPath "watch-agents.latest.log"))
    $expectedLockPath = [IO.Path]::GetFullPath((Join-Path $scratchpadPath "watch-agents.lock"))

    if (-not (Test-Path -LiteralPath $expectedWatcherPath -PathType Leaf)) {
        return New-CheckErrorResult -Code "WATCH_SCRIPT_MISSING" -Message "監視scriptが存在しません: $expectedWatcherPath"
    }
    if (-not (Test-Path -LiteralPath $expectedRuntimePath -PathType Leaf)) {
        return New-PolicyResult -Code "STATE_MISSING" -Message "継続監視のruntime stateがありません: $expectedRuntimePath"
    }

    try {
        if (Test-ReparsePoint -Path $expectedRuntimePath) {
            return New-PolicyResult -Code "STATE_REPARSE_POINT" -Message "runtime stateにreparse pointは使えません: $expectedRuntimePath"
        }
        $stateBytes = Read-SharedFileBytes -Path $expectedRuntimePath
        $stateText = $script:Utf8NoBom.GetString($stateBytes)
        $state = $stateText | ConvertFrom-Json
    }
    catch {
        return New-CheckErrorResult -Code "STATE_READ_ERROR" -Message "runtime stateを検査できません: $($_.Exception.Message)"
    }

    try {
        $schemaVersion = [int](Get-RequiredStateValue -State $state -Name "schemaVersion")
        $instanceId = [string](Get-RequiredStateValue -State $state -Name "instanceId")
        $watchPid = [int](Get-RequiredStateValue -State $state -Name "pid")
        $processStartUtc = Parse-RoundtripUtc -Text ([string](Get-RequiredStateValue -State $state -Name "processStartUtc")) -FieldName "processStartUtc"
        $processExecutablePath = [string](Get-RequiredStateValue -State $state -Name "processExecutablePath")
        $scriptPath = [string](Get-RequiredStateValue -State $state -Name "scriptPath")
        $scriptSha256 = [string](Get-RequiredStateValue -State $state -Name "scriptSha256")
        $stateRoot = [string](Get-RequiredStateValue -State $state -Name "repositoryRoot")
        $definitionPath = [string](Get-RequiredStateValue -State $state -Name "definitionPath")
        $definitionSha256 = [string](Get-RequiredStateValue -State $state -Name "definitionSha256")
        $runtimePath = [string](Get-RequiredStateValue -State $state -Name "runtimePath")
        $outputPath = [string](Get-RequiredStateValue -State $state -Name "outputPath")
        $outputSha256 = [string](Get-RequiredStateValue -State $state -Name "outputSha256")
        $outputLength = [Int64](Get-RequiredStateValue -State $state -Name "outputLength")
        $outputLastWriteUtc = Parse-RoundtripUtc -Text ([string](Get-RequiredStateValue -State $state -Name "outputLastWriteUtc")) -FieldName "outputLastWriteUtc"
        $lockPath = [string](Get-RequiredStateValue -State $state -Name "lockPath")
        $mode = [string](Get-RequiredStateValue -State $state -Name "mode")
        $intervalMinutes = [int](Get-RequiredStateValue -State $state -Name "intervalMinutes")
        $staleAfterMinutes = [int](Get-RequiredStateValue -State $state -Name "staleAfterMinutes")
        $agentCount = [int](Get-RequiredStateValue -State $state -Name "agentCount")
        $scanSequence = [Int64](Get-RequiredStateValue -State $state -Name "scanSequence")
        $scanCompletedUtc = Parse-RoundtripUtc -Text ([string](Get-RequiredStateValue -State $state -Name "scanCompletedUtc")) -FieldName "scanCompletedUtc"
        $stateWrittenUtc = Parse-RoundtripUtc -Text ([string](Get-RequiredStateValue -State $state -Name "stateWrittenUtc")) -FieldName "stateWrittenUtc"
    }
    catch {
        return New-CheckErrorResult -Code "STATE_SCHEMA_ERROR" -Message $_.Exception.Message
    }

    if ($schemaVersion -ne 1) {
        return New-PolicyResult -Code "SCHEMA_MISMATCH" -Message "runtime state schemaVersionが1ではありません: $schemaVersion"
    }
    $parsedInstanceId = [Guid]::Empty
    if (-not [Guid]::TryParse($instanceId, [ref]$parsedInstanceId) -or $parsedInstanceId -eq [Guid]::Empty) {
        return New-CheckErrorResult -Code "INSTANCE_ID_INVALID" -Message "runtime stateのinstanceIdが不正です: $instanceId"
    }
    if ($mode -ne "continuous") {
        return New-PolicyResult -Code "MODE_NOT_CONTINUOUS" -Message "-Once/単発判定は有効な継続監視として扱いません: mode=$mode"
    }
    if ($intervalMinutes -ne $script:RequiredIntervalMinutes) {
        return New-PolicyResult -Code "INTERVAL_MISMATCH" -Message (
            "監視間隔が10分ではありません: intervalMinutes=$intervalMinutes"
        )
    }
    if ($staleAfterMinutes -ne $script:RequiredStaleAfterMinutes) {
        return New-PolicyResult -Code "STALE_THRESHOLD_MISMATCH" -Message (
            "担当停滞の閾値が40分ではありません: staleAfterMinutes=$staleAfterMinutes"
        )
    }
    if ($watchPid -le 0 -or $agentCount -le 0 -or $scanSequence -le 0) {
        return New-CheckErrorResult -Code "STATE_VALUE_INVALID" -Message (
            "runtime stateの数値が不正です: pid=$watchPid agentCount=$agentCount scanSequence=$scanSequence"
        )
    }

    $pathChecks = @(
        @("repositoryRoot", $stateRoot, $Root),
        @("scriptPath", $scriptPath, $expectedWatcherPath),
        @("runtimePath", $runtimePath, $expectedRuntimePath),
        @("outputPath", $outputPath, $expectedOutputPath),
        @("lockPath", $lockPath, $expectedLockPath)
    )
    foreach ($pathCheck in $pathChecks) {
        if (-not (Test-SamePath -Actual ([string]$pathCheck[1]) -Expected ([string]$pathCheck[2]))) {
            return New-PolicyResult -Code "PATH_MISMATCH" -Message (
                "runtime stateの{0}が固定実パスと一致しません: actual={1} expected={2}" -f
                    $pathCheck[0], $pathCheck[1], $pathCheck[2]
            )
        }
    }

    foreach ($requiredFile in @($expectedWatcherPath, $definitionPath, $expectedOutputPath, $expectedLockPath)) {
        if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
            return New-PolicyResult -Code "RUNTIME_FILE_MISSING" -Message "監視runtimeの必須fileがありません: $requiredFile"
        }
        try {
            if (Test-ReparsePoint -Path $requiredFile) {
                return New-PolicyResult -Code "RUNTIME_REPARSE_POINT" -Message "監視runtimeにreparse pointは使えません: $requiredFile"
            }
        }
        catch {
            return New-CheckErrorResult -Code "FILE_METADATA_ERROR" -Message "file metadataを検査できません: $requiredFile ($($_.Exception.Message))"
        }
    }

    try {
        $process = Get-Process -Id $watchPid -ErrorAction Stop
    }
    catch {
        if ($_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound) {
            return New-PolicyResult -Code "PROCESS_MISSING" -Message "runtime stateの監視processが存在しません: PID=$watchPid"
        }
        return New-CheckErrorResult -Code "PROCESS_CHECK_ERROR" -Message "監視processを検査できません: PID=$watchPid ($($_.Exception.Message))"
    }

    try {
        $actualStartUtc = $process.StartTime.ToUniversalTime()
        $actualProcessPath = [IO.Path]::GetFullPath([string]$process.Path)
    }
    catch {
        return New-CheckErrorResult -Code "PROCESS_METADATA_ERROR" -Message "監視processの開始時刻/実パスを取得できません: PID=$watchPid ($($_.Exception.Message))"
    }
    if ($actualStartUtc.Ticks -ne $processStartUtc.Ticks) {
        return New-PolicyResult -Code "PROCESS_START_MISMATCH" -Message (
            "PIDの開始時刻がruntime stateと一致しません: PID=$watchPid state=$($processStartUtc.ToString('o')) actual=$($actualStartUtc.ToString('o'))"
        )
    }
    if (-not (Test-SamePath -Actual $actualProcessPath -Expected $processExecutablePath)) {
        return New-PolicyResult -Code "PROCESS_PATH_MISMATCH" -Message (
            "監視processの実行ファイルがruntime stateと一致しません: PID=$watchPid"
        )
    }

    $lockResult = Test-LockHeld -LockPath $expectedLockPath
    if ($null -ne $lockResult) {
        return $lockResult
    }

    try {
        if ((Get-SharedFileSha256Hex -Path $expectedWatcherPath) -ne $scriptSha256.ToLowerInvariant()) {
            return New-PolicyResult -Code "SCRIPT_HASH_MISMATCH" -Message "稼働中watcherと現在のwatch-agents.ps1のhashが一致しません。再起動してください。"
        }
        if ((Get-SharedFileSha256Hex -Path $definitionPath) -ne $definitionSha256.ToLowerInvariant()) {
            return New-PolicyResult -Code "DEFINITION_HASH_MISMATCH" -Message "監視定義が最後のscan後に変わっています。次のscan完了まで委譲できません。"
        }
        $outputBytes = Read-SharedFileBytes -Path $expectedOutputPath
        if ([Int64]$outputBytes.Length -ne $outputLength) {
            return New-PolicyResult -Code "OUTPUT_METADATA_MISMATCH" -Message "latest outputの長さがruntime stateと一致しません。"
        }
        if ((Get-Sha256HexFromBytes -Bytes $outputBytes) -ne $outputSha256.ToLowerInvariant()) {
            return New-PolicyResult -Code "OUTPUT_HASH_MISMATCH" -Message "latest outputのhashがruntime stateと一致しません。"
        }
        $runtimeItem = Get-Item -LiteralPath $expectedRuntimePath -Force -ErrorAction Stop
        $outputItem = Get-Item -LiteralPath $expectedOutputPath -Force -ErrorAction Stop
    }
    catch {
        return New-CheckErrorResult -Code "HASH_CHECK_ERROR" -Message "監視fileのhash/metadataを検査できません: $($_.Exception.Message)"
    }
    if ($outputItem.LastWriteTimeUtc.Ticks -ne $outputLastWriteUtc.Ticks) {
        return New-PolicyResult -Code "OUTPUT_METADATA_MISMATCH" -Message "latest outputの更新時刻がruntime stateと一致しません。"
    }

    $nowUtc = [DateTime]::UtcNow
    foreach ($freshnessCheck in @(
        @($scanCompletedUtc, "runtime stateのscanCompletedUtc"),
        @($stateWrittenUtc, "runtime stateのstateWrittenUtc"),
        @($runtimeItem.LastWriteTimeUtc, "runtime state file"),
        @($outputItem.LastWriteTimeUtc, "latest output file")
    )) {
        $freshnessResult = Test-FreshTimestamp -TimestampUtc ([DateTime]$freshnessCheck[0]) -NowUtc $nowUtc -Label ([string]$freshnessCheck[1])
        if ($null -ne $freshnessResult) {
            return $freshnessResult
        }
    }

    return New-AgentWatchResult -ExitCode 0 -Code "OK" -Message (
        "継続監視は正常です: PID={0} interval=10分 agents={1} scanAge={2:N1}分 output={3}" -f
            $watchPid, $agentCount, (($nowUtc - $scanCompletedUtc).TotalMinutes), $expectedOutputPath
    )
}

function Invoke-AgentWatchCheck {
    param([Parameter(Mandatory = $true)][string]$Root)

    $retryCodes = @("STATE_READ_ERROR", "OUTPUT_METADATA_MISMATCH", "OUTPUT_HASH_MISMATCH", "HASH_CHECK_ERROR")
    $result = $null
    for ($attempt = 1; $attempt -le $script:RetryCount; $attempt++) {
        $result = Test-AgentWatchSnapshot -Root $Root
        if ($result.ExitCode -eq 0 -or $retryCodes -notcontains $result.Code -or $attempt -eq $script:RetryCount) {
            return $result
        }
        Start-Sleep -Milliseconds $script:RetryMilliseconds
    }
    return $result
}

function Write-HookDeny {
    param(
        [Parameter(Mandatory = $true)]$Result,
        [Parameter(Mandatory = $true)][string]$Root
    )

    $kind = if ($Result.ExitCode -eq 1) { "AGENT_WATCH_POLICY_NG" } else { "AGENT_WATCH_CHECK_ERROR" }
    $reason = @(
        ("{0} [{1}]: {2}" -f $kind, $Result.Code, $Result.Message),
        "担当へ委譲する前に scripts/watch-agents.ps1 を -Once なし・10分間隔で継続稼働させてください。",
        ("確認先: {0}" -f (Join-Path $Root "scratchpad\watch-agents.latest.log"))
    ) -join " "
    $payload = [ordered]@{
        hookSpecificOutput = [ordered]@{
            hookEventName = "PreToolUse"
            permissionDecision = "deny"
            permissionDecisionReason = $reason
        }
    }
    $payload | ConvertTo-Json -Depth 5 -Compress | Write-Output
}

if ($Action -eq "Hook") {
    try {
        $raw = Read-Utf8StandardInput
        if ([string]::IsNullOrWhiteSpace($raw)) {
            $missingPayload = New-CheckErrorResult -Code "HOOK_PAYLOAD_MISSING" -Message "PreToolUse payloadが空です。"
            $fallbackRoot = if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) { [string]$env:CLAUDE_PROJECT_DIR } else { $RepositoryRoot }
            if ([string]::IsNullOrWhiteSpace($fallbackRoot)) { $fallbackRoot = "<不明>" }
            Write-HookDeny -Result $missingPayload -Root $fallbackRoot
            exit 0
        }
        $hookPayload = $raw | ConvertFrom-Json
        $toolName = [string]$hookPayload.tool_name
        $gatedTools = @("mcp__codex__codex", "mcp__codex__codex-reply", "Agent", "SendMessage")
        if ($gatedTools -notcontains $toolName) {
            exit 0
        }
        # Claude Codeのidentity契約では、agent_idが非空の文字列の場合だけsubagentである。
        # 欠落、空文字列、null、数値、配列、objectはmainとしてfail-closed検査へ進める。
        $agentIdProperty = $hookPayload.PSObject.Properties["agent_id"]
        if ($null -ne $agentIdProperty -and
            $agentIdProperty.Value -is [string] -and
            ([string]$agentIdProperty.Value).Length -gt 0) {
            exit 0
        }
        $resolvedRoot = Resolve-RepositoryRoot -SuppliedRoot $RepositoryRoot
        $hookResult = Invoke-AgentWatchCheck -Root $resolvedRoot
        if ($hookResult.ExitCode -ne 0) {
            Write-HookDeny -Result $hookResult -Root $resolvedRoot
        }
        exit 0
    }
    catch {
        $hookError = New-CheckErrorResult -Code "HOOK_CHECK_ERROR" -Message $_.Exception.Message
        $fallbackRoot = if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) { [string]$env:CLAUDE_PROJECT_DIR } else { $RepositoryRoot }
        if ([string]::IsNullOrWhiteSpace($fallbackRoot)) { $fallbackRoot = "<不明>" }
        Write-HookDeny -Result $hookError -Root $fallbackRoot
        exit 0
    }
}

try {
    $resolvedRoot = Resolve-RepositoryRoot -SuppliedRoot $RepositoryRoot
    $checkResult = Invoke-AgentWatchCheck -Root $resolvedRoot
}
catch {
    $checkResult = New-CheckErrorResult -Code "CHECK_ERROR" -Message $_.Exception.Message
}
Write-Output ("[{0}] {1}" -f $checkResult.Code, $checkResult.Message)
exit $checkResult.ExitCode
