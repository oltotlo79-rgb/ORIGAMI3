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
$preCommitPath = Join-Path $repoRoot "scripts\hooks\pre-commit"
$powerShellPath = (Get-Process -Id $PID).Path
$tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-check-report-log-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [IO.Path]::GetFullPath((Join-Path $tempParent $sandboxName))
$stagedRepoRoot = Join-Path $sandboxRoot "staged-repo"
$script:assertions = 0
$script:legacyBoundaryHeader = '## 2026-08-31 19:45 — 検証の結論。Codex sol は死んでいなかった。統括の誤判定である'
$script:legacySuffixText = ''

function Assert-True {
    param([bool]$Condition, [string]$Message)
    $script:assertions++
    if (-not $Condition) { throw "ASSERTION FAILED: $Message" }
}

function Get-Utf8Sha256 {
    param([Parameter(Mandatory = $true)][string]$Text)

    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Text)
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        return (($sha256.ComputeHash($bytes) | ForEach-Object { $_.ToString('x2') }) -join '')
    }
    finally {
        $sha256.Dispose()
    }
}

function ConvertTo-LfNormalizedBytesForTest {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)
    $normalized = New-Object System.Collections.Generic.List[byte]
    for ($index = 0; $index -lt $Bytes.Length; $index++) {
        $byteValue = $Bytes[$index]
        if ($byteValue -eq 13) {
            $normalized.Add(10)
            if ($index + 1 -lt $Bytes.Length -and $Bytes[$index + 1] -eq 10) { $index++ }
            continue
        }
        $normalized.Add($byteValue)
    }
    return ,$normalized.ToArray()
}

function ConvertTo-CrlfBytesForTest {
    param([Parameter(Mandatory = $true)][byte[]]$LfBytes)
    $crlf = New-Object System.Collections.Generic.List[byte]
    foreach ($byteValue in $LfBytes) {
        if ($byteValue -eq 10) { $crlf.Add(13) }
        $crlf.Add($byteValue)
    }
    return ,$crlf.ToArray()
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

function Invoke-StagedReportCheck {
    param([switch]$RequireNewRecord)

    $arguments = @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $scriptPath, "-RepositoryRoot", $stagedRepoRoot, "-StagedNewRecordsOnly"
    )
    if ($RequireNewRecord) { $arguments += "-RequireNewRecord" }
    return Invoke-ProcessCapture -Arguments $arguments
}

function Invoke-StagedFixtureGit {
    param([Parameter(Mandatory = $true)][string[]]$GitArguments)

    $previousErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        $output = @(& git -C $stagedRepoRoot @GitArguments 2>&1)
        $exitCode = $LASTEXITCODE
        if ($exitCode -ne 0) {
            throw "fixture git failed ($exitCode): git $($GitArguments -join ' ')`n$($output -join "`n")"
        }
        return ($output -join "`n")
    }
    finally {
        $ErrorActionPreference = $previousErrorAction
    }
}

function Set-StagedFixtureReport {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text,
        [switch]$DoNotStage
    )

    $reportPath = Join-Path $stagedRepoRoot "docs\報告記録.md"
    [IO.File]::WriteAllText($reportPath, $Text, (New-Object Text.UTF8Encoding($false)))
    if (-not $DoNotStage) {
        [void](Invoke-StagedFixtureGit -GitArguments @('add', '--', 'docs/報告記録.md'))
    }
}

function Get-ProductionRecordFixture {
    param(
        [Parameter(Mandatory = $true)][string]$ReportText,
        [Parameter(Mandatory = $true)][string]$Header
    )

    $lines = [regex]::Split($ReportText, "\r\n|\n|\r")
    $matchingIndices = @()
    for ($index = 0; $index -lt $lines.Count; $index++) {
        if ([string]::Equals($lines[$index], $Header, [StringComparison]::Ordinal)) { $matchingIndices += $index }
    }
    Assert-True ($matchingIndices.Count -eq 1) "production fixture header must exist exactly once: $Header"
    $startIndex = $matchingIndices[0]
    $endIndex = $lines.Count
    for ($index = $startIndex + 1; $index -lt $lines.Count; $index++) {
        if ($lines[$index].StartsWith('## ', [StringComparison]::Ordinal)) {
            $endIndex = $index
            break
        }
    }
    $bodyLines = New-Object System.Collections.Generic.List[string]
    for ($index = $startIndex + 1; $index -lt $endIndex; $index++) {
        if ($lines[$index] -match '^Roadmap-(?:Claim|Snapshot|Bounds|Progress):') { continue }
        $bodyLines.Add([string]$lines[$index])
    }
    return [pscustomobject]@{
        Header = $Header
        BodyLines = $bodyLines.ToArray()
    }
}

function Format-StagedFixtureDocument {
    param(
        [Parameter(Mandatory = $true)][object[]]$Records
    )

    $texts = New-Object System.Collections.Generic.List[string]
    foreach ($record in $Records) {
        $lines = New-Object System.Collections.Generic.List[string]
        $lines.Add([string]$record.Header)
        $lines.Add('')
        foreach ($bodyLine in @($record.BodyLines)) { $lines.Add([string]$bodyLine) }
        $texts.Add($lines -join "`n")
    }
    return ($texts -join "`n`n---`n`n") + "`n"
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
    Assert-True ($script:legacySuffixText.Contains('## 2026-08-31 12:16 — 折り鶴の担当が死んでいた。停滞対策は効いたが、統括が宣言だけして動かなかった')) "確定死亡の過去recordはimmutable suffixに保存すること"
    Assert-True ($script:legacySuffixText.Contains('問い合わせ2件（kydnb03n3、ki17jjb07）→ いずれも 7,200秒で正式に時間切れ')) "確定死亡の過去recordは実測本文を改変せず保存すること"

    $snapshotResult = Invoke-ProcessCapture -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $snapshotPath, "-Format", "Json"
    )
    Assert-True ($snapshotResult.ExitCode -eq 0) "production snapshot failed: $($snapshotResult.Output)"
    $snapshot = $snapshotResult.Output.Trim() | ConvertFrom-Json
    Assert-True ([int]$snapshot.total -gt 0 -and [int]$snapshot.audited -eq [int]$snapshot.total -and [int]$snapshot.checked + [int]$snapshot.unchecked -eq [int]$snapshot.total) "production snapshot accounting is invalid"

    # snapshot hashの2経路一致(2026-09-04委譲): 作業ツリーがCRLFでもindexが
    # 保存するLFでも、get-roadmap-status.ps1へ渡すbytesの改行が違うだけで
    # roadmap_sha256/policy_sha256が変わらないことを実測する。本番の
    # roadmap/policyをLF正規化した版と、そこから作ったCRLF版の両方を
    # 別々の一時fileへ書き、同じ生成器へ通して比較する。
    $productionRoadmapBytes = [IO.File]::ReadAllBytes($roadmapPath)
    $productionPolicyBytes = [IO.File]::ReadAllBytes($policyPath)
    $lfRoadmapBytes = ConvertTo-LfNormalizedBytesForTest -Bytes $productionRoadmapBytes
    $crlfRoadmapBytes = ConvertTo-CrlfBytesForTest -LfBytes $lfRoadmapBytes
    $lfPolicyBytes = ConvertTo-LfNormalizedBytesForTest -Bytes $productionPolicyBytes
    $crlfPolicyBytes = ConvertTo-CrlfBytesForTest -LfBytes $lfPolicyBytes
    $lfRoadmapPath = Join-Path $sandboxRoot 'lf-eol-roadmap.md'
    $crlfRoadmapPath = Join-Path $sandboxRoot 'crlf-eol-roadmap.md'
    $lfPolicyPath = Join-Path $sandboxRoot 'lf-eol-policy.json'
    $crlfPolicyPath = Join-Path $sandboxRoot 'crlf-eol-policy.json'
    [IO.File]::WriteAllBytes($lfRoadmapPath, $lfRoadmapBytes)
    [IO.File]::WriteAllBytes($crlfRoadmapPath, $crlfRoadmapBytes)
    [IO.File]::WriteAllBytes($lfPolicyPath, $lfPolicyBytes)
    [IO.File]::WriteAllBytes($crlfPolicyPath, $crlfPolicyBytes)
    $lfEolSnapshotResult = Invoke-ProcessCapture -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $snapshotPath, "-RoadmapPath", $lfRoadmapPath, "-PolicyPath", $lfPolicyPath, "-Format", "Json"
    )
    $crlfEolSnapshotResult = Invoke-ProcessCapture -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $snapshotPath, "-RoadmapPath", $crlfRoadmapPath, "-PolicyPath", $crlfPolicyPath, "-Format", "Json"
    )
    Assert-True ($lfEolSnapshotResult.ExitCode -eq 0) "LF-eol snapshot failed: $($lfEolSnapshotResult.Output)"
    Assert-True ($crlfEolSnapshotResult.ExitCode -eq 0) "CRLF-eol snapshot failed: $($crlfEolSnapshotResult.Output)"
    $lfEolSnapshot = $lfEolSnapshotResult.Output.Trim() | ConvertFrom-Json
    $crlfEolSnapshot = $crlfEolSnapshotResult.Output.Trim() | ConvertFrom-Json
    Assert-True ([string]::Equals([string]$lfEolSnapshot.roadmap_sha256, [string]$crlfEolSnapshot.roadmap_sha256, [StringComparison]::Ordinal)) "roadmap_sha256 must be identical whether the working tree is LF or CRLF"
    Assert-True ([string]::Equals([string]$lfEolSnapshot.policy_sha256, [string]$crlfEolSnapshot.policy_sha256, [StringComparison]::Ordinal)) "policy_sha256 must be identical whether the working tree is LF or CRLF"
    Assert-True ([string]::Equals([string]$lfEolSnapshot.roadmap_sha256, [string]$snapshot.roadmap_sha256, [StringComparison]::Ordinal)) "LF/CRLF-normalized snapshot must match the production working-tree snapshot for the same underlying content"
    Assert-True ([string]::Equals([string]$lfEolSnapshot.policy_sha256, [string]$snapshot.policy_sha256, [StringComparison]::Ordinal)) "LF/CRLF-normalized policy snapshot must match the production working-tree snapshot for the same underlying content"

    $snapshotLine = [string]$snapshot.report_snapshot_line
    $progressLine = [string]$snapshot.report_progress_line
    $expectedPercent = [regex]::Match($progressLine, ' percent=(?<percent>\d+(?:\.\d+)?)$').Groups['percent'].Value
    $checkedItem = @($snapshot.items | Where-Object { $_.state -eq 'checked' })[0]
    $uncheckedItem = @($snapshot.items | Where-Object { $_.state -eq 'unchecked' })[0]
    $boundsLine = "Roadmap-Bounds: ids=$($checkedItem.id),$($uncheckedItem.id) total=2 checked=1 unchecked=1"

    # pre-commitはRust有無のearly exitより前にindex専用gateを呼び、製品変更時は
    # 本文修復だけでなく新規recordを必須にする。
    $preCommitSource = [IO.File]::ReadAllText($preCommitPath, (New-Object Text.UTF8Encoding($false, $true)))
    $stagedGatePosition = $preCommitSource.IndexOf('-StagedNewRecordsOnly', [StringComparison]::Ordinal)
    $rustReceiptPosition = $preCommitSource.IndexOf('receipt_script="$repo_root/scripts/check-receipt.ps1"', [StringComparison]::Ordinal)
    Assert-True ($stagedGatePosition -ge 0) "pre-commit must invoke staged report mode"
    Assert-True ($preCommitSource.Contains('-RequireNewRecord')) "pre-commit must require a new report record for apps/crates"
    Assert-True ($rustReceiptPosition -gt $stagedGatePosition) "staged report gate must run before the Rust-only path"
    Assert-True ($preCommitSource.Contains('staged_for_report')) "staged report gate must include deletions and inspect the index path"

    # 使い捨てgit repoでHEADとindexを分離する。HEADには意図的に古い契約違反を
    # 残し、incremental gateが過去違反ではなく新しい見出しだけを見ることを固定する。
    [void][IO.Directory]::CreateDirectory((Join-Path $stagedRepoRoot 'docs'))
    [void][IO.Directory]::CreateDirectory((Join-Path $stagedRepoRoot 'scripts'))
    [IO.File]::WriteAllBytes((Join-Path $stagedRepoRoot 'docs\implementation-roadmap.md'), [IO.File]::ReadAllBytes($roadmapPath))
    [IO.File]::WriteAllBytes((Join-Path $stagedRepoRoot 'scripts\roadmap-status-policy.json'), [IO.File]::ReadAllBytes($policyPath))
    $baseHeader = '## 2026-08-30 10:00 — 既存の契約行抜け（incremental gate施行前）'
    $baseRecord = [pscustomobject]@{
        Header = $baseHeader
        BodyLines = @('この古いrecordには意図的にRoadmap-Claimがありません。')
    }
    $baseValidRecord = [pscustomobject]@{
        Header = '## 2026-08-30 09:59 — 既存の有効な局所record'
        BodyLines = @('Roadmap-Claim: none', '既存の局所報告です。')
    }
    $baseReportText = Format-StagedFixtureDocument -Records @($baseRecord, $baseValidRecord)
    [IO.File]::WriteAllText((Join-Path $stagedRepoRoot 'docs\報告記録.md'), $baseReportText, (New-Object Text.UTF8Encoding($false)))
    [void](Invoke-StagedFixtureGit -GitArguments @('init', '--quiet'))
    [void](Invoke-StagedFixtureGit -GitArguments @('config', 'user.name', 'ORIGAMI3 report gate test'))
    [void](Invoke-StagedFixtureGit -GitArguments @('config', 'user.email', 'report-gate-test@example.invalid'))
    [void](Invoke-StagedFixtureGit -GitArguments @('config', 'core.autocrlf', 'false'))
    [void](Invoke-StagedFixtureGit -GitArguments @('add', '--', 'docs/報告記録.md', 'docs/implementation-roadmap.md', 'scripts/roadmap-status-policy.json'))
    [void](Invoke-StagedFixtureGit -GitArguments @('commit', '--quiet', '-m', 'base fixture'))

    $oldBodyChanged = [pscustomobject]@{
        Header = $baseHeader
        BodyLines = @('古いrecordの本文だけを修復したが、契約行はまだありません。')
    }
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($oldBodyChanged))
    Assert-Exit (Invoke-StagedReportCheck) 0 "old invalid record body-only repair is ignored"
    Assert-Exit (Invoke-StagedReportCheck -RequireNewRecord) 1 "RequireNewRecord rejects body-only repair" "新しい ## recordがありません"

    # 本日、統括が書いた4件を実データfixtureにする。修復の進行で本番recordへ
    # 契約行が足されても、fixtureから機械可読行を除去して負例を維持する。
    $realFixtureDefinitions = @(
        [pscustomobject]@{
            Header = '## 2026-09-01 13:26 — リリースまでの見通し（利用者の求めに応じて）'
            Claim = 'whole'
            IncludeProgress = $true
        },
        [pscustomobject]@{
            Header = '## 2026-09-01 13:15 — 正本970は既に満たされていた。残り11件'
            Claim = 'whole'
            IncludeProgress = $false
        },
        [pscustomobject]@{
            Header = '## 2026-09-01 13:05 — 提案の4候補は「本当に閉じない」と確定。折り鶴は数字の出る解を捨てた'
            Claim = 'none'
            IncludeProgress = $false
        },
        [pscustomobject]@{
            Header = '## 2026-09-01 12:07 — 残りが14件から12件へ。つまんで動かす土台が入り、型で塞げる穴を1つ塞いだ'
            Claim = 'whole'
            IncludeProgress = $false
        }
    )
    foreach ($definition in $realFixtureDefinitions) {
        $fixture = Get-ProductionRecordFixture -ReportText $productionReportText -Header $definition.Header
        $missingContractRecord = [pscustomobject]@{
            Header = $fixture.Header
            BodyLines = @($fixture.BodyLines)
        }
        Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $missingContractRecord))
        Assert-Exit (Invoke-StagedReportCheck) 2 "real record missing contract: $($definition.Header)" "Roadmap-Claim"

        $correctedBody = New-Object System.Collections.Generic.List[string]
        foreach ($bodyLine in @($fixture.BodyLines)) { $correctedBody.Add([string]$bodyLine) }
        $correctedBody.Add("Roadmap-Claim: $($definition.Claim)")
        if ($definition.Claim -eq 'whole') {
            $correctedBody.Add($snapshotLine)
            if ($definition.IncludeProgress) { $correctedBody.Add($progressLine) }
        }
        else {
            Assert-True ((@($fixture.BodyLines) -join "`n") -match 'すべて') "local-all real fixture must exercise the none false-positive control"
        }
        $correctedRecord = [pscustomobject]@{
            Header = $fixture.Header
            BodyLines = $correctedBody.ToArray()
        }
        Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $correctedRecord))
        Assert-Exit (Invoke-StagedReportCheck) 0 "real record corrected contract: $($definition.Header)"
    }

    $malformedHistoricalHeader = '## 2026-08-31 08:00頃 — 残作業の本当の数が出た。40件ではなく16件。並行4本で進行中'
    Assert-True ($productionReportText.Contains($malformedHistoricalHeader)) "malformed historical header fixture must come from production data"
    $malformedNewRecord = [pscustomobject]@{
        Header = $malformedHistoricalHeader
        BodyLines = @('Roadmap-Claim: none', '新規recordとして複製した場合は見出し書式で止める。')
    }
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $malformedNewRecord))
    Assert-Exit (Invoke-StagedReportCheck) 2 "new malformed real header" "新規見出しが書式に合いません"

    $incompleteSnapshot = 'Roadmap-Snapshot: docs/implementation-roadmap.md checked=172 unchecked=14 total=186'
    Assert-True ($productionReportText.Contains($incompleteSnapshot)) "incomplete schema fixture must come from production data"
    $incompleteSnapshotRecord = [pscustomobject]@{
        Header = '## 2026-09-01 23:50 — 不完全なsnapshotの新規record'
        BodyLines = @('Roadmap-Claim: whole', $incompleteSnapshot, '全体の残件を報告した。')
    }
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $incompleteSnapshotRecord))
    Assert-Exit (Invoke-StagedReportCheck) 2 "incomplete historical snapshot copied into new record" "schema=1"

    $validNoneRecord = [pscustomobject]@{
        Header = '## 2026-09-01 23:49 — 局所検査の新規record'
        BodyLines = @('Roadmap-Claim: none', '局所検査5件すべてが通った。')
    }
    $secondMissingRecord = [pscustomobject]@{
        Header = '## 2026-09-01 23:48 — 契約行の無い2件目'
        BodyLines = @('本文だけがある。')
    }
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $validNoneRecord, $secondMissingRecord))
    Assert-Exit (Invoke-StagedReportCheck) 2 "one of two new records missing claim" "Roadmap-Claim"

    $fencedClaimRecord = [pscustomobject]@{
        Header = '## 2026-09-01 23:47 — code fence内だけにclaimがあるrecord'
        BodyLines = @('```text', 'Roadmap-Claim: none', '```', '本文です。')
    }
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $fencedClaimRecord))
    Assert-Exit (Invoke-StagedReportCheck) 2 "claim hidden in code fence" "実際: 0行"

    $fencedHeadingRecord = [pscustomobject]@{
        Header = '## 2026-09-01 23:46 — code fence内の見出し引用'
        BodyLines = @(
            'Roadmap-Claim: none',
            '本文中で過去の壊れた見出しを引用する。',
            '```markdown',
            '## 2026-08-31 08:00頃 — backtick fence内の引用見出し',
            'Roadmap-Claim: whole',
            '```',
            '~~~~text',
            '## 2026-09-01頃 — tilde fence内の引用見出し',
            'Roadmap-Claim: bounded',
            '~~~~'
        )
    }
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $fencedHeadingRecord))
    Assert-Exit (Invoke-StagedReportCheck) 0 "Markdown fenced headings are not report boundaries"

    $duplicateHeaderRecord = [pscustomobject]@{
        Header = $baseHeader
        BodyLines = @('同じ見出しをもう1件足し、契約行を省いた。')
    }
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $duplicateHeaderRecord))
    Assert-Exit (Invoke-StagedReportCheck) 2 "extra duplicate header is rejected as ambiguous" "見出しと重複"

    # 順方向と逆方向の両方を固定する。特に「新invalid→旧valid」の逆順は、
    # 先頭からHEAD件数を消費するだけだとinvalidを旧扱いにできた回避形である。
    $invalidDuplicateOfValid = [pscustomobject]@{
        Header = $baseValidRecord.Header
        BodyLines = @('新しく足した側には契約行が無い。')
    }
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $baseValidRecord, $invalidDuplicateOfValid))
    Assert-Exit (Invoke-StagedReportCheck) 2 "old valid then new invalid duplicate header" "見出しと重複"
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($invalidDuplicateOfValid, $baseRecord, $baseValidRecord))
    Assert-Exit (Invoke-StagedReportCheck) 2 "new invalid then old valid duplicate header" "見出しと重複"

    $boundedRecord = [pscustomobject]@{
        Header = '## 2026-09-01 23:46 — IDを限定した進捗'
        BodyLines = @('Roadmap-Claim: bounded', $snapshotLine, $boundsLine, '指定した2項目だけを照合した。')
    }
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $boundedRecord))
    Assert-Exit (Invoke-StagedReportCheck) 0 "valid bounded staged contract"
    $boundedWithoutBoundsRecord = [pscustomobject]@{
        Header = $boundedRecord.Header
        BodyLines = @('Roadmap-Claim: bounded', $snapshotLine, 'Boundsを省いた。')
    }
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $boundedWithoutBoundsRecord))
    Assert-Exit (Invoke-StagedReportCheck) 2 "bounded staged contract needs bounds" "Roadmap-Bounds"

    $noneWithSnapshotRecord = [pscustomobject]@{
        Header = '## 2026-09-01 23:45 — noneへsnapshotを混在したrecord'
        BodyLines = @('Roadmap-Claim: none', $snapshotLine, '局所報告です。')
    }
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $noneWithSnapshotRecord))
    Assert-Exit (Invoke-StagedReportCheck) 2 "none rejects machine-wide evidence" "混在させない"

    $wrongProgressRecord = [pscustomobject]@{
        Header = '## 2026-09-01 23:44 — 壊れたprogressのrecord'
        BodyLines = @('Roadmap-Claim: whole', $snapshotLine, 'Roadmap-Progress: checked=1 total=2 percent=50.0')
    }
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $wrongProgressRecord))
    Assert-Exit (Invoke-StagedReportCheck) 2 "wrong staged progress" "実測値と一致しません"

    # checkerは常にindexを読む。stage後のworktree修正で負例を隠せず、逆向きの
    # unstaged破壊でも正しいindexを誤って落とさない。
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $secondMissingRecord))
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $validNoneRecord)) -DoNotStage
    Assert-Exit (Invoke-StagedReportCheck) 2 "invalid index cannot be hidden by valid worktree" "Roadmap-Claim"

    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $validNoneRecord))
    Set-StagedFixtureReport -Text (Format-StagedFixtureDocument -Records @($baseRecord, $secondMissingRecord)) -DoNotStage
    Assert-Exit (Invoke-StagedReportCheck) 0 "valid index is independent from invalid worktree"

    $now = Get-Date
    $headingTime = $now.AddMinutes(-1)
    $legacyBoundaryTimestamp = [datetime]'2026-08-31 19:45'
    Assert-True ($headingTime.AddMinutes(-1) -gt $legacyBoundaryTimestamp) "self-test records must be newer than the immutable boundary"

    $validNonePath = Write-TestReport "valid-none" $headingTime "通常の作業報告" @("Roadmap-Claim: none", "通常の本文です。") $now
    Assert-Exit (Invoke-ReportCheck $validNonePath) 0 "valid none"
    $validNoneText = [IO.File]::ReadAllText($validNonePath, (New-Object Text.UTF8Encoding($false, $true)))
    $validNoneBoundaryOffset = $validNoneText.IndexOf($script:legacyBoundaryHeader, [StringComparison]::Ordinal)
    Assert-True ($validNoneBoundaryOffset -ge 0) "先頭record追加後もimmutable境界を見つけること"
    $validNoneSuffix = $validNoneText.Substring($validNoneBoundaryOffset)
    Assert-True (
        (Get-Utf8Sha256 $validNoneSuffix) -eq (Get-Utf8Sha256 $script:legacySuffixText)
    ) "先頭record追加は境界以下のhashを変えないこと"

    $agentDeathMissingEvidencePath = Write-TestReport "agent-death-missing-evidence" $headingTime "担当が死んだという報告" @("Roadmap-Claim: none", "担当が死んだと判断した。") $now
    Assert-Exit (Invoke-ReportCheck $agentDeathMissingEvidencePath) 2 "agent death missing evidence" "Agent-Death-Evidence"

    $agentDeathMtimeOnlyPath = Write-TestReport "agent-death-mtime-only" $headingTime "担当が死んだという報告" @("Roadmap-Claim: none", "担当が死んだと判断した。", "Agent-Death-Evidence: 最終更新時刻が40分を超えた") $now
    Assert-Exit (Invoke-ReportCheck $agentDeathMtimeOnlyPath) 2 "agent death mtime only" "Agent-Death-Evidence"

    $agentDeathEvidenceLine = "Agent-Death-Evidence: agent-inquiry-timeout-v1 attempt1=timeout:7200s attempt2=timeout:7200s"
    $validAgentDeathPath = Write-TestReport "valid-agent-death" $headingTime "担当が死んだという報告" @("Roadmap-Claim: none", "担当が死んだと判断した。", $agentDeathEvidenceLine) $now
    Assert-Exit (Invoke-ReportCheck $validAgentDeathPath) 0 "valid agent death evidence"

    $agentDeathEvidenceWithoutClaimPath = Write-TestReport "agent-death-evidence-without-claim" $headingTime "通常の作業報告" @("Roadmap-Claim: none", "通常の本文です。", $agentDeathEvidenceLine) $now
    Assert-Exit (Invoke-ReportCheck $agentDeathEvidenceWithoutClaimPath) 2 "agent death evidence without claim" "混在させない"

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

    $appendedBelowBoundarySuffix = $script:legacySuffixText + "`n境界下へ末尾追記した行"
    $appendedBelowBoundaryPath = Write-TestDocument "appended-below-legacy-boundary" @($enforcementAnchorRecord) $now $appendedBelowBoundarySuffix
    Assert-Exit (Invoke-ReportCheck $appendedBelowBoundaryPath) 2 "appended below legacy boundary" "旧履歴suffix hash"

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
