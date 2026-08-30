[CmdletBinding()]
param([switch]$SkipSettingsConnectionCheck)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$RequireScriptPath = Join-Path $PSScriptRoot "require-report-log.ps1"
$HealthScriptPath = Join-Path $PSScriptRoot "checks\hook-health.ps1"
$SourceRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$SettingsPath = Join-Path $SourceRoot ".claude\settings.json"
$PowerShellPath = (Get-Process -Id $PID).Path
$TempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\\/")
$Sandbox = Join-Path $TempBase ("ori3-require-report-log-test-{0}" -f [Guid]::NewGuid().ToString("N"))
$Repository = Join-Path $Sandbox "repo"
$script:AssertionCount = 0

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)][AllowNull()]$Actual,
        [Parameter(Mandatory = $true)][AllowNull()]$Expected,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$Output = ""
    )

    $script:AssertionCount += 1
    if ($Actual -ne $Expected) {
        throw "ASSERTION FAILED: $Message (expected=$Expected, actual=$Actual)`n$Output"
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
        $escaped = [regex]::Replace($value, '(\\*)"', '$1$1\\"')
        $trailingBackslashes = [regex]::Match($escaped, '\\*$').Value
        $escaped = $escaped + $trailingBackslashes
        '"' + $escaped + '"'
    }
    return ($parts -join " ")
}

function Invoke-ChildPowerShell {
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [hashtable]$EnvironmentVariables = @{},
        [AllowNull()][string]$StandardInput = $null
    )

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $PowerShellPath
    $startInfo.Arguments = ConvertTo-ProcessArgumentString -Values $Arguments
    $startInfo.WorkingDirectory = $Repository
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.RedirectStandardInput = $null -ne $StandardInput
    $startInfo.StandardOutputEncoding = [Text.Encoding]::UTF8
    $startInfo.StandardErrorEncoding = [Text.Encoding]::UTF8
    foreach ($key in $EnvironmentVariables.Keys) {
        $startInfo.EnvironmentVariables[[string]$key] = [string]$EnvironmentVariables[$key]
    }
    $process = [Diagnostics.Process]::Start($startInfo)
    if ($null -ne $StandardInput) {
        $inputBytes = (New-Object Text.UTF8Encoding($false)).GetBytes($StandardInput)
        if ($inputBytes.Length -gt 0) {
            $process.StandardInput.BaseStream.Write($inputBytes, 0, $inputBytes.Length)
        }
        $process.StandardInput.BaseStream.Close()
    }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    return [PSCustomObject]@{ ExitCode = $process.ExitCode; Output = $stdout + $stderr }
}

function Invoke-Health {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)

    return Invoke-ChildPowerShell -Arguments (@(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", (Join-Path $Repository "scripts\hooks\checks\hook-health.ps1")
    ) + $Arguments)
}

function Invoke-RequireReport {
    param([string]$Payload = '{"tool_name":"SendMessage","tool_input":{"message":"follow-up"}}')

    return Invoke-ChildPowerShell -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", (Join-Path $Repository "scripts\hooks\require-report-log.ps1")
    ) -EnvironmentVariables @{ CLAUDE_PROJECT_DIR = $Repository } -StandardInput $Payload
}

function Get-HealthStatePaths {
    $normalized = [IO.Path]::GetFullPath($Repository).Replace("\", "/").TrimEnd("/").ToLowerInvariant()
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $key = -join ($sha256.ComputeHash([Text.Encoding]::UTF8.GetBytes($normalized)) | ForEach-Object { $_.ToString("x2") })
    }
    finally {
        $sha256.Dispose()
    }
    $statePath = Join-Path (Join-Path ([IO.Path]::GetTempPath()) "ori3-hook-health") ("{0}-test-claim-scope-warning.json" -f $key)
    return [PSCustomObject]@{ State = $statePath; Block = ($statePath + ".block") }
}

function Remove-TestArtifacts {
    $healthPaths = Get-HealthStatePaths
    foreach ($path in @($healthPaths.State, $healthPaths.Block)) {
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            Remove-Item -LiteralPath $path -Force
        }
    }
    if (-not (Test-Path -LiteralPath $Sandbox)) { return }
    $fullSandbox = [IO.Path]::GetFullPath($Sandbox).TrimEnd([char[]]"\\/")
    if ([IO.Path]::GetDirectoryName($fullSandbox) -ne $TempBase -or [IO.Path]::GetFileName($fullSandbox) -notmatch '^ori3-require-report-log-test-[0-9a-f]{32}$') {
        throw "Refusing unsafe self-test cleanup: $fullSandbox"
    }
    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

[void][IO.Directory]::CreateDirectory((Join-Path $Repository "scripts\hooks\checks"))
[void][IO.Directory]::CreateDirectory((Join-Path $Repository "docs"))
try {
    [IO.File]::Copy($RequireScriptPath, (Join-Path $Repository "scripts\hooks\require-report-log.ps1"), $true)
    [IO.File]::Copy($HealthScriptPath, (Join-Path $Repository "scripts\hooks\checks\hook-health.ps1"), $true)
    $reportName = ([string][char]0x5831) + ([string][char]0x544A) + ([string][char]0x8A18) + ([string][char]0x9332) + ".md"
    [IO.File]::WriteAllText((Join-Path $Repository (Join-Path "docs" $reportName)), "# current`n", [Text.UTF8Encoding]::new($false))

    Write-Host "[1/10] missing health state allows a follow-up instruction"
    $missing = Invoke-RequireReport
    Assert-Equal $missing.ExitCode 0 "PreToolUse hook must complete" $missing.Output
    Assert-Equal $missing.Output.Trim() "" "missing health state must not deny the first instruction" $missing.Output

    Write-Host "[2/10] two failed scans deny the next instruction with actionable state"
    $first = Invoke-Health @("-Action", "RecordFailure", "-RepositoryRoot", $Repository, "-FailureExitCode", "9", "-FailureKind", "scanner-exit")
    Assert-Equal $first.ExitCode 0 "first health failure recording must complete" $first.Output
    $second = Invoke-Health @("-Action", "RecordFailure", "-RepositoryRoot", $Repository, "-FailureExitCode", "127", "-FailureKind", "powershell-unavailable")
    Assert-Equal $second.ExitCode 0 "second health failure recording must complete" $second.Output
    $denied = Invoke-RequireReport
    Assert-Equal $denied.ExitCode 0 "PreToolUse denial must be a successful hook response" $denied.Output
    $denial = $denied.Output | ConvertFrom-Json
    Assert-Equal $denial.hookSpecificOutput.permissionDecision "deny" "two failures must deny the next instruction" $denied.Output
    Assert-Contains $denial.hookSpecificOutput.permissionDecisionReason "check=test-claim-scope-warning" "denial must name the failed check"
    Assert-Contains $denial.hookSpecificOutput.permissionDecisionReason "failures=2" "denial must show the consecutive failure count"
    Assert-Contains $denial.hookSpecificOutput.permissionDecisionReason "lastSuccess=never" "denial must show the last successful scan"
    $healthPaths = Get-HealthStatePaths
    Assert-Contains $denial.hookSpecificOutput.permissionDecisionReason $healthPaths.Block "denial must give the exact acknowledgement file to delete"
    Assert-Contains $denial.hookSpecificOutput.permissionDecisionReason ("instruction=delete-file-at-" + $healthPaths.Block) "denial must state the exact deletion instruction"

    Write-Host "[3/10] an explicit acknowledgement release permits delegation but remains visible"
    Remove-Item -LiteralPath $healthPaths.Block -Force
    $released = Invoke-RequireReport
    Assert-Equal $released.ExitCode 0 "released PreToolUse hook must complete" $released.Output
    Assert-Contains $released.Output "HOOK_HEALTH_RELEASED" "released instruction path must display the release state"
    Assert-Equal ($released.Output.Contains('"permissionDecision":"deny"')) $false "explicit release must allow delegation" $released.Output

    $third = Invoke-Health @("-Action", "RecordFailure", "-RepositoryRoot", $Repository, "-FailureExitCode", "9", "-FailureKind", "scanner-exit")
    Assert-Equal $third.ExitCode 0 "a post-release failure must be recordable" $third.Output
    $deniedAgain = Invoke-RequireReport
    Assert-Equal $deniedAgain.ExitCode 0 "reblocked PreToolUse hook must complete" $deniedAgain.Output
    $denialAgain = $deniedAgain.Output | ConvertFrom-Json
    Assert-Equal $denialAgain.hookSpecificOutput.permissionDecision "deny" "a new failed scan after release must deny delegation again" $deniedAgain.Output

    Write-Host "[4/10] one successful scan restores the instruction path and clears release disclosure"
    $success = Invoke-Health @("-Action", "RecordSuccess", "-RepositoryRoot", $Repository)
    Assert-Equal $success.ExitCode 0 "health success recording must complete" $success.Output
    $restored = Invoke-RequireReport
    Assert-Equal $restored.ExitCode 0 "restored PreToolUse hook must complete" $restored.Output
    Assert-Equal $restored.Output.Trim() "" "one successful scan must restore the instruction path" $restored.Output

    Write-Host "[5/10] only a nonempty string agent_id bypasses the coordinator-only guard"
    $first = Invoke-Health @("-Action", "RecordFailure", "-RepositoryRoot", $Repository, "-FailureExitCode", "9", "-FailureKind", "scanner-exit")
    $second = Invoke-Health @("-Action", "RecordFailure", "-RepositoryRoot", $Repository, "-FailureExitCode", "9", "-FailureKind", "scanner-exit")
    $subagentPayload = @{ tool_name = "Agent"; agent_id = "agent-test-1"; tool_input = @{ prompt = "intentionally incomplete"; model = "opus" } } | ConvertTo-Json -Compress -Depth 5
    $subagent = Invoke-RequireReport -Payload $subagentPayload
    Assert-Equal $subagent.ExitCode 0 "subagent hook invocation must complete" $subagent.Output
    Assert-Equal $subagent.Output.Trim() "" "nonempty agent_id must bypass the coordinator guard" $subagent.Output
    $nonStringAgentPayload = @{ tool_name = "Agent"; agent_id = @{ forged = $true }; tool_input = @{ prompt = "intentionally incomplete"; model = "opus" } } | ConvertTo-Json -Compress -Depth 5
    $nonStringAgent = Invoke-RequireReport -Payload $nonStringAgentPayload
    Assert-Equal $nonStringAgent.ExitCode 0 "non-string agent_id denial must use the hook protocol" $nonStringAgent.Output
    $nonStringAgentJson = $nonStringAgent.Output | ConvertFrom-Json
    Assert-Equal $nonStringAgentJson.hookSpecificOutput.permissionDecision "deny" "non-string agent_id must not bypass the coordinator guard" $nonStringAgent.Output
    $success = Invoke-Health @("-Action", "RecordSuccess", "-RepositoryRoot", $Repository)

    Write-Host "[6/10] initial delegation without a prompt is denied before launch"
    $missingPrompt = Invoke-RequireReport -Payload '{"tool_name":"Agent","tool_input":{"model":"opus"}}'
    Assert-Equal $missingPrompt.ExitCode 0 "missing prompt denial must use the hook protocol" $missingPrompt.Output
    $missingPromptJson = $missingPrompt.Output | ConvertFrom-Json
    Assert-Equal $missingPromptJson.hookSpecificOutput.permissionDecision "deny" "missing initial prompt must be denied" $missingPrompt.Output
    Assert-Contains $missingPromptJson.hookSpecificOutput.permissionDecisionReason "AGENT_INSTRUCTION_MISSING" "denial must identify the missing instruction"

    Write-Host "[7/10] initial delegation invokes check-agent-instruction through stdin and propagates NG"
    $stubChecker = @'
[CmdletBinding()]
param([switch]$ReadFromStdin, [string]$RepositoryRoot, [switch]$VerifyLiveBaseline, [string]$ExpectedModel)
$stream = [Console]::OpenStandardInput()
$reader = New-Object IO.StreamReader($stream, (New-Object Text.UTF8Encoding($false, $true)), $false)
try { $text = $reader.ReadToEnd() } finally { $reader.Dispose() }
$observedOne = "統括です、E*接続しました"
$observedTwo = "統括です、E*接続しました。墁E��の判定�E正しく動いてぁE��す"
$utf8Exact = "統括です。**接続しました。境界の判定は正しく動いています。回帰標本: $observedOne / $observedTwo"
if ($ReadFromStdin -and $VerifyLiveBaseline -and $ExpectedModel -eq "opus" -and ($text.Contains("HOOK_TEST_PASS") -or $text -eq $utf8Exact)) {
    Write-Output "[OK] STUB_INSTRUCTION"
    exit 0
}
Write-Output "[NG] STUB_INSTRUCTION stdin=$ReadFromStdin live=$VerifyLiveBaseline model=$ExpectedModel text=$text"
exit 1
'@
    [IO.File]::WriteAllText((Join-Path $Repository "scripts\check-agent-instruction.ps1"), $stubChecker, [Text.UTF8Encoding]::new($true))
    $badInstructionPayload = @{ tool_name = "Agent"; tool_input = @{ prompt = "HOOK_TEST_FAIL"; model = "opus" } } | ConvertTo-Json -Compress -Depth 5
    $badInstruction = Invoke-RequireReport -Payload $badInstructionPayload
    $badInstructionJson = $badInstruction.Output | ConvertFrom-Json
    Assert-Equal $badInstructionJson.hookSpecificOutput.permissionDecision "deny" "checker exit 1 must deny initial delegation" $badInstruction.Output
    Assert-Contains $badInstructionJson.hookSpecificOutput.permissionDecisionReason "STUB_INSTRUCTION" "checker output must reach the denial reason"

    Write-Host "[8/10] a passing stdin checker result permits initial delegation"
    $goodInstructionPayload = @{ tool_name = "Agent"; tool_input = @{ prompt = "HOOK_TEST_PASS"; model = "opus" } } | ConvertTo-Json -Compress -Depth 5
    $goodInstruction = Invoke-RequireReport -Payload $goodInstructionPayload
    Assert-Equal $goodInstruction.ExitCode 0 "passing checker hook invocation must complete" $goodInstruction.Output
    Assert-Equal $goodInstruction.Output.Trim() "" "passing checker must permit the initial delegation" $goodInstruction.Output

    Write-Host "[9/10] raw UTF-8 Japanese prompt survives both hook input and checker forwarding"
    $observedCorruptionOne = "統括です、E*接続しました"
    $observedCorruptionTwo = "統括です、E*接続しました。墁E��の判定�E正しく動いてぁE��す"
    $utf8Prompt = "統括です。**接続しました。境界の判定は正しく動いています。回帰標本: $observedCorruptionOne / $observedCorruptionTwo"
    $utf8Payload = @{ tool_name = "Agent"; tool_input = @{ prompt = $utf8Prompt; model = "opus" } } | ConvertTo-Json -Compress -Depth 5
    Assert-Contains $utf8Payload $observedCorruptionOne "実障害1の文字列をraw UTF-8回帰入力へ含めること"
    Assert-Contains $utf8Payload $observedCorruptionTwo "実障害2の文字列をraw UTF-8回帰入力へ含めること"
    $utf8Result = Invoke-RequireReport -Payload $utf8Payload
    Assert-Equal $utf8Result.ExitCode 0 "日本語promptのhook invocationを完了すること" $utf8Result.Output
    Assert-Equal $utf8Result.Output.Trim() "" "日本語promptをbyte単位でcheckerまで保つこと" $utf8Result.Output

    Write-Host "[10/11] actual codex reply payload parses a long raw UTF-8 Japanese prompt"
    $longUtf8Prompt = New-LongJapaneseDelegationPrompt
    $longUtf8Payload = [ordered]@{
        session_id = "session-japanese-utf8-regression"
        tool_name = "mcp__codex__codex-reply"
        tool_input = [ordered]@{
            threadId = "thread-japanese-utf8-regression"
            prompt = $longUtf8Prompt
        }
    } | ConvertTo-Json -Compress -Depth 5
    $longUtf8PayloadByteCount = ([Text.UTF8Encoding]::new($false)).GetByteCount($longUtf8Payload)
    Write-Host ("UTF8_REGRESSION_PAYLOAD_BYTES={0}" -f $longUtf8PayloadByteCount)
    Assert-Equal ($longUtf8PayloadByteCount -gt 5033) $true "実障害の5033バイト目を越える日本語payloadを使うこと"
    Assert-Contains $longUtf8Payload '"session_id":"session-japanese-utf8-regression"' "実際のsession_id fieldを含めること"
    Assert-Contains $longUtf8Payload '"threadId":"thread-japanese-utf8-regression"' "実際のtool_input.threadId fieldを含めること"
    Assert-Contains $longUtf8Payload "**接続しました。**" "日本語と記号の境目を含めること"
    $longUtf8Result = Invoke-RequireReport -Payload $longUtf8Payload
    Assert-Equal $longUtf8Result.ExitCode 0 "長い日本語promptのhook invocationを完了すること" $longUtf8Result.Output
    Assert-Equal $longUtf8Result.Output.Trim() "" "長い日本語payloadをJSON parse errorで拒否しないこと" $longUtf8Result.Output

    Write-Host "[11/11] actual settings route the delegation matcher to this hook and malformed JSON fails closed"
    $malformed = Invoke-RequireReport -Payload '{broken-json'
    $malformedJson = $malformed.Output | ConvertFrom-Json
    Assert-Equal $malformedJson.hookSpecificOutput.permissionDecision "deny" "malformed hook JSON must fail closed" $malformed.Output
    Assert-Contains $malformedJson.hookSpecificOutput.permissionDecisionReason "DELEGATION_GUARD_ERROR" "malformed payload denial must name the guard error"
    if (-not $SkipSettingsConnectionCheck) {
        $settings = [IO.File]::ReadAllText($SettingsPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
        $routes = @($settings.hooks.PreToolUse | Where-Object { [string]$_.matcher -match 'Agent' })
        Assert-Equal ($routes.Count -ge 1) $true "actual settings must have an Agent PreToolUse route"
        $connectedCommands = @($routes | ForEach-Object { @($_.hooks) } | ForEach-Object { (@($_.args) -join ' ') })
        Assert-Equal (@($connectedCommands | Where-Object { $_ -match 'scripts[/\\]hooks[/\\]require-report-log\.ps1' }).Count -ge 1) $true "actual Agent route must invoke require-report-log.ps1"
    }
    else {
        Write-Warning "actual settings connection check skipped only for the coordinator's recorded UTF-8 repair disconnect"
    }

    Write-Host "[EVIDENCE] missing=$($missing.ExitCode); denied=$($denied.ExitCode); released=$($released.ExitCode); reblocked=$($deniedAgain.ExitCode); restored=$($restored.ExitCode)"
    Write-Host "require-report-log self-test passed: $script:AssertionCount assertions"
}
finally {
    Remove-TestArtifacts
}
