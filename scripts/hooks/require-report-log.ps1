# 統括が「報告と同時に記録する」規約を守れるようにするための物的な歯止め。
#
# docs/報告記録.md が一定時間更新されていない間は、担当へ指示を出す道具
# (mcp__codex__codex / mcp__codex__codex-reply / Agent / SendMessage) を拒否する。
# 記録そのもの (Bash / Write / Edit) は決して止めないので、詰まることはない。
#
# 2026-08-30: 統括が05:01〜12:11の6回ぶんの報告を記録し忘れた。監視も定義を
# 古いまま放置して出力を見ていなかった。いずれも「守らなくてもその場では何も
# 止まらない」規約で、心構えでは直らないと判断して機械化した。
$ErrorActionPreference = 'Stop'

$StaleMinutes = 90
$HookHealthThreshold = 2

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
    $raw = [Console]::In.ReadToEnd()
    if ([string]::IsNullOrWhiteSpace($raw)) { exit 0 }

    # UTF-16 の BOM で来ることがあるため判定する
    $bytes = [Text.Encoding]::Unicode.GetBytes($raw)
    if ($raw.Length -ge 2 -and $raw[0] -eq [char]0xFEFF) { $raw = $raw.Substring(1) }

    $payload = $raw | ConvertFrom-Json
    $tool = [string]$payload.tool_name
    if ([string]::IsNullOrWhiteSpace($tool)) { exit 0 }

    $gated = @(
        'mcp__codex__codex',
        'mcp__codex__codex-reply',
        'Agent',
        'SendMessage'
    )
    if ($gated -notcontains $tool) { exit 0 }

    $root = $env:CLAUDE_PROJECT_DIR
    if ([string]::IsNullOrWhiteSpace($root)) { exit 0 }
    $log = Join-Path $root 'docs/報告記録.md'
    if (-not (Test-Path -LiteralPath $log -PathType Leaf)) { exit 0 }

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
    if ($age -le $StaleMinutes) { exit 0 }

    $reason = @(
        ("docs/報告記録.md が {0:N0} 分間更新されていません（上限 {1} 分）。" -f $age, $StaleMinutes),
        "規約は「利用者へ報告するのと同時に、統括が docs/報告記録.md へ書く」と定めています。",
        "担当へ指示を出す前に、直近の報告・判断・実測値を記録してください。",
        "記録そのもの（Bash / Write / Edit）はこの検査で止まりません。"
    ) -join ' '

    Write-ToolDeny $reason
}
catch {
    # 検査自身の誤りで作業を止めない（fail-open）。ただし理由は見えるようにする。
    Write-Warning ("[require-report-log] 検査に失敗したため素通りします: {0}" -f $_.Exception.Message)
    exit 0
}
