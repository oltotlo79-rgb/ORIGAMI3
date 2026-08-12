# ORIGAMI3 git hook installer (Windows PowerShell 5.1 compatible)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$hookNames = @("pre-commit", "pre-push")

foreach ($name in $hookNames) {
    $hookPath = Join-Path $repoRoot "scripts\hooks\$name"
    if (-not (Test-Path -LiteralPath $hookPath -PathType Leaf)) {
        throw "$name フックが見つかりません: $hookPath"
    }
}

try {
    $global:LASTEXITCODE = 0
    & git -C $repoRoot config --local core.hooksPath scripts/hooks
    if ($LASTEXITCODE -ne 0) {
        throw "git config が終了コード $LASTEXITCODE で失敗しました"
    }

    $global:LASTEXITCODE = 0
    $configuredPath = ((& git -C $repoRoot config --local --get core.hooksPath) -join "`n").Trim()
    if ($LASTEXITCODE -ne 0 -or $configuredPath -ne "scripts/hooks") {
        throw "core.hooksPath の確認に失敗しました (現在値: $configuredPath)"
    }
}
catch {
    throw "git hook の有効化に失敗しました: $($_.Exception.Message)"
}

Write-Host "[OK] pre-commit / pre-push フックを有効化しました (core.hooksPath=scripts/hooks)" -ForegroundColor Green
