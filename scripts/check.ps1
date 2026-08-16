# ORIGAMI3 一括検査スクリプト(Windows PowerShell 5.1 対応)
# 5つの検査を順に実行し、いずれかが失敗したら非0で終了する。
#   (1) cargo test --workspace
#   (2) cargo clippy --workspace --all-targets -- -D warnings
#   (3) apps/desktop で npm run build
#   (4) apps/desktop で npm run lint
#   (5) apps/desktop で npm run test (vitest)

$root = Split-Path -Parent $PSScriptRoot

function Invoke-Check {
    param([string]$Name, [string]$Command, [string[]]$CommandArgs)
    Write-Host ""
    Write-Host "=== $Name ===" -ForegroundColor Cyan
    # 直前のコマンドの終了コードが残って偽の合格にならないよう必ずリセットする
    $global:LASTEXITCODE = 0
    # 注意: コマンド不在などの起動失敗は文終了エラーになり、外側のtry/finally配下では
    # 後続の判定行に制御が届かない。ここでcatchして確実に失敗終了させる。
    try {
        & $Command @CommandArgs
    } catch {
        Write-Host "[NG] $Name を起動できませんでした: $($_.Exception.Message)" -ForegroundColor Red
        exit 1
    }
    if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        Write-Host "[NG] $Name が失敗しました(終了コード: $LASTEXITCODE)" -ForegroundColor Red
        exit $LASTEXITCODE
    }
}

Push-Location $root
try {
    # テストが追跡対象のファイルを書き換えていないかを、実行の前後で比べる。
    # 途中経過を書き出すテストが tests/fixtures/ の作品ファイルを毎回上書きしており、
    # テストを走らせるだけで作業ツリーが汚れていた。
    # 同じファイルを別のテストが読むため、実行順で読む内容が変わる状態でもあった。
    $global:LASTEXITCODE = 0
    $beforeTracked = (& git -C $root status --porcelain --untracked-files=no) -join "`n"

    Invoke-Check "(1/5) cargo test --workspace" cargo @("test", "--workspace")

    $global:LASTEXITCODE = 0
    $afterTracked = (& git -C $root status --porcelain --untracked-files=no) -join "`n"
    if ($afterTracked -ne $beforeTracked) {
        Write-Host ""
        Write-Host "[NG] テストが追跡対象のファイルを書き換えました" -ForegroundColor Red
        Write-Host "実行前:" -ForegroundColor Yellow
        Write-Host $beforeTracked
        Write-Host "実行後:" -ForegroundColor Yellow
        Write-Host $afterTracked
        exit 1
    }
    Invoke-Check "(2/5) cargo clippy --workspace --all-targets -- -D warnings" cargo @("clippy", "--workspace", "--all-targets", "--", "-D", "warnings")

    Set-Location (Join-Path $root "apps\desktop")

    Invoke-Check "(3/5) npm run build (apps/desktop)" npm @("run", "build")
    Invoke-Check "(4/5) npm run lint (apps/desktop)" npm @("run", "lint")
    Invoke-Check "(5/5) npm run test (apps/desktop)" npm @("run", "test")

    Write-Host ""
    Write-Host "[OK] 全ての検査に合格しました" -ForegroundColor Green
}
finally {
    Pop-Location
}
exit 0
