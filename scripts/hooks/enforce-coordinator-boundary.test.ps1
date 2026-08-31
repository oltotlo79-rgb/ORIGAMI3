[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)

$HookPath = Join-Path $PSScriptRoot "enforce-coordinator-boundary.ps1"
$PowerShellPath = (Get-Process -Id $PID).Path
$TempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$Sandbox = Join-Path $TempBase ("ori3-coordinator-boundary-test-{0}" -f [Guid]::NewGuid().ToString("N"))
$Repository = Join-Path $Sandbox "repo"
$Repository2 = Join-Path $Sandbox "repo-two"
$GitRepository = Join-Path $Sandbox "git-integration"
$BareOrigin = Join-Path $Sandbox "origin.git"
$StateRoot = Join-Path $Sandbox "state"
$script:Assertions = 0
$script:Cases = 0
$script:OriginalInputEncoding = [Console]::InputEncoding
$script:Utf8NoBom = New-Object Text.UTF8Encoding($false)

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$Output = ""
    )

    $script:Assertions++
    if (-not $Condition) {
        throw "ASSERTION FAILED: $Message`n$Output"
    }
}

function Assert-Equal {
    param(
        [AllowNull()]$Actual,
        [AllowNull()]$Expected,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$Output = ""
    )

    $script:Assertions++
    if ($Actual -ne $Expected) {
        throw "ASSERTION FAILED: $Message (expected=$Expected actual=$Actual)`n$Output"
    }
}

function Assert-Contains {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $script:Assertions++
    if (-not $Text.Contains($Expected)) {
        throw "ASSERTION FAILED: $Message (missing='$Expected')`n$Text"
    }
}

function New-LongJapaneseCommitMessage {
    $sections = @(
        "統括です。**接続しました。** 日本語を含むコミットメッセージが統括境界フックの標準入力で壊れず、PowerShellのAST解析まで到達することを確認します。",
        "この検査は短い一文字では再現しない入力経路を対象にします。漢字、ひらがな、カタカナ、句読点と ** の記号が混在しても、UTF-8の生バイトを別の符号化として解釈してJSONの引用符を壊してはいけません。",
        "統括は品質ゲート、git操作、成果物確認、利用者への報告だけを直接実行します。担当は実装、調査、個別検査、資料作成を受け持ち、gitへの書き込み、ブラウザの窓、desktop.exe、配信サーバーの起動を行いません。",
        "署名鍵を修復するときは scripts\\check-receipt.ps1 -RepairSigningKey -RepoRoot 当リポジトリの組合せだけを使います。Windows DPAPI鍵は統括利用者の識別子で作る必要があり、別の利用者や別のrepositoryで作ると不具合を再生産します。",
        "コミットの前には品質ゲートを省略しません。--no-verify と -n は検査を飛ばすため拒否し、必要な例外は受領を通して理由、時刻、command hash、実行結果を記録します。",
        "検査結果は新しいプロセスの終了コードとともに報告します。入力が5033バイト目を越えても、境界フックがJSON解析失敗で統括を止めず、許可された日本語コミットメッセージを通すことを確認します。"
    )
    return (($sections -join "`n`n") + "`n`n" + ($sections -join "`n`n") + "`n`n" + ($sections -join "`n`n"))
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

function Invoke-HookProcess {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$RawInput,
        [string]$RepositoryRoot = $Repository,
        [switch]$UseInheritedUnicodeWriter
    )

    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = $PowerShellPath
    $startInfo.Arguments = ConvertTo-ProcessArgumentString @(
        "-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $HookPath, "-StateRoot", $StateRoot
    )
    $startInfo.WorkingDirectory = $RepositoryRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = New-Object Text.UTF8Encoding($false)
    $startInfo.StandardErrorEncoding = New-Object Text.UTF8Encoding($false)
    $startInfo.EnvironmentVariables["CLAUDE_PROJECT_DIR"] = $RepositoryRoot

    $process = New-Object Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "hook child process could not be started" }
    if ($UseInheritedUnicodeWriter) {
        $savedEncoding = [Console]::InputEncoding
        try {
            [Console]::InputEncoding = [Text.Encoding]::Unicode
            $process.StandardInput.Write($RawInput)
            $process.StandardInput.Close()
        }
        finally {
            [Console]::InputEncoding = $savedEncoding
        }
    }
    else {
        $inputBytes = (New-Object Text.UTF8Encoding($false)).GetBytes($RawInput)
        if ($inputBytes.Length -gt 0) {
            $process.StandardInput.BaseStream.Write($inputBytes, 0, $inputBytes.Length)
        }
        $process.StandardInput.BaseStream.Close()
    }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    return [PSCustomObject]@{
        ExitCode = $process.ExitCode
        Stdout = $stdout
        Stderr = $stderr
        Combined = ($stdout + $stderr)
    }
}

function Invoke-GitFixture {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [switch]$CountCase
    )

    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = (Get-Command git.exe -ErrorAction Stop).Source
    $startInfo.Arguments = ConvertTo-ProcessArgumentString $Arguments
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = New-Object Text.UTF8Encoding($false)
    $startInfo.StandardErrorEncoding = New-Object Text.UTF8Encoding($false)
    $startInfo.EnvironmentVariables["GIT_TERMINAL_PROMPT"] = "0"
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "git fixture process could not be started: $Name" }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    if ($CountCase) { $script:Cases++ }
    Assert-Equal $process.ExitCode 0 "$Name actual git exit code" ($stdout + $stderr)
    Write-Host ("ACTUAL-GIT {0}: exit={1}" -f $Name, $process.ExitCode)
    return [PSCustomObject]@{ ExitCode = $process.ExitCode; Stdout = $stdout; Stderr = $stderr }
}

function New-HookPayload {
    param(
        [Parameter(Mandatory = $true)][string]$EventName,
        [Parameter(Mandatory = $true)][string]$ToolName,
        [string]$Command = "",
        [Parameter(Mandatory = $true)][string]$ToolUseId,
        [string]$AgentId,
        [string]$AgentType,
        [string]$ErrorText = "",
        [string]$RepositoryRoot = $Repository
    )

    $payload = [ordered]@{
        session_id = "session-test"
        transcript_path = (Join-Path $Sandbox "transcript.jsonl")
        cwd = $RepositoryRoot
        permission_mode = "default"
        hook_event_name = $EventName
        tool_name = $ToolName
        tool_input = [ordered]@{ command = $Command }
        tool_use_id = $ToolUseId
    }
    if ($PSBoundParameters.ContainsKey("AgentId")) { $payload.agent_id = $AgentId }
    if ($PSBoundParameters.ContainsKey("AgentType")) { $payload.agent_type = $AgentType }
    if ($EventName -eq "PostToolUse") { $payload.tool_response = [ordered]@{ ok = $true } }
    if ($EventName -eq "PostToolUseFailure") { $payload.error = $ErrorText }
    return $payload
}

function Invoke-Pre {
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [string]$ToolName = "PowerShell",
        [string]$ToolUseId = "tool-test",
        [string]$AgentId,
        [string]$AgentType,
        [string]$RepositoryRoot = $Repository,
        [switch]$Pretty,
        [switch]$UseInheritedUnicodeWriter
    )

    $payloadArguments = @{
        EventName = "PreToolUse"
        ToolName = $ToolName
        Command = $Command
        ToolUseId = $ToolUseId
        RepositoryRoot = $RepositoryRoot
    }
    if ($PSBoundParameters.ContainsKey("AgentId")) { $payloadArguments.AgentId = $AgentId }
    if ($PSBoundParameters.ContainsKey("AgentType")) { $payloadArguments.AgentType = $AgentType }
    $payload = New-HookPayload @payloadArguments
    $json = if ($Pretty) { $payload | ConvertTo-Json -Depth 6 } else { $payload | ConvertTo-Json -Depth 6 -Compress }
    return Invoke-HookProcess -RawInput $json -RepositoryRoot $RepositoryRoot -UseInheritedUnicodeWriter:$UseInheritedUnicodeWriter
}

function Invoke-Post {
    param(
        [Parameter(Mandatory = $true)][ValidateSet("PostToolUse", "PostToolUseFailure")][string]$EventName,
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string]$ToolUseId,
        [string]$ToolName = "PowerShell",
        [string]$ErrorText = "",
        [string]$RepositoryRoot = $Repository
    )

    $payload = New-HookPayload -EventName $EventName -ToolName $ToolName -Command $Command -ToolUseId $ToolUseId -ErrorText $ErrorText -RepositoryRoot $RepositoryRoot
    return Invoke-HookProcess -RawInput ($payload | ConvertTo-Json -Depth 6 -Compress) -RepositoryRoot $RepositoryRoot
}

function Assert-DeniedResult {
    param(
        [Parameter(Mandatory = $true)]$Result,
        [Parameter(Mandatory = $true)][string]$Name,
        [switch]$CheckFullReason
    )

    $script:Cases++
    Assert-Equal $Result.ExitCode 0 "$Name denial hook exit code" $Result.Combined
    $parsed = $null
    try { $parsed = $Result.Stdout | ConvertFrom-Json }
    catch { throw "ASSERTION FAILED: $Name did not emit valid denial JSON`n$($Result.Combined)" }
    Assert-Equal ([string]$parsed.hookSpecificOutput.permissionDecision) "deny" "$Name must be denied" $Result.Combined
    $reason = [string]$parsed.hookSpecificOutput.permissionDecisionReason
    Assert-Contains $reason "ORIGAMI3_COORDINATOR_BOUNDARY_DENY" "$Name denial needs the stable identifier"
    Assert-Contains $reason "Delegate implementation" "$Name denial must instruct delegation"
    if ($CheckFullReason) {
        foreach ($number in 1..6) {
            Assert-Contains $reason ("ALLOW-{0}:" -f $number) "$Name denial must list ALLOW-$number"
        }
    }
    Write-Host ("DENY {0}: exit={1}" -f $Name, $Result.ExitCode)
    return $reason
}

function Assert-AllowedResult {
    param(
        [Parameter(Mandatory = $true)]$Result,
        [Parameter(Mandatory = $true)][string]$Name,
        [switch]$ExpectReleaseWarning
    )

    $script:Cases++
    Assert-Equal $Result.ExitCode 0 "$Name allow hook exit code" $Result.Combined
    Assert-True (-not $Result.Stdout.Contains('"permissionDecision":"deny"')) "$Name unexpectedly emitted a denial" $Result.Combined
    if ($ExpectReleaseWarning) {
        Assert-Contains $Result.Combined "HOOK_HEALTH_RELEASED" "$Name must disclose the active release"
    }
    else {
        Assert-True ([string]::IsNullOrWhiteSpace($Result.Stdout)) "$Name allowed stdout must be empty" $Result.Combined
        Assert-True ([string]::IsNullOrWhiteSpace($Result.Stderr)) "$Name allowed stderr must be empty" $Result.Combined
    }
    Write-Host ("ALLOW {0}: exit={1} releaseWarning={2}" -f $Name, $Result.ExitCode, [bool]$ExpectReleaseWarning)
}

function Get-RepositoryKey {
    param([Parameter(Mandatory = $true)][string]$Root)

    $normalized = [IO.Path]::GetFullPath($Root).Replace("\", "/").TrimEnd("/").ToLowerInvariant()
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return -join ($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($normalized)) | ForEach-Object { $_.ToString("x2") }) }
    finally { $sha.Dispose() }
}

function Get-StatePaths {
    param([string]$RepositoryRoot = $Repository)

    $key = Get-RepositoryKey $RepositoryRoot
    $directory = Join-Path $StateRoot "ori3-coordinator-boundary"
    $state = Join-Path $directory ("{0}.json" -f $key)
    return [PSCustomObject]@{
        Key = $key
        Directory = $directory
        State = $state
        Acknowledgement = $state + ".block"
        Audit = Join-Path $directory ("{0}.audit.jsonl" -f $key)
    }
}

function Reset-ActiveState {
    param([string]$RepositoryRoot = $Repository)

    $paths = Get-StatePaths $RepositoryRoot
    foreach ($path in @($paths.State, $paths.Acknowledgement)) {
        if (Test-Path -LiteralPath $path -PathType Leaf) { Remove-Item -LiteralPath $path -Force }
    }
}

function Assert-PreDenied {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Command,
        [string]$ToolName = "PowerShell"
    )

    Reset-ActiveState
    [void](Assert-DeniedResult (Invoke-Pre -Command $Command -ToolName $ToolName -ToolUseId ("deny-" + $script:Cases)) $Name)
}

function Assert-PreAllowed {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Command,
        [string]$ToolName = "PowerShell",
        [string]$RepositoryRoot = $Repository
    )

    Reset-ActiveState $RepositoryRoot
    Assert-AllowedResult (Invoke-Pre -Command $Command -ToolName $ToolName -ToolUseId ("allow-" + $script:Cases) -RepositoryRoot $RepositoryRoot) $Name
}

function Initialize-TestRepository {
    param([Parameter(Mandatory = $true)][string]$Root)

    foreach ($directory in @("scripts", "scripts\hooks", "scratchpad", "docs", "target\release")) {
        [void][IO.Directory]::CreateDirectory((Join-Path $Root $directory))
    }
    foreach ($file in @("scripts\check.ps1", "scripts\check-ci.ps1", "scripts\check-release-ready.ps1", "scripts\check-receipt.ps1", "scripts\watch-agents.ps1")) {
        [IO.File]::WriteAllText((Join-Path $Root $file), "# test placeholder`r`n", [Text.Encoding]::ASCII)
    }
    [IO.File]::WriteAllText((Join-Path $Root "scratchpad\watch.json"), '{"agents":[]}', [Text.Encoding]::ASCII)
    [IO.File]::WriteAllText((Join-Path $Root "docs\sample.md"), "sample", [Text.Encoding]::ASCII)
    [IO.File]::WriteAllText((Join-Path $Root "docs\second.md"), "second", [Text.Encoding]::ASCII)
    [IO.File]::WriteAllText((Join-Path $Root "scratchpad\commit-message.txt"), "日本語のコミットメッセージ", (New-Object Text.UTF8Encoding($false)))
    [IO.File]::WriteAllBytes((Join-Path $Root "scripts\powershell.exe"), [byte[]]@(77, 90, 0, 0))
    [IO.File]::WriteAllBytes((Join-Path $Root "target\release\desktop.exe"), [byte[]]@(77, 90, 0, 0))
}

function Test-ActualGitCoordinatorFlow {
    [void][IO.Directory]::CreateDirectory($GitRepository)
    [void][IO.Directory]::CreateDirectory((Join-Path $GitRepository "docs"))
    [void][IO.Directory]::CreateDirectory((Join-Path $GitRepository "scripts"))
    [void][IO.Directory]::CreateDirectory((Join-Path $GitRepository "scratchpad"))
    [void][IO.Directory]::CreateDirectory((Join-Path $GitRepository "empty-hooks"))

    [void](Invoke-GitFixture -Name "setup bare origin" -WorkingDirectory $Sandbox -Arguments @("init", "--bare", $BareOrigin))
    [void](Invoke-GitFixture -Name "setup working repository" -WorkingDirectory $Sandbox -Arguments @("init", $GitRepository))
    [void](Invoke-GitFixture -Name "setup main branch" -WorkingDirectory $GitRepository -Arguments @("branch", "-M", "main"))
    [void](Invoke-GitFixture -Name "setup user email" -WorkingDirectory $GitRepository -Arguments @("config", "user.email", "boundary-test@example.invalid"))
    [void](Invoke-GitFixture -Name "setup user name" -WorkingDirectory $GitRepository -Arguments @("config", "user.name", "Boundary Integration Test"))
    [void](Invoke-GitFixture -Name "setup unsigned commit" -WorkingDirectory $GitRepository -Arguments @("config", "commit.gpgSign", "false"))
    [void](Invoke-GitFixture -Name "setup empty hooks directory" -WorkingDirectory $GitRepository -Arguments @("config", "core.hooksPath", "empty-hooks"))

    $firstPath = Join-Path $GitRepository "docs\integration-a.txt"
    $secondPath = Join-Path $GitRepository "scripts\integration-b.ps1"
    $messagePath = Join-Path $GitRepository "scratchpad\integration-commit-message.txt"
    $firstMessage = "日本語の初期コミット。統括です。**接続しました"
    [IO.File]::WriteAllText($firstPath, "first`r`n", $script:Utf8NoBom)
    [IO.File]::WriteAllText($secondPath, "# second`r`n", $script:Utf8NoBom)
    [IO.File]::WriteAllText($messagePath, $firstMessage + "`r`n", $script:Utf8NoBom)

    Assert-PreAllowed -Name "actual git add multiple real paths policy" -Command "git add docs/integration-a.txt scripts/integration-b.ps1" -RepositoryRoot $GitRepository
    [void](Invoke-GitFixture -Name "git add multiple real paths" -WorkingDirectory $GitRepository -Arguments @("add", "docs/integration-a.txt", "scripts/integration-b.ps1") -CountCase)
    Assert-PreAllowed -Name "actual git commit -F policy" -Command "git commit -F scratchpad/integration-commit-message.txt" -RepositoryRoot $GitRepository
    [void](Invoke-GitFixture -Name "git commit -F Japanese UTF-8 message" -WorkingDirectory $GitRepository -Arguments @("commit", "-F", "scratchpad/integration-commit-message.txt") -CountCase)
    [void](Invoke-GitFixture -Name "setup origin remote" -WorkingDirectory $GitRepository -Arguments @("remote", "add", "origin", $BareOrigin))
    [void](Invoke-GitFixture -Name "setup origin main" -WorkingDirectory $GitRepository -Arguments @("push", "-u", "origin", "main"))

    $secondMessage = "日本語のaheadコミット。統括です、E*接続しました。墁E��の判定�E正しく動いてぁE��す"
    [IO.File]::AppendAllText($firstPath, "ahead`r`n", $script:Utf8NoBom)
    [IO.File]::AppendAllText($secondPath, "# ahead`r`n", $script:Utf8NoBom)
    [IO.File]::WriteAllText($messagePath, $secondMessage + "`r`n", $script:Utf8NoBom)
    [void](Invoke-GitFixture -Name "git add multiple changed real paths" -WorkingDirectory $GitRepository -Arguments @("add", "docs/integration-a.txt", "scripts/integration-b.ps1") -CountCase)
    [void](Invoke-GitFixture -Name "git commit -F exact corrupted-string regression" -WorkingDirectory $GitRepository -Arguments @("commit", "-F", "scratchpad/integration-commit-message.txt") -CountCase)

    $messageRead = Invoke-GitFixture -Name "verify committed Japanese message" -WorkingDirectory $GitRepository -Arguments @("log", "-1", "--format=%B")
    Assert-Equal $messageRead.Stdout.Trim() $secondMessage "git commit -F must preserve the exact UTF-8 Japanese/corruption regression text" $messageRead.Stdout
    Assert-PreAllowed -Name "actual git fetch origin policy" -Command "git fetch origin" -RepositoryRoot $GitRepository
    [void](Invoke-GitFixture -Name "git fetch origin" -WorkingDirectory $GitRepository -Arguments @("fetch", "origin") -CountCase)
    Assert-PreAllowed -Name "actual ahead log policy" -Command "git log --oneline origin/main..HEAD" -RepositoryRoot $GitRepository
    $aheadLog = Invoke-GitFixture -Name "git log --oneline origin/main..HEAD" -WorkingDirectory $GitRepository -Arguments @("log", "--oneline", "origin/main..HEAD") -CountCase
    Assert-Contains $aheadLog.Stdout "日本語のaheadコミット" "ahead log must show the UTF-8 commit subject"
    Assert-PreAllowed -Name "actual ahead count policy" -Command "git rev-list --count origin/main..HEAD" -RepositoryRoot $GitRepository
    $aheadCount = Invoke-GitFixture -Name "git rev-list --count origin/main..HEAD" -WorkingDirectory $GitRepository -Arguments @("rev-list", "--count", "origin/main..HEAD") -CountCase
    Assert-Equal $aheadCount.Stdout.Trim() "1" "ahead count after local commit"
    Assert-PreAllowed -Name "actual worktree porcelain policy" -Command "git worktree list --porcelain" -RepositoryRoot $GitRepository
    $worktreeList = Invoke-GitFixture -Name "git worktree list --porcelain" -WorkingDirectory $GitRepository -Arguments @("worktree", "list", "--porcelain") -CountCase
    Assert-Contains $worktreeList.Stdout "worktree " "porcelain worktree output"

    Assert-PreAllowed -Name "actual write-tree policy" -Command "git write-tree" -RepositoryRoot $GitRepository
    $tree = Invoke-GitFixture -Name "git write-tree" -WorkingDirectory $GitRepository -Arguments @("write-tree") -CountCase
    $treeId = $tree.Stdout.Trim()
    Assert-True ($treeId -match '^(?:[0-9a-f]{40}|[0-9a-f]{64})$') "write-tree must return a literal object id" $tree.Stdout
    $head = Invoke-GitFixture -Name "setup snapshot parent read" -WorkingDirectory $GitRepository -Arguments @("rev-parse", "HEAD")
    $headId = $head.Stdout.Trim()
    Assert-True ($headId -match '^(?:[0-9a-f]{40}|[0-9a-f]{64})$') "snapshot parent must be a literal object id" $head.Stdout
    Assert-PreAllowed -Name "actual commit-tree policy" -Command "git commit-tree $treeId -p $headId -m 'WIP snapshot integration'" -RepositoryRoot $GitRepository
    $snapshot = Invoke-GitFixture -Name "git commit-tree snapshot" -WorkingDirectory $GitRepository -Arguments @("commit-tree", $treeId, "-p", $headId, "-m", "WIP snapshot integration") -CountCase
    $snapshotId = $snapshot.Stdout.Trim()
    Assert-True ($snapshotId -match '^(?:[0-9a-f]{40}|[0-9a-f]{64})$') "commit-tree must return a literal object id" $snapshot.Stdout
    Assert-PreAllowed -Name "actual update refs/wip policy" -Command "git update-ref refs/wip/integration $snapshotId" -RepositoryRoot $GitRepository
    [void](Invoke-GitFixture -Name "git update-ref refs/wip/integration" -WorkingDirectory $GitRepository -Arguments @("update-ref", "refs/wip/integration", $snapshotId) -CountCase)
    Assert-PreAllowed -Name "actual refs/wip inventory policy" -Command "git for-each-ref refs/wip" -RepositoryRoot $GitRepository
    $wipRefs = Invoke-GitFixture -Name "git for-each-ref refs/wip" -WorkingDirectory $GitRepository -Arguments @("for-each-ref", "refs/wip") -CountCase
    Assert-Contains $wipRefs.Stdout $snapshotId "refs/wip inventory must contain the exact snapshot object"
    Assert-Contains $wipRefs.Stdout "refs/wip/integration" "refs/wip inventory must contain the exact snapshot ref"
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $Sandbox)) { return }
    $full = [IO.Path]::GetFullPath($Sandbox).TrimEnd([char[]]"\/")
    if ([IO.Path]::GetDirectoryName($full) -ne $TempBase -or [IO.Path]::GetFileName($full) -notmatch '^ori3-coordinator-boundary-test-[0-9a-f]{32}$') {
        throw "refusing unsafe self-test cleanup: $full"
    }
    Remove-Item -LiteralPath $full -Recurse -Force
}

[void][IO.Directory]::CreateDirectory($Sandbox)
Initialize-TestRepository $Repository
Initialize-TestRepository $Repository2

try {
    $quotedArguments = ConvertTo-ProcessArgumentString @('alpha"beta', 'C:\quoted path\')
    Assert-Contains $quotedArguments '"alpha\\"beta"' "process argument quoting must escape an embedded quote"
    Assert-Contains $quotedArguments '"C:\quoted path\\"' "process argument quoting must double a trailing backslash before the closing quote"

    Write-Host "[1/8] main-thread and subagent identity contract"
    Reset-ActiveState
    [void](Assert-DeniedResult (Invoke-Pre -Command "cargo test --workspace" -ToolUseId "identity-main") "main agent_id absent" -CheckFullReason)
    Reset-ActiveState
    [void](Assert-DeniedResult (Invoke-Pre -Command "cargo test --workspace" -ToolUseId "identity-empty" -AgentId "") "empty agent_id")
    Reset-ActiveState
    [void](Assert-DeniedResult (Invoke-Pre -Command "cargo test --workspace" -ToolUseId "identity-agent-type" -AgentType "general-purpose") "agent_type without agent_id")
    Reset-ActiveState
    $numericAgentPayload = New-HookPayload -EventName "PreToolUse" -ToolName "PowerShell" -Command "cargo test --workspace" -ToolUseId "identity-numeric"
    $numericAgentPayload.agent_id = 123
    [void](Assert-DeniedResult (Invoke-HookProcess -RawInput ($numericAgentPayload | ConvertTo-Json -Depth 6 -Compress)) "non-string agent_id")
    Reset-ActiveState
    Assert-AllowedResult (Invoke-Pre -Command "cargo test --workspace; npm test; & `$anything" -ToolUseId "identity-subagent" -AgentId "agent-aa86b4da765f1d2d8" -AgentType "general-purpose") "nonempty subagent agent_id"

    Write-Host "[2/8] exact coordinator allowlist"
    $gate = Join-Path $Repository "scripts\check.ps1"
    $ciGate = Join-Path $Repository "scripts\check-ci.ps1"
    $releaseGate = Join-Path $Repository "scripts\check-release-ready.ps1"
    $receipt = Join-Path $Repository "scripts\check-receipt.ps1"
    $watch = Join-Path $Repository "scripts\watch-agents.ps1"
    $watchDefinition = Join-Path $Repository "scratchpad\watch.json"
    $desktop = Join-Path $Repository "target\release\desktop.exe"
    $otherPowerShell = Join-Path $Repository "scripts\powershell.exe"
    $sample = Join-Path $Repository "docs\sample.md"
    $commitMessage = Join-Path $Repository "scratchpad\commit-message.txt"
    $watchArgumentList = "-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$watch`" -DefinitionPath `"$watchDefinition`" -RepositoryRoot `"$Repository`" -IntervalMinutes 10 -StaleAfterMinutes 40"
    $detachedWatchCommand = "Start-Process -FilePath '$PowerShellPath' -ArgumentList '$watchArgumentList' -WindowStyle Hidden"
    $allowedCases = @(
        @{ Name = "git status"; Command = "git status --porcelain" },
        @{ Name = "git diff"; Command = "git diff --cached" },
        @{ Name = "git rev-parse"; Command = "git rev-parse HEAD" },
        @{ Name = "git commit"; Command = "git commit -m 'test message'" },
        @{ Name = "git add multiple literal paths"; Command = "git add docs/sample.md docs/second.md" },
        @{ Name = "git add multiple literal paths after separator"; Command = "git add -- docs/sample.md docs/second.md" },
        @{ Name = "git commit Japanese UTF-8 file"; Command = "git commit -F '$commitMessage'" },
        @{ Name = "git fetch fixed origin"; Command = "git fetch origin" },
        @{ Name = "git ahead log"; Command = "git log --oneline origin/main..HEAD" },
        @{ Name = "git ahead count"; Command = "git rev-list --count origin/main..HEAD" },
        @{ Name = "git worktree porcelain"; Command = "git worktree list --porcelain" },
        @{ Name = "git refs/wip inventory"; Command = "git for-each-ref refs/wip" },
        @{ Name = "git write-tree"; Command = "git write-tree" },
        @{ Name = "git commit-tree snapshot"; Command = "git commit-tree a111111111111111111111111111111111111111 -p HEAD -m 'WIP snapshot'" },
        @{ Name = "git update refs/wip"; Command = "git update-ref refs/wip/sample a222222222222222222222222222222222222222" },
        @{ Name = "git push"; Command = "git push origin main" },
        @{ Name = "git tag"; Command = "git tag v1.2.3" },
        @{ Name = "check gate"; Command = "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$gate`"" },
        @{ Name = "CI gate"; Command = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$ciGate`"" },
        @{ Name = "release gate with tag"; Command = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$releaseGate`" -Tag v1.2.3" },
        @{ Name = "coordinator signing key repair direct"; Command = "scripts/check-receipt.ps1 -RepairSigningKey -RepoRoot '$Repository'" },
        @{ Name = "coordinator signing key repair wrapped"; Command = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$receipt`" -RepairSigningKey -RepoRoot `"$Repository`"" },
        @{ Name = "coordinator signing key repair current host wrapper"; Command = "$PowerShellPath -NoProfile -ExecutionPolicy Bypass -File `"$receipt`" -RepairSigningKey -RepoRoot `"$Repository`"" },
        @{ Name = "literal file read"; Command = "Get-Content -LiteralPath '$sample'" },
        @{ Name = "rg with exclusion"; Command = "rg needle '$Repository' --glob '!docs/competitive-review-2026-08-20.md'" },
        @{ Name = "process pipeline"; Command = "Get-Process -Name cargo | Measure-Object" },
        @{ Name = "free capacity"; Command = "Get-PSDrive -Name C" },
        @{ Name = "production payload exact local report time"; Command = "Get-Date -Format 'yyyy-MM-dd HH:mm'" },
        @{ Name = "desktop start"; Command = "Start-Process -FilePath '$desktop' -WorkingDirectory '$([IO.Path]::GetDirectoryName($desktop))' -PassThru" },
        @{ Name = "desktop close"; Command = "(Get-Process -Name desktop -ErrorAction Stop).CloseMainWindow()" },
        @{ Name = "exact host hidden detached continuous watcher"; Command = $detachedWatchCommand }
    )
    foreach ($case in $allowedCases) { Assert-PreAllowed -Name $case.Name -Command $case.Command }
    Assert-PreAllowed -Name "Bash simple git read" -Command "git status --porcelain" -ToolName "Bash"

    Write-Host "[3/8] violation scripts and unlisted commands are denied"
    $deniedCases = @(
        @{ Name = "direct cargo"; Command = "cargo test --workspace" },
        @{ Name = "direct npm"; Command = "npm test" },
        @{ Name = "receipt self-test"; Command = "powershell.exe -NoProfile -File scripts/check-receipt-self-test.ps1" },
        @{ Name = "individual test script"; Command = "powershell.exe -NoProfile -File scripts/check-agent-instruction.test.ps1" },
        @{ Name = "roadmap generator"; Command = "powershell.exe -NoProfile -File scripts/generate-roadmap-links.ps1" },
        @{ Name = "rules split script"; Command = "powershell.exe -NoProfile -File scripts/check-rules-split.ps1" },
        @{ Name = "signing key repair missing repository"; Command = "scripts/check-receipt.ps1 -RepairSigningKey" },
        @{ Name = "signing key repair wrong repository"; Command = "scripts/check-receipt.ps1 -RepairSigningKey -RepoRoot '$Repository2'" },
        @{ Name = "signing key repair relative repository"; Command = "scripts/check-receipt.ps1 -RepairSigningKey -RepoRoot '.'" },
        @{ Name = "signing key repair extra argument"; Command = "scripts/check-receipt.ps1 -RepairSigningKey -RepoRoot '$Repository' -RunRustW4" },
        @{ Name = "signing key repair reordered arguments"; Command = "scripts/check-receipt.ps1 -RepoRoot '$Repository' -RepairSigningKey" },
        @{ Name = "receipt Rust gate remains delegated"; Command = "scripts/check-receipt.ps1 -RunRustW4 -RepoRoot '$Repository'" },
        @{ Name = "receipt without mode remains delegated"; Command = "scripts/check-receipt.ps1 -RepoRoot '$Repository'" },
        @{ Name = "signing key repair alternate PowerShell wrapper path"; Command = "scripts/powershell.exe -NoProfile -File `"$receipt`" -RepairSigningKey -RepoRoot `"$Repository`"" },
        @{ Name = "signing key repair drive-relative PowerShell wrapper path"; Command = "C:powershell.exe -NoProfile -File `"$receipt`" -RepairSigningKey -RepoRoot `"$Repository`"" },
        @{ Name = "CI narrowed static mode"; Command = "powershell.exe -NoProfile -File `"$ciGate`" -StaticContractOnly" },
        @{ Name = "direct blocking watcher"; Command = "powershell.exe -NoProfile -File `"$watch`" -DefinitionPath `"$watchDefinition`" -RepositoryRoot `"$Repository`" -IntervalMinutes 10 -StaleAfterMinutes 40" },
        @{ Name = "detached watch relative PowerShell host"; Command = $detachedWatchCommand.Replace("-FilePath '$PowerShellPath'", "-FilePath 'powershell.exe'") },
        @{ Name = "detached watch different absolute executable"; Command = $detachedWatchCommand.Replace("-FilePath '$PowerShellPath'", "-FilePath '$otherPowerShell'") },
        @{ Name = "detached watch once"; Command = $detachedWatchCommand.Replace(" -IntervalMinutes 10", " -Once -IntervalMinutes 10") },
        @{ Name = "detached watch missing NonInteractive"; Command = $detachedWatchCommand.Replace("-NonInteractive ", "") },
        @{ Name = "detached watch added child argument"; Command = $detachedWatchCommand.Replace("-NoProfile ", "-NoProfile -NoExit ") },
        @{ Name = "detached watch wrong interval"; Command = $detachedWatchCommand.Replace("-IntervalMinutes 10", "-IntervalMinutes 11") },
        @{ Name = "detached watch wrong stale threshold"; Command = $detachedWatchCommand.Replace("-StaleAfterMinutes 40", "-StaleAfterMinutes 41") },
        @{ Name = "detached watch missing DefinitionPath"; Command = $detachedWatchCommand.Replace(" -DefinitionPath `"$watchDefinition`"", "") },
        @{ Name = "detached watch missing definition file"; Command = $detachedWatchCommand.Replace($watchDefinition, (Join-Path $Repository "scratchpad\missing-watch.json")) },
        @{ Name = "detached watch wrong repository"; Command = $detachedWatchCommand.Replace("-RepositoryRoot `"$Repository`"", "-RepositoryRoot `"$Repository2`"") },
        @{ Name = "detached watch wrong script"; Command = $detachedWatchCommand.Replace("-File `"$watch`"", "-File `"$gate`"") },
        @{ Name = "detached watch missing Hidden"; Command = $detachedWatchCommand.Replace(" -WindowStyle Hidden", "") },
        @{ Name = "detached watch non-Hidden"; Command = $detachedWatchCommand.Replace("-WindowStyle Hidden", "-WindowStyle Normal") },
        @{ Name = "detached watch added outer argument"; Command = $detachedWatchCommand.Replace(" -WindowStyle Hidden", " -PassThru -WindowStyle Hidden") },
        @{ Name = "detached watch dynamic ArgumentList"; Command = "Start-Process -FilePath 'powershell.exe' -ArgumentList `$env:WATCH_ARGS -WindowStyle Hidden" },
        @{ Name = "git add option"; Command = "git add -A docs/sample.md" },
        @{ Name = "git add wildcard"; Command = "git add 'docs/*.md'" },
        @{ Name = "git add repository root"; Command = "git add ." },
        @{ Name = "git add outside repository"; Command = "git add '..\outside.md'" },
        @{ Name = "git fetch extra option"; Command = "git fetch --prune origin" },
        @{ Name = "git fetch wrong remote"; Command = "git fetch upstream" },
        @{ Name = "git checkout"; Command = "git checkout main" },
        @{ Name = "git merge"; Command = "git merge topic" },
        @{ Name = "git stash"; Command = "git stash" },
        @{ Name = "git reset"; Command = "git reset --hard" },
        @{ Name = "git alias escape"; Command = "git -c alias.x=!cargo x" },
        @{ Name = "git ext diff"; Command = "git diff --ext-diff" },
        @{ Name = "git commit no verify"; Command = "git commit --no-verify -m test" },
        @{ Name = "git commit short no verify"; Command = "git commit -n -m test" },
        @{ Name = "git commit combined short no verify"; Command = "git commit -nF '$commitMessage'" },
        @{ Name = "git commit bundled short no verify"; Command = "git commit -an -m test" },
        @{ Name = "git commit message stdin"; Command = "git commit -F -" },
        @{ Name = "git commit message outside repo"; Command = "git commit -F '..\message.txt'" },
        @{ Name = "git for-each-ref arbitrary refs"; Command = "git for-each-ref refs/heads" },
        @{ Name = "git worktree list unsafe option"; Command = "git worktree list --expire now" },
        @{ Name = "git write-tree option"; Command = "git write-tree --missing-ok" },
        @{ Name = "git commit-tree signing"; Command = "git commit-tree a111111111111111111111111111111111111111 -S -m snapshot" },
        @{ Name = "git commit-tree no message"; Command = "git commit-tree a111111111111111111111111111111111111111 -p HEAD" },
        @{ Name = "git update arbitrary ref"; Command = "git update-ref refs/heads/main a222222222222222222222222222222222222222" },
        @{ Name = "git update symbolic value"; Command = "git update-ref refs/wip/sample HEAD" },
        @{ Name = "git push wrong remote"; Command = "git push ext::cargo main" },
        @{ Name = "git show output"; Command = "git show --output=out.txt" },
        @{ Name = "rg missing exclusion"; Command = "rg needle docs" },
        @{ Name = "rg preprocessor"; Command = "rg needle . --glob '!docs/competitive-review-2026-08-20.md' --pre cargo" },
        @{ Name = "rg prohibited reinclude"; Command = "rg needle . --glob '!docs/competitive-review-2026-08-20.md' --glob 'docs/competitive-review-2026-08-20.md'" },
        @{ Name = "prohibited document read"; Command = "Get-Content -LiteralPath docs/competitive-review-2026-08-20.md" },
        @{ Name = "non-filesystem provider read"; Command = "Get-ChildItem -LiteralPath Env:" },
        @{ Name = "production payload bare Get-Date"; Command = "Get-Date" },
        @{ Name = "production payload Get-Date with seconds"; Command = "Get-Date -Format 'yyyy-MM-dd HH:mm:ss'" },
        @{ Name = "production payload Get-Date ISO format"; Command = "Get-Date -Format o" },
        @{ Name = "production payload Get-Date UFormat"; Command = "Get-Date -UFormat '%Y-%m-%d %H:%M'" },
        @{ Name = "production payload Get-Date extra AsUTC"; Command = "Get-Date -Format 'yyyy-MM-dd HH:mm' -AsUTC" },
        @{ Name = "production payload date alias"; Command = "date -Format 'yyyy-MM-dd HH:mm'" },
        @{ Name = "production payload module-qualified Get-Date"; Command = "Microsoft.PowerShell.Utility\Get-Date -Format 'yyyy-MM-dd HH:mm'" },
        @{ Name = "production payload Set-Date"; Command = "Set-Date -Date '2026-08-31 12:00'" },
        @{ Name = "production payload Get-Date pipeline"; Command = "Get-Date -Format 'yyyy-MM-dd HH:mm' | Out-String" },
        @{ Name = "production payload Get-Date assignment"; Command = "`$now = Get-Date -Format 'yyyy-MM-dd HH:mm'" },
        @{ Name = "production payload Get-Date wrapper"; Command = 'powershell.exe -NoProfile -Command "Get-Date -Format ''yyyy-MM-dd HH:mm''"' },
        @{ Name = "production payload Get-Date redirection"; Command = "Get-Date -Format 'yyyy-MM-dd HH:mm' > now.txt" },
        @{ Name = "production payload Get-Date reordered arguments"; Command = "Get-Date 'yyyy-MM-dd HH:mm' -Format" },
        @{ Name = "production payload Get-Date dynamic format"; Command = "Get-Date -Format `$format" },
        @{ Name = "production payload Get-Date chained command"; Command = "Get-Date -Format 'yyyy-MM-dd HH:mm'; git status" },
        @{ Name = "cmd wrapper"; Command = "cmd.exe /c git status" },
        @{ Name = "PowerShell command wrapper"; Command = "powershell.exe -Command 'git status'" },
        @{ Name = "bash wrapper"; Command = "bash -c 'git status'" },
        @{ Name = "wsl wrapper"; Command = "wsl git status" },
        @{ Name = "desktop wrong executable"; Command = "Start-Process -FilePath cargo.exe" },
        @{ Name = "arbitrary Remove-Item"; Command = "Remove-Item -LiteralPath '$sample'" }
    )
    foreach ($case in $deniedCases) { Assert-PreDenied -Name $case.Name -Command $case.Command }
    Assert-PreDenied -Name "production payload Get-Date through Bash" -Command "Get-Date -Format 'yyyy-MM-dd HH:mm'" -ToolName "Bash"

    Write-Host "[4/8] parse, dynamic, mixed, and member bypasses are denied"
    $parseDenied = @(
        @{ Name = "mixed statements"; Command = "git status; cargo test"; Tool = "PowerShell" },
        @{ Name = "redirection"; Command = "git status > out.txt"; Tool = "PowerShell" },
        @{ Name = "assignment and dynamic call"; Command = "`$command = 'git'; & `$command status"; Tool = "PowerShell" },
        @{ Name = "expandable environment path"; Command = 'Get-Content "$env:TEMP\sample.md"'; Tool = "PowerShell" },
        @{ Name = "loop"; Command = "foreach (`$item in 1) { git status }"; Tool = "PowerShell" },
        @{ Name = "scriptblock pipeline"; Command = "Get-Process | Where-Object { Stop-Process -Id `$_.Id }"; Tool = "PowerShell" },
        @{ Name = "Invoke-Expression pipeline"; Command = "Get-Content '$sample' | Invoke-Expression"; Tool = "PowerShell" },
        @{ Name = "static dotnet"; Command = "[IO.File]::ReadAllText('$sample')"; Tool = "PowerShell" },
        @{ Name = "kill member"; Command = "(Get-Process desktop).Kill()"; Tool = "PowerShell" },
        @{ Name = "parse error"; Command = "&"; Tool = "PowerShell" },
        @{ Name = "Bash chain"; Command = "git status && cargo test"; Tool = "Bash" },
        @{ Name = "Bash substitution"; Command = "git `$(`$echo status`)"; Tool = "Bash" },
        @{ Name = "Bash glob expansion"; Command = "git status *"; Tool = "Bash" }
    )
    foreach ($case in $parseDenied) { Assert-PreDenied -Name $case.Name -Command $case.Command -ToolName $case.Tool }

    Write-Host "[5/8] malformed and encoded payloads fail closed"
    Reset-ActiveState
    [void](Assert-DeniedResult (Invoke-HookProcess -RawInput "{not-json") "invalid JSON")
    Reset-ActiveState
    [void](Assert-DeniedResult (Invoke-HookProcess -RawInput "") "empty stdin")
    Reset-ActiveState
    $bomPayload = New-HookPayload -EventName "PreToolUse" -ToolName "PowerShell" -Command "cargo test" -ToolUseId "bom"
    [void](Assert-DeniedResult (Invoke-HookProcess -RawInput (([string][char]0xFEFF) + ($bomPayload | ConvertTo-Json -Depth 6 -Compress))) "UTF-8 BOM payload")
    Reset-ActiveState
    [void](Assert-DeniedResult (Invoke-Pre -Command "cargo test" -ToolUseId "pretty" -Pretty) "multiline JSON")
    Reset-ActiveState
    [void](Assert-DeniedResult (Invoke-Pre -Command "cargo test" -ToolUseId "utf16" -UseInheritedUnicodeWriter) "inherited UTF-16 writer")
    Reset-ActiveState
    $utf8Message = New-LongJapaneseCommitMessage
    $utf8Commit = "git commit -m '$utf8Message'"
    $utf8Payload = New-HookPayload -EventName "PreToolUse" -ToolName "PowerShell" -Command $utf8Commit -ToolUseId "utf8-japanese-commit"
    $utf8PayloadJson = $utf8Payload | ConvertTo-Json -Depth 6 -Compress
    $utf8PayloadByteCount = $script:Utf8NoBom.GetByteCount($utf8PayloadJson)
    Write-Host ("UTF8_REGRESSION_PAYLOAD_BYTES={0}" -f $utf8PayloadByteCount)
    Assert-True ($utf8PayloadByteCount -gt 5033) "実障害の5033バイト目を越える日本語commit payloadを使うこと"
    Assert-Contains $utf8Commit "**接続しました。**" "日本語と記号の境目を含めること"
    Assert-AllowedResult (Invoke-HookProcess -RawInput $utf8PayloadJson) "raw UTF-8 Japanese commit message"

    Write-Host "[6/8] actual throwaway git coordinator flow"
    Test-ActualGitCoordinatorFlow

    Write-Host "[7/8] exact one-time receipt and PostToolUse success"
    Reset-ActiveState
    $releasedCommand = "cargo test --workspace"
    $initial = Invoke-Pre -Command $releasedCommand -ToolUseId "receipt-deny"
    $initialReason = Assert-DeniedResult $initial "receipt initial denial" -CheckFullReason
    $paths = Get-StatePaths
    Assert-True (Test-Path -LiteralPath $paths.State -PathType Leaf) "denial must create state" $initialReason
    Assert-True (Test-Path -LiteralPath $paths.Acknowledgement -PathType Leaf) "denial must create acknowledgement" $initialReason
    Assert-True (Test-Path -LiteralPath $paths.Audit -PathType Leaf) "denial must create append-only audit" $initialReason
    Assert-Contains $initialReason $paths.Acknowledgement "denial must report the exact acknowledgement path"

    $ackCommand = "Remove-Item -LiteralPath '$($paths.Acknowledgement)'"
    Assert-AllowedResult (Invoke-Pre -Command $ackCommand -ToolUseId "ack-control") "exact acknowledgement delete control"
    Remove-Item -LiteralPath $paths.Acknowledgement -Force
    [void](Invoke-Post -EventName "PostToolUse" -Command $ackCommand -ToolUseId "ack-control")

    $release = Invoke-Pre -Command $releasedCommand -ToolUseId "release-success"
    Assert-AllowedResult $release "same hash one-time release" -ExpectReleaseWarning
    Assert-True (Test-Path -LiteralPath $paths.State -PathType Leaf) "release must retain state until PostToolUse"
    $stateInFlight = [IO.File]::ReadAllText($paths.State, [Text.Encoding]::UTF8) | ConvertFrom-Json
    Assert-Equal ([string]$stateInFlight.status) "in-flight" "release state must be in-flight"
    Assert-Equal ([string]$stateInFlight.releaseToolUseId) "release-success" "release must bind the tool_use_id"

    Assert-AllowedResult (Invoke-Pre -Command "git status --porcelain" -ToolUseId "ordinary-during-release") "ordinary allow while release awaits Post" -ExpectReleaseWarning
    $wrongPost = Invoke-Post -EventName "PostToolUse" -Command $releasedCommand -ToolUseId "wrong-id"
    Assert-Equal $wrongPost.ExitCode 0 "wrong PostToolUse id hook exit" $wrongPost.Combined
    Assert-Contains $wrongPost.Combined "HOOK_HEALTH_RELEASED" "wrong PostToolUse id must keep release visible"
    Assert-True (Test-Path -LiteralPath $paths.State -PathType Leaf) "wrong PostToolUse id must not clear state"

    $successPost = Invoke-Post -EventName "PostToolUse" -Command $releasedCommand -ToolUseId "release-success"
    Assert-Equal $successPost.ExitCode 0 "matching PostToolUse success hook exit" $successPost.Combined
    Assert-True (-not (Test-Path -LiteralPath $paths.State)) "matching success must clear active state"
    Assert-True (Test-Path -LiteralPath $paths.Audit -PathType Leaf) "matching success must retain audit"
    $audit = [IO.File]::ReadAllText($paths.Audit, [Text.Encoding]::UTF8)
    foreach ($event in @('"event":"deny"', '"event":"ack-delete-control"', '"event":"acknowledged"', '"event":"release-used"', '"event":"release-success"')) {
        Assert-Contains $audit $event "audit must retain $event"
    }
    [void](Assert-DeniedResult (Invoke-Pre -Command $releasedCommand -ToolUseId "receipt-reuse") "successful receipt cannot be reused")

    Write-Host "[8/8] hash mismatch, failure, corrupt state, and repository isolation"
    Reset-ActiveState
    [void](Assert-DeniedResult (Invoke-Pre -Command $releasedCommand -ToolUseId "hash-deny") "hash mismatch setup")
    $paths = Get-StatePaths
    $originalState = [IO.File]::ReadAllText($paths.State, [Text.Encoding]::UTF8) | ConvertFrom-Json
    Remove-Item -LiteralPath $paths.Acknowledgement -Force
    [void](Assert-DeniedResult (Invoke-Pre -Command "npm test" -ToolUseId "hash-other") "different hash cannot use release")
    $replacementState = [IO.File]::ReadAllText($paths.State, [Text.Encoding]::UTF8) | ConvertFrom-Json
    Assert-True ([string]$replacementState.commandHash -ne [string]$originalState.commandHash) "hash mismatch denial must issue a receipt for the new command"

    Reset-ActiveState
    [void](Assert-DeniedResult (Invoke-Pre -Command $releasedCommand -ToolUseId "failure-deny") "failure setup denial")
    $paths = Get-StatePaths
    Remove-Item -LiteralPath $paths.Acknowledgement -Force
    Assert-AllowedResult (Invoke-Pre -Command $releasedCommand -ToolUseId "release-failure") "failure-path release" -ExpectReleaseWarning
    $failurePost = Invoke-Post -EventName "PostToolUseFailure" -Command $releasedCommand -ToolUseId "release-failure" -ErrorText "simulated failure"
    Assert-Equal $failurePost.ExitCode 0 "PostToolUseFailure hook exit" $failurePost.Combined
    Assert-Contains $failurePost.Combined "HOOK_HEALTH_DEGRADED" "PostToolUseFailure must disclose reblock"
    Assert-True (Test-Path -LiteralPath $paths.Acknowledgement -PathType Leaf) "PostToolUseFailure must recreate acknowledgement"
    $failedState = [IO.File]::ReadAllText($paths.State, [Text.Encoding]::UTF8) | ConvertFrom-Json
    Assert-Equal ([string]$failedState.status) "blocked" "PostToolUseFailure must reblock"
    [void](Assert-DeniedResult (Invoke-Pre -Command $releasedCommand -ToolUseId "failure-retry") "failed release cannot be reused without acknowledgement")

    Reset-ActiveState
    $paths = Get-StatePaths
    [void][IO.Directory]::CreateDirectory($paths.Directory)
    [IO.File]::WriteAllText($paths.State, "{broken-state", [Text.Encoding]::ASCII)
    $broken = Invoke-Pre -Command "git status" -ToolUseId "broken-state"
    $brokenReason = Assert-DeniedResult $broken "broken state fails closed"
    Assert-Contains $brokenReason "state is unreadable" "broken state denial must identify state corruption"
    Assert-True (Test-Path -LiteralPath $paths.Acknowledgement -PathType Leaf) "broken state denial must issue a recovery acknowledgement"
    [void]([IO.File]::ReadAllText($paths.State, [Text.Encoding]::UTF8) | ConvertFrom-Json)
    $script:Assertions++

    Reset-ActiveState
    Reset-ActiveState $Repository2
    [void](Assert-DeniedResult (Invoke-Pre -Command "cargo test" -ToolUseId "repo-one" -RepositoryRoot $Repository) "repository one receipt")
    [void](Assert-DeniedResult (Invoke-Pre -Command "cargo test" -ToolUseId "repo-two" -RepositoryRoot $Repository2) "repository two receipt")
    $repoOnePaths = Get-StatePaths $Repository
    $repoTwoPaths = Get-StatePaths $Repository2
    Assert-True ($repoOnePaths.Key -ne $repoTwoPaths.Key) "repository keys must differ"
    Assert-True (Test-Path -LiteralPath $repoOnePaths.Acknowledgement -PathType Leaf) "repository one acknowledgement must exist"
    Assert-True (Test-Path -LiteralPath $repoTwoPaths.Acknowledgement -PathType Leaf) "repository two acknowledgement must exist"

    Write-Output ("test result: {0} cases, {1} assertions, 0 failures" -f $script:Cases, $script:Assertions)
    exit 0
}
finally {
    [Console]::InputEncoding = $script:OriginalInputEncoding
    Remove-TestSandbox
}
