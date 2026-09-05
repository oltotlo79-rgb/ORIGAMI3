# get-roadmap-status.ps1 の自己試験。
# 小型の別実装は作らず、本番roadmap全文を本番scriptの別processへ渡し、
# その全文を1箇所だけ壊した負例で「止まること」を確かめる。

$ErrorActionPreference = "Stop"
$scriptDirectory = [string]$PSScriptRoot
$root = Split-Path -Parent $scriptDirectory
$sut = Join-Path $scriptDirectory "get-roadmap-status.ps1"
$roadmap = Join-Path $root "docs\implementation-roadmap.md"
$policy = Join-Path $scriptDirectory "roadmap-status-policy.json"
$powershellExe = (Get-Process -Id $PID).Path
$script:assertions = 0

function Assert-True {
    param([bool]$Condition, [string]$Message)
    $script:assertions++
    if (-not $Condition) {
        throw "[TEST NG] $Message"
    }
}

function Invoke-Sut {
    param([string[]]$Arguments)
    $allArguments = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $sut) + @($Arguments)
    $quotedArguments = @($allArguments | ForEach-Object { '"' + ([string]$_).Replace('"', '\"') + '"' })
    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = $powershellExe
    $startInfo.Arguments = $quotedArguments -join " "
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $windowsJapanese = [Text.Encoding]::GetEncoding(932)
    $startInfo.StandardOutputEncoding = $windowsJapanese
    $startInfo.StandardErrorEncoding = $windowsJapanese
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $startInfo
    [void]$process.Start()
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    $stdout = $stdoutTask.Result.Trim()
    $stderr = $stderrTask.Result.Trim()
    $text = (@($stdout, $stderr) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }) -join "`n"
    $lines = if ([string]::IsNullOrWhiteSpace($text)) { @() } else { @($text -split "`r?`n") }
    return [pscustomobject]@{
        ExitCode = $process.ExitCode
        Text = $text
        Lines = $lines
    }
}

function Write-Utf8NoBom {
    param([string]$Path, [string]$Text)
    $utf8NoBom = New-Object Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($Path, $Text, $utf8NoBom)
}

$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("ori3-roadmap-status-test-" + [Guid]::NewGuid().ToString("N"))
$tempFullPath = [IO.Path]::GetFullPath($tempRoot)
if (-not $tempFullPath.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase)) {
    throw "一時試験先がTEMP外です: $tempFullPath"
}
New-Item -ItemType Directory -Path $tempFullPath | Out-Null

try {
    $actual = Invoke-Sut -Arguments @("-RoadmapPath", $roadmap, "-PolicyPath", $policy, "-Format", "Text")
    Assert-True ($actual.ExitCode -eq 0) "本番roadmapのsnapshotが終了0ではありません: $($actual.Text)"
    Assert-True ($actual.Lines.Count -eq 1) "正常時のsnapshotが1行ではありません"
    Assert-True ($actual.Text -match 'audited=187/187') "全187件を監査した表示がありません"
    Assert-True ($actual.Text -match 'checked=\d+ unchecked=\d+') "完了・未完了の実測件数がありません"
    Assert-True ($actual.Text -match 'evidence_linked=186 explicit_outside=1 unclassified=0') "186+1の会計がありません"

    $jsonResult = Invoke-Sut -Arguments @("-RoadmapPath", $roadmap, "-PolicyPath", $policy, "-Format", "Json")
    Assert-True ($jsonResult.ExitCode -eq 0) "JSON snapshotが終了0ではありません"
    Assert-True ($jsonResult.Lines.Count -eq 1) "JSON snapshotが1行ではありません"
    $json = $jsonResult.Text | ConvertFrom-Json
    Assert-True ([int]$json.total -eq 187 -and [int]$json.audited -eq 187) "JSONの全件会計が187ではありません"
    Assert-True ([int]$json.checked + [int]$json.unchecked -eq [int]$json.total) "checked+uncheckedがtotalと一致しません"
    Assert-True ([int]$json.scopes.M0 -eq 11) "M0の部分scope件数が11ではありません"
    Assert-True (@($json.items).Count -eq 187) "items明細が187件ではありません"
    $foldAllItem = @($json.items | Where-Object { $_.id -eq 'ADDITIONAL.FOLD-ALL.C01' })
    $foldIoItem = @($json.items | Where-Object { $_.id -eq 'ADDITIONAL.FOLD-IO.C01' })
    Assert-True ($foldAllItem.Count -eq 1 -and [string]$foldAllItem[0].source_kind -eq 'evidence_linked') "fold-all追加目標が証拠linkへ昇格していません"
    Assert-True ($foldIoItem.Count -eq 1 -and [string]$foldIoItem[0].source_kind -eq 'explicit_outside') "FOLD-IOの明示対象外会計が変わりました"

    $reportResult = Invoke-Sut -Arguments @("-RoadmapPath", $roadmap, "-PolicyPath", $policy, "-Format", "Report")
    Assert-True ($reportResult.ExitCode -eq 0 -and $reportResult.Lines.Count -eq 2) "報告へ貼る正本が2行で生成されません"
    Assert-True ([string]::Equals([string]$reportResult.Lines[0], [string]$json.report_snapshot_line, [StringComparison]::Ordinal)) "報告用snapshot行がJSON正本と一致しません"
    Assert-True ([string]::Equals([string]$reportResult.Lines[1], [string]$json.report_progress_line, [StringComparison]::Ordinal)) "報告用進捗率行がJSON正本と一致しません"

    $productionText = [IO.File]::ReadAllText($roadmap, (New-Object Text.UTF8Encoding($false, $true)))

    $incompletePath = Join-Path $tempFullPath "forced-incomplete.md"
    $checkedRegex = New-Object Text.RegularExpressions.Regex('^- \[x\] ', [Text.RegularExpressions.RegexOptions]::Multiline)
    $incompleteText = $checkedRegex.Replace($productionText, '- [ ] ', 1)
    Write-Utf8NoBom -Path $incompletePath -Text $incompleteText
    $incomplete = Invoke-Sut -Arguments @("-RoadmapPath", $incompletePath, "-PolicyPath", $policy, "-RequireComplete")
    $expectedUnchecked = [int]$json.unchecked + 1
    Assert-True ($incomplete.ExitCode -eq 1) "本番全文の完了印を1件外した入力をrelease可として通しました"
    Assert-True ($incomplete.Text -match "unchecked=$expectedUnchecked") "未完了で止まる時も変異後の残件数を表示していません"

    $extraPath = Join-Path $tempFullPath "extra-unclassified.md"
    Write-Utf8NoBom -Path $extraPath -Text ($productionText + "`r`n- [ ] 新しい未分類項目`r`n")
    $extra = Invoke-Sut -Arguments @("-RoadmapPath", $extraPath, "-PolicyPath", $policy)
    Assert-True ($extra.ExitCode -eq 2) "全体が188件へ増えたのに未分類項目を通しました"
    Assert-True ($extra.Text -match '証拠リンクも明示対象外policyもありません') "未分類項目の診断がありません: $($extra.Text)"
    Assert-True ($extra.Text -match 'audited=187 total=188 unclassified=1') "187/188の部分会計を診断していません: $($extra.Text)"

    $malformedPath = Join-Path $tempFullPath "malformed-checkbox.md"
    $checkboxRegex = New-Object Text.RegularExpressions.Regex('^- \[x\] ', [Text.RegularExpressions.RegexOptions]::Multiline)
    $malformedText = $checkboxRegex.Replace($productionText, '- [?] ', 1)
    Write-Utf8NoBom -Path $malformedPath -Text $malformedText
    $malformed = Invoke-Sut -Arguments @("-RoadmapPath", $malformedPath, "-PolicyPath", $policy)
    Assert-True ($malformed.ExitCode -eq 2) "壊れたcheckbox書式を通しました"
    Assert-True ($malformed.Text -match 'checkbox書式') "壊れたcheckboxの診断がありません"

    $brokenLinkPath = Join-Path $tempFullPath "broken-link.md"
    $brokenLinkText = $productionText.Replace(
        'ORIGAMI3-ROADMAP-LINK schema=1 id=M0.T0-1.C01',
        'ORIGAMI3-ROADMAP-LINK schema=2 id=M0.T0-1.C01'
    )
    Write-Utf8NoBom -Path $brokenLinkPath -Text $brokenLinkText
    $brokenLink = Invoke-Sut -Arguments @("-RoadmapPath", $brokenLinkPath, "-PolicyPath", $policy)
    Assert-True ($brokenLink.ExitCode -eq 2) "壊れた証拠リンクmarkerを通しました"
    Assert-True ($brokenLink.Text -match '証拠リンクmarker') "壊れた証拠リンクの診断がありません"

    $duplicatePath = Join-Path $tempFullPath "duplicate-id.md"
    $duplicateText = $productionText.Replace('id=M0.T0-1.C02 evidence=', 'id=M0.T0-1.C01 evidence=')
    Write-Utf8NoBom -Path $duplicatePath -Text $duplicateText
    $duplicate = Invoke-Sut -Arguments @("-RoadmapPath", $duplicatePath, "-PolicyPath", $policy)
    Assert-True ($duplicate.ExitCode -eq 2) "重複した証拠IDを通しました"
    Assert-True ($duplicate.Text -match '証拠IDが重複') "重複IDの診断がありません"

    $missingPolicy = Invoke-Sut -Arguments @("-RoadmapPath", $roadmap, "-PolicyPath", (Join-Path $tempFullPath "missing.json"))
    Assert-True ($missingPolicy.ExitCode -eq 2) "policy欠落を通しました"
    Assert-True ($missingPolicy.Text -match 'ファイルが見つかりません') "policy欠落の診断がありません"

    $stalePolicyPath = Join-Path $tempFullPath "stale-policy.json"
    $policyText = [IO.File]::ReadAllText($policy, (New-Object Text.UTF8Encoding($false, $true)))
    $stalePolicyText = $policyText.Replace(
        'e21cff4b5125e05fdbce5645a6b4ef4e2f0c0947f822b0de91d214e9f9d02f64',
        '021cff4b5125e05fdbce5645a6b4ef4e2f0c0947f822b0de91d214e9f9d02f64'
    )
    Write-Utf8NoBom -Path $stalePolicyPath -Text $stalePolicyText
    $stalePolicy = Invoke-Sut -Arguments @("-RoadmapPath", $roadmap, "-PolicyPath", $stalePolicyPath)
    Assert-True ($stalePolicy.ExitCode -eq 2) "古い明示対象外hashを通しました"
    Assert-True ($stalePolicy.Text -match '明示対象外項目がロードマップにありません') "古いpolicy hashの診断がありません"

    Write-Host "[TEST OK] get-roadmap-status: $script:assertions assertions"
    exit 0
}
finally {
    if (Test-Path -LiteralPath $tempFullPath) {
        $resolved = [IO.Path]::GetFullPath($tempFullPath)
        if (-not $resolved.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase) -or
            (Split-Path -Leaf $resolved) -notmatch '^ori3-roadmap-status-test-[0-9a-f]{32}$') {
            throw "一時試験先の削除前安全確認に失敗しました: $resolved"
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
