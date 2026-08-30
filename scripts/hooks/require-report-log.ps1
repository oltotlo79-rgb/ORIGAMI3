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

    $age = (New-TimeSpan -Start (Get-Item -LiteralPath $log).LastWriteTime -End (Get-Date)).TotalMinutes
    if ($age -le $StaleMinutes) { exit 0 }

    $reason = @(
        ("docs/報告記録.md が {0:N0} 分間更新されていません（上限 {1} 分）。" -f $age, $StaleMinutes),
        "規約は「利用者へ報告するのと同時に、統括が docs/報告記録.md へ書く」と定めています。",
        "担当へ指示を出す前に、直近の報告・判断・実測値を記録してください。",
        "記録そのもの（Bash / Write / Edit）はこの検査で止まりません。"
    ) -join ' '

    $out = [ordered]@{
        hookSpecificOutput = [ordered]@{
            hookEventName            = 'PreToolUse'
            permissionDecision       = 'deny'
            permissionDecisionReason = $reason
        }
    }
    $out | ConvertTo-Json -Depth 5 -Compress | Write-Output
    exit 0
}
catch {
    # 検査自身の誤りで作業を止めない（fail-open）。ただし理由は見えるようにする。
    Write-Warning ("[require-report-log] 検査に失敗したため素通りします: {0}" -f $_.Exception.Message)
    exit 0
}
