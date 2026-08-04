# ORIGAMI3 一括検査スクリプト(Windows PowerShell 5.1 対応)
# 4つの検査を順に実行し、いずれかが失敗したら非0で終了する。
#   (1) cargo test --workspace
#   (2) cargo clippy --workspace --all-targets -- -D warnings
#   (3) apps/desktop で npm run build
#   (4) apps/desktop で npm run lint

$root = Split-Path -Parent $PSScriptRoot

function Invoke-Check {
    param([string]$Name, [string]$Command, [string[]]$CommandArgs)
    Write-Host ""
    Write-Host "=== $Name ===" -ForegroundColor Cyan
    # 直前のコマンドの終了コードが残って偽の合格にならないよう必ずリセットする
    $global:LASTEXITCODE = 0
    # 注意: スクリプトブロック経由(& { ... })だと起動失敗時に $? が真のまま残るため、
    # コマンドと引数を直接 & で呼び出す形にしている
    & $Command @CommandArgs
    $started = $?
    if (-not $started -or $LASTEXITCODE -ne 0) {
        $code = $LASTEXITCODE
        if ($code -eq 0) { $code = 1 }  # コマンドが起動すらできなかった場合など
        Write-Host ""
        Write-Host "[NG] $Name が失敗しました(終了コード: $code)" -ForegroundColor Red
        exit $code
    }
}

Push-Location $root
try {
    Invoke-Check "(1/4) cargo test --workspace" cargo @("test", "--workspace")
    Invoke-Check "(2/4) cargo clippy --workspace --all-targets -- -D warnings" cargo @("clippy", "--workspace", "--all-targets", "--", "-D", "warnings")

    Set-Location (Join-Path $root "apps\desktop")

    Invoke-Check "(3/4) npm run build (apps/desktop)" npm @("run", "build")
    Invoke-Check "(4/4) npm run lint (apps/desktop)" npm @("run", "lint")

    Write-Host ""
    Write-Host "[OK] 全ての検査に合格しました" -ForegroundColor Green
}
finally {
    Pop-Location
}
exit 0
