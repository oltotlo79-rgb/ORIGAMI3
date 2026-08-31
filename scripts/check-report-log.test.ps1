[CmdletBinding()]
param()

# 本番check-report-log.ps1を本repoのroadmap/gitへ接続したまま別processで呼ぶ。
# ReportPathだけを、本番報告書を元にした施行後recordへ差し替える。

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
$scriptPath = Join-Path $PSScriptRoot "check-report-log.ps1"
$snapshotPath = Join-Path $PSScriptRoot "get-roadmap-status.ps1"
$repoRoot = Split-Path -Parent $PSScriptRoot
$productionReportPath = Join-Path $repoRoot "docs\報告記録.md"
$roadmapPath = Join-Path $repoRoot "docs\implementation-roadmap.md"
$policyPath = Join-Path $PSScriptRoot "roadmap-status-policy.json"
$attributesPath = Join-Path $repoRoot ".gitattributes"
$powerShellPath = (Get-Process -Id $PID).Path
$tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-check-report-log-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [IO.Path]::GetFullPath((Join-Path $tempParent $sandboxName))
$script:assertions = 0
$script:legacyBoundaryHeader = '## 2026-08-31 10:46 — 未チェックが40件→16件になり、統括が時刻を読めるようになった'
$script:legacySuffixText = ''

function Assert-True {
    param([bool]$Condition, [string]$Message)
    $script:assertions++
    if (-not $Condition) { throw "ASSERTION FAILED: $Message" }
}

function ConvertTo-FullWidthNumber {
    param([Parameter(Mandatory = $true)][string]$Value)
    $builder = New-Object Text.StringBuilder
    foreach ($character in $Value.ToCharArray()) {
        if ($character -ge '0' -and $character -le '9') {
            [void]$builder.Append([char]([int]$character + 0xFEE0))
        }
        elseif ($character -eq '.') {
            [void]$builder.Append([char]0xFF0E)
        }
        else {
            [void]$builder.Append($character)
        }
    }
    return $builder.ToString()
}

function ConvertTo-ProcessArgumentString {
    param([string[]]$Values)
    $parts = foreach ($value in $Values) {
        $escaped = [regex]::Replace([string]$value, '(\*)"', '$1$1\"')
        $trailingBackslashes = [regex]::Match($escaped, '\*$').Value
        '"' + $escaped + $trailingBackslashes + '"'
    }
    return $parts -join " "
}

function Invoke-ProcessCapture {
    param([string[]]$Arguments)
    $previousErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        $output = @(& $powerShellPath @Arguments 2>&1)
        $exitCode = $LASTEXITCODE
        return [pscustomobject]@{
            ExitCode = $exitCode
            Output = ($output -join "`n")
            Arguments = ($Arguments -join "|")
        }
    }
    finally {
        $ErrorActionPreference = $previousErrorAction
    }
}

function Invoke-ReportCheck {
    param([string]$ReportPath)
    return Invoke-ProcessCapture -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $scriptPath, "-ReportPath", $ReportPath
    )
}

function Format-TestRecord {
    param(
        [datetime]$HeadingTime,
        [string]$Title,
        [string[]]$BodyLines
    )
    $heading = "## {0} {1} {2} {3}" -f $HeadingTime.ToString("yyyy-MM-dd"), $HeadingTime.ToString("HH:mm"), [char]0x2014, $Title
    return $heading + "`r`n`r`n" + ($BodyLines -join "`r`n") + "`r`n"
}

function Write-TestDocument {
    param(
        [string]$Name,
        [string[]]$RecordTexts,
        [datetime]$FileTime,
        [string]$LegacySuffix = $script:legacySuffixText
    )
    $path = Join-Path $sandboxRoot ($Name + ".md")
    $text = ($RecordTexts -join "`r`n---`r`n") + "`r`n---`r`n" + $LegacySuffix
    [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding($false)))
    [IO.File]::SetLastWriteTime($path, $FileTime)
    return $path
}

function Write-TestReport {
    param(
        [string]$Name,
        [datetime]$HeadingTime,
        [string]$Title,
        [string[]]$BodyLines,
        [datetime]$FileTime
    )
    $recordText = Format-TestRecord -HeadingTime $HeadingTime -Title $Title -BodyLines $BodyLines
    return Write-TestDocument -Name $Name -RecordTexts @($recordText) -FileTime $FileTime
}

function Assert-Exit {
    param($Result, [int]$Expected, [string]$Name, [string]$Diagnostic = "")
    Assert-True ($Result.ExitCode -eq $Expected) "$Name exit: expected=$Expected actual=$($Result.ExitCode) args=$($Result.Arguments)`n$($Result.Output)"
    if (-not [string]::IsNullOrWhiteSpace($Diagnostic)) {
        Assert-True ($Result.Output -match [regex]::Escape($Diagnostic)) "$Name diagnostic '$Diagnostic' missing`n$($Result.Output)"
    }
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $sandboxRoot)) { return }
    $resolved = [IO.Path]::GetFullPath($sandboxRoot).TrimEnd([char[]]"\/")
    if ([IO.Path]::GetDirectoryName($resolved) -ne $tempParent -or
        [IO.Path]::GetFileName($resolved) -notmatch '^ori3-check-report-log-test-[0-9a-f]{32}$') {
        throw "Refusing unsafe self-test cleanup: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

[void][IO.Directory]::CreateDirectory($sandboxRoot)
try {
    $checkerSource = [IO.File]::ReadAllText($scriptPath, (New-Object Text.UTF8Encoding($false, $true)))
    Assert-True ($checkerSource.Contains('function Get-RecordIntroductionCommit')) "historical evidence must bind the exact record to its introduction commit"
    Assert-True ($checkerSource.Contains("'--reverse', '--follow', 'HEAD'")) "historical search must be limited to the followed HEAD ancestry"
    Assert-True (-not $checkerSource.Contains('--all')) "historical search must not trust snapshots from non-HEAD refs"
    $attributeLines = [regex]::Split(
        [IO.File]::ReadAllText($attributesPath, (New-Object Text.UTF8Encoding($false, $true))),
        "\r\n|\n|\r"
    )
    Assert-True (@($attributeLines | Where-Object { $_ -ceq 'docs/報告記録.md text eol=lf' }).Count -eq 1) "immutable report bytes must remain LF in clean checkouts"

    $productionReportText = [IO.File]::ReadAllText($productionReportPath, (New-Object Text.UTF8Encoding($false, $true)))
    $productionReportLines = [regex]::Split($productionReportText, "\r\n|\n|\r")
    $boundaryIndices = @()
    for ($lineIndex = 0; $lineIndex -lt $productionReportLines.Count; $lineIndex++) {
        if ([string]::Equals($productionReportLines[$lineIndex], $script:legacyBoundaryHeader, [StringComparison]::Ordinal)) {
            $boundaryIndices += $lineIndex
        }
    }
    Assert-True ($boundaryIndices.Count -eq 1) "production legacy boundary must exist exactly once"
    $script:legacySuffixText = $productionReportLines[(($boundaryIndices[0])..($productionReportLines.Count - 1))] -join "`n"

    $snapshotResult = Invoke-ProcessCapture -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $snapshotPath, "-Format", "Json"
    )
    Assert-True ($snapshotResult.ExitCode -eq 0) "production snapshot failed: $($snapshotResult.Output)"
    $snapshot = $snapshotResult.Output.Trim() | ConvertFrom-Json
    Assert-True ([int]$snapshot.total -gt 0 -and [int]$snapshot.audited -eq [int]$snapshot.total -and [int]$snapshot.checked + [int]$snapshot.unchecked -eq [int]$snapshot.total) "production snapshot accounting is invalid"

    $snapshotLine = [string]$snapshot.report_snapshot_line
    $progressLine = [string]$snapshot.report_progress_line
    $expectedPercent = [regex]::Match($progressLine, ' percent=(?<percent>\d+(?:\.\d+)?)$').Groups['percent'].Value
    $checkedItem = @($snapshot.items | Where-Object { $_.state -eq 'checked' })[0]
    $uncheckedItem = @($snapshot.items | Where-Object { $_.state -eq 'unchecked' })[0]
    $boundsLine = "Roadmap-Bounds: ids=$($checkedItem.id),$($uncheckedItem.id) total=2 checked=1 unchecked=1"

    $now = Get-Date
    $headingTime = $now.AddMinutes(-1)
    $legacyBoundaryTimestamp = [datetime]'2026-08-31 10:46'
    Assert-True ($headingTime.AddMinutes(-1) -gt $legacyBoundaryTimestamp) "self-test records must be newer than the immutable boundary"

    $validNonePath = Write-TestReport "valid-none" $headingTime "通常の作業報告" @("Roadmap-Claim: none", "通常の本文です。") $now
    Assert-Exit (Invoke-ReportCheck $validNonePath) 0 "valid none"

    $missingClaimPath = Write-TestReport "missing-claim" $headingTime "残作業の報告" @("残りは2件だけです。") $now
    Assert-Exit (Invoke-ReportCheck $missingClaimPath) 2 "missing claim" "Roadmap-Claim"

    $noneLiePath = Write-TestReport "none-lie" $headingTime "残作業の報告" @("Roadmap-Claim: none", "残りは2件だけです。") $now
    Assert-Exit (Invoke-ReportCheck $noneLiePath) 2 "none completeness claim" "none"

    $missingSnapshotPath = Write-TestReport "missing-snapshot" $headingTime "残作業を全件報告" @("Roadmap-Claim: whole", "これがすべてです。") $now
    Assert-Exit (Invoke-ReportCheck $missingSnapshotPath) 2 "missing snapshot" "Roadmap-Snapshot"

    $staleHash = if ($snapshot.roadmap_sha256.StartsWith('0')) { '1' + $snapshot.roadmap_sha256.Substring(1) } else { '0' + $snapshot.roadmap_sha256.Substring(1) }
    $staleSnapshotLine = $snapshotLine.Replace([string]$snapshot.roadmap_sha256, $staleHash)
    $stalePath = Write-TestReport "stale-hash" $headingTime "残作業を全件報告" @("Roadmap-Claim: whole", $staleSnapshotLine, "これがすべてです。") $now
    Assert-Exit (Invoke-ReportCheck $stalePath) 2 "stale hash" "現在の実測値"

    $partialLine = $snapshotLine.Replace("audited=$($snapshot.audited)/$($snapshot.total)", "audited=11/$($snapshot.total)")
    $partialPath = Write-TestReport "partial" $headingTime "残作業を全件報告" @("Roadmap-Claim: whole", $partialLine, "これがすべてです。") $now
    Assert-Exit (Invoke-ReportCheck $partialPath) 2 "partial 11/$($snapshot.total)" "全件会計"

    $duplicatePath = Write-TestReport "duplicate" $headingTime "残作業を全件報告" @("Roadmap-Claim: whole", $snapshotLine, $snapshotLine, "これがすべてです。") $now
    Assert-Exit (Invoke-ReportCheck $duplicatePath) 2 "duplicate snapshot" "2"

    $validWholePath = Write-TestReport "valid-whole" $headingTime "リリースの残作業を全件報告" @("Roadmap-Claim: whole", $snapshotLine, "正本の全件を照合しました。") $now
    Assert-Exit (Invoke-ReportCheck $validWholePath) 0 "valid whole"

    $unboundedBoundedPath = Write-TestReport "unbounded-bounded" $headingTime "限定した残作業の報告" @("Roadmap-Claim: bounded", $snapshotLine, $boundsLine, "残りは2件だけです。") $now
    Assert-Exit (Invoke-ReportCheck $unboundedBoundedPath) 2 "unbounded remainder in bounded claim" "無限定"

    $validBoundedPath = Write-TestReport "valid-bounded" $headingTime "限定した残作業の報告" @("Roadmap-Claim: bounded", $snapshotLine, $boundsLine, "Roadmap-Bounds の対象ID内に限定した残りは1件だけです。") $now
    Assert-Exit (Invoke-ReportCheck $validBoundedPath) 0 "valid bounded"

    $badBoundsPath = Write-TestReport "bad-bounds" $headingTime "限定した残作業の報告" @("Roadmap-Claim: bounded", $snapshotLine, $boundsLine.Replace('checked=1 unchecked=1', 'checked=2 unchecked=0'), "Roadmap-Bounds の対象ID内に限定した残りは0件だけです。") $now
    Assert-Exit (Invoke-ReportCheck $badBoundsPath) 2 "false bounded counts" "実測と一致しません"

    $badLimitedRemainderPath = Write-TestReport "bad-limited-remainder" $headingTime "限定した残作業の報告" @("Roadmap-Claim: bounded", $snapshotLine, $boundsLine, "Roadmap-Bounds の対象ID内に限定した残りは2件だけです。") $now
    Assert-Exit (Invoke-ReportCheck $badLimitedRemainderPath) 2 "false limited remainder text" "unchecked=1"

    $wrongRemainderCount = if ([int]$snapshot.unchecked -eq 16) { 17 } else { 16 }

    $wholeFalseRemainderPath = Write-TestReport "whole-false-remainder" $headingTime "残作業の全件報告" @("Roadmap-Claim: whole", $snapshotLine, "残りは2件だけです。") $now
    Assert-Exit (Invoke-ReportCheck $wholeFalseRemainderPath) 2 "whole false remainder" "unchecked=$($snapshot.unchecked)"

    $wholeFalseActualCountPath = Write-TestReport "whole-false-actual-count" $headingTime "残作業の本当の数" @("Roadmap-Claim: whole", $snapshotLine, "残作業の本当の数は$($wrongRemainderCount)件です。") $now
    Assert-Exit (Invoke-ReportCheck $wholeFalseActualCountPath) 2 "whole false actual count" "unchecked=$($snapshot.unchecked)"

    $wholeFalseResolutionTargetPath = Write-TestReport "whole-false-resolution-target" $headingTime "解消対象の全件報告" @("Roadmap-Claim: whole", $snapshotLine, "解消対象として残す件数 $($wrongRemainderCount)件") $now
    Assert-Exit (Invoke-ReportCheck $wholeFalseResolutionTargetPath) 2 "whole false resolution target count" "unchecked=$($snapshot.unchecked)"

    $wholeFalseResolutionRemainingPath = Write-TestReport "whole-false-resolution-remaining" $headingTime "解消対象の全件報告" @("Roadmap-Claim: whole", $snapshotLine, "解消対象として残る$($wrongRemainderCount)件") $now
    Assert-Exit (Invoke-ReportCheck $wholeFalseResolutionRemainingPath) 2 "whole false resolution remaining count" "unchecked=$($snapshot.unchecked)"

    $wholeFalseResidualCountPath = Write-TestReport "whole-false-residual-count" $headingTime "残件の全件報告" @("Roadmap-Claim: whole", $snapshotLine, "残件$($wrongRemainderCount)件です。") $now
    Assert-Exit (Invoke-ReportCheck $wholeFalseResidualCountPath) 2 "whole false residual count" "unchecked=$($snapshot.unchecked)"

    $nonePlainCountPath = Write-TestReport "none-plain-count" $headingTime "通常報告" @("Roadmap-Claim: none", "残作業は$($snapshot.unchecked)件です。") $now
    Assert-Exit (Invoke-ReportCheck $nonePlainCountPath) 2 "none plain remainder count" "none"

    $noneResolutionTargetPath = Write-TestReport "none-resolution-target" $headingTime "通常報告" @("Roadmap-Claim: none", "解消対象として残す件数 16件") $now
    Assert-Exit (Invoke-ReportCheck $noneResolutionTargetPath) 2 "none resolution target count" "none"

    $noneResolutionRemainingPath = Write-TestReport "none-resolution-remaining" $headingTime "通常報告" @("Roadmap-Claim: none", "解消対象として残る16件") $now
    Assert-Exit (Invoke-ReportCheck $noneResolutionRemainingPath) 2 "none resolution remaining count" "none"

    $noneResidualCountPath = Write-TestReport "none-residual-count" $headingTime "通常報告" @("Roadmap-Claim: none", "残件$($snapshot.unchecked)件です。") $now
    Assert-Exit (Invoke-ReportCheck $noneResidualCountPath) 2 "none residual count" "none"

    # 利用者への実報告で出た言い回しも、none claimで完全性断言を回避できない。
    $noneUncheckedCountPath = Write-TestReport "none-unchecked-count" $headingTime "通常報告" @("Roadmap-Claim: none", "未チェック40件") $now
    Assert-Exit (Invoke-ReportCheck $noneUncheckedCountPath) 2 "none unchecked count" "none"

    $noneUncheckedCountNounPath = Write-TestReport "none-unchecked-count-noun" $headingTime "通常報告" @("Roadmap-Claim: none", "未チェック件数40") $now
    Assert-Exit (Invoke-ReportCheck $noneUncheckedCountNounPath) 2 "none unchecked count noun" "none"

    $noneIncompleteCountPath = Write-TestReport "none-incomplete-count" $headingTime "通常報告" @("Roadmap-Claim: none", "未完了は16件") $now
    Assert-Exit (Invoke-ReportCheck $noneIncompleteCountPath) 2 "none incomplete count" "none"

    $wholeWrongUncheckedPath = Write-TestReport "whole-wrong-unchecked" $headingTime "未チェック数の全件報告" @("Roadmap-Claim: whole", $snapshotLine, "未チェック$($wrongRemainderCount)件") $now
    Assert-Exit (Invoke-ReportCheck $wholeWrongUncheckedPath) 2 "whole wrong unchecked count" "unchecked=$($snapshot.unchecked)"

    $wholeWrongUncheckedNounPath = Write-TestReport "whole-wrong-unchecked-noun" $headingTime "未チェック数の全件報告" @("Roadmap-Claim: whole", $snapshotLine, "未チェック件数$($wrongRemainderCount)") $now
    Assert-Exit (Invoke-ReportCheck $wholeWrongUncheckedNounPath) 2 "whole wrong unchecked count noun" "unchecked=$($snapshot.unchecked)"

    $wholeWrongIncompletePath = Write-TestReport "whole-wrong-incomplete" $headingTime "未完了数の全件報告" @("Roadmap-Claim: whole", $snapshotLine, "未完了は$($wrongRemainderCount)件") $now
    Assert-Exit (Invoke-ReportCheck $wholeWrongIncompletePath) 2 "whole wrong incomplete count" "unchecked=$($snapshot.unchecked)"

    $wholeCorrectUncheckedWordingPath = Write-TestReport "whole-correct-unchecked-wording" $headingTime "未チェック数の全件報告" @(
        "Roadmap-Claim: whole", $snapshotLine,
        "未チェック$($snapshot.unchecked)件",
        "未チェック件数$($snapshot.unchecked)",
        "未完了は$($snapshot.unchecked)件"
    ) $now
    Assert-Exit (Invoke-ReportCheck $wholeCorrectUncheckedWordingPath) 0 "whole correct unchecked wording"

    $fullWidthWrongCount = ConvertTo-FullWidthNumber -Value ([string]$wrongRemainderCount)
    $wholeFullWidthWrongCountPath = Write-TestReport "whole-fullwidth-wrong-count" $headingTime "未チェック数の全件報告" @("Roadmap-Claim: whole", $snapshotLine, "未チェック$($fullWidthWrongCount)件") $now
    Assert-Exit (Invoke-ReportCheck $wholeFullWidthWrongCountPath) 2 "whole full-width wrong count" "unchecked=$($snapshot.unchecked)"

    $wholeCorrectCountPath = Write-TestReport "whole-correct-count" $headingTime "残作業の実測" @("Roadmap-Claim: whole", $snapshotLine, "残作業は$($snapshot.unchecked)件です。") $now
    Assert-Exit (Invoke-ReportCheck $wholeCorrectCountPath) 0 "whole correct remainder count"

    $wholeFalseCompletePath = Write-TestReport "whole-false-complete" $headingTime "全部完了の報告" @("Roadmap-Claim: whole", $snapshotLine, "全部完了しました。") $now
    Assert-Exit (Invoke-ReportCheck $wholeFalseCompletePath) 2 "whole false all-complete" "unchecked=0"

    $badProgressPath = Write-TestReport "bad-progress" $headingTime "進捗率の報告" @("Roadmap-Claim: whole", $snapshotLine, "Roadmap-Progress: checked=$($snapshot.checked) total=$($snapshot.total) percent=99.0", "進捗率: 99.0%です。") $now
    Assert-Exit (Invoke-ReportCheck $badProgressPath) 2 "false progress" "Roadmap-Progress"

    $validProgressPath = Write-TestReport "valid-progress" $headingTime "進捗率の報告" @("Roadmap-Claim: whole", $snapshotLine, $progressLine, "進捗率: $expectedPercent%です。") $now
    Assert-Exit (Invoke-ReportCheck $validProgressPath) 0 "valid progress"

    $wrongPercent = if ($expectedPercent -eq '99.0') { '98.0' } else { '99.0' }
    $fullWidthWrongPercent = ConvertTo-FullWidthNumber -Value $wrongPercent
    $fullWidthBadProgressPath = Write-TestReport "fullwidth-bad-progress" $headingTime "進捗率の報告" @("Roadmap-Claim: whole", $snapshotLine, $progressLine, "進捗率：$($fullWidthWrongPercent)％です。") $now
    Assert-Exit (Invoke-ReportCheck $fullWidthBadProgressPath) 2 "full-width false progress" "実測 $expectedPercent%"

    # 見出しを施行時刻より前へ偽装しても、固定した旧履歴境界より上なら新規recordである。
    $backdatedRecord = Format-TestRecord ([datetime]'2026-08-31 06:59') "backdateした残作業報告" @("残りは2件だけです。")
    $enforcementAnchorRecord = Format-TestRecord $headingTime "施行後の通常報告" @("Roadmap-Claim: none", "通常の本文です。")
    $backdatedPath = Write-TestDocument "backdated-before-policy" @($backdatedRecord, $enforcementAnchorRecord) $now
    Assert-Exit (Invoke-ReportCheck $backdatedPath) 2 "backdated record" "Roadmap-Claim"

    # production checkerとproduction旧履歴suffixをそのまま使い、正しいclaimを付けても
    # 同日内で古い時刻を先頭へ差し込めないことを確かめる。
    $laterSameDayRecord = Format-TestRecord $headingTime "先に記録済みの通常報告" @("Roadmap-Claim: none", "通常の本文です。")
    $validClaimBackdatedRecord = Format-TestRecord ($headingTime.AddMinutes(-1)) "同日内でbackdateした通常報告" @("Roadmap-Claim: none", "通常の本文です。")
    $validClaimBackdatedPath = Write-TestDocument "valid-claim-same-day-backdate" @($validClaimBackdatedRecord, $laterSameDayRecord) $now
    Assert-Exit (Invoke-ReportCheck $validClaimBackdatedPath) 2 "valid-claim same-day backdate" "時刻が厳密降順ではありません"

    $sameTimestampRecord = Format-TestRecord $headingTime "同一時刻の通常報告" @("Roadmap-Claim: none", "別の本文です。")
    $sameTimestampPath = Write-TestDocument "duplicate-post-enforcement-timestamp" @($sameTimestampRecord, $laterSameDayRecord) $now
    Assert-Exit (Invoke-ReportCheck $sameTimestampPath) 2 "duplicate post-enforcement timestamp" "同一時刻も使えません"

    # 同じcurrent snapshotを持つ2recordは通る。snapshotが異なる過去recordは、
    # そのrecord本文がHEAD履歴で初出したcommitのroadmap/policyからの再生が必要。
    $historicalRoadmapHash = if ($snapshot.roadmap_sha256.StartsWith('a')) { ('b' * 64) -join '' } else { ('a' * 64) -join '' }
    $historicalPolicyHash = if ($snapshot.policy_sha256.StartsWith('c')) { ('d' * 64) -join '' } else { ('c' * 64) -join '' }
    $historicalSnapshotLine = "Roadmap-Snapshot: schema=1 roadmap_sha256=$historicalRoadmapHash policy_sha256=$historicalPolicyHash scope=whole audited=10/10 partial=false checked=6 unchecked=4 evidence_linked=8 explicit_outside=2 unclassified=0"
    $newestGeneration = Format-TestRecord $headingTime "現在の全件報告" @("Roadmap-Claim: whole", $snapshotLine, "現在の全件を照合しました。")
    $olderGeneration = Format-TestRecord ($headingTime.AddMinutes(-1)) "過去の全件報告" @("Roadmap-Claim: whole", $snapshotLine, "当時の全件を照合しました。")
    $twoGenerationPath = Write-TestDocument "two-generations" @($newestGeneration, $olderGeneration) $now
    Assert-Exit (Invoke-ReportCheck $twoGenerationPath) 0 "two snapshot generations" "過去 Roadmap-Snapshot"

    $badHistoricalSnapshotLine = $historicalSnapshotLine.Replace('checked=6 unchecked=4', 'checked=6 unchecked=5')
    $badOlderGeneration = Format-TestRecord ($headingTime.AddMinutes(-1)) "壊れた過去の全件報告" @("Roadmap-Claim: whole", $badHistoricalSnapshotLine, "当時の全件を照合しました。")
    $badGenerationPath = Write-TestDocument "bad-historical-accounting" @($newestGeneration, $badOlderGeneration) $now
    Assert-Exit (Invoke-ReportCheck $badGenerationPath) 2 "bad historical accounting" "全件会計"

    # 正しいlatest recordの直下へ、内部会計だけ整えたfake snapshotを差し込んでも、
    # tracked roadmap/policy blobから本番生成器で再現できないため過去扱いにしない。
    $fakeOlderGeneration = Format-TestRecord ($headingTime.AddMinutes(-1)) "偽の過去全件報告" @("Roadmap-Claim: whole", $historicalSnapshotLine, "当時の全件を照合しました。")
    $fakeInsertedPath = Write-TestDocument "fake-inserted-below-valid-top" @($newestGeneration, $fakeOlderGeneration) $now
    Assert-Exit (Invoke-ReportCheck $fakeInsertedPath) 2 "internally consistent fake historical snapshot" "tracked roadmap/policy blob"

    # 本番snapshot生成器に現行roadmapの1項目だけ過去相当のstateを与え、実在する
    # 別snapshot行を作る。その行を今日新しいrecordとして差し込んでも、初出commitに
    # record本文が存在しないため過去の証拠とは認めない。
    $roadmapText = [IO.File]::ReadAllText($roadmapPath, (New-Object Text.UTF8Encoding($false, $true)))
    $firstCheckbox = [regex]::Match($roadmapText, '(?m)^- \[(?<state>[ xX])\] ')
    Assert-True $firstCheckbox.Success "production roadmap must contain a top-level checkbox"
    $oldState = if ($firstCheckbox.Groups['state'].Value -match '^[xX]$') { ' ' } else { 'x' }
    $oldRoadmapText = $roadmapText.Remove($firstCheckbox.Groups['state'].Index, 1).Insert($firstCheckbox.Groups['state'].Index, $oldState)
    $oldRoadmapPath = Join-Path $sandboxRoot "production-generated-old-roadmap.md"
    [IO.File]::WriteAllText($oldRoadmapPath, $oldRoadmapText, (New-Object Text.UTF8Encoding($false)))
    $oldSnapshotResult = Invoke-ProcessCapture -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $snapshotPath, "-RoadmapPath", $oldRoadmapPath, "-PolicyPath", $policyPath, "-Format", "Json"
    )
    Assert-True ($oldSnapshotResult.ExitCode -eq 0) "production-generated old snapshot failed: $($oldSnapshotResult.Output)"
    $oldSnapshot = $oldSnapshotResult.Output.Trim() | ConvertFrom-Json
    $oldSnapshotLine = [string]$oldSnapshot.report_snapshot_line
    Assert-True (-not [string]::Equals($oldSnapshotLine, $snapshotLine, [StringComparison]::Ordinal)) "old snapshot fixture must differ from current"
    $realOldGeneration = Format-TestRecord ($headingTime.AddMinutes(-1)) "今日差し込んだ過去snapshot報告" @("Roadmap-Claim: whole", $oldSnapshotLine, "当時の全件を照合しました。")
    $realOldInsertedPath = Write-TestDocument "production-generated-old-snapshot-inserted-today" @($newestGeneration, $realOldGeneration) $now
    Assert-Exit (Invoke-ReportCheck $realOldInsertedPath) 2 "production-generated old snapshot inserted today" "tracked roadmap/policy blob"

    # production suffixを元にした負例。anchor以下は概算理由も含めて不変であり、
    # 1 byte改変、境界下挿入、anchorの削除・複製・改名をすべて終了2で止める。
    $legacyMutationPath = Write-TestDocument "mutated-legacy" @($enforcementAnchorRecord) $now ($script:legacySuffixText + ' ')
    Assert-Exit (Invoke-ReportCheck $legacyMutationPath) 2 "mutated legacy suffix" "旧履歴suffix hash"

    $firstSuffixLineBreak = $script:legacySuffixText.IndexOf("`n", [StringComparison]::Ordinal)
    Assert-True ($firstSuffixLineBreak -gt 0) "production legacy suffix must contain the boundary line and body"

    $insertedBelowBoundarySuffix = $script:legacySuffixText.Insert($firstSuffixLineBreak + 1, "境界下へ挿入した行`n")
    $insertedBelowBoundaryPath = Write-TestDocument "inserted-below-legacy-boundary" @($enforcementAnchorRecord) $now $insertedBelowBoundarySuffix
    Assert-Exit (Invoke-ReportCheck $insertedBelowBoundaryPath) 2 "inserted below legacy boundary" "旧履歴suffix hash"

    $changedLineEndingSuffix = $script:legacySuffixText.Replace("`n", "`r`n")
    $changedLineEndingPath = Write-TestDocument "changed-legacy-line-endings" @($enforcementAnchorRecord) $now $changedLineEndingSuffix
    Assert-Exit (Invoke-ReportCheck $changedLineEndingPath) 2 "changed legacy line endings" "旧履歴suffix hash"

    $suffixWithoutBoundary = $script:legacySuffixText.Substring($firstSuffixLineBreak + 1)
    $missingBoundaryPath = Write-TestDocument "missing-legacy-boundary" @($enforcementAnchorRecord) $now $suffixWithoutBoundary
    Assert-Exit (Invoke-ReportCheck $missingBoundaryPath) 2 "missing legacy boundary" "旧履歴境界"

    $renamedBoundarySuffix = $script:legacySuffixText.Replace($script:legacyBoundaryHeader, ($script:legacyBoundaryHeader + '（改名）'))
    $renamedBoundaryPath = Write-TestDocument "renamed-legacy-boundary" @($enforcementAnchorRecord) $now $renamedBoundarySuffix
    Assert-Exit (Invoke-ReportCheck $renamedBoundaryPath) 2 "renamed legacy boundary" "旧履歴境界"

    $duplicatedBoundarySuffix = $script:legacySuffixText + "`n" + $script:legacySuffixText
    $duplicatedBoundaryPath = Write-TestDocument "duplicated-legacy-boundary" @($enforcementAnchorRecord) $now $duplicatedBoundarySuffix
    Assert-Exit (Invoke-ReportCheck $duplicatedBoundaryPath) 2 "duplicated legacy boundary" "実際: 2行"

    $futurePath = Write-TestReport "future" $now.AddDays(1) "通常の作業報告" @("Roadmap-Claim: none", "通常の本文です。") $now
    Assert-Exit (Invoke-ReportCheck $futurePath) 2 "future heading" "later than the file update time"

    Write-Host "[TEST OK] check-report-log: $script:assertions assertions"
    exit 0
}
finally {
    Remove-TestSandbox
}
