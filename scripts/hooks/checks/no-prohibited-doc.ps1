[CmdletBinding()]
param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [AllowEmptyCollection()]
    [string[]]$CommitPaths = @()
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

$prohibitedPath = "docs/competitive-review-2026-08-20.md"
$violations = @($CommitPaths | Where-Object {
    [string]::Equals((ConvertTo-RepositoryPath $_), $prohibitedPath, [System.StringComparison]::OrdinalIgnoreCase)
})

if ($violations.Count -gt 0) {
    [Console]::Error.WriteLine("[NG] 利用者が変更を禁止した文書がコミット対象に含まれています: $prohibitedPath")
    [Console]::Error.WriteLine("     この文書をステージ対象から外してください。")
    exit 1
}

$targetCount = @($CommitPaths | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }).Count
Write-Output "[OK] 変更禁止文書はコミット対象に含まれていません（対象 $targetCount 件）。"
exit 0
