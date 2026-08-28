[CmdletBinding()]
param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [AllowEmptyCollection()]
    [string[]]$CommitPaths = @(),

    [Parameter()]
    [AllowEmptyString()]
    [string]$CommitMessageBody,

    [Parameter()]
    [string]$CommitMessagePath
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

function ConvertTo-RepositoryPath {
    param([AllowNull()][string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        return ""
    }
    $normalized = $Path.Replace('\', '/')
    while ($normalized.StartsWith("./", [System.StringComparison]::Ordinal)) {
        $normalized = $normalized.Substring(2)
    }
    return $normalized
}

function Test-ApprovalRequiredPath {
    param([AllowNull()][string]$Path)

    $normalized = ConvertTo-RepositoryPath $Path
    if ([string]::IsNullOrWhiteSpace($normalized)) {
        return $false
    }
    if ([regex]::IsMatch($normalized, '(^|/)Cargo\.(toml|lock)$', [System.Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        return $true
    }
    return [regex]::IsMatch($normalized, '^vendor(?:/|$)', [System.Text.RegularExpressions.RegexOptions]::IgnoreCase)
}

$approvalTargets = @($CommitPaths | Where-Object { Test-ApprovalRequiredPath $_ } | Select-Object -Unique)
if ($approvalTargets.Count -eq 0) {
    $targetCount = @($CommitPaths | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }).Count
    Write-Output "[OK] 依存関係の承認が必要な変更はありません（対象 $targetCount 件）。"
    exit 0
}

$hasBody = $PSBoundParameters.ContainsKey("CommitMessageBody")
$hasPath = $PSBoundParameters.ContainsKey("CommitMessagePath") -and -not [string]::IsNullOrWhiteSpace($CommitMessagePath)
if ($hasBody -and $hasPath) {
    [Console]::Error.WriteLine("[NG] コミットメッセージは -CommitMessageBody または -CommitMessagePath のどちらか一方で指定してください。")
    exit 2
}

$body = ""
if ($hasBody) {
    $body = [string]$CommitMessageBody
}
elseif ($hasPath) {
    if (-not (Test-Path -LiteralPath $CommitMessagePath -PathType Leaf)) {
        [Console]::Error.WriteLine("[NG] コミットメッセージのファイルが見つかりません: $CommitMessagePath")
        exit 2
    }
    $fullMessage = [System.IO.File]::ReadAllText((Resolve-Path -LiteralPath $CommitMessagePath), [System.Text.Encoding]::UTF8)
    $firstNewline = $fullMessage.IndexOf("`n", [System.StringComparison]::Ordinal)
    if ($firstNewline -ge 0) {
        $body = $fullMessage.Substring($firstNewline + 1)
    }
}

if (-not [regex]::IsMatch($body, '(?m)^承認:')) {
    [Console]::Error.WriteLine("[NG] Cargo.toml / Cargo.lock / vendor/ の変更には、コミットメッセージ本文で『承認:』から始まる行が必要です（規約 §5）。")
    foreach ($path in $approvalTargets) {
        [Console]::Error.WriteLine("     対象: $path")
    }
    exit 1
}

Write-Output "[OK] 依存関係変更の承認記録を確認しました（対象 $($approvalTargets.Count) 件）。"
exit 0
