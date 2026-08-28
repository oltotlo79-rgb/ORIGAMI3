[CmdletBinding()]
param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [AllowEmptyCollection()]
    [string[]]$CommitPaths = @(),

    [Parameter()]
    [string]$RepositoryRoot = (Get-Location).Path
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$paths = @($CommitPaths | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -Unique)
if ($paths.Count -eq 0) {
    Write-Output "[OK] #[allow( の追加差分はありません（対象 0 件）。"
    exit 0
}

if (-not (Test-Path -LiteralPath $RepositoryRoot -PathType Container)) {
    [Console]::Error.WriteLine("[NG] リポジトリの場所が存在しません: $RepositoryRoot")
    exit 2
}

$repository = [System.IO.Path]::GetFullPath($RepositoryRoot)
$violations = New-Object System.Collections.Generic.List[string]

foreach ($path in $paths) {
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        $diffLines = @(& git -c core.excludesFile=NUL -C $repository --literal-pathspecs diff --cached --no-ext-diff --unified=0 --no-color --diff-filter=ACMR -- $path 2>$null)
        $gitExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($gitExitCode -ne 0) {
        [Console]::Error.WriteLine("[NG] 追加差分を取得できませんでした: $path（git 終了コード: $gitExitCode）")
        exit 2
    }

    $hasViolation = $false
    $insideHunk = $false
    foreach ($line in $diffLines) {
        $text = [string]$line
        if ($text.StartsWith("@@", [System.StringComparison]::Ordinal)) {
            $insideHunk = $true
            continue
        }
        if ($insideHunk -and [regex]::IsMatch($text, '^\+.*#\[allow\(')) {
            $hasViolation = $true
            break
        }
    }
    if ($hasViolation) {
        $violations.Add($path)
    }
}

if ($violations.Count -gt 0) {
    [Console]::Error.WriteLine("[NG] 追加された差分に #[allow( が含まれています。警告の原因を修正し、属性で隠さないでください（規約 §5）。")
    foreach ($path in $violations) {
        [Console]::Error.WriteLine("     対象: $path")
    }
    exit 1
}

Write-Output "[OK] 追加差分に #[allow( はありません（対象 $($paths.Count) 件）。"
exit 0
