# generate-roadmap-links.ps1 の本番呼出し自己試験。
# 実roadmap・実generator・実snapshotを別processで使い、部分監査の誤緑を防ぐ。

$ErrorActionPreference = "Stop"
$sut = Join-Path $PSScriptRoot "generate-roadmap-links.ps1"
$snapshotSut = Join-Path $PSScriptRoot "get-roadmap-status.ps1"
$powershellExe = (Get-Process -Id $PID).Path
$script:assertions = 0

function Assert-True {
    param([bool]$Condition, [string]$Message)
    $script:assertions++
    if (-not $Condition) { throw "[TEST NG] $Message" }
}

function Invoke-Sut {
    param([string[]]$Arguments)
    $global:LASTEXITCODE = 0
    $output = @(& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $sut @Arguments 2>&1)
    return [pscustomobject]@{
        ExitCode = $LASTEXITCODE
        Text = ($output -join "`n")
    }
}

$global:LASTEXITCODE = 0
$snapshotOutput = @(& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $snapshotSut -Format Json)
Assert-True ($LASTEXITCODE -eq 0 -and $snapshotOutput.Count -eq 1) "whole snapshotを取得できません"
$whole = [int](([string]$snapshotOutput[0] | ConvertFrom-Json).total)

$plain = Invoke-Sut -Arguments @("-Check")
Assert-True ($plain.ExitCode -eq 1) "11/$whole の部分監査を明示承認なしで終了0にしました: $($plain.Text)"
Assert-True ($plain.Text -match "\[PARTIAL\] scope=M0 audited=11/$whole partial=true full_coverage=false") "11/$whole の対象範囲を表示していません"
Assert-True ($plain.Text -match 'cannot be treated as whole-roadmap verification') "部分監査を全体検証と見なせない診断がありません"

$allowed = Invoke-Sut -Arguments @("-Check", "-AllowPartialScope", "M0")
Assert-True ($allowed.ExitCode -eq 0) "M0だけを意図した明示呼出しが失敗しました: $($allowed.Text)"
Assert-True ($allowed.Text -match "\[PARTIAL\] scope=M0 audited=11/$whole") "明示呼出しでも部分監査表示がありません"
Assert-True ($allowed.Text -match 'explicitly accepted') "明示承認したことを表示していません"

$fixtures = Invoke-Sut -Arguments @("-Fixtures")
Assert-True ($fixtures.ExitCode -eq 0) "既存fixtureが失敗しました: $($fixtures.Text)"
Assert-True ($fixtures.Text -match '\[FIXTURE\] cases=11 passed=11') "fixture件数と監査件数を別ラベルで表示していません"
Assert-True ($fixtures.Text -match "\[PARTIAL\] scope=M0 audited=11/$whole") "fixture時に全体母数を表示していません"

Write-Host "[TEST OK] generate-roadmap-links: $script:assertions assertions"
exit 0
