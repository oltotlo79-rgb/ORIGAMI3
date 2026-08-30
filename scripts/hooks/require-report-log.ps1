# 統括が「報告と同時に記録する」規約を守れるようにするための物的な歯止め。
#
# docs/報告記録.md が一定時間更新されていない間は、担当へ指示を出す道具
# (mcp__codex__codex / mcp__codex__codex-reply / Agent / SendMessage) を拒否する。
# 初回投入（mcp__codex__codex / Agent）は同時に check-agent-instruction.ps1 の
# 14項目をstdin経由で検査する。検査未接続のまま自己試験だけ緑になる穴を作らない。
# 記録そのもの (Bash / Write / Edit) は決して止めないので、詰まることはない。
#
# 2026-08-30: 統括が05:01〜12:11の6回ぶんの報告を記録し忘れた。監視も定義を
# 古いまま放置して出力を見ていなかった。いずれも「守らなくてもその場では何も
# 止まらない」規約で、心構えでは直らないと判断して機械化した。
$ErrorActionPreference = 'Stop'
$script:Utf8Strict = New-Object Text.UTF8Encoding($false, $true)
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)

$StaleMinutes = 90
$HookHealthThreshold = 2

function Read-Utf8StandardInput {
    $stream = [Console]::OpenStandardInput()
    $reader = New-Object IO.StreamReader($stream, $script:Utf8Strict, $false)
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

function Write-Utf8StandardInput {
    param(
        [Parameter(Mandatory = $true)]$Process,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text
    )

    $bytes = (New-Object Text.UTF8Encoding($false)).GetBytes($Text)
    if ($bytes.Length -gt 0) {
        $Process.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length)
    }
    $Process.StandardInput.BaseStream.Close()
}

function ConvertTo-ProcessArgumentString {
    param([Parameter(Mandatory = $true)][string[]]$Values)

    $parts = foreach ($value in $Values) {
        $escaped = [regex]::Replace($value, '(\\*)"', '$1$1\\"')
        $trailingBackslashes = [regex]::Match($escaped, '\\*$').Value
        $escaped = $escaped + $trailingBackslashes
        '"' + $escaped + '"'
    }
    return ($parts -join ' ')
}

function Invoke-HookHealthCheck {
    param(
        [Parameter(Mandatory = $true)][string]$ScriptPath,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][int]$Threshold
    )

    $powerShellPath = (Get-Process -Id $PID).Path
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $powerShellPath
    $startInfo.Arguments = ConvertTo-ProcessArgumentString -Values @(
        '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
        '-File', $ScriptPath,
        '-Action', 'Check',
        '-RepositoryRoot', $RepositoryRoot,
        '-Threshold', [string]$Threshold
    )
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [Text.Encoding]::UTF8
    $startInfo.StandardErrorEncoding = [Text.Encoding]::UTF8
    $process = [Diagnostics.Process]::Start($startInfo)
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    return [PSCustomObject]@{
        ExitCode = $process.ExitCode
        Output = ($stdout + $stderr).Trim()
    }
}

function Invoke-AgentInstructionCheck {
    param(
        [Parameter(Mandatory = $true)][string]$ScriptPath,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$InstructionText,
        [string]$ExpectedModel = ''
    )

    $arguments = @(
        '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
        '-File', $ScriptPath,
        '-ReadFromStdin',
        '-RepositoryRoot', $RepositoryRoot,
        '-VerifyLiveBaseline'
    )
    if (-not [string]::IsNullOrWhiteSpace($ExpectedModel)) {
        $arguments += @('-ExpectedModel', $ExpectedModel)
    }

    $powerShellPath = (Get-Process -Id $PID).Path
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $powerShellPath
    $startInfo.Arguments = ConvertTo-ProcessArgumentString -Values $arguments
    $startInfo.WorkingDirectory = $RepositoryRoot
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [Text.Encoding]::UTF8
    $startInfo.StandardErrorEncoding = [Text.Encoding]::UTF8
    $process = [Diagnostics.Process]::Start($startInfo)
    Write-Utf8StandardInput -Process $process -Text $InstructionText
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    return [PSCustomObject]@{
        ExitCode = $process.ExitCode
        Output = ($stdout + $stderr).Trim()
    }
}

function Resolve-RepositoryRoot {
    $root = [string]$env:CLAUDE_PROJECT_DIR
    if ([string]::IsNullOrWhiteSpace($root)) {
        $scriptsDirectory = Split-Path -Parent $PSScriptRoot
        $root = Split-Path -Parent $scriptsDirectory
    }
    if ([string]::IsNullOrWhiteSpace($root)) {
        throw 'repository rootを解決できません。'
    }
    $resolved = [IO.Path]::GetFullPath($root).TrimEnd([char[]]'\/')
    if (-not (Test-Path -LiteralPath $resolved -PathType Container)) {
        throw "repository rootが存在しません: $resolved"
    }
    return $resolved
}

function Get-ObjectStringProperty {
    param(
        [AllowNull()]$Object,
        [Parameter(Mandatory = $true)][string]$Name
    )

    if ($null -eq $Object) { return '' }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property -or $null -eq $property.Value) { return '' }
    return [string]$property.Value
}

function Get-InitialInstructionText {
    param([AllowNull()]$ToolInput)

    if ($ToolInput -is [string] -and -not [string]::IsNullOrWhiteSpace([string]$ToolInput)) {
        return [string]$ToolInput
    }
    foreach ($name in @('prompt', 'instruction', 'message', 'task')) {
        $value = Get-ObjectStringProperty -Object $ToolInput -Name $name
        if (-not [string]::IsNullOrWhiteSpace($value)) {
            return $value
        }
    }
    return ''
}

function Get-ExpectedModelName {
    param([AllowNull()]$ToolInput)

    $rawModel = (Get-ObjectStringProperty -Object $ToolInput -Name 'model').Trim().ToLowerInvariant()
    if ([string]::IsNullOrWhiteSpace($rawModel)) { return '' }
    if (@('opus', 'sonnet', 'gpt-5.6-sol') -contains $rawModel) { return $rawModel }
    throw "投入toolのmodelが規約の許可値ではありません: $rawModel"
}

function Write-ToolDeny {
    param([Parameter(Mandatory = $true)][string]$Reason)

    $out = [ordered]@{
        hookSpecificOutput = [ordered]@{
            hookEventName            = 'PreToolUse'
            permissionDecision       = 'deny'
            permissionDecisionReason = $Reason
        }
    }
    $out | ConvertTo-Json -Depth 5 -Compress | Write-Output
    exit 0
}

try {
    $raw = Read-Utf8StandardInput
    if ([string]::IsNullOrWhiteSpace($raw)) {
        Write-ToolDeny 'DELEGATION_GUARD_ERROR: PreToolUse payloadが空です。担当へ指示する前にhook入力経路を復旧してください。'
    }

    $payload = $raw | ConvertFrom-Json
    $tool = [string]$payload.tool_name
    if ([string]::IsNullOrWhiteSpace($tool)) {
        Write-ToolDeny 'DELEGATION_GUARD_ERROR: PreToolUse payloadにtool_nameがありません。'
    }

    $gated = @(
        'mcp__codex__codex',
        'mcp__codex__codex-reply',
        'Agent',
        'SendMessage'
    )
    if ($gated -notcontains $tool) { exit 0 }

    # Claude Code 2.1.251のhook契約ではagent_idはsubagent内だけに存在する。
    # 本検査は統括の委譲境界だけを対象にし、作業担当からのtool利用には介入しない。
    $agentIdProperty = $payload.PSObject.Properties['agent_id']
    if ($null -ne $agentIdProperty -and $agentIdProperty.Value -is [string] -and
        -not [string]::IsNullOrWhiteSpace([string]$agentIdProperty.Value)) {
        exit 0
    }

    $root = Resolve-RepositoryRoot
    $log = Join-Path $root 'docs/報告記録.md'
    if (-not (Test-Path -LiteralPath $log -PathType Leaf)) {
        Write-ToolDeny 'REPORT_LOG_MISSING: docs/報告記録.md が見つかりません。担当への指示前に記録を復旧してください。'
    }

    $healthScript = Join-Path $root 'scripts\hooks\checks\hook-health.ps1'
    if (-not (Test-Path -LiteralPath $healthScript -PathType Leaf)) {
        Write-ToolDeny 'HOOK_HEALTH_DEGRADED: フック健全性検査台本が見つかりません。担当への次の指示を出す前に scripts/hooks/checks/hook-health.ps1 を復旧してください。'
    }
    $health = Invoke-HookHealthCheck -ScriptPath $healthScript -RepositoryRoot $root -Threshold $HookHealthThreshold
    if ($health.ExitCode -ne 0) {
        $healthDetail = [regex]::Replace($health.Output, '\s+', ' ').Trim()
        if ([string]::IsNullOrWhiteSpace($healthDetail)) {
            $healthDetail = "HOOK_HEALTH_DEGRADED: health checker exit=$($health.ExitCode)"
        }
        Write-ToolDeny (
            "$healthDetail 担当への次の指示を出す前に、該当する検査を正常に1回実行して連続失敗数を0へ戻してください。"
        )
    }
    if ($health.Output -match 'HOOK_HEALTH_RELEASED') {
        # 解除は作業継続を許すが、次の成功まで解除済みであることを隠さない。
        Write-Warning ([regex]::Replace($health.Output, '\s+', ' ').Trim())
    }

    $age = (New-TimeSpan -Start (Get-Item -LiteralPath $log).LastWriteTime -End (Get-Date)).TotalMinutes
    if ($age -gt $StaleMinutes) {
        $reason = @(
            ("docs/報告記録.md が {0:N0} 分間更新されていません（上限 {1} 分）。" -f $age, $StaleMinutes),
            "規約は「利用者へ報告するのと同時に、統括が docs/報告記録.md へ書く」と定めています。",
            "担当へ指示を出す前に、直近の報告・判断・実測値を記録してください。",
            "記録そのもの（Bash / Write / Edit）はこの検査で止まりません。"
        ) -join ' '

        Write-ToolDeny $reason
    }

    $initialTools = @('mcp__codex__codex', 'Agent')
    if ($initialTools -contains $tool) {
        $instruction = Get-InitialInstructionText -ToolInput $payload.tool_input
        if ([string]::IsNullOrWhiteSpace($instruction)) {
            Write-ToolDeny 'AGENT_INSTRUCTION_MISSING: 初回投入payloadからprompt/instruction/message/taskを取得できません。14項目を含む指示文をtool_inputへ渡してください。'
        }
        $checkerPath = Join-Path $root 'scripts\check-agent-instruction.ps1'
        if (-not (Test-Path -LiteralPath $checkerPath -PathType Leaf)) {
            Write-ToolDeny 'AGENT_INSTRUCTION_CHECKER_MISSING: scripts/check-agent-instruction.ps1 が見つかりません。'
        }
        $expectedModel = Get-ExpectedModelName -ToolInput $payload.tool_input
        $instructionCheck = Invoke-AgentInstructionCheck -ScriptPath $checkerPath -RepositoryRoot $root -InstructionText $instruction -ExpectedModel $expectedModel
        if ($instructionCheck.ExitCode -ne 0) {
            $detail = [regex]::Replace($instructionCheck.Output, '\s+', ' ').Trim()
            if ($detail.Length -gt 6000) { $detail = $detail.Substring(0, 6000) + ' …(truncated)' }
            if ([string]::IsNullOrWhiteSpace($detail)) { $detail = "checker exit=$($instructionCheck.ExitCode)" }
            Write-ToolDeny ("AGENT_INSTRUCTION_NG: $detail 指示書へモデルと理由、HEAD・未コミット件数・対象検査baseline、対象実名ごとのrg/grepコマンドと実出力を追加してください。")
        }
    }

    exit 0
}
catch {
    Write-ToolDeny ("DELEGATION_GUARD_ERROR: 検査を完了できないためfail-closedで拒否します: {0}" -f $_.Exception.Message)
}
