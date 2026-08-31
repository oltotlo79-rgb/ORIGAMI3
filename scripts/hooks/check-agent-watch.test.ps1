[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$sourceWatcherPath = Join-Path (Split-Path -Parent $PSScriptRoot) "watch-agents.ps1"
$sourceCheckerPath = Join-Path $PSScriptRoot "check-agent-watch.ps1"
$sourceRepositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-check-agent-watch-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [IO.Path]::GetFullPath((Join-Path $tempBase $sandboxName))
$repositoryRoot = Join-Path $sandboxRoot "repo"
$watcherPath = Join-Path $repositoryRoot "scripts\watch-agents.ps1"
$checkerPath = Join-Path $repositoryRoot "scripts\hooks\check-agent-watch.ps1"
$definitionPath = Join-Path $repositoryRoot "scratchpad\agents.json"
$reportPath = Join-Path $repositoryRoot "scratchpad\agent-report.md"
$reportPath2 = Join-Path $repositoryRoot "scratchpad\agent-report-2.md"
$sourcePath = Join-Path $repositoryRoot "src\value.rs"
$sourcePath2 = Join-Path $repositoryRoot "src\value-2.rs"
$runtimePath = Join-Path $repositoryRoot "scratchpad\watch-agents.runtime.json"
$outputPath = Join-Path $repositoryRoot "scratchpad\watch-agents.latest.log"
$lockPath = Join-Path $repositoryRoot "scratchpad\watch-agents.lock"
$script:AssertionCount = 0
$script:OwnedProcesses = New-Object System.Collections.Generic.List[object]
$script:HeldLock = $null
$script:Utf8NoBom = [Text.UTF8Encoding]::new($false)

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )
    $script:AssertionCount += 1
    if (-not $Condition) {
        throw "ASSERTION FAILED: $Message"
    }
}

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)][AllowNull()]$Actual,
        [Parameter(Mandatory = $true)][AllowNull()]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )
    $script:AssertionCount += 1
    if ($Actual -ne $Expected) {
        throw "ASSERTION FAILED: $Message (expected=$Expected, actual=$Actual)"
    }
}

function Assert-Contains {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )
    $script:AssertionCount += 1
    if (-not $Text.Contains($Expected)) {
        throw "ASSERTION FAILED: $Message (missing='$Expected')`n$Text"
    }
}

function New-LongJapaneseDelegationPrompt {
    $sections = @(
        "統括です。**接続しました。** 今回は統括の委譲を止めているフック入力の文字化けを調べます。標準入力に渡されるJSONはUTF-8であり、日本語と記号の境目を含んでも解析前に別の符号化へ変換してはいけません。",
        "対象は scripts\\hooks\\check-agent-watch.ps1 と scripts\\hooks\\require-report-log.ps1 です。実際の入力では session_id、tool_name、tool_input.threadId、tool_input.prompt が同じJSONに入り、promptには作業の背景、対象ファイル、禁止事項、検査条件、報告先が長文で入ります。",
        "作業開始前に規約を最後まで読み、既存の未コミット変更を消さず、gitへの書き込みを行わないでください。ブラウザの窓、desktop.exe、配信サーバーを起動せず、Cargo.toml、Cargo.lock、vendor、競合レビュー文書にも触れません。",
        "原因は推測で断定せず、標準入力の生バイト、UTF-8の復号、JSON解析、子プロセスへ渡す指示文の順に実測します。日本語が壊れた結果としてJSONの引用符や区切りが失われると、fail-closedのフックが統括の委譲を全て拒否します。",
        "回帰試験では短い一文字ではなく、通常の漢字、ひらがな、カタカナと ** の記号が混在する長い指示書を使います。本文は実際の指示書と同程度の長さにし、3759バイト目や5033バイト目の近くでもJSON解析が成功することを新しいプロセスで確認します。",
        "検査は終了コードだけでなく、拒否JSONにHOOK_CHECK_ERRORやDELEGATION_GUARD_ERRORが出ないことを確認します。監視stateが正常な場合は委譲を許可し、stateが無い場合だけ監視policyの理由で拒否するという既存契約を狭めません。",
        "修正後は統括が .claude\\settings.json のPreToolUseへ戻せる完全なhooks断片を報告します。設定ファイルそのものは作業担当が書き換えず、統括が内容を確認して戻します。",
        "署名鍵の修復は統括利用者のWindows DPAPI識別子でしか正しく行えません。scripts\\check-receipt.ps1 -RepairSigningKey -RepoRoot 当リポジトリだけを許可し、ほかの引数、相対パス、別リポジトリ、検査modeは拒否します。",
        "コミットでは --no-verify と -n による検査回避を使いません。例外が必要なら受領を経由して理由と時刻を記録し、同じcommand hashを一回だけ解除する既存の境界契約を使います。",
        "成果物は scratchpad\\hook-encoding-fix-report.md に書き、各段階で新しいプロセスの終了コード、JSON入力のUTF-8バイト数、確認できたことと不明なことを分けて記録します。"
    )
    return (($sections -join "`n`n") + "`n`n" + ($sections -join "`n`n"))
}

function ConvertTo-ProcessArgumentString {
    param([Parameter(Mandatory = $true)][string[]]$Values)

    $parts = foreach ($value in $Values) {
        $escaped = [regex]::Replace($value, '(\\*)"', '$1$1\"')
        $trailingBackslashes = [regex]::Match($escaped, '\\*$').Value
        $escaped = $escaped + $trailingBackslashes
        '"' + $escaped + '"'
    }
    return ($parts -join ' ')
}

function Invoke-ChildPowerShell {
    param(
        [Parameter(Mandatory = $true)][string]$PowerShellPath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [string]$StandardInput = ""
    )

    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = $PowerShellPath
    $startInfo.Arguments = ConvertTo-ProcessArgumentString -Values $Arguments
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw "child PowerShellを起動できません"
        }
        $inputBytes = (New-Object Text.UTF8Encoding($false)).GetBytes($StandardInput)
        if ($inputBytes.Length -gt 0) {
            $process.StandardInput.BaseStream.Write($inputBytes, 0, $inputBytes.Length)
        }
        $process.StandardInput.BaseStream.Close()
        $stdout = $process.StandardOutput.ReadToEnd()
        $stderr = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        return [pscustomobject]@{
            ExitCode = $process.ExitCode
            Output = ($stdout + "`n" + $stderr).Trim()
        }
    }
    finally {
        $process.Dispose()
    }
}

function Invoke-Checker {
    param(
        [Parameter(Mandatory = $true)][string]$PowerShellPath,
        [ValidateSet("Check", "Hook")][string]$Action = "Check",
        [string]$Payload = ""
    )

    return Invoke-ChildPowerShell -PowerShellPath $PowerShellPath -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $checkerPath,
        "-Action", $Action,
        "-RepositoryRoot", $repositoryRoot
    ) -StandardInput $Payload
}

function Start-ContinuousWatcher {
    param([Parameter(Mandatory = $true)][string]$PowerShellPath)

    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = $PowerShellPath
    $startInfo.Arguments = ConvertTo-ProcessArgumentString -Values @(
        "-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $watcherPath,
        "-DefinitionPath", $definitionPath,
        "-RepositoryRoot", $repositoryRoot,
        "-IntervalMinutes", "10",
        "-StaleAfterMinutes", "40"
    )
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        $process.Dispose()
        throw "継続watcherを起動できません"
    }
    $script:OwnedProcesses.Add($process)
    return $process
}

function Stop-OwnedProcesses {
    foreach ($process in $script:OwnedProcesses) {
        try {
            if (-not $process.HasExited) {
                # この試験自身が起動し、Process objectを保持しているwatcherだけを終了する。
                $process.Kill()
                [void]$process.WaitForExit(10000)
            }
        }
        catch {
            # finallyでlock解放と一時領域の安全確認を続ける。
        }
        finally {
            $process.Dispose()
        }
    }
    $script:OwnedProcesses.Clear()
}

function Get-Sha256HexFromBytes {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][byte[]]$Bytes)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace("-", "").ToLowerInvariant()
    }
    finally {
        $sha.Dispose()
    }
}

function Get-FileSha256Hex {
    param([Parameter(Mandatory = $true)][string]$Path)
    return Get-Sha256HexFromBytes -Bytes ([IO.File]::ReadAllBytes($Path))
}

function Get-Sha256HexFromText {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)
    return Get-Sha256HexFromBytes -Bytes $script:Utf8NoBom.GetBytes($Text)
}

function ConvertTo-WatchBase64 {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)
    return [Convert]::ToBase64String($script:Utf8NoBom.GetBytes($Text))
}

function ConvertTo-CanonicalWatchPath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$BasePath
    )
    $fullPath = if ([IO.Path]::IsPathRooted($Path)) {
        [IO.Path]::GetFullPath($Path)
    }
    else {
        [IO.Path]::GetFullPath((Join-Path $BasePath $Path))
    }
    return $fullPath.Replace([IO.Path]::AltDirectorySeparatorChar, [IO.Path]::DirectorySeparatorChar)
}

function Get-TestAgentKey {
    param([Parameter(Mandatory = $true)]$Agent)
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("version=1")
    $lines.Add("reportPath=$(ConvertTo-WatchBase64 -Text (ConvertTo-CanonicalWatchPath -Path ([string]$Agent.reportPath) -BasePath $repositoryRoot))")
    foreach ($configuredPath in @($Agent.sourcePaths)) {
        $lines.Add("sourcePath=$(ConvertTo-WatchBase64 -Text (ConvertTo-CanonicalWatchPath -Path ([string]$configuredPath) -BasePath $repositoryRoot))")
    }
    return Get-Sha256HexFromText -Text ($lines -join "`n")
}

function Get-TestIncidentId {
    param([Parameter(Mandatory = $true)]$AgentState)
    return Get-Sha256HexFromText -Text (@(
        "version=1",
        "agentKey=$([string]$AgentState.agentKey)",
        "status=$([string]$AgentState.status)",
        "latestPath=$(ConvertTo-WatchBase64 -Text ([string]$AgentState.latestPath))",
        "latestWriteUtc=$([string]$AgentState.latestWriteUtc)",
        "problemDigest=$([string]$AgentState.problemDigest)"
    ) -join "`n")
}

function Get-TestAgentStatesHash {
    param([Parameter(Mandatory = $true)][object[]]$AgentStates)
    $lines = foreach ($agentState in $AgentStates) {
        @(
            "agentKey=$([string]$agentState.agentKey)",
            "name=$(ConvertTo-WatchBase64 -Text ([string]$agentState.name))",
            "status=$([string]$agentState.status)",
            "latestPath=$(ConvertTo-WatchBase64 -Text ([string]$agentState.latestPath))",
            "latestWriteUtc=$([string]$agentState.latestWriteUtc)",
            "problemDigest=$([string]$agentState.problemDigest)",
            "incidentId=$([string]$agentState.incidentId)"
        ) -join "`t"
    }
    return Get-Sha256HexFromText -Text (@($lines) -join "`n")
}

function New-StallResponseBlock {
    param(
        [Parameter(Mandatory = $true)][string]$Incident,
        [string]$Action = "investigate",
        [string]$Evidence = "監視出力の停滞IDと対象を確認した",
        [string]$Next = "担当へ現状確認を送り、次の走査で成果物更新を再測する"
    )
    return @(
        "[AGENT_WATCH_RESPONSE schema=1]",
        "incident=$Incident",
        "action=$Action",
        "evidence=$Evidence",
        "next=$Next",
        "[/AGENT_WATCH_RESPONSE]"
    ) -join "`n"
}

function New-HookPayload {
    param(
        [Parameter(Mandatory = $true)][ValidateSet("Agent", "SendMessage", "mcp__codex__codex", "mcp__codex__codex-reply")][string]$ToolName,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text
    )
    $toolInput = [ordered]@{}
    switch ($ToolName) {
        "Agent" { $toolInput.prompt = $Text }
        "SendMessage" { $toolInput.message = $Text; $toolInput.recipient = "agent-1" }
        "mcp__codex__codex" { $toolInput.prompt = $Text }
        "mcp__codex__codex-reply" { $toolInput.threadId = "thread-1"; $toolInput.prompt = $Text }
    }
    return ([ordered]@{
        session_id = "actual-payload-shape"
        tool_name = $ToolName
        tool_input = $toolInput
    } | ConvertTo-Json -Compress -Depth 6)
}

function Get-ResponseTextForRuntime {
    param(
        [string]$FirstAction = "investigate",
        [string]$FirstNext = "担当へ現状確認を送り、次の走査で成果物更新を再測する"
    )
    $runtimeState = [IO.File]::ReadAllText($runtimePath, $script:Utf8NoBom) | ConvertFrom-Json
    $blocks = New-Object System.Collections.Generic.List[string]
    $first = $true
    foreach ($agentState in @($runtimeState.agentStates)) {
        if ([string]$agentState.status -eq "active") { continue }
        if ($first) {
            $blocks.Add((New-StallResponseBlock -Incident ([string]$agentState.incidentId) -Action $FirstAction -Next $FirstNext))
            $first = $false
        }
        else {
            $blocks.Add((New-StallResponseBlock -Incident ([string]$agentState.incidentId)))
        }
    }
    return (($blocks.ToArray() -join "`n") + "`n委譲本文")
}

function New-ResponseTextForIncidents {
    param([Parameter(Mandatory = $true)][string[]]$IncidentIds)
    $blocks = @($IncidentIds | ForEach-Object { New-StallResponseBlock -Incident $_ })
    return (($blocks -join "`n") + "`n委譲本文")
}

function Release-TestLock {
    if ($null -ne $script:HeldLock) {
        $script:HeldLock.Dispose()
        $script:HeldLock = $null
    }
}

function Hold-TestLock {
    Release-TestLock
    if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
        [IO.File]::WriteAllText($lockPath, "lock", $script:Utf8NoBom)
    }
    $script:HeldLock = New-Object IO.FileStream(
        $lockPath,
        [IO.FileMode]::Open,
        [IO.FileAccess]::ReadWrite,
        [IO.FileShare]::None
    )
}

function Write-FakeRuntime {
    param(
        [string]$Mode = "continuous",
        [int]$ProcessId = $PID,
        [DateTime]$ProcessStartUtc = (Get-Process -Id $PID).StartTime.ToUniversalTime(),
        [DateTime]$ScanUtc = [DateTime]::UtcNow,
        [DateTime]$OutputUtc = [DateTime]::UtcNow,
        [string]$OutputText = "",
        [string]$StoredOutputHash = "",
        [string]$StoredOutputPath = "",
        [string[]]$AgentStatuses = @()
    )

    $definition = [IO.File]::ReadAllText($definitionPath, $script:Utf8NoBom) | ConvertFrom-Json
    $definitionAgents = @($definition.agents)
    if ($AgentStatuses.Count -eq 0) {
        $AgentStatuses = @($definitionAgents | ForEach-Object { "active" })
    }
    if ($AgentStatuses.Count -ne $definitionAgents.Count) {
        throw "fake runtimeのstatus件数がdefinitionと一致しません"
    }
    $emptyDigest = Get-Sha256HexFromText -Text ""
    $agentStates = @(
        for ($index = 0; $index -lt $definitionAgents.Count; $index++) {
            $agent = $definitionAgents[$index]
            $status = [string]$AgentStatuses[$index]
            $configuredPaths = @([string]$agent.reportPath) + @($agent.sourcePaths | ForEach-Object { [string]$_ })
            $existingFiles = @(
                foreach ($configuredPath in $configuredPaths) {
                    $resolvedPath = ConvertTo-CanonicalWatchPath -Path $configuredPath -BasePath $repositoryRoot
                    if (Test-Path -LiteralPath $resolvedPath -PathType Leaf) {
                        Get-Item -LiteralPath $resolvedPath -Force
                    }
                }
            )
            $latestItem = $existingFiles |
                Sort-Object `
                    @{ Expression = { $_.LastWriteTimeUtc }; Descending = $true },
                    @{ Expression = { ConvertTo-CanonicalWatchPath -Path $_.FullName -BasePath $repositoryRoot }; Ascending = $true } |
                Select-Object -First 1
            $latestPath = if ($null -eq $latestItem) { "" } else { ConvertTo-CanonicalWatchPath -Path $latestItem.FullName -BasePath $repositoryRoot }
            $latestWriteUtc = if ($null -eq $latestItem) { "" } else { $latestItem.LastWriteTimeUtc.ToUniversalTime().ToString("o") }
            $problemDigest = if ($status -eq "unmonitorable") {
                $problemLines = @(
                    foreach ($configuredPath in $configuredPaths) {
                        $resolvedPath = ConvertTo-CanonicalWatchPath -Path $configuredPath -BasePath $repositoryRoot
                        if (-not (Test-Path -LiteralPath $resolvedPath)) {
                            "path=$(ConvertTo-WatchBase64 -Text $resolvedPath)`tproblem=$(ConvertTo-WatchBase64 -Text '存在しません')"
                        }
                    }
                )
                Get-Sha256HexFromText -Text ($problemLines -join "`n")
            }
            else {
                $emptyDigest
            }
            $agentState = [ordered]@{
                agentKey = Get-TestAgentKey -Agent $agent
                name = [string]$agent.name
                status = $status
                latestPath = $latestPath
                latestWriteUtc = $latestWriteUtc
                problemDigest = $problemDigest
                incidentId = ""
            }
            if ($status -ne "active") {
                $agentState.incidentId = Get-TestIncidentId -AgentState ([pscustomobject]$agentState)
            }
            [pscustomobject]$agentState
        }
    )
    $activeCount = @($agentStates | Where-Object status -eq "active").Count
    $stalledCount = @($agentStates | Where-Object status -eq "stalled").Count
    $unmonitorableCount = @($agentStates | Where-Object status -eq "unmonitorable").Count
    $statesHash = Get-TestAgentStatesHash -AgentStates $agentStates
    if ([string]::IsNullOrEmpty($OutputText)) {
        $lines = New-Object System.Collections.Generic.List[string]
        $lines.Add("fresh watcher output")
        $lines.Add("AGENT_WATCH_STATUS schema=2 total=$($agentStates.Count) active=$activeCount stalled=$stalledCount unmonitorable=$unmonitorableCount states_sha256=$statesHash")
        foreach ($agentState in $agentStates) {
            if ([string]$agentState.status -eq "active") { continue }
            $lines.Add("AGENT_WATCH_INCIDENT schema=2 status=$([string]$agentState.status) incident=$([string]$agentState.incidentId) agent_key=$([string]$agentState.agentKey) name_b64=$(ConvertTo-WatchBase64 -Text ([string]$agentState.name))")
        }
        $OutputText = ($lines -join "`n") + "`n"
    }
    $outputBytes = $script:Utf8NoBom.GetBytes($OutputText)
    [IO.File]::WriteAllBytes($outputPath, $outputBytes)
    [IO.File]::SetLastWriteTimeUtc($outputPath, $OutputUtc)
    $outputItem = Get-Item -LiteralPath $outputPath
    if ([string]::IsNullOrWhiteSpace($StoredOutputHash)) {
        $StoredOutputHash = Get-Sha256HexFromBytes -Bytes $outputBytes
    }
    if ([string]::IsNullOrWhiteSpace($StoredOutputPath)) {
        $StoredOutputPath = [IO.Path]::GetFullPath($outputPath)
    }
    if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
        [IO.File]::WriteAllText($lockPath, "lock", $script:Utf8NoBom)
    }
    $owner = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    $processPath = if ($null -eq $owner) {
        (Get-Process -Id $PID).Path
    }
    else {
        $owner.Path
    }
    $state = [ordered]@{
        schemaVersion = 2
        instanceId = [Guid]::NewGuid().ToString("D")
        pid = $ProcessId
        processStartUtc = $ProcessStartUtc.ToString("o")
        processExecutablePath = [IO.Path]::GetFullPath($processPath)
        scriptPath = [IO.Path]::GetFullPath($watcherPath)
        scriptSha256 = Get-FileSha256Hex -Path $watcherPath
        repositoryRoot = [IO.Path]::GetFullPath($repositoryRoot).TrimEnd([char[]]"\/")
        definitionPath = [IO.Path]::GetFullPath($definitionPath)
        definitionSha256 = Get-FileSha256Hex -Path $definitionPath
        runtimePath = [IO.Path]::GetFullPath($runtimePath)
        outputPath = $StoredOutputPath
        outputSha256 = $StoredOutputHash
        outputLength = [Int64]$outputBytes.Length
        outputLastWriteUtc = $outputItem.LastWriteTimeUtc.ToString("o")
        lockPath = [IO.Path]::GetFullPath($lockPath)
        mode = $Mode
        intervalMinutes = 10
        staleAfterMinutes = 40
        agentCount = $agentStates.Count
        activeCount = $activeCount
        stalledCount = $stalledCount
        unmonitorableCount = $unmonitorableCount
        agentStatesSha256 = $statesHash
        agentStates = $agentStates
        scanSequence = 1
        scanCompletedUtc = $ScanUtc.ToString("o")
        stateWrittenUtc = [DateTime]::UtcNow.ToString("o")
    }
    [IO.File]::WriteAllText($runtimePath, ($state | ConvertTo-Json -Depth 10), $script:Utf8NoBom)
}

function Update-FakeRuntime {
    param([Parameter(Mandatory = $true)][scriptblock]$Mutation)
    $state = [IO.File]::ReadAllText($runtimePath, $script:Utf8NoBom) | ConvertFrom-Json
    & $Mutation $state
    [IO.File]::WriteAllText($runtimePath, ($state | ConvertTo-Json -Depth 10), $script:Utf8NoBom)
}

function Remove-TestSandbox {
    Release-TestLock
    Stop-OwnedProcesses
    if (-not (Test-Path -LiteralPath $sandboxRoot)) {
        return
    }
    $resolved = [IO.Path]::GetFullPath($sandboxRoot).TrimEnd([char[]]"\/")
    $parent = [IO.Path]::GetDirectoryName($resolved)
    $leaf = [IO.Path]::GetFileName($resolved)
    if (($parent -ne $tempBase) -or
        (-not [regex]::IsMatch($leaf, '^ori3-check-agent-watch-test-[0-9a-f]{32}$', [Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
        throw "安全でない一時領域の削除を拒否しました: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

if (-not (Test-Path -LiteralPath $sourceWatcherPath -PathType Leaf)) {
    throw "watcher本体が見つかりません: $sourceWatcherPath"
}
if (-not (Test-Path -LiteralPath $sourceCheckerPath -PathType Leaf)) {
    throw "検査本体が見つかりません: $sourceCheckerPath"
}
$powerShellCommand = Get-Command powershell.exe, pwsh.exe, pwsh -ErrorAction SilentlyContinue | Select-Object -First 1
if ($null -eq $powerShellCommand) {
    throw "新しいprocessを起動するPowerShellが見つかりません"
}

[void][IO.Directory]::CreateDirectory((Split-Path -Parent $checkerPath))
[void][IO.Directory]::CreateDirectory((Split-Path -Parent $definitionPath))
[void][IO.Directory]::CreateDirectory((Split-Path -Parent $sourcePath))
[IO.File]::Copy($sourceWatcherPath, $watcherPath)
[IO.File]::Copy($sourceCheckerPath, $checkerPath)
[IO.File]::WriteAllText($reportPath, "report", $script:Utf8NoBom)
[IO.File]::WriteAllText($reportPath2, "report 2", $script:Utf8NoBom)
[IO.File]::WriteAllText($sourcePath, "source", $script:Utf8NoBom)
[IO.File]::WriteAllText($sourcePath2, "source 2", $script:Utf8NoBom)
[IO.File]::WriteAllText(
    $definitionPath,
    (([ordered]@{
        agents = @(
            [ordered]@{
                name = "test-agent"
                reportPath = "scratchpad/agent-report.md"
                sourcePaths = @("src/value.rs")
            },
            [ordered]@{
                name = "test-agent-2"
                reportPath = "scratchpad/agent-report-2.md"
                sourcePaths = @("src/value-2.rs")
            }
        )
    }) | ConvertTo-Json -Depth 8),
    $script:Utf8NoBom
)

try {
    Write-Output "[1/20] runtime stateが無ければpolicy NG(1)"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "stateなしをpolicy NGにすること"
    Assert-Contains $result.Output "STATE_MISSING" "stateなしの理由codeを出すこと"

    Write-Output "[2/20] -Onceは成功しても有効watcherを作らない"
    $once = Invoke-ChildPowerShell -PowerShellPath $powerShellCommand.Source -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $watcherPath,
        "-DefinitionPath", $definitionPath,
        "-RepositoryRoot", $repositoryRoot,
        "-Once"
    )
    Assert-Equal $once.ExitCode 0 "-Once自体の既存用途は成功すること"
    Assert-True (-not (Test-Path -LiteralPath $runtimePath)) "-Onceがruntime stateを作らないこと"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "-Once後も委譲前検査を通さないこと"

    Write-Output "[3/20] 実watcherのfresh stateは正常(0)"
    $watcher = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline -and -not (Test-Path -LiteralPath $runtimePath -PathType Leaf)) {
        if ($watcher.HasExited) { break }
        Start-Sleep -Milliseconds 100
    }
    Assert-True (-not $watcher.HasExited) "実watcherが継続稼働すること"
    Assert-True (Test-Path -LiteralPath $runtimePath -PathType Leaf) "実watcherがruntime stateを発行すること"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 0 "freshな実watcherを正常とすること"
    Assert-Contains $result.Output "[OK]" "正常理由を出すこと"

    Write-Output "[4/20] watcher終了後はPID不在でpolicy NG(1)"
    $watcher.Kill()
    [void]$watcher.WaitForExit(10000)
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "終了済みwatcherを通さないこと"
    Assert-Contains $result.Output "PROCESS_MISSING" "PID不在の理由codeを出すこと"

    Write-Output "[5/20] -OnceとPID/start不一致をpolicy NG(1)"
    Hold-TestLock
    Write-FakeRuntime -Mode "once"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "state上のOnceも通さないこと"
    Assert-Contains $result.Output "MODE_NOT_CONTINUOUS" "Onceの理由codeを出すこと"
    Write-FakeRuntime -ProcessStartUtc ((Get-Process -Id $PID).StartTime.ToUniversalTime().AddSeconds(1))
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "PID開始時刻不一致を通さないこと"
    Assert-Contains $result.Output "PROCESS_START_MISMATCH" "開始時刻不一致の理由codeを出すこと"
    Write-FakeRuntime -ProcessId 2147483000
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "存在しないPIDを通さないこと"
    Assert-Contains $result.Output "PROCESS_MISSING" "存在しないPIDの理由codeを出すこと"

    Write-Output "[6/20] singleton lockなしをpolicy NG(1)"
    Write-FakeRuntime
    Release-TestLock
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "lockなしを通さないこと"
    Assert-Contains $result.Output "LOCK_NOT_HELD" "lockなしの理由codeを出すこと"

    Write-Output "[7/20] 12分を超えたoutput/scanをpolicy NG(1)"
    Hold-TestLock
    $old = [DateTime]::UtcNow.AddMinutes(-13)
    Write-FakeRuntime -ScanUtc $old -OutputUtc $old
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "stale outputを通さないこと"
    Assert-Contains $result.Output "STALE" "staleの理由codeを出すこと"

    Write-Output "[8/20] freshで自己整合してもwatcherでないcurrent PID/lockを拒否する"
    Write-FakeRuntime
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "test runner PIDをwatcherとして通さないこと"
    Assert-Contains $result.Output "PROCESS_COMMAND_MISMATCH" "native argvが固定watcher起動形でない理由を出すこと"

    Write-Output "[9/20] output hash/path不一致をpolicy NG(1)"
    Write-FakeRuntime -StoredOutputHash ("0" * 64)
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "output hash不一致を通さないこと"
    Assert-Contains $result.Output "OUTPUT_HASH_MISMATCH" "hash不一致の理由codeを出すこと"
    Write-FakeRuntime -StoredOutputPath (Join-Path $repositoryRoot "scratchpad\other.log")
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "固定output path不一致を通さないこと"
    Assert-Contains $result.Output "PATH_MISMATCH" "path不一致の理由codeを出すこと"

    Write-Output "[10/20] 壊れたstateは検査不能(2)"
    [IO.File]::WriteAllText($runtimePath, "{broken", $script:Utf8NoBom)
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 2 "壊れたJSONを検査不能にすること"
    Assert-Contains $result.Output "STATE_READ_ERROR" "検査不能の理由codeを出すこと"

    Write-Output "[11/20] Hook modeはmainの委譲toolをfail-closedにする"
    Release-TestLock
    foreach ($runtimeFile in @($runtimePath, $outputPath, $lockPath)) {
        if (Test-Path -LiteralPath $runtimeFile -PathType Leaf) {
            Remove-Item -LiteralPath $runtimeFile -Force
        }
    }
    $freshTime = [DateTime]::UtcNow
    foreach ($watchedFile in @($reportPath, $reportPath2, $sourcePath, $sourcePath2)) {
        [IO.File]::SetLastWriteTimeUtc($watchedFile, $freshTime)
    }
    $activeWatcher = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline -and -not (Test-Path -LiteralPath $runtimePath -PathType Leaf)) {
        if ($activeWatcher.HasExited) { break }
        Start-Sleep -Milliseconds 100
    }
    Assert-True (-not $activeWatcher.HasExited) "固定argvの実watcherが継続稼働すること"
    Assert-True (Test-Path -LiteralPath $runtimePath -PathType Leaf) "固定argvの実watcherがruntime stateを発行すること"
    $agentPayload = ([ordered]@{ tool_name = "Agent"; tool_input = [ordered]@{ prompt = "test" } } | ConvertTo-Json -Compress)
    $hookStopwatch = [Diagnostics.Stopwatch]::StartNew()
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload $agentPayload
    $hookStopwatch.Stop()
    Write-Output ("NATIVE_ARGV_HOOK_ELAPSED_MS={0}" -f $hookStopwatch.ElapsedMilliseconds)
    Assert-True ($hookStopwatch.Elapsed.TotalSeconds -lt 5.0) "実設定timeout 5秒以内にnative argvとlive stateを検査すること"
    Assert-Equal $result.ExitCode 0 "Hook protocol自体は0で返すこと"
    Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "fresh watcherなら委譲を拒否しないこと"
    Remove-Item -LiteralPath $runtimePath -Force
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload $agentPayload
    Assert-Equal $result.ExitCode 0 "拒否もHook protocolでは0で返すこと"
    Assert-Contains $result.Output '"permissionDecision":"deny"' "stateなしなら委譲を拒否すること"
    Assert-Contains $result.Output "AGENT_WATCH_POLICY_NG" "拒否理由へpolicy区分を出すこと"

    Write-Output "[12/20] agent_idは非空文字列だけsubagentとして検査を省略する"
    $subagentPayload = ([ordered]@{
        tool_name = "Agent"
        agent_id = "agent-123"
        tool_input = [ordered]@{ prompt = "test" }
    } | ConvertTo-Json -Compress)
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload $subagentPayload
    Assert-Equal $result.ExitCode 0 "subagent Hook protocolを0で返すこと"
    Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "非空文字列agent_idだけはmain用監視検査を省略すること"

    $emptyAgentPayload = ([ordered]@{
        tool_name = "Agent"
        agent_id = ""
        tool_input = [ordered]@{ prompt = "test" }
    } | ConvertTo-Json -Compress)
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload $emptyAgentPayload
    Assert-Equal $result.ExitCode 0 "空agent_idの拒否もHook protocolでは0で返すこと"
    Assert-Contains $result.Output '"permissionDecision":"deny"' "空agent_idをsubagent扱いせずmainとして検査すること"

    $numericAgentPayload = ([ordered]@{
        tool_name = "Agent"
        agent_id = 42
        tool_input = [ordered]@{ prompt = "test" }
    } | ConvertTo-Json -Compress)
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload $numericAgentPayload
    Assert-Equal $result.ExitCode 0 "非文字列agent_idの拒否もHook protocolでは0で返すこと"
    Assert-Contains $result.Output '"permissionDecision":"deny"' "非文字列agent_idをsubagent扱いせずmainとして検査すること"

    Write-Output "[13/20] 実際の委譲payload形の長い日本語promptをraw UTF-8で壊さずJSON parseする"
    # 直前のmain扱いケースはSTATE_MISSINGを意図的に確認した。ここではpolicy拒否を
    # 排除して、長文UTF-8 payloadの復号とJSON解析だけを検査する。
    Write-FakeRuntime `
        -ProcessId $activeWatcher.Id `
        -ProcessStartUtc $activeWatcher.StartTime.ToUniversalTime()
    $utf8Prompt = New-LongJapaneseDelegationPrompt
    $utf8Payload = ([ordered]@{
        session_id = "session-japanese-utf8-regression"
        tool_name = "mcp__codex__codex-reply"
        tool_input = [ordered]@{
            threadId = "thread-japanese-utf8-regression"
            prompt = $utf8Prompt
        }
    } | ConvertTo-Json -Compress -Depth 5)
    $utf8PayloadByteCount = $script:Utf8NoBom.GetByteCount($utf8Payload)
    Write-Output ("UTF8_REGRESSION_PAYLOAD_BYTES={0}" -f $utf8PayloadByteCount)
    Assert-True ($utf8PayloadByteCount -gt 5033) "実障害の5033バイト目を越える日本語payloadを使うこと"
    Assert-Contains $utf8Payload '"session_id":"session-japanese-utf8-regression"' "実際のsession_id fieldを含めること"
    Assert-Contains $utf8Payload '"threadId":"thread-japanese-utf8-regression"' "実際のtool_input.threadId fieldを含めること"
    Assert-Contains $utf8Payload "**接続しました。**" "日本語と記号の境目を含めること"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload $utf8Payload
    Assert-Equal $result.ExitCode 0 "日本語payloadのHook protocolを0で返すこと"
    Assert-Equal ($result.Output.Trim()) "" "長い日本語payloadをJSON parse errorで拒否しないこと"

    Write-Output "[14/20] stalled/unmonitorableは全incidentへの宣言がある4 toolだけを通す"
    $staleTime = [DateTime]::UtcNow.AddMinutes(-61)
    foreach ($watchedFile in @($reportPath, $reportPath2, $sourcePath, $sourcePath2)) {
        [IO.File]::SetLastWriteTimeUtc($watchedFile, $staleTime)
    }
    Write-FakeRuntime `
        -ProcessId $activeWatcher.Id `
        -ProcessStartUtc $activeWatcher.StartTime.ToUniversalTime() `
        -AgentStatuses @("stalled", "stalled")
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "停滞中の直接Checkを緑にしないこと"
    Assert-Contains $result.Output "STALL_RESPONSE_REQUIRED" "直接Checkに対応宣言が必要な理由を出すこと"
    $noResponsePayload = New-HookPayload -ToolName "Agent" -Text "新しい担当へ委譲する"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload $noResponsePayload
    Assert-Contains $result.Output '"permissionDecision":"deny"' "宣言なしの新規委譲を拒否すること"
    Assert-Contains $result.Output "STALL_RESPONSE_MISSING" "宣言不足の理由codeを出すこと"
    $validResponseText = Get-ResponseTextForRuntime
    foreach ($toolName in @("Agent", "SendMessage", "mcp__codex__codex", "mcp__codex__codex-reply")) {
        $payload = New-HookPayload -ToolName $toolName -Text $validResponseText
        $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload $payload
        Assert-Equal $result.ExitCode 0 "$toolName のHook protocolを0で返すこと"
        Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "$toolName の実text fieldに全宣言があれば修復委譲を通すこと"
    }
    $invalidContinueText = Get-ResponseTextForRuntime -FirstAction "continue" -FirstNext "次の走査まで待つ"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $invalidContinueText)
    Assert-Contains $result.Output "STALL_CONTINUE_CONDITION" "continueの一般的nextを拒否すること"
    $validContinueText = Get-ResponseTextForRuntime -FirstAction "continue" -FirstNext "progress-when:報告書または対象ソースのlatestWriteUtcが現在値から変わる"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $validContinueText)
    Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "continueへ具体的なprogress-when条件があれば通すこと"
    $emptyContinueText = Get-ResponseTextForRuntime -FirstAction "continue" -FirstNext "progress-when:"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $emptyContinueText)
    Assert-Contains $result.Output "STALL_CONTINUE_CONDITION" "continueの空progress-when条件を拒否すること"
    $blankEvidenceText = $validResponseText.Replace("evidence=監視出力の停滞IDと対象を確認した", "evidence=   ")
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $blankEvidenceText)
    Assert-Contains $result.Output "STALL_RESPONSE_EVIDENCE" "空白だけのevidenceを拒否すること"
    $blankNextText = $validResponseText.Replace("next=担当へ現状確認を送り、次の走査で成果物更新を再測する", "next=   ")
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $blankNextText)
    Assert-Contains $result.Output "STALL_RESPONSE_NEXT" "空白だけのnextを拒否すること"
    foreach ($repairAction in @("investigate", "reassign", "stop-request", "complete-check")) {
        $repairText = Get-ResponseTextForRuntime -FirstAction $repairAction
        $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "SendMessage" -Text $repairText)
        Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "修復action=$repairAction を同じ宣言で締め出さないこと"
    }

    Write-Output "[15/20] 未知・古い・重複・一部不足のincidentを拒否する"
    $runtimeState = [IO.File]::ReadAllText($runtimePath, $script:Utf8NoBom) | ConvertFrom-Json
    $incident1 = [string]$runtimeState.agentStates[0].incidentId
    $incident2 = [string]$runtimeState.agentStates[1].incidentId
    $unknown = "0" * 64
    $unknownText = (New-StallResponseBlock -Incident $unknown) + "`n" + (New-StallResponseBlock -Incident $incident2) + "`n委譲本文"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $unknownText)
    Assert-Contains $result.Output "STALL_RESPONSE_UNKNOWN" "未知または古いincidentを拒否すること"
    $duplicateText = (New-StallResponseBlock -Incident $incident1) + "`n" + (New-StallResponseBlock -Incident $incident1) + "`n" + (New-StallResponseBlock -Incident $incident2)
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $duplicateText)
    Assert-Contains $result.Output "STALL_RESPONSE_DUPLICATE" "同じincidentの重複宣言を拒否すること"
    $partialText = (New-StallResponseBlock -Incident $incident1) + "`n委譲本文"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $partialText)
    Assert-Contains $result.Output "STALL_RESPONSE_INCOMPLETE" "複数停滞の一部だけの宣言を拒否すること"

    Write-Output "[16/20] 引用・code fence・宣言領域外のmarkerを拒否する"
    $quotedText = ($validResponseText -split "`n" | ForEach-Object { "> $_" }) -join "`n"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $quotedText)
    Assert-Contains $result.Output "STALL_RESPONSE_MISSING" "Markdown引用内のmarkerを宣言として扱わないこと"
    $fencedText = ('```text' + "`n" + $validResponseText + "`n" + '```')
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $fencedText)
    Assert-Contains $result.Output "STALL_RESPONSE_MISSING" "code fence内のmarkerを宣言として扱わないこと"
    $outsideMarkerText = $validResponseText + "`n> [AGENT_WATCH_RESPONSE schema=1]"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $outsideMarkerText)
    Assert-Contains $result.Output "STALL_RESPONSE_QUOTED" "先頭の宣言領域外に残るmarkerを拒否すること"

    Write-Output "[17/20] 4 toolの実text field以外へ置いた宣言を読まない"
    $wrongFieldPayloads = @(
        ([ordered]@{ tool_name = "Agent"; tool_input = [ordered]@{ prompt = "本文"; description = $validResponseText } } | ConvertTo-Json -Compress -Depth 6),
        ([ordered]@{ tool_name = "SendMessage"; tool_input = [ordered]@{ message = "本文"; prompt = $validResponseText } } | ConvertTo-Json -Compress -Depth 6),
        ([ordered]@{ tool_name = "mcp__codex__codex"; tool_input = [ordered]@{ prompt = "本文"; message = $validResponseText } } | ConvertTo-Json -Compress -Depth 6),
        ([ordered]@{ tool_name = "mcp__codex__codex-reply"; tool_input = [ordered]@{ threadId = "thread"; prompt = "本文"; message = $validResponseText } } | ConvertTo-Json -Compress -Depth 6)
    )
    foreach ($payload in $wrongFieldPayloads) {
        $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload $payload
        Assert-Contains $result.Output "STALL_RESPONSE_MISSING" "実text field以外の宣言では通さないこと"
    }
    $nonStringPayload = ([ordered]@{ tool_name = "Agent"; tool_input = [ordered]@{ prompt = @($validResponseText) } } | ConvertTo-Json -Compress -Depth 6)
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload $nonStringPayload
    Assert-Contains $result.Output "HOOK_CHECK_ERROR" "実text fieldが文字列でないpayloadをfail-closedにすること"

    Write-Output "[18/20] schema/count/status/hash/output summary不一致をfail-closedにする"
    Write-FakeRuntime
    Update-FakeRuntime -Mutation { param($state) $state.schemaVersion = 1 }
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Contains $result.Output "SCHEMA_MISMATCH" "schema 1を通さないこと"
    Write-FakeRuntime
    Update-FakeRuntime -Mutation { param($state) $state.activeCount = "2" }
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Contains $result.Output "STATE_SCHEMA_ERROR" "文字列へ変えたcountをJSON整数として扱わないこと"
    Write-FakeRuntime
    Update-FakeRuntime -Mutation { param($state) $state.activeCount = 1; $state.stalledCount = 1 }
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Contains $result.Output "AGENT_STATUS_COUNT_MISMATCH" "top countとstate statusの不一致を通さないこと"
    Write-FakeRuntime
    Update-FakeRuntime -Mutation { param($state) $state.agentStatesSha256 = "0" * 64 }
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Contains $result.Output "AGENT_STATES_HASH_MISMATCH" "agentStates hash不一致を通さないこと"
    Write-FakeRuntime
    Update-FakeRuntime -Mutation { param($state) $state.agentStates[0].status = "busy" }
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Contains $result.Output "AGENT_STATUS_INVALID" "未知statusを通さないこと"
    Write-FakeRuntime -AgentStatuses @("stalled", "active")
    Update-FakeRuntime -Mutation { param($state) $state.agentStates[0].incidentId = "f" * 64 }
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Contains $result.Output "INCIDENT_HASH_MISMATCH" "incident hash不一致を通さないこと"
    Write-FakeRuntime -OutputText "fresh output`nAGENT_WATCH_STATUS schema=2 total=2 active=9 stalled=0 unmonitorable=0 states_sha256=$('0' * 64)`n"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Contains $result.Output "AGENT_STATUS_SUMMARY_MISMATCH" "output summaryとruntimeの不一致を通さないこと"

    # runtime/outputだけをall-activeへ自己整合させても、61分前の実fileをhookが
    # read-only再走査してlive incidentを作る。runtimeの偽装値はgateの正本にしない。
    Write-FakeRuntime `
        -ProcessId $activeWatcher.Id `
        -ProcessStartUtc $activeWatcher.StartTime.ToUniversalTime() `
        -AgentStatuses @("active", "active")
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text "宣言なし")
    Assert-Contains $result.Output "STALL_RESPONSE_MISSING" "自己整合したall-active偽装でもlive停滞への宣言なしを拒否すること"
    $currentMatch = [regex]::Match($result.Output, 'current=(?<ids>[0-9a-f]{64}(?:,[0-9a-f]{64})*)')
    Assert-True $currentMatch.Success "deny理由へruntimeでなく現在のlive incident全件を出すこと"
    $liveIncidentIds = @($currentMatch.Groups["ids"].Value -split ',')
    Assert-Equal $liveIncidentIds.Count 2 "all-active偽装下でもlive停滞2件を数えること"
    $liveResponse = New-ResponseTextForIncidents -IncidentIds $liveIncidentIds
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "SendMessage" -Text $liveResponse)
    Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "live incident全件への修復宣言ならruntime lag中も締め出さないこと"

    Write-FakeRuntime `
        -ProcessId $activeWatcher.Id `
        -ProcessStartUtc $activeWatcher.StartTime.ToUniversalTime() `
        -AgentStatuses @("stalled", "stalled")
    $oldRuntimeState = [IO.File]::ReadAllText($runtimePath, $script:Utf8NoBom) | ConvertFrom-Json
    $oldIncidentIds = @($oldRuntimeState.agentStates | ForEach-Object { [string]$_.incidentId })
    $changedStaleTime = $staleTime.AddMinutes(-1)
    foreach ($watchedFile in @($reportPath, $reportPath2, $sourcePath, $sourcePath2)) {
        [IO.File]::SetLastWriteTimeUtc($watchedFile, $changedStaleTime)
    }
    $oldResponse = New-ResponseTextForIncidents -IncidentIds $oldIncidentIds
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $oldResponse)
    Assert-Contains $result.Output "STALL_RESPONSE_UNKNOWN" "runtime旧attention IDをlive IDが変化した後に通さないこと"
    $changedMatch = [regex]::Match($result.Output, 'current=(?<ids>[0-9a-f]{64}(?:,[0-9a-f]{64})*)')
    Assert-True $changedMatch.Success "旧ID拒否理由へ変化後のlive ID全件を表示すること"
    $changedLiveIds = @($changedMatch.Groups["ids"].Value -split ',')
    Assert-True ($changedLiveIds[0] -notin $oldIncidentIds) "mtime変化後のlive incidentが旧IDと異なること"
    $changedResponse = New-ResponseTextForIncidents -IncidentIds $changedLiveIds
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $changedResponse)
    Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "runtime旧IDの次scan前でも新live IDへの宣言なら通すこと"

    $futureTime = [DateTime]::UtcNow.AddMinutes(3)
    foreach ($watchedFile in @($reportPath, $reportPath2, $sourcePath, $sourcePath2)) {
        [IO.File]::SetLastWriteTimeUtc($watchedFile, $futureTime)
    }
    Write-FakeRuntime `
        -ProcessId $activeWatcher.Id `
        -ProcessStartUtc $activeWatcher.StartTime.ToUniversalTime() `
        -AgentStatuses @("active", "active")
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text "宣言なし")
    Assert-Contains $result.Output "STALL_RESPONSE_MISSING" "3分未来mtimeをself-consistent activeへ偽装しても宣言なしを拒否すること"
    $futureMatch = [regex]::Match($result.Output, 'current=(?<ids>[0-9a-f]{64}(?:,[0-9a-f]{64})*)')
    Assert-True $futureMatch.Success "未来mtimeのdenyへlive unmonitorable ID全件を出すこと"
    $futureIncidentIds = @($futureMatch.Groups["ids"].Value -split ',')
    Assert-Equal $futureIncidentIds.Count 2 "未来mtimeの2担当をlive unmonitorableとして数えること"
    $futureResponse = New-ResponseTextForIncidents -IncidentIds $futureIncidentIds
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "SendMessage" -Text $futureResponse)
    Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "未来mtimeのlive incident全件へ調査宣言すれば修復委譲を通すこと"

    Write-Output "[19/20] production watcherの61分stale fixtureと実際の4 payload形を別processで検査する"
    if (-not $activeWatcher.HasExited) {
        $activeWatcher.Kill()
        [void]$activeWatcher.WaitForExit(10000)
    }
    Release-TestLock
    foreach ($runtimeFile in @($runtimePath, $outputPath, $lockPath)) {
        if (Test-Path -LiteralPath $runtimeFile -PathType Leaf) {
            Remove-Item -LiteralPath $runtimeFile -Force
        }
    }
    $staleTime = [DateTime]::UtcNow.AddMinutes(-61)
    foreach ($watchedFile in @($reportPath, $reportPath2, $sourcePath, $sourcePath2)) {
        [IO.File]::SetLastWriteTimeUtc($watchedFile, $staleTime)
    }
    $staleWatcher = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline -and -not (Test-Path -LiteralPath $runtimePath -PathType Leaf)) {
        if ($staleWatcher.HasExited) { break }
        Start-Sleep -Milliseconds 100
    }
    Assert-True (-not $staleWatcher.HasExited) "61分fixtureのproduction watcherが継続稼働すること"
    Assert-True (Test-Path -LiteralPath $runtimePath -PathType Leaf) "61分fixtureがruntime stateを発行すること"
    $productionState = [IO.File]::ReadAllText($runtimePath, $script:Utf8NoBom) | ConvertFrom-Json
    Assert-Equal ([int]$productionState.schemaVersion) 2 "production watcherがschema 2を発行すること"
    Assert-Equal ([int]$productionState.stalledCount) 2 "61分無変化の2担当をstalledにすること"
    Assert-Equal ([int]$productionState.unmonitorableCount) 0 "実在fixtureを監視不能にしないこと"
    $productionResponse = Get-ResponseTextForRuntime
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text "宣言なし")
    Assert-Contains $result.Output "STALL_RESPONSE_MISSING" "production stateでも宣言なしを拒否すること"
    foreach ($toolName in @("Agent", "SendMessage", "mcp__codex__codex", "mcp__codex__codex-reply")) {
        $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName $toolName -Text $productionResponse)
        Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "production stateと$toolName の実fieldで全宣言を通すこと"
    }
    $productionIncidentBefore = [string]$productionState.agentStates[0].incidentId
    Assert-True ([regex]::IsMatch($productionIncidentBefore, '^[0-9a-f]{64}$')) "production incidentIdをlowercase SHA-256にすること"
    $staleWatcher.Kill()
    [void]$staleWatcher.WaitForExit(10000)

    Write-Output "[20/20] activeへ戻った後は古い宣言を拒否し、宣言なしの通常委譲を通す"
    foreach ($runtimeFile in @($runtimePath, $outputPath, $lockPath)) {
        if (Test-Path -LiteralPath $runtimeFile -PathType Leaf) {
            Remove-Item -LiteralPath $runtimeFile -Force
        }
    }
    $activeAgainTime = [DateTime]::UtcNow
    foreach ($watchedFile in @($reportPath, $reportPath2, $sourcePath, $sourcePath2)) {
        [IO.File]::SetLastWriteTimeUtc($watchedFile, $activeAgainTime)
    }
    $finalWatcher = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline -and -not (Test-Path -LiteralPath $runtimePath -PathType Leaf)) {
        if ($finalWatcher.HasExited) { break }
        Start-Sleep -Milliseconds 100
    }
    Assert-True (-not $finalWatcher.HasExited) "active復帰fixtureの実watcherが継続稼働すること"
    Assert-True (Test-Path -LiteralPath $runtimePath -PathType Leaf) "active復帰fixtureがruntimeを発行すること"
    $oldResponse = (New-StallResponseBlock -Incident $productionIncidentBefore) + "`n委譲本文"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text $oldResponse)
    Assert-Contains $result.Output "STALL_RESPONSE_UNEXPECTED" "active stateで古いincidentを再利用できないこと"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text "通常の委譲本文")
    Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "全担当activeなら従来どおり宣言なしで通すこと"
    [IO.File]::SetLastWriteTimeUtc($sourcePath, [DateTime]::UtcNow.AddSeconds(1))
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload (New-HookPayload -ToolName "Agent" -Text "進捗後の通常委譲本文")
    Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "runtime旧activeからlive新activeへの正当な進捗を次scanまで締め出さないこと"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "direct Checkはactive同士のruntime lagも静かに緑へしないこと"
    Assert-Contains $result.Output "WATCH_STATE_LAG" "direct Checkへruntime追随待ちを表示すること"

    # 実設定はPreToolUse timeout=5秒。cold Add-Typeだけの小fixtureでなく、実repoの
    # scripts/docs配下を3担当が再帰走査するproduction相当入力を別processで測る。
    if (-not $finalWatcher.HasExited) {
        $finalWatcher.Kill()
        [void]$finalWatcher.WaitForExit(10000)
    }
    foreach ($runtimeFile in @($runtimePath, $outputPath, $lockPath)) {
        if (Test-Path -LiteralPath $runtimeFile -PathType Leaf) {
            Remove-Item -LiteralPath $runtimeFile -Force
        }
    }
    $performanceReports = @(
        (Join-Path $repositoryRoot "scratchpad\performance-report-1.md"),
        (Join-Path $repositoryRoot "scratchpad\performance-report-2.md"),
        (Join-Path $repositoryRoot "scratchpad\performance-report-3.md")
    )
    foreach ($performanceReport in $performanceReports) {
        [IO.File]::WriteAllText($performanceReport, "performance fixture", $script:Utf8NoBom)
    }
    $performanceDefinition = [ordered]@{
        agents = @(
            [ordered]@{
                name = "production-scale-scripts"
                reportPath = $performanceReports[0]
                sourcePaths = @((Join-Path $sourceRepositoryRoot "scripts"))
            },
            [ordered]@{
                name = "production-scale-rules"
                reportPath = $performanceReports[1]
                sourcePaths = @((Join-Path $sourceRepositoryRoot "docs\rules"))
            },
            [ordered]@{
                name = "production-scale-traceability"
                reportPath = $performanceReports[2]
                sourcePaths = @((Join-Path $sourceRepositoryRoot "docs\traceability"))
            }
        )
    }
    [IO.File]::WriteAllText($definitionPath, ($performanceDefinition | ConvertTo-Json -Depth 8), $script:Utf8NoBom)
    $performanceWatcher = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline -and -not (Test-Path -LiteralPath $runtimePath -PathType Leaf)) {
        if ($performanceWatcher.HasExited) { break }
        Start-Sleep -Milliseconds 100
    }
    Assert-True (-not $performanceWatcher.HasExited) "production相当3担当の実watcherが継続稼働すること"
    $performanceRuntime = [IO.File]::ReadAllText($runtimePath, $script:Utf8NoBom) | ConvertFrom-Json
    Assert-Equal ([int]$performanceRuntime.agentCount) 3 "production相当入力を3担当で発行すること"
    $performanceStopwatch = [Diagnostics.Stopwatch]::StartNew()
    $result = Invoke-Checker `
        -PowerShellPath $powerShellCommand.Source `
        -Action "Hook" `
        -Payload (New-HookPayload -ToolName "Agent" -Text "production相当入力の通常委譲")
    $performanceStopwatch.Stop()
    Write-Output ("PRODUCTION_SCALE_COLD_HOOK_ELAPSED_MS={0}" -f $performanceStopwatch.ElapsedMilliseconds)
    Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "production相当3担当のcold Hookを通すこと"
    Assert-True ($performanceStopwatch.Elapsed.TotalSeconds -lt 5.0) "production相当3担当のcold Hookを実設定timeout 5秒未満で終えること"

    Write-Output ("check-agent-watch self-test passed: 20 cases, {0} assertions" -f $script:AssertionCount)
}
finally {
    Remove-TestSandbox
}
