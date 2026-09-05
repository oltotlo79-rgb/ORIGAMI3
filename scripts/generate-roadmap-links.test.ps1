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

# 2026-09-05: `subject:`selectorはgitが出す日本語のcommit subjectをOrdinalで
# 比べる。PowerShellは外部commandの出力を [Console]::OutputEncoding で復号する
# ため、この値がUTF-8でない作業機(実測: cp932)では本当は一致しているsubjectが
# 壊れ、3件が `authoritative selector is not resolved` になって上の33行目の
# 診断まで到達しなかった。判定がコードページに依存しないことを、UTF-8でない
# コードページを明示的に立てた別processで確かめる(C08と同じ形の環境依存欠陥)。
$sutSource = [IO.File]::ReadAllText($sut, (New-Object Text.UTF8Encoding($false, $true)))
Assert-True ($sutSource -match '\[Console\]::OutputEncoding\s*=\s*New-Object\s+System\.Text\.UTF8Encoding') "git出力の復号をUTF-8へ固定していません"
# [Console]::OutputEncoding への代入は SetConsoleOutputCP を呼ぶので、同じ
# コンソールを共有する親・兄弟processまで巻き込む。cp437 を敷いたまま抜けると、
# 後段の自己試験が捕まえる日本語が `?` に潰れて誤って赤になる(実測: governanceの
# 7/11・9/11・11/11)。子でも親でも必ず元へ戻し、最後に戻ったことを表明する。
# 下の照合語はすべてASCIIなので、子が cp437 で書いても親の復号で壊れない。
$consoleEncodingBefore = $null
try { $consoleEncodingBefore = [Console]::OutputEncoding } catch { $consoleEncodingBefore = $null }
$nonUtf8Command = '$prev = [Console]::OutputEncoding; ' +
    '[Console]::OutputEncoding = [Text.Encoding]::GetEncoding(437); ' +
    'if ([Console]::OutputEncoding.CodePage -ne 437) { Write-Output "[TEST SETUP] cannot force cp437"; exit 3 }; ' +
    '& "' + $sut + '" -Check; $code = $LASTEXITCODE; ' +
    'try { [Console]::OutputEncoding = $prev } catch { }; exit $code'
$global:LASTEXITCODE = 0
try {
    $nonUtf8Output = @(& $powershellExe -NoProfile -ExecutionPolicy Bypass -Command $nonUtf8Command 2>&1)
    $nonUtf8Exit = $LASTEXITCODE
}
finally {
    if ($null -ne $consoleEncodingBefore) {
        try { [Console]::OutputEncoding = $consoleEncodingBefore } catch { }
    }
}
$nonUtf8Text = ($nonUtf8Output -join "`n")
Assert-True ($nonUtf8Exit -ne 3) "cp437を立てられませんでした: $nonUtf8Text"
Assert-True ($nonUtf8Text -notmatch 'authoritative selector is not resolved') "UTF-8でないコードページでsubject selectorが解決できていません: $nonUtf8Text"
Assert-True ($nonUtf8Text -match 'cannot be treated as whole-roadmap verification') "UTF-8でないコードページで部分監査の診断まで到達していません: $nonUtf8Text"
$consoleEncodingAfter = $null
try { $consoleEncodingAfter = [Console]::OutputEncoding } catch { $consoleEncodingAfter = $null }
Assert-True (
    ($null -eq $consoleEncodingBefore -and $null -eq $consoleEncodingAfter) -or
    ($null -ne $consoleEncodingBefore -and $null -ne $consoleEncodingAfter -and
        $consoleEncodingBefore.CodePage -eq $consoleEncodingAfter.CodePage)
) "コンソールのコードページを元へ戻していません (前=$(if ($null -ne $consoleEncodingBefore) { $consoleEncodingBefore.CodePage } else { 'none' }) 後=$(if ($null -ne $consoleEncodingAfter) { $consoleEncodingAfter.CodePage } else { 'none' }))"

$fixtures = Invoke-Sut -Arguments @("-Fixtures")
Assert-True ($fixtures.ExitCode -eq 0) "既存fixtureが失敗しました: $($fixtures.Text)"
Assert-True ($fixtures.Text -match '\[FIXTURE\] cases=11 passed=11') "fixture件数と監査件数を別ラベルで表示していません"
Assert-True ($fixtures.Text -match "\[PARTIAL\] scope=M0 audited=11/$whole") "fixture時に全体母数を表示していません"

Write-Host "[TEST OK] generate-roadmap-links: $script:assertions assertions"
exit 0
