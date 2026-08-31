# ロードマップsnapshot・証拠台帳・報告根拠・各負例自己試験をCIから1本で実行する。
# 途中で赤が出ても残りを実行し、planned/begun/invoked/ended receiptで未実行を見える化する。

$ErrorActionPreference = "Stop"
$powershellExe = (Get-Process -Id $PID).Path
$script:failureCount = 0
$script:planned = 0
$script:begun = 0
$script:invoked = 0
$script:ended = 0
$checks = @(
    [pscustomobject]@{ Name = "whole roadmap snapshot"; Script = "get-roadmap-status.ps1"; Arguments = @("-Format", "Text") },
    [pscustomobject]@{ Name = "traceability byte freshness"; Script = "doc-link-audit.ps1"; Arguments = @("-CheckTraceability") },
    [pscustomobject]@{ Name = "report claim evidence"; Script = "check-report-log.ps1"; Arguments = @() },
    [pscustomobject]@{ Name = "roadmap snapshot negative tests"; Script = "get-roadmap-status.test.ps1"; Arguments = @() },
    [pscustomobject]@{ Name = "partial scope negative tests"; Script = "generate-roadmap-links.test.ps1"; Arguments = @() },
    [pscustomobject]@{ Name = "traceability drift negative tests"; Script = "doc-link-audit.test.ps1"; Arguments = @() },
    [pscustomobject]@{ Name = "report claim negative tests"; Script = "check-report-log.test.ps1"; Arguments = @() },
    [pscustomobject]@{ Name = "release gate production-path test"; Script = "check-release-ready.test.ps1"; Arguments = @() },
    [pscustomobject]@{ Name = "CI contract negative tests"; Script = "check-ci.test.ps1"; Arguments = @() }
)
$script:planned = $checks.Count

for ($index = 0; $index -lt $script:planned; $index++) {
    $number = $index + 1
    $check = $checks[$index]
    $stageInvoked = 0
    $checkExit = [int]::MinValue
    $script:begun++
    Write-Host ""
    Write-Host "=== BEGIN ($number/$($script:planned)) $($check.Name) ===" -ForegroundColor Cyan
    try {
        $scriptPath = Join-Path $PSScriptRoot $check.Script
        if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
            throw "検査scriptがありません: $scriptPath"
        }
        $global:LASTEXITCODE = [int]::MinValue
        $checkArguments = @($check.Arguments)
        & $powershellExe -NoProfile -ExecutionPolicy Bypass -File $scriptPath @checkArguments
        $checkExit = $LASTEXITCODE
        if ($checkExit -eq [int]::MinValue) {
            throw "検査scriptから終了コードが返りませんでした: $scriptPath"
        }
        $stageInvoked = 1
        $script:invoked++
        if ($checkExit -ne 0) {
            $script:failureCount++
            Write-Host "[NG] $($check.Name) failed (exit=$checkExit)" -ForegroundColor Red
        }
        else {
            Write-Host "[OK] $($check.Name) passed (exit=0)" -ForegroundColor Green
        }
    }
    catch {
        $script:failureCount++
        Write-Host "[NG] $($check.Name) could not run: $($_.Exception.Message)" -ForegroundColor Red
    }
    finally {
        $script:ended++
        $stageExit = if ($stageInvoked -eq 1) { [string]$checkExit } else { "not-run" }
        Write-Host "ROADMAP_GOVERNANCE_STAGE number=$number planned=$($script:planned) script=$($check.Script) invoked=$stageInvoked exit=$stageExit"
        Write-Host "=== END ($number/$($script:planned)) $($check.Name) ===" -ForegroundColor Cyan
    }
}

$coverageOk = $script:begun -eq $script:planned -and
    $script:invoked -eq $script:planned -and
    $script:ended -eq $script:planned
if (-not $coverageOk) {
    $script:failureCount++
    Write-Host "[NG] 計画したroadmap governance検査をすべて実行できませんでした。" -ForegroundColor Red
}
Write-Host ""
Write-Host "ROADMAP_GOVERNANCE_STAGES planned=$($script:planned) begun=$($script:begun) invoked=$($script:invoked) ended=$($script:ended) failures=$($script:failureCount)"

if ($script:failureCount -gt 0) {
    Write-Host "[NG] roadmap governance failed: failures=$($script:failureCount)" -ForegroundColor Red
    exit 1
}
Write-Host "[OK] roadmap governance passed: failures=0" -ForegroundColor Green
exit 0
