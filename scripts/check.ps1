# ORIGAMI3 一括検査スクリプト(Windows PowerShell 5.1 対応)
# 4つの検査を順に実行し、いずれかが失敗したら非0で終了する。
#   (1) cargo test --workspace
#   (2) cargo clippy --workspace --all-targets -- -D warnings
#   (3) apps/desktop で npm run build
#   (4) apps/desktop で npm run lint

$root = Split-Path -Parent $PSScriptRoot

function Invoke-Check {
    param([string]$Name, [scriptblock]$Body)
    Write-Host ""
    Write-Host "=== $Name ===" -ForegroundColor Cyan
    & $Body
    if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        Write-Host "[NG] $Name が失敗しました(終了コード: $LASTEXITCODE)" -ForegroundColor Red
        Pop-Location
        exit 1
    }
}

Push-Location $root

Invoke-Check "(1/4) cargo test --workspace" { cargo test --workspace }
Invoke-Check "(2/4) cargo clippy --workspace --all-targets -- -D warnings" { cargo clippy --workspace --all-targets -- -D warnings }

Set-Location (Join-Path $root "apps\desktop")

Invoke-Check "(3/4) npm run build (apps/desktop)" { npm run build }
Invoke-Check "(4/4) npm run lint (apps/desktop)" { npm run lint }

Pop-Location
Write-Host ""
Write-Host "[OK] 全ての検査に合格しました" -ForegroundColor Green
exit 0
