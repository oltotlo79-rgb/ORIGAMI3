[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$sourceWatcherPath = Join-Path (Split-Path -Parent $PSScriptRoot) "watch-agents.ps1"
$sourceCheckerPath = Join-Path $PSScriptRoot "check-agent-watch.ps1"
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-check-agent-watch-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [IO.Path]::GetFullPath((Join-Path $tempBase $sandboxName))
$repositoryRoot = Join-Path $sandboxRoot "repo"
$watcherPath = Join-Path $repositoryRoot "scripts\watch-agents.ps1"
$checkerPath = Join-Path $repositoryRoot "scripts\hooks\check-agent-watch.ps1"
$definitionPath = Join-Path $repositoryRoot "scratchpad\agents.json"
$reportPath = Join-Path $repositoryRoot "scratchpad\agent-report.md"
$sourcePath = Join-Path $repositoryRoot "src\value.rs"
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
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
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
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)
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
        [string]$OutputText = "fresh watcher output`n",
        [string]$StoredOutputHash = "",
        [string]$StoredOutputPath = ""
    )

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
        schemaVersion = 1
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
        agentCount = 1
        scanSequence = 1
        scanCompletedUtc = $ScanUtc.ToString("o")
        stateWrittenUtc = [DateTime]::UtcNow.ToString("o")
    }
    [IO.File]::WriteAllText($runtimePath, ($state | ConvertTo-Json -Depth 6), $script:Utf8NoBom)
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
[IO.File]::WriteAllText($sourcePath, "source", $script:Utf8NoBom)
[IO.File]::WriteAllText(
    $definitionPath,
    (([ordered]@{
        agents = @(
            [ordered]@{
                name = "test-agent"
                reportPath = "scratchpad/agent-report.md"
                sourcePaths = @("src/value.rs")
            }
        )
    }) | ConvertTo-Json -Depth 8),
    $script:Utf8NoBom
)

try {
    Write-Output "[1/13] runtime stateが無ければpolicy NG(1)"
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "stateなしをpolicy NGにすること"
    Assert-Contains $result.Output "STATE_MISSING" "stateなしの理由codeを出すこと"

    Write-Output "[2/13] -Onceは成功しても有効watcherを作らない"
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

    Write-Output "[3/13] 実watcherのfresh stateは正常(0)"
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

    Write-Output "[4/13] watcher終了後はPID不在でpolicy NG(1)"
    $watcher.Kill()
    [void]$watcher.WaitForExit(10000)
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "終了済みwatcherを通さないこと"
    Assert-Contains $result.Output "PROCESS_MISSING" "PID不在の理由codeを出すこと"

    Write-Output "[5/13] -OnceとPID/start不一致をpolicy NG(1)"
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

    Write-Output "[6/13] singleton lockなしをpolicy NG(1)"
    Write-FakeRuntime
    Release-TestLock
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "lockなしを通さないこと"
    Assert-Contains $result.Output "LOCK_NOT_HELD" "lockなしの理由codeを出すこと"

    Write-Output "[7/13] 12分を超えたoutput/scanをpolicy NG(1)"
    Hold-TestLock
    $old = [DateTime]::UtcNow.AddMinutes(-13)
    Write-FakeRuntime -ScanUtc $old -OutputUtc $old
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "stale outputを通さないこと"
    Assert-Contains $result.Output "STALE" "staleの理由codeを出すこと"

    Write-Output "[8/13] freshなstate/output/hash/lockを正常(0)"
    Write-FakeRuntime
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 0 "freshな完全stateを通すこと"
    Assert-Contains $result.Output "interval=10" "正常時に10分契約を表示すること"

    Write-Output "[9/13] output hash/path不一致をpolicy NG(1)"
    Write-FakeRuntime -StoredOutputHash ("0" * 64)
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "output hash不一致を通さないこと"
    Assert-Contains $result.Output "OUTPUT_HASH_MISMATCH" "hash不一致の理由codeを出すこと"
    Write-FakeRuntime -StoredOutputPath (Join-Path $repositoryRoot "scratchpad\other.log")
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "固定output path不一致を通さないこと"
    Assert-Contains $result.Output "PATH_MISMATCH" "path不一致の理由codeを出すこと"

    Write-Output "[10/13] 壊れたstateは検査不能(2)"
    [IO.File]::WriteAllText($runtimePath, "{broken", $script:Utf8NoBom)
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 2 "壊れたJSONを検査不能にすること"
    Assert-Contains $result.Output "STATE_READ_ERROR" "検査不能の理由codeを出すこと"

    Write-Output "[11/13] Hook modeはmainの委譲toolをfail-closedにする"
    Write-FakeRuntime
    $agentPayload = ([ordered]@{ tool_name = "Agent"; tool_input = [ordered]@{ prompt = "test" } } | ConvertTo-Json -Compress)
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload $agentPayload
    Assert-Equal $result.ExitCode 0 "Hook protocol自体は0で返すこと"
    Assert-True (-not $result.Output.Contains('"permissionDecision":"deny"')) "fresh watcherなら委譲を拒否しないこと"
    Remove-Item -LiteralPath $runtimePath -Force
    $result = Invoke-Checker -PowerShellPath $powerShellCommand.Source -Action "Hook" -Payload $agentPayload
    Assert-Equal $result.ExitCode 0 "拒否もHook protocolでは0で返すこと"
    Assert-Contains $result.Output '"permissionDecision":"deny"' "stateなしなら委譲を拒否すること"
    Assert-Contains $result.Output "AGENT_WATCH_POLICY_NG" "拒否理由へpolicy区分を出すこと"

    Write-Output "[12/13] agent_idは非空文字列だけsubagentとして検査を省略する"
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

    Write-Output "[13/13] 実際の委譲payload形の長い日本語promptをraw UTF-8で壊さずJSON parseする"
    # 直前のmain扱いケースはSTATE_MISSINGを意図的に確認した。ここではpolicy拒否を
    # 排除して、長文UTF-8 payloadの復号とJSON解析だけを検査する。
    Hold-TestLock
    Write-FakeRuntime
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

    Write-Output ("check-agent-watch self-test passed: 13 cases, {0} assertions" -f $script:AssertionCount)
}
finally {
    Remove-TestSandbox
}
