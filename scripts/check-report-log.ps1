[CmdletBinding()]
param(
    # 実装コミットより何日前の報告まで許容するか。既定値0は同じ日を要求する。
    [ValidateRange(0, 2147483647)]
    [int]$AllowedDelayDays = 0,

    # 検査用の複製で使う場合だけ指定する。通常は docs/報告記録.md を検査する。
    [string]$ReportPath,

    # pre-commitから使う。HEADとindexの報告記録を比べ、indexで新しく増えたrecordだけを検査する。
    [switch]$StagedNewRecordsOnly,

    # apps/ / crates/ と同じcommitの報告で使い、pathを変更しただけの空振りを拒否する。
    [switch]$RequireNewRecord,

    # staged modeの使い捨てrepo自己検査用。省略時は従来どおり本repo。
    [string]$RepositoryRoot
)

# ORIGAMI3 利用者への報告記録検査 (Windows PowerShell 5.1 / PowerShell 7 対応)
#
# 記録見出しの正本:
#   ## YYYY-MM-DD HH:mm — 概要

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$defaultRoot = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot)).TrimEnd([char[]]"\/")
$root = $defaultRoot
if ($PSBoundParameters.ContainsKey("RepositoryRoot")) {
    $root = [System.IO.Path]::GetFullPath($RepositoryRoot).TrimEnd([char[]]"\/")
}
$effectiveReportPath = Join-Path $root "docs\報告記録.md"
if ($PSBoundParameters.ContainsKey("ReportPath")) {
    $effectiveReportPath = [System.IO.Path]::GetFullPath($ReportPath)
}
$headerPattern = [regex]::new(
    '^## (?<date>\d{4}-\d{2}-\d{2}) (?<time>(?:[01]\d|2[0-3]):[0-5]\d) — (?<title>\S(?:.*\S)?)$'
)
$script:formatProblems = New-Object System.Collections.Generic.List[string]
$script:missingProblems = New-Object System.Collections.Generic.List[string]
$script:snapshot = $null
$script:reportGateTimestamp = [datetime]::MinValue
$script:legacyBoundaryHeader = '## 2026-08-31 19:45 — 検証の結論。Codex sol は死んでいなかった。統括の誤判定である'
$script:legacySuffixSha256 = '47cb9d9cc60935d688fd3209cac8effa68e84365684690e8092e194d03df5872'
$script:legacyBoundaryLineIndex = -1
$script:historicalSnapshotEvidence = @{}
$script:recordIntroductionCommits = @{}
$script:gitExecutable = $null

# 自然文の母集合・時制・否定は、通常経路とrelease経路が同じ判定器を1つだけ呼ぶ。
# 裸の「すべて|全て|全件」を2箇所へ別々に書くと、staged経路の逆契約と食い違って
# 局所全数を誤拒否する。読み込めない場合は緑にせず、ここで止める。
$script:roadmapScopeScriptPath = Join-Path $PSScriptRoot 'roadmap-claim-scope.ps1'
if (-not (Test-Path -LiteralPath $script:roadmapScopeScriptPath -PathType Leaf)) {
    throw "roadmap claim scope判定器がありません: $($script:roadmapScopeScriptPath)"
}
. $script:roadmapScopeScriptPath
if ($null -eq (Get-Command Get-RoadmapScopeAssertions -CommandType Function -ErrorAction SilentlyContinue)) {
    throw "roadmap claim scope判定器が Get-RoadmapScopeAssertions を公開していません: $($script:roadmapScopeScriptPath)"
}

function Add-FormatProblem {
    param([string]$Message)

    $script:formatProblems.Add($Message)
}

function Add-MissingProblem {
    param([string]$Message)

    $script:missingProblems.Add($Message)
}

function Read-Utf8Text {
    param([string]$Path)

    $utf8 = [System.Text.UTF8Encoding]::new($false, $true)
    return [System.IO.File]::ReadAllText($Path, $utf8)
}

function Get-Utf8Sha256 {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = (New-Object System.Text.UTF8Encoding($false)).GetBytes($Text)
        return ([System.BitConverter]::ToString($sha256.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
    }
}

function Get-BytesSha256 {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)

    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([System.BitConverter]::ToString($sha256.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
    }
}

function Get-ImmutableSuffixInfo {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$BoundaryHeader
    )

    $allBytes = [System.IO.File]::ReadAllBytes($Path)
    $bomLength = 0
    if ($allBytes.Length -ge 3 -and
        $allBytes[0] -eq 0xEF -and $allBytes[1] -eq 0xBB -and $allBytes[2] -eq 0xBF) {
        $bomLength = 3
    }
    $textBytes = New-Object byte[] ($allBytes.Length - $bomLength)
    if ($textBytes.Length -gt 0) {
        [System.Array]::Copy($allBytes, $bomLength, $textBytes, 0, $textBytes.Length)
    }
    $utf8 = [System.Text.UTF8Encoding]::new($false, $true)
    $text = $utf8.GetString($textBytes)

    $characterMatches = New-Object System.Collections.Generic.List[int]
    $searchFrom = 0
    while ($searchFrom -le $text.Length - $BoundaryHeader.Length) {
        $matchIndex = $text.IndexOf($BoundaryHeader, $searchFrom, [System.StringComparison]::Ordinal)
        if ($matchIndex -lt 0) {
            break
        }
        $afterIndex = $matchIndex + $BoundaryHeader.Length
        $startsLine = $matchIndex -eq 0 -or $text[$matchIndex - 1] -eq "`n" -or $text[$matchIndex - 1] -eq "`r"
        $endsLine = $afterIndex -eq $text.Length -or $text[$afterIndex] -eq "`n" -or $text[$afterIndex] -eq "`r"
        if ($startsLine -and $endsLine) {
            $characterMatches.Add($matchIndex)
        }
        $searchFrom = $matchIndex + 1
    }
    if ($characterMatches.Count -ne 1) {
        throw "報告記録の旧履歴境界をraw UTF-8 bytes内で1行に特定できません (実際: $($characterMatches.Count)行)。"
    }

    $prefixByteCount = $utf8.GetByteCount($text.Substring(0, $characterMatches[0]))
    $suffixOffset = $bomLength + $prefixByteCount
    $suffixBytes = New-Object byte[] ($allBytes.Length - $suffixOffset)
    [System.Array]::Copy($allBytes, $suffixOffset, $suffixBytes, 0, $suffixBytes.Length)
    return [pscustomobject]@{
        Offset = $suffixOffset
        Length = $suffixBytes.Length
        Sha256 = Get-BytesSha256 -Bytes $suffixBytes
        Text = $text
    }
}

function ConvertTo-NativeArgumentString {
    param([Parameter(Mandatory = $true)][string[]]$Values)

    $parts = foreach ($value in $Values) {
        $escaped = [regex]::Replace([string]$value, '(\\*)"', '$1$1\"')
        $trailingBackslashes = [regex]::Match($escaped, '\\*$').Value
        '"' + $escaped + $trailingBackslashes + '"'
    }
    return $parts -join ' '
}

function Invoke-NativeBytes {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $FilePath
    $startInfo.Arguments = ConvertTo-NativeArgumentString -Values $Arguments
    $startInfo.WorkingDirectory = $root
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::Start($startInfo)
    $memory = New-Object System.IO.MemoryStream
    try {
        $copyTask = $process.StandardOutput.BaseStream.CopyToAsync($memory)
        $errorTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        [void]$copyTask.GetAwaiter().GetResult()
        $errorText = $errorTask.GetAwaiter().GetResult()
        return [PSCustomObject]@{
            ExitCode = $process.ExitCode
            Bytes     = $memory.ToArray()
            Error     = $errorText
        }
    }
    finally {
        $memory.Dispose()
        $process.Dispose()
    }
}

function Get-GitExecutable {
    if ($null -eq $script:gitExecutable) {
        $command = Get-Command git -ErrorAction Stop
        $script:gitExecutable = [string]$command.Source
        if ([string]::IsNullOrWhiteSpace($script:gitExecutable)) {
            throw 'git executableを解決できません。'
        }
    }
    return $script:gitExecutable
}

function Get-TrackedFileBytesAtCommit {
    param(
        [Parameter(Mandatory = $true)][string]$Commit,
        [Parameter(Mandatory = $true)][string]$RelativePath
    )

    # snapshotはworktreeの生bytesをhashするため、git blobへ現在のcheckout filterを
    # 適用したbytesを使う。これによりWindowsの改行filterをraw blobと混同しない。
    $result = Invoke-NativeBytes -FilePath (Get-GitExecutable) -Arguments @(
        '-C', $root, 'cat-file', '--filters', "--path=$RelativePath", "${Commit}:$RelativePath"
    )
    if ($result.ExitCode -ne 0) {
        return $null
    }
    return ,([byte[]]$result.Bytes)
}

function Get-CanonicalRecordSha256 {
    param([Parameter(Mandatory = $true)]$Record)

    $canonicalLines = New-Object System.Collections.Generic.List[string]
    $canonicalLines.Add([string]$Record.Header)
    foreach ($line in @($Record.BodyLines)) {
        $canonicalLines.Add([string]$line)
    }
    while ($canonicalLines.Count -gt 1) {
        $last = $canonicalLines[$canonicalLines.Count - 1].Trim()
        if ($last.Length -ne 0 -and $last -notmatch '^(?:---|\*\*\*|___)$') {
            break
        }
        $canonicalLines.RemoveAt($canonicalLines.Count - 1)
    }
    return Get-Utf8Sha256 -Text ($canonicalLines -join "`n")
}

function Test-ReportBlobContainsRecordHash {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][string]$ExpectedHash
    )

    $utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
    $blobLines = [regex]::Split($utf8Strict.GetString($Bytes), "\r\n|\n|\r")
    for ($lineIndex = 0; $lineIndex -lt $blobLines.Count; $lineIndex++) {
        $headerMatch = $headerPattern.Match($blobLines[$lineIndex])
        if (-not $headerMatch.Success) {
            continue
        }
        $endLineIndex = $blobLines.Count
        for ($nextIndex = $lineIndex + 1; $nextIndex -lt $blobLines.Count; $nextIndex++) {
            if ($blobLines[$nextIndex].StartsWith('## ', [StringComparison]::Ordinal)) {
                $endLineIndex = $nextIndex
                break
            }
        }
        $bodyLines = if ($endLineIndex -gt $lineIndex + 1) {
            @($blobLines[($lineIndex + 1)..($endLineIndex - 1)])
        }
        else { @() }
        $candidate = [PSCustomObject]@{
            Header = $blobLines[$lineIndex]
            BodyLines = $bodyLines
        }
        if ([string]::Equals((Get-CanonicalRecordSha256 -Record $candidate), $ExpectedHash, [StringComparison]::Ordinal)) {
            return $true
        }
    }
    return $false
}

function Get-RecordIntroductionCommit {
    param([Parameter(Mandatory = $true)]$Record)

    $recordHash = Get-CanonicalRecordSha256 -Record $Record
    if ($script:recordIntroductionCommits.ContainsKey($recordHash)) {
        $cached = [string]$script:recordIntroductionCommits[$recordHash]
        if ($cached.Length -eq 0) { return $null }
        return $cached
    }

    # HEAD祖先だけを正本とする。refs/wipや別branchだけにあるreportを証拠にしない。
    $historyResult = Invoke-NativeBytes -FilePath (Get-GitExecutable) -Arguments @(
        '-C', $root, 'log', '--format=%H', '--reverse', '--follow', 'HEAD', '--', 'docs/報告記録.md'
    )
    if ($historyResult.ExitCode -ne 0) {
        throw "報告記録のHEAD履歴を列挙できません: $($historyResult.Error.Trim())"
    }
    $utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
    $commits = @($utf8Strict.GetString([byte[]]$historyResult.Bytes) -split '\r?\n' | Where-Object { $_ -match '^[0-9a-f]{40,64}$' })
    foreach ($commit in $commits) {
        $reportBytes = Get-TrackedFileBytesAtCommit -Commit $commit -RelativePath 'docs/報告記録.md'
        if ($null -ne $reportBytes -and (Test-ReportBlobContainsRecordHash -Bytes $reportBytes -ExpectedHash $recordHash)) {
            $script:recordIntroductionCommits[$recordHash] = $commit
            return $commit
        }
    }
    $script:recordIntroductionCommits[$recordHash] = ''
    return $null
}

function Test-HistoricalSnapshotEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$SnapshotLine,
        [Parameter(Mandatory = $true)]$Accounting,
        [Parameter(Mandatory = $true)]$Record
    )

    $expectedCurrentLine = [string]$script:snapshot.report_snapshot_line
    if ([string]::Equals($SnapshotLine, $expectedCurrentLine, [StringComparison]::Ordinal)) {
        return $true
    }
    $recordHash = Get-CanonicalRecordSha256 -Record $Record
    $cacheKey = $recordHash + ':' + $SnapshotLine
    if ($script:historicalSnapshotEvidence.ContainsKey($cacheKey)) {
        return [bool]$script:historicalSnapshotEvidence[$cacheKey]
    }

    # 任意の古いsnapshotを今日挿入できないよう、record本文そのものがHEAD履歴へ
    # 初めて現れたcommitだけを根拠にする。未commitのrecordはcurrent exactだけが上で通る。
    $introductionCommit = Get-RecordIntroductionCommit -Record $Record
    if ([string]::IsNullOrWhiteSpace([string]$introductionCommit)) {
        $script:historicalSnapshotEvidence[$cacheKey] = $false
        return $false
    }
    $utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
    $roadmapBytes = Get-TrackedFileBytesAtCommit -Commit $introductionCommit -RelativePath 'docs/implementation-roadmap.md'
    $policyBytes = Get-TrackedFileBytesAtCommit -Commit $introductionCommit -RelativePath 'scripts/roadmap-status-policy.json'
    if ($null -eq $roadmapBytes -or $null -eq $policyBytes) {
        $script:historicalSnapshotEvidence[$cacheKey] = $false
        return $false
    }
    # roadmap_sha256 / policy_sha256はget-roadmap-status.ps1の内部でだけ計算する
    # (CRLF/CRをLFへ正規化してからhashする)。ここで独自にGet-BytesSha256を
    # bytesへ直接かけて$Accountingと比べると、正規化前のbytesに対する判定に
    # なり、通常/staged/履歴復元の3経路が別々の関数でhashを判定してしまう
    # (past bug: 同じ判定を2箇所に別々に書いて誤拒否した)。判定は必ず下の
    # subprocess呼び出し(get-roadmap-status.ps1)が返すreport_snapshot_line
    # の完全一致だけで行う。

    $tempParent = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd([char[]]'\/')
    $tempName = 'ori3-report-history-{0}' -f [Guid]::NewGuid().ToString('N')
    $tempRoot = [System.IO.Path]::GetFullPath((Join-Path $tempParent $tempName))
    [void][System.IO.Directory]::CreateDirectory($tempRoot)
    try {
        $roadmapPath = Join-Path $tempRoot 'implementation-roadmap.md'
        $policyPath = Join-Path $tempRoot 'roadmap-status-policy.json'
        [System.IO.File]::WriteAllBytes($roadmapPath, $roadmapBytes)
        [System.IO.File]::WriteAllBytes($policyPath, $policyBytes)
        $statusResult = Invoke-NativeBytes -FilePath ((Get-Process -Id $PID).Path) -Arguments @(
            '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
            '-File', (Join-Path $PSScriptRoot 'get-roadmap-status.ps1'),
            '-RoadmapPath', $roadmapPath, '-PolicyPath', $policyPath, '-Format', 'Json'
        )
        if ($statusResult.ExitCode -eq 0) {
            $statusLines = @($utf8Strict.GetString([byte[]]$statusResult.Bytes) -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
            if ($statusLines.Count -eq 1) {
                $historicalStatus = $statusLines[0] | ConvertFrom-Json
                if ([string]::Equals([string]$historicalStatus.report_snapshot_line, $SnapshotLine, [StringComparison]::Ordinal)) {
                    $script:historicalSnapshotEvidence[$cacheKey] = $true
                    return $true
                }
            }
        }
    }
    finally {
        $resolvedTemp = [System.IO.Path]::GetFullPath($tempRoot).TrimEnd([char[]]'\/')
        if ([System.IO.Path]::GetDirectoryName($resolvedTemp) -ne $tempParent -or
            [System.IO.Path]::GetFileName($resolvedTemp) -notmatch '^ori3-report-history-[0-9a-f]{32}$') {
            throw "unsafe historical snapshot cleanup path: $resolvedTemp"
        }
        Remove-Item -LiteralPath $resolvedTemp -Recurse -Force
    }

    $script:historicalSnapshotEvidence[$cacheKey] = $false
    return $false
}

function Read-ReportSnapshotAccounting {
    param([Parameter(Mandatory = $true)][string]$Line)

    $pattern = [regex]::new(
        '^Roadmap-Snapshot: schema=(?<schema>\d+) roadmap_sha256=(?<roadmap>[0-9a-f]{64}) policy_sha256=(?<policy>[0-9a-f]{64}) scope=(?<scope>\S+) audited=(?<audited>\d+)/(?<total>\d+) partial=(?<partial>true|false) checked=(?<checked>\d+) unchecked=(?<unchecked>\d+) evidence_linked=(?<linked>\d+) explicit_outside=(?<outside>\d+) unclassified=(?<unclassified>\d+)$'
    )
    $match = $pattern.Match($Line)
    if (-not $match.Success) {
        return $null
    }

    $accounting = [PSCustomObject]@{
        Schema          = [int]$match.Groups['schema'].Value
        RoadmapSha256   = $match.Groups['roadmap'].Value
        PolicySha256    = $match.Groups['policy'].Value
        Scope           = $match.Groups['scope'].Value
        Audited         = [int]$match.Groups['audited'].Value
        Total           = [int]$match.Groups['total'].Value
        Partial         = $match.Groups['partial'].Value
        Checked         = [int]$match.Groups['checked'].Value
        Unchecked       = [int]$match.Groups['unchecked'].Value
        EvidenceLinked  = [int]$match.Groups['linked'].Value
        ExplicitOutside = [int]$match.Groups['outside'].Value
        Unclassified    = [int]$match.Groups['unclassified'].Value
    }
    if ($accounting.Schema -ne 1 -or $accounting.Scope -ne 'whole' -or $accounting.Partial -ne 'false' -or
        $accounting.Total -le 0 -or $accounting.Audited -ne $accounting.Total -or
        $accounting.Checked + $accounting.Unchecked -ne $accounting.Total -or
        $accounting.EvidenceLinked + $accounting.ExplicitOutside -ne $accounting.Total -or
        $accounting.Unclassified -ne 0) {
        return $null
    }
    return $accounting
}

function Get-CurrentRoadmapSnapshot {
    $snapshotScript = Join-Path $PSScriptRoot "get-roadmap-status.ps1"
    if (-not (Test-Path -LiteralPath $snapshotScript -PathType Leaf)) {
        throw "roadmap snapshot scriptがありません: $snapshotScript"
    }
    $powershellExe = (Get-Process -Id $PID).Path
    $global:LASTEXITCODE = 0
    $output = @(& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $snapshotScript -Format Json)
    $snapshotExitCode = $LASTEXITCODE
    if ($snapshotExitCode -ne 0) {
        throw "roadmap snapshotが失敗しました (終了コード: $snapshotExitCode)"
    }
    if ($output.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$output[0])) {
        throw "roadmap snapshotがJSON 1行を返しませんでした (行数: $($output.Count))"
    }
    $snapshot = [string]$output[0] | ConvertFrom-Json
    if ([int]$snapshot.schema -ne 1 -or [string]$snapshot.scope -ne "whole" -or [bool]$snapshot.partial -or
        [int]$snapshot.audited -ne [int]$snapshot.total -or [int]$snapshot.unclassified -ne 0 -or
        [int]$snapshot.checked + [int]$snapshot.unchecked -ne [int]$snapshot.total -or
        [string]$snapshot.roadmap_sha256 -notmatch '^[0-9a-f]{64}$' -or
        [string]$snapshot.policy_sha256 -notmatch '^[0-9a-f]{64}$' -or
        [string]$snapshot.report_snapshot_line -notmatch '^Roadmap-Snapshot: schema=1 ' -or
        [string]$snapshot.report_progress_line -notmatch '^Roadmap-Progress: checked=') {
        throw "roadmap snapshotの全件会計が不正です"
    }
    return $snapshot
}

function Get-RecordScopeAssertionsByLine {
    param([Parameter(Mandatory = $true)]$Record)

    $recordLines = @([string]$Record.Header)
    foreach ($bodyLine in @($Record.BodyLines)) {
        $recordLines += [string]$bodyLine
    }
    $roadmapTotal = 0
    if ($null -ne $script:snapshot) {
        $roadmapTotal = [int]$script:snapshot.total
    }
    $assertions = @(Get-RoadmapScopeAssertions `
        -Text $recordLines `
        -StartLine ([int]$Record.LineIndex + 1) `
        -RoadmapTotal $roadmapTotal)
    $byLine = @{}
    foreach ($assertion in $assertions) {
        $key = [int]$assertion.Line
        if (-not $byLine.ContainsKey($key)) {
            $byLine[$key] = New-Object System.Collections.Generic.List[object]
        }
        $byLine[$key].Add($assertion)
    }
    return $byLine
}

function Get-BlockingScopeClaim {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$LineText,
        [Parameter(Mandatory = $true)][string]$Pattern,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][AllowNull()][object[]]$Assertions
    )

    # 判定器は「分かったものだけを免除する」。敏感語のraw matchに対応する判定が
    # 1件も無ければ、未知の表現としてambiguous扱いで止める。unknownをlocalと
    # 推測しない。したがってこの関数は、従来の裸の語による拒否を減らすことしか
    # できず、新しい見逃しを作らない。
    foreach ($rawMatch in [regex]::Matches($LineText, $Pattern)) {
        $rawText = [string]$rawMatch.Value
        if ($rawText.Length -eq 0) {
            continue
        }
        $covering = @(@($Assertions) | Where-Object {
            $trigger = [string]$_.Trigger
            $trigger.Length -gt 0 -and ($rawText.Contains($trigger) -or $trigger.Contains($rawText))
        })
        if ($covering.Count -eq 0) {
            return [PSCustomObject]@{
                Text     = $rawText
                Scope    = 'ambiguous'
                Temporal = 'current'
                Reason   = 'no-explicit-scope-anchor:unclassified-expression'
            }
        }
        foreach ($assertion in $covering) {
            $scope = [string]$assertion.Scope
            $temporal = [string]$assertion.Temporal
            if ($scope -eq 'ambiguous' -or
                (($scope -eq 'whole' -or $scope -eq 'bounded') -and $temporal -eq 'current')) {
                return [PSCustomObject]@{
                    Text     = $rawText
                    Scope    = $scope
                    Temporal = $temporal
                    Reason   = [string]$assertion.Reason
                }
            }
        }
    }
    return $null
}

function Get-ScopeAssertionsForLine {
    param(
        [Parameter(Mandatory = $true)][hashtable]$AssertionsByLine,
        [Parameter(Mandatory = $true)][int]$LineNumber
    )

    if (-not $AssertionsByLine.ContainsKey($LineNumber)) {
        return @()
    }
    $bucket = $AssertionsByLine[$LineNumber]
    $result = New-Object System.Collections.Generic.List[object]
    foreach ($assertion in $bucket) {
        $result.Add($assertion)
    }
    return $result.ToArray()
}

function Get-FirstBlockingScopeClaim {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][AllowEmptyString()][string[]]$RecordLines,
        [Parameter(Mandatory = $true)][int]$FirstLineNumber,
        [Parameter(Mandatory = $true)][hashtable]$AssertionsByLine,
        [Parameter(Mandatory = $true)][string]$Pattern
    )

    for ($offset = 0; $offset -lt $RecordLines.Count; $offset++) {
        $lineNumber = $FirstLineNumber + $offset
        $blocking = Get-BlockingScopeClaim `
            -LineText ([string]$RecordLines[$offset]) `
            -Pattern $Pattern `
            -Assertions (Get-ScopeAssertionsForLine -AssertionsByLine $AssertionsByLine -LineNumber $lineNumber)
        if ($null -ne $blocking) {
            return [PSCustomObject]@{
                Line     = $lineNumber
                Text     = [string]$blocking.Text
                Scope    = [string]$blocking.Scope
                Temporal = [string]$blocking.Temporal
                Reason   = [string]$blocking.Reason
            }
        }
    }
    return $null
}

function Test-RoadmapClaimRecord {
    param(
        [Parameter(Mandatory = $true)]$Record,
        [Parameter(Mandatory = $true)][bool]$RequireCurrentSnapshot
    )

    $bodyLines = @($Record.BodyLines)
    $claimLines = @($bodyLines | Where-Object { $_ -match '^Roadmap-Claim:' })
    if ($claimLines.Count -ne 1 -or $claimLines[0] -notmatch '^Roadmap-Claim: (?<kind>none|whole|bounded)$') {
        Add-FormatProblem "$($Record.LineIndex + 1)行目の施行後記録には Roadmap-Claim: none|whole|bounded を正確に1行書いてください。"
        return
    }
    $claimKind = [string]$Matches['kind']
    # 機械可読行はraw textのまま厳密に検査する。一方、自然言語の断言はNFKC化し、
    # 全角数字や全角％で数値照合を回避できないようにする。
    $normalizationForm = [Text.NormalizationForm]::FormKC
    $normalizedHeader = ([string]$Record.Header).Normalize($normalizationForm)
    $normalizedBodyLines = @($bodyLines | ForEach-Object { ([string]$_).Normalize($normalizationForm) })
    $claimText = $normalizedHeader + "`n" + ($normalizedBodyLines -join "`n")
    $agentDeathClaimPattern = '(?:担当|エージェント)\s*(?:は|が|の)?\s*(?:死んだ|死んで(?!いない|いません|はない)(?:いる|いた)?|死亡(?!していない|していません|ではない|でない)(?:した|している|と(?:断定|判断)した|を(?:断定|確認)した)?)|\bagent\s+(?:died|is\s+dead)\b'
    $agentDeathEvidenceLine = 'Agent-Death-Evidence: agent-inquiry-timeout-v1 attempt1=timeout:7200s attempt2=timeout:7200s'
    $agentDeathEvidenceLines = @($bodyLines | Where-Object { $_ -match '^Agent-Death-Evidence:' })
    if ([regex]::IsMatch($claimText, $agentDeathClaimPattern, [Text.RegularExpressions.RegexOptions]::CultureInvariant)) {
        if ($agentDeathEvidenceLines.Count -ne 1 -or -not [string]::Equals([string]$agentDeathEvidenceLines[0], $agentDeathEvidenceLine, [StringComparison]::Ordinal)) {
            Add-FormatProblem (
                "$($Record.LineIndex + 1)行目の担当の死亡を主張する記録には " +
                "$agentDeathEvidenceLine を正確に1行併記してください。更新時刻・process数・CPU・空応答だけは死亡の証拠になりません。"
            )
        }
    }
    elseif ($agentDeathEvidenceLines.Count -ne 0) {
        Add-FormatProblem "$($Record.LineIndex + 1)行目の担当死亡を主張しない記録に Agent-Death-Evidence を混在させないでください。"
    }
    $existingRemainderSubjectPattern = '(?:残り|残件|残作業(?:の本当の数)?|解消対象として残(?:す件数|る))'
    $uncheckedRemainderSubjectPattern = '(?:(?:未チェック|未完了)(?:件数)?)'
    $existingNumericRemainderPattern = "$existingRemainderSubjectPattern(?:は|が|[:：])?\s*(?<count>[0-9]+)\s*件(?:だけ|のみ)?"
    $uncheckedNumericRemainderPattern = "$uncheckedRemainderSubjectPattern(?:は|が|[:：])?\s*(?<count>[0-9]+)\s*(?:件)?(?:だけ|のみ)?"
    $numericRemainderPattern = "(?:$existingNumericRemainderPattern|$uncheckedNumericRemainderPattern)"
    $vagueRemainderPattern = '(?:残り|残件|残作業|解消対象として残(?:す件数|る)|未チェック|未完了)(?:は|が)?.{0,40}(?:だけ|のみ)'
    $remainderPattern = "(?:$numericRemainderPattern|$vagueRemainderPattern)"
    $completenessPattern = "すべて|全て|全件|全部完了|これで全部|これが全部|$remainderPattern|リリース(?:まで)?の(?:範囲|残作業)|進捗率"
    $universalPattern = 'すべて|全て|全件|全部完了|これで全部|これが全部'
    $zeroRemainderPattern = '全部完了|これで全部|これが全部|(?:すべて|全て|全件)(?:の)?(?:作業|項目|残作業)?(?:は|が)?\s*(?:完了|終了|済み)'

    # 敏感語はそのまま残し、母集合・時制・否定だけを判定器で足す。行ごとに
    # 見るのは、record全体を1つのboolへ潰すと局所全数と全体断言が混ざるためである。
    $recordLines = @(@($normalizedHeader) + $normalizedBodyLines)
    $firstRecordLineNumber = [int]$Record.LineIndex + 1
    $scopeAssertionsByLine = Get-RecordScopeAssertionsByLine -Record $Record
    $blockingCompleteness = Get-FirstBlockingScopeClaim `
        -RecordLines $recordLines -FirstLineNumber $firstRecordLineNumber `
        -AssertionsByLine $scopeAssertionsByLine -Pattern $completenessPattern

    $expectedSnapshotLine = [string]$script:snapshot.report_snapshot_line
    $snapshotLines = @($bodyLines | Where-Object { $_ -match '^Roadmap-Snapshot:' })
    $boundLines = @($bodyLines | Where-Object { $_ -match '^Roadmap-Bounds:' })
    $progressLines = @($bodyLines | Where-Object { $_ -match '^Roadmap-Progress:' })

    if ($claimKind -eq "none") {
        if ($null -ne $blockingCompleteness) {
            Add-FormatProblem (
                "$($Record.LineIndex + 1)行目の記録は完全性表現を含むため Roadmap-Claim: none にはできません。" +
                " 発火した原文: $($blockingCompleteness.Line)行目「$($blockingCompleteness.Text)」" +
                " (scope=$($blockingCompleteness.Scope) 時制=$($blockingCompleteness.Temporal) 理由=$($blockingCompleteness.Reason))"
            )
        }
        if ($snapshotLines.Count -ne 0 -or $boundLines.Count -ne 0 -or $progressLines.Count -ne 0) {
            Add-FormatProblem "$($Record.LineIndex + 1)行目の Roadmap-Claim: none にroadmap根拠行を混在させないでください。"
        }
        return
    }

    $recordSnapshot = $null
    if ($snapshotLines.Count -ne 1) {
        Add-FormatProblem "$($Record.LineIndex + 1)行目の記録には Roadmap-Snapshot を正確に1行書いてください (実際: $($snapshotLines.Count)行)。"
    }
    else {
        $recordSnapshot = Read-ReportSnapshotAccounting -Line ([string]$snapshotLines[0])
        if ($null -eq $recordSnapshot) {
            Add-FormatProblem "$($Record.LineIndex + 1)行目の Roadmap-Snapshot はschema=1の全件会計になっていません。"
        }
        elseif ($RequireCurrentSnapshot -and -not [string]::Equals([string]$snapshotLines[0], $expectedSnapshotLine, [StringComparison]::Ordinal)) {
            Add-FormatProblem "$($Record.LineIndex + 1)行目の最新 Roadmap-Snapshot が現在の実測値と一致しません。期待値: $expectedSnapshotLine"
        }
        elseif (-not $RequireCurrentSnapshot) {
            if (Test-HistoricalSnapshotEvidence -SnapshotLine ([string]$snapshotLines[0]) -Accounting $recordSnapshot -Record $Record) {
                Write-Host "[OK] $($Record.LineIndex + 1)行目の過去 Roadmap-Snapshot は、current snapshot又はrecord本文のHEAD初出commitにあるtracked roadmap/policy blobから本番生成器で再現できました。"
            }
            else {
                Add-FormatProblem "$($Record.LineIndex + 1)行目の過去 Roadmap-Snapshot を、current snapshot又はrecord本文のHEAD初出commitにあるtracked roadmap/policy blobから再現できません。今日挿入した古いsnapshotや内部会計だけでは当時正しかった証拠になりません。"
            }
        }
    }

    $boundedUnchecked = $null
    if ($claimKind -eq "whole") {
        if ($boundLines.Count -ne 0) {
            Add-FormatProblem "$($Record.LineIndex + 1)行目の whole claim に Roadmap-Bounds を書かないでください。"
        }
    }
    else {
        $blockingUniversal = Get-FirstBlockingScopeClaim `
            -RecordLines $recordLines -FirstLineNumber $firstRecordLineNumber `
            -AssertionsByLine $scopeAssertionsByLine -Pattern $universalPattern
        if ($null -ne $blockingUniversal) {
            Add-FormatProblem (
                "$($Record.LineIndex + 1)行目の bounded claim で全体を表す語は使えません。" +
                " 発火した原文: $($blockingUniversal.Line)行目「$($blockingUniversal.Text)」" +
                " (scope=$($blockingUniversal.Scope) 時制=$($blockingUniversal.Temporal) 理由=$($blockingUniversal.Reason))"
            )
        }
        $boundsMatch = if ($boundLines.Count -eq 1) {
            [regex]::Match(
                [string]$boundLines[0],
                '^Roadmap-Bounds: ids=(?<ids>[A-Za-z0-9][A-Za-z0-9._-]*(?:,[A-Za-z0-9][A-Za-z0-9._-]*)*) total=(?<total>\d+) checked=(?<checked>\d+) unchecked=(?<unchecked>\d+)$'
            )
        }
        else { $null }
        if ($null -eq $boundsMatch -or -not $boundsMatch.Success) {
            Add-FormatProblem "$($Record.LineIndex + 1)行目の bounded claim には Roadmap-Bounds: ids=... total=N checked=N unchecked=N を正確に1行書いてください。"
        }
        else {
            $ids = @($boundsMatch.Groups['ids'].Value -split ',')
            $uniqueIds = @($ids | Select-Object -Unique)
            $declaredTotal = [int]$boundsMatch.Groups['total'].Value
            $declaredChecked = [int]$boundsMatch.Groups['checked'].Value
            $declaredUnchecked = [int]$boundsMatch.Groups['unchecked'].Value
            $boundedUnchecked = $declaredUnchecked
            if ($uniqueIds.Count -ne $ids.Count -or $declaredTotal -ne $uniqueIds.Count -or
                $declaredChecked + $declaredUnchecked -ne $declaredTotal) {
                Add-FormatProblem "$($Record.LineIndex + 1)行目の Roadmap-Bounds はID件数とchecked/uncheckedの内部会計が一致しません。"
            }
            elseif ($RequireCurrentSnapshot) {
                $itemMap = @{}
                foreach ($item in @($script:snapshot.items)) { $itemMap[[string]$item.id] = $item }
                $unknownIds = @($uniqueIds | Where-Object { -not $itemMap.ContainsKey($_) })
                if ($unknownIds.Count -gt 0) {
                    Add-FormatProblem "$($Record.LineIndex + 1)行目の Roadmap-Bounds に現在の正本に無いIDがあります: $($unknownIds -join ',')"
                    return
                }
                $boundChecked = @($uniqueIds | Where-Object { $itemMap[$_].state -eq 'checked' }).Count
                $boundUnchecked = @($uniqueIds | Where-Object { $itemMap[$_].state -eq 'unchecked' }).Count
                if ($declaredChecked -ne $boundChecked -or $declaredUnchecked -ne $boundUnchecked) {
                    Add-FormatProblem "$($Record.LineIndex + 1)行目の Roadmap-Bounds 件数が実測と一致しません: total=$($uniqueIds.Count) checked=$boundChecked unchecked=$boundUnchecked"
                }
            }
        }
    }

    # whole/bounded claimでは、同じrecordの局所全数・過去値・将来条件をsnapshotへ
    # 混ぜない。roadmap全体を今の値として述べている行だけを照合対象にする。
    $remainderLines = New-Object System.Collections.Generic.List[string]
    for ($recordLineOffset = 0; $recordLineOffset -lt $recordLines.Count; $recordLineOffset++) {
        $recordLineText = [string]$recordLines[$recordLineOffset]
        if ($recordLineText -notmatch $remainderPattern) {
            continue
        }
        $blockingRemainder = Get-BlockingScopeClaim `
            -LineText $recordLineText -Pattern $remainderPattern `
            -Assertions (Get-ScopeAssertionsForLine `
                -AssertionsByLine $scopeAssertionsByLine `
                -LineNumber ($firstRecordLineNumber + $recordLineOffset))
        if ($null -ne $blockingRemainder) {
            $remainderLines.Add($recordLineText)
        }
    }
    $blockingZeroRemainder = Get-FirstBlockingScopeClaim `
        -RecordLines $recordLines -FirstLineNumber $firstRecordLineNumber `
        -AssertionsByLine $scopeAssertionsByLine -Pattern $zeroRemainderPattern
    if ($claimKind -eq 'whole') {
        foreach ($remainderLine in $remainderLines) {
            $remainderMatch = [regex]::Match([string]$remainderLine, $numericRemainderPattern)
            if (-not $remainderMatch.Success) {
                Add-FormatProblem "$($Record.LineIndex + 1)行目の無限定な残件断言は、whole claimで『残りはN件だけ』のように実測件数を数字で書いてください: $remainderLine"
            }
            elseif ($null -ne $recordSnapshot -and [int]$remainderMatch.Groups['count'].Value -ne $recordSnapshot.Unchecked) {
                Add-FormatProblem "$($Record.LineIndex + 1)行目の残件断言 $($remainderMatch.Groups['count'].Value)件がsnapshotのunchecked=$($recordSnapshot.Unchecked)と一致しません。"
            }
        }
        if ($null -ne $blockingZeroRemainder -and $null -ne $recordSnapshot -and $recordSnapshot.Unchecked -ne 0) {
            Add-FormatProblem (
                "$($Record.LineIndex + 1)行目の全部完了・これで全部という断言はunchecked=0のときだけ書けます (実測: $($recordSnapshot.Unchecked))。" +
                " 発火した原文: $($blockingZeroRemainder.Line)行目「$($blockingZeroRemainder.Text)」"
            )
        }
    }
    else {
        $boundedRemainderPattern = "Roadmap-Bounds\s*の対象ID内(?:に限定した場合|に限定した|では|で|の)?[^。．]{0,20}$numericRemainderPattern"
        foreach ($remainderLine in $remainderLines) {
            $boundedRemainderMatch = [regex]::Match([string]$remainderLine, $boundedRemainderPattern)
            if (-not $boundedRemainderMatch.Success) {
                Add-FormatProblem "$($Record.LineIndex + 1)行目の bounded claim で無限定な『残りは〜だけ』は使えません。本文に Roadmap-Bounds の対象ID内という限定を同じ行で明記してください: $remainderLine"
            }
            elseif ($null -ne $boundedUnchecked -and [int]$boundedRemainderMatch.Groups['count'].Value -ne $boundedUnchecked) {
                Add-FormatProblem "$($Record.LineIndex + 1)行目の限定残件 $($boundedRemainderMatch.Groups['count'].Value)件がRoadmap-Boundsのunchecked=$($boundedUnchecked)と一致しません。"
            }
        }
    }

    if ($claimText -match '進捗率') {
        $progressMatch = if ($progressLines.Count -eq 1) {
            [regex]::Match([string]$progressLines[0], '^Roadmap-Progress: checked=(?<checked>\d+) total=(?<total>\d+) percent=(?<percent>\d+(?:\.\d+)?)$')
        }
        else { $null }
        $expectedProgressLine = if ($RequireCurrentSnapshot) { [string]$script:snapshot.report_progress_line } else { '' }
        $expectedPercent = ''
        if ($null -eq $recordSnapshot -or $null -eq $progressMatch -or -not $progressMatch.Success) {
            Add-FormatProblem "$($Record.LineIndex + 1)行目で進捗率を語る場合は Roadmap-Progress を正確に1行書いてください。"
        }
        else {
            $expectedPercent = [Math]::Round(
                ([double]$recordSnapshot.Checked * 100.0 / [double]$recordSnapshot.Total),
                1,
                [MidpointRounding]::AwayFromZero
            ).ToString('0.0', [Globalization.CultureInfo]::InvariantCulture)
            if ([int]$progressMatch.Groups['checked'].Value -ne $recordSnapshot.Checked -or
                [int]$progressMatch.Groups['total'].Value -ne $recordSnapshot.Total -or
                $progressMatch.Groups['percent'].Value -ne $expectedPercent -or
                ($RequireCurrentSnapshot -and -not [string]::Equals([string]$progressLines[0], $expectedProgressLine, [StringComparison]::Ordinal))) {
                Add-FormatProblem "$($Record.LineIndex + 1)行目の Roadmap-Progress が同じ記録のsnapshot会計と一致しません。"
            }
        }
        foreach ($percentMention in [regex]::Matches($claimText, '進捗率\s*[:：]?\s*(?<value>[0-9]{1,3}(?:\.[0-9]+)?)\s*%')) {
            if ($expectedPercent.Length -gt 0 -and
                [Math]::Abs([double]::Parse($percentMention.Groups['value'].Value, [Globalization.CultureInfo]::InvariantCulture) - [double]$expectedPercent) -gt 0.05) {
                Add-FormatProblem "$($Record.LineIndex + 1)行目の進捗率 $($percentMention.Groups['value'].Value)% が実測 $expectedPercent% と一致しません。"
            }
        }
    }
    elseif ($progressLines.Count -ne 0) {
        Add-FormatProblem "$($Record.LineIndex + 1)行目に不要な Roadmap-Progress があります。"
    }
}

function ConvertFrom-StrictUtf8Bytes {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][string]$SourceName
    )

    if ($Bytes.Length -ge 3 -and $Bytes[0] -eq 0xEF -and $Bytes[1] -eq 0xBB -and $Bytes[2] -eq 0xBF) {
        throw "UTF-8 BOMは禁止です: $SourceName"
    }
    $utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
    return $utf8Strict.GetString($Bytes)
}

function Get-GitBlobBytes {
    param(
        [Parameter(Mandatory = $true)][string]$ObjectSpec,
        [Parameter(Mandatory = $true)][string]$DisplayName,
        [switch]$AllowMissing,
        [string]$FilterPath = ''
    )

    $arguments = @('-C', $root, 'cat-file')
    if ([string]::IsNullOrWhiteSpace($FilterPath)) {
        $arguments += @('blob', $ObjectSpec)
    }
    else {
        # get-roadmap-status.ps1はcheckout後のbytesをhashする。index blobにも
        # 同じcheckout filterを適用し、commit後の実測と違うhashを要求しない。
        $arguments += @('--filters', "--path=$FilterPath", $ObjectSpec)
    }
    $result = Invoke-NativeBytes -FilePath (Get-GitExecutable) -Arguments $arguments
    if ($result.ExitCode -ne 0) {
        if ($AllowMissing) {
            return $null
        }
        throw "$DisplayName をgit indexから読めません: $($result.Error.Trim())"
    }
    return ,([byte[]]$result.Bytes)
}

function ConvertTo-StagedReportRecords {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    $lines = [regex]::Split($Text, "\r\n|\n|\r")
    $records = New-Object System.Collections.Generic.List[object]
    $fenceCharacter = ''
    $fenceLength = 0
    for ($lineIndex = 0; $lineIndex -lt $lines.Count; $lineIndex++) {
        $line = [string]$lines[$lineIndex]
        if ($fenceLength -ne 0) {
            $closingPattern = '^\s*' + [regex]::Escape($fenceCharacter) + '{' + $fenceLength + ',}\s*$'
            if ($line -match $closingPattern) {
                $fenceCharacter = ''
                $fenceLength = 0
            }
            continue
        }
        $opening = [regex]::Match($line, '^\s*(?<mark>`{3,}|~{3,})')
        if ($opening.Success) {
            $fenceCharacter = $opening.Groups['mark'].Value.Substring(0, 1)
            $fenceLength = $opening.Groups['mark'].Value.Length
            continue
        }
        # 正規見出しだけを境界にすると「## 2026-09-01頃」のような新しい
        # 壊れたrecordを本文として見逃す。Markdown fenceの外にある ## で
        # 始まる全行を先に境界とし、本文中の引用コードはrecordへ数えない。
        if (-not $line.StartsWith('## ', [StringComparison]::Ordinal)) {
            continue
        }
        $records.Add([pscustomobject]@{
            Header = $line
            LineIndex = $lineIndex
            EndLineIndex = $lines.Count
            BodyLines = @()
        })
    }
    for ($recordIndex = 0; $recordIndex -lt $records.Count; $recordIndex++) {
        $endLineIndex = $lines.Count
        if ($recordIndex + 1 -lt $records.Count) {
            $endLineIndex = [int]$records[$recordIndex + 1].LineIndex
        }
        $records[$recordIndex].EndLineIndex = $endLineIndex
        if ($endLineIndex -gt [int]$records[$recordIndex].LineIndex + 1) {
            $records[$recordIndex].BodyLines = @($lines[([int]$records[$recordIndex].LineIndex + 1)..($endLineIndex - 1)])
        }
    }
    return $records.ToArray()
}

function Get-NewStagedReportRecords {
    param(
        [Parameter(Mandatory = $true)][object[]]$HeadRecords,
        [Parameter(Mandatory = $true)][object[]]$IndexRecords
    )

    # 本文の修復は過去違反を再検査しない。HEADに存在した「見出しの個数」を
    # indexから1件ずつ差し引き、同じ見出しを追加した場合も余分な1件を新規扱いする。
    $headHeaderCounts = @{}
    foreach ($record in $HeadRecords) {
        $header = [string]$record.Header
        if (-not $headHeaderCounts.ContainsKey($header)) { $headHeaderCounts[$header] = 0 }
        $headHeaderCounts[$header] = [int]$headHeaderCounts[$header] + 1
    }
    $indexHeaderCounts = @{}
    foreach ($record in $IndexRecords) {
        $header = [string]$record.Header
        if (-not $indexHeaderCounts.ContainsKey($header)) { $indexHeaderCounts[$header] = 0 }
        $indexHeaderCounts[$header] = [int]$indexHeaderCounts[$header] + 1
    }
    foreach ($header in @($indexHeaderCounts.Keys)) {
        $headCount = if ($headHeaderCounts.ContainsKey($header)) { [int]$headHeaderCounts[$header] } else { 0 }
        $indexCount = [int]$indexHeaderCounts[$header]
        if ($indexCount -gt $headCount -and $indexCount -gt 1) {
            # 先頭からHEAD件数を消費するだけでは「新しいinvalid→古いvalid」の順で
            # invalidを旧扱いにできる。同一見出しのどのoccurrenceが新規か判別不能な
            # 場合は順序を推測せず、見出しを一意に直すまでcommitを止める。
            Add-FormatProblem "staged docs/報告記録.md で追加された見出しが既存又は同じcommitの見出しと重複しています。新規recordの見出し日時と概要を一意にしてください: $header (HEAD=$headCount, index=$indexCount)"
        }
    }
    $newRecords = New-Object System.Collections.Generic.List[object]
    foreach ($record in $IndexRecords) {
        $header = [string]$record.Header
        if ($headHeaderCounts.ContainsKey($header) -and [int]$headHeaderCounts[$header] -gt 0) {
            $headHeaderCounts[$header] = [int]$headHeaderCounts[$header] - 1
        }
        else {
            $newRecords.Add($record)
        }
    }
    return $newRecords.ToArray()
}

function Get-UnfencedRecordLines {
    param([Parameter(Mandatory = $true)]$Record)

    $result = New-Object System.Collections.Generic.List[string]
    $fenceCharacter = ''
    $fenceLength = 0
    foreach ($lineValue in @($Record.BodyLines)) {
        $line = [string]$lineValue
        if ($fenceLength -eq 0) {
            $opening = [regex]::Match($line, '^\s*(?<mark>`{3,}|~{3,})')
            if ($opening.Success) {
                $fenceCharacter = $opening.Groups['mark'].Value.Substring(0, 1)
                $fenceLength = $opening.Groups['mark'].Value.Length
                continue
            }
            $result.Add($line)
            continue
        }

        $closingPattern = '^\s*' + [regex]::Escape($fenceCharacter) + '{' + $fenceLength + ',}\s*$'
        if ($line -match $closingPattern) {
            $fenceCharacter = ''
            $fenceLength = 0
        }
    }
    return [pscustomobject]@{
        Lines = $result.ToArray()
        HasUnclosedFence = ($fenceLength -ne 0)
    }
}

function Get-StagedRoadmapSnapshot {
    $roadmapRelativePath = 'docs/implementation-roadmap.md'
    $policyRelativePath = 'scripts/roadmap-status-policy.json'
    $roadmapBytes = Get-GitBlobBytes -ObjectSpec ":$roadmapRelativePath" -DisplayName $roadmapRelativePath -FilterPath $roadmapRelativePath
    $policyBytes = Get-GitBlobBytes -ObjectSpec ":$policyRelativePath" -DisplayName $policyRelativePath -FilterPath $policyRelativePath

    $tempParent = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd([char[]]'\/')
    $tempName = 'ori3-report-staged-{0}' -f [Guid]::NewGuid().ToString('N')
    $tempRoot = [System.IO.Path]::GetFullPath((Join-Path $tempParent $tempName))
    [void][System.IO.Directory]::CreateDirectory($tempRoot)
    try {
        $roadmapPath = Join-Path $tempRoot 'implementation-roadmap.md'
        $policyPath = Join-Path $tempRoot 'roadmap-status-policy.json'
        [System.IO.File]::WriteAllBytes($roadmapPath, $roadmapBytes)
        [System.IO.File]::WriteAllBytes($policyPath, $policyBytes)
        $statusResult = Invoke-NativeBytes -FilePath ((Get-Process -Id $PID).Path) -Arguments @(
            '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
            '-File', (Join-Path $PSScriptRoot 'get-roadmap-status.ps1'),
            '-RoadmapPath', $roadmapPath, '-PolicyPath', $policyPath, '-Format', 'Json'
        )
        if ($statusResult.ExitCode -ne 0) {
            throw "staged roadmap snapshotが失敗しました (終了コード: $($statusResult.ExitCode)): $($statusResult.Error.Trim())"
        }
        $statusText = ConvertFrom-StrictUtf8Bytes -Bytes ([byte[]]$statusResult.Bytes) -SourceName 'staged roadmap snapshot output'
        $statusLines = @($statusText -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        if ($statusLines.Count -ne 1) {
            throw "staged roadmap snapshotがJSON 1行を返しませんでした (行数: $($statusLines.Count))"
        }
        $snapshot = $statusLines[0] | ConvertFrom-Json
        if ([int]$snapshot.schema -ne 1 -or [string]$snapshot.scope -ne 'whole' -or [bool]$snapshot.partial -or
            [int]$snapshot.audited -ne [int]$snapshot.total -or [int]$snapshot.unclassified -ne 0 -or
            [int]$snapshot.checked + [int]$snapshot.unchecked -ne [int]$snapshot.total -or
            [string]$snapshot.report_snapshot_line -notmatch '^Roadmap-Snapshot: schema=1 ' -or
            [string]$snapshot.report_progress_line -notmatch '^Roadmap-Progress: checked=') {
            throw 'staged roadmap snapshotの全件会計が不正です'
        }
        return $snapshot
    }
    finally {
        $resolvedTemp = [System.IO.Path]::GetFullPath($tempRoot).TrimEnd([char[]]'\/')
        if ([System.IO.Path]::GetDirectoryName($resolvedTemp) -ne $tempParent -or
            [System.IO.Path]::GetFileName($resolvedTemp) -notmatch '^ori3-report-staged-[0-9a-f]{32}$') {
            throw "unsafe staged snapshot cleanup path: $resolvedTemp"
        }
        Remove-Item -LiteralPath $resolvedTemp -Recurse -Force
    }
}

function Test-StagedNewRecordContract {
    param(
        [Parameter(Mandatory = $true)]$Record,
        [Parameter(Mandatory = $true)]$Snapshot
    )

    $lineNumber = [int]$Record.LineIndex + 1
    $headerMatch = $headerPattern.Match([string]$Record.Header)
    if (-not $headerMatch.Success) {
        Add-FormatProblem "$lineNumber 行目の新規見出しが書式に合いません: $($Record.Header)"
    }
    else {
        $recordTimestamp = [datetime]::MinValue
        $timestampText = $headerMatch.Groups['date'].Value + ' ' + $headerMatch.Groups['time'].Value
        if (-not [datetime]::TryParseExact(
            $timestampText, 'yyyy-MM-dd HH:mm', [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::None, [ref]$recordTimestamp
        )) {
            Add-FormatProblem "$lineNumber 行目の新規見出しの日時が実在しません: $timestampText"
        }
    }
    $allIndexLines = [regex]::Split((([string]$Record.Header) + "`n" + (@($Record.BodyLines) -join "`n")), "\r\n|\n|\r")
    if (-not (Test-RecordHasBody -Lines $allIndexLines -StartIndex 0 -EndIndex $allIndexLines.Count)) {
        Add-FormatProblem "$lineNumber 行目の新規recordは見出しだけで、本文がありません。"
    }

    $unfenced = Get-UnfencedRecordLines -Record $Record
    if ($unfenced.HasUnclosedFence) {
        Add-FormatProblem "$lineNumber 行目の新規recordに閉じていないMarkdown code fenceがあります。"
    }
    $bodyLines = @($unfenced.Lines)
    $claimLines = @($bodyLines | Where-Object { ([string]$_).StartsWith('Roadmap-Claim:', [StringComparison]::Ordinal) })
    $claimKind = ''
    if ($claimLines.Count -ne 1) {
        Add-FormatProblem "$lineNumber 行目の新規recordには Roadmap-Claim: none|whole|bounded を正確に1行書いてください (実際: $($claimLines.Count)行)。"
    }
    else {
        $claimMatch = [regex]::Match([string]$claimLines[0], '^Roadmap-Claim: (?<kind>none|whole|bounded)$')
        if (-not $claimMatch.Success) {
            Add-FormatProblem "$lineNumber 行目のRoadmap-Claimは none|whole|bounded のexact行ではありません: $($claimLines[0])"
        }
        else {
            $claimKind = $claimMatch.Groups['kind'].Value
        }
    }

    $snapshotLines = @($bodyLines | Where-Object { ([string]$_).StartsWith('Roadmap-Snapshot:', [StringComparison]::Ordinal) })
    $boundsLines = @($bodyLines | Where-Object { ([string]$_).StartsWith('Roadmap-Bounds:', [StringComparison]::Ordinal) })
    $progressLines = @($bodyLines | Where-Object { ([string]$_).StartsWith('Roadmap-Progress:', [StringComparison]::Ordinal) })
    if ($progressLines.Count -gt 1) {
        Add-FormatProblem "$lineNumber 行目の新規recordには Roadmap-Progress を2行以上書けません。"
    }
    elseif ($progressLines.Count -eq 1 -and
        -not [string]::Equals([string]$progressLines[0], [string]$Snapshot.report_progress_line, [StringComparison]::Ordinal)) {
        Add-FormatProblem "$lineNumber 行目の Roadmap-Progress がstaged roadmap/policyの実測値と一致しません。期待値: $($Snapshot.report_progress_line)"
    }

    if ($claimKind -eq 'none') {
        # 自然文の「5検査すべて」のような局所全数を誤拒否しない。noneで禁止するのは
        # roadmap全体の会計に使う機械可読行だけに限定する。
        if ($snapshotLines.Count -ne 0 -or $boundsLines.Count -ne 0 -or $progressLines.Count -ne 0) {
            Add-FormatProblem "$lineNumber 行目の Roadmap-Claim: none に Roadmap-Snapshot/Bounds/Progress を混在させないでください。"
        }
        return
    }
    if ($claimKind.Length -eq 0) { return }

    if ($snapshotLines.Count -ne 1) {
        Add-FormatProblem "$lineNumber 行目の $claimKind claimには Roadmap-Snapshot を正確に1行書いてください (実際: $($snapshotLines.Count)行)。"
    }
    elseif ($null -eq (Read-ReportSnapshotAccounting -Line ([string]$snapshotLines[0]))) {
        Add-FormatProblem "$lineNumber 行目の Roadmap-Snapshot はschema=1の全件会計になっていません。"
    }
    elseif (-not [string]::Equals([string]$snapshotLines[0], [string]$Snapshot.report_snapshot_line, [StringComparison]::Ordinal)) {
        Add-FormatProblem "$lineNumber 行目の Roadmap-Snapshot がstaged roadmap/policyの実測値と一致しません。期待値: $($Snapshot.report_snapshot_line)"
    }

    if ($claimKind -eq 'whole') {
        if ($boundsLines.Count -ne 0) {
            Add-FormatProblem "$lineNumber 行目の whole claim に Roadmap-Bounds を書かないでください。"
        }
        return
    }

    $boundsMatch = if ($boundsLines.Count -eq 1) {
        [regex]::Match(
            [string]$boundsLines[0],
            '^Roadmap-Bounds: ids=(?<ids>[A-Za-z0-9][A-Za-z0-9._-]*(?:,[A-Za-z0-9][A-Za-z0-9._-]*)*) total=(?<total>\d+) checked=(?<checked>\d+) unchecked=(?<unchecked>\d+)$'
        )
    }
    else { $null }
    if ($null -eq $boundsMatch -or -not $boundsMatch.Success) {
        Add-FormatProblem "$lineNumber 行目の bounded claimには Roadmap-Bounds: ids=... total=N checked=N unchecked=N を正確に1行書いてください。"
        return
    }
    $ids = @($boundsMatch.Groups['ids'].Value -split ',')
    $uniqueIds = @($ids | Select-Object -Unique)
    $declaredTotal = [int]$boundsMatch.Groups['total'].Value
    $declaredChecked = [int]$boundsMatch.Groups['checked'].Value
    $declaredUnchecked = [int]$boundsMatch.Groups['unchecked'].Value
    if ($uniqueIds.Count -ne $ids.Count -or $declaredTotal -ne $uniqueIds.Count -or
        $declaredChecked + $declaredUnchecked -ne $declaredTotal) {
        Add-FormatProblem "$lineNumber 行目の Roadmap-Bounds はID件数とchecked/uncheckedの内部会計が一致しません。"
        return
    }
    $itemMap = @{}
    foreach ($item in @($Snapshot.items)) { $itemMap[[string]$item.id] = $item }
    $unknownIds = @($uniqueIds | Where-Object { -not $itemMap.ContainsKey($_) })
    if ($unknownIds.Count -gt 0) {
        Add-FormatProblem "$lineNumber 行目の Roadmap-Bounds にstaged正本に無いIDがあります: $($unknownIds -join ',')"
        return
    }
    $actualChecked = @($uniqueIds | Where-Object { $itemMap[$_].state -eq 'checked' }).Count
    $actualUnchecked = @($uniqueIds | Where-Object { $itemMap[$_].state -eq 'unchecked' }).Count
    if ($declaredChecked -ne $actualChecked -or $declaredUnchecked -ne $actualUnchecked) {
        Add-FormatProblem "$lineNumber 行目の Roadmap-Bounds 件数がstaged正本と一致しません: total=$($uniqueIds.Count) checked=$actualChecked unchecked=$actualUnchecked"
    }
}

function Invoke-StagedNewRecordCheck {
    $indexReportBytes = Get-GitBlobBytes -ObjectSpec ':docs/報告記録.md' -DisplayName 'staged docs/報告記録.md'
    $headReportBytes = Get-GitBlobBytes -ObjectSpec 'HEAD:docs/報告記録.md' -DisplayName 'HEAD docs/報告記録.md' -AllowMissing
    $indexReportText = ConvertFrom-StrictUtf8Bytes -Bytes $indexReportBytes -SourceName 'staged docs/報告記録.md'
    $headReportText = ''
    if ($null -ne $headReportBytes) {
        $headReportText = ConvertFrom-StrictUtf8Bytes -Bytes $headReportBytes -SourceName 'HEAD docs/報告記録.md'
    }
    $headRecords = @(ConvertTo-StagedReportRecords -Text $headReportText)
    $indexRecords = @(ConvertTo-StagedReportRecords -Text $indexReportText)
    $newRecords = @(Get-NewStagedReportRecords -HeadRecords $headRecords -IndexRecords $indexRecords)
    if ($RequireNewRecord -and $newRecords.Count -eq 0) {
        Add-MissingProblem 'apps/ または crates/ と同じcommitの staged docs/報告記録.md に、新しい ## recordがありません。既存recordの本文修復だけでは利用者への新しい報告になりません。'
        return
    }
    if ($newRecords.Count -eq 0) {
        Write-Host '[OK] staged docs/報告記録.md に新規recordはありません。既存recordの修復はincremental契約の対象外です。'
        return
    }
    $snapshot = Get-StagedRoadmapSnapshot
    foreach ($record in $newRecords) {
        Test-StagedNewRecordContract -Record $record -Snapshot $snapshot
    }
    if ($script:formatProblems.Count -eq 0 -and $script:missingProblems.Count -eq 0) {
        Write-Host "[OK] staged docs/報告記録.md の新規record $($newRecords.Count)件は契約行を満たしています。"
    }
}

function Test-RecordHasBody {
    param(
        [string[]]$Lines,
        [int]$StartIndex,
        [int]$EndIndex
    )

    for ($index = $StartIndex + 1; $index -lt $EndIndex; $index++) {
        $trimmed = $Lines[$index].Trim()
        if ($trimmed.Length -eq 0) {
            continue
        }
        # 記録間の水平線だけでは、記録本文があることにならない。
        if ($trimmed -match '^(?:---|\*\*\*|___)$') {
            continue
        }
        return $true
    }
    return $false
}

if ($StagedNewRecordsOnly) {
    try {
        Invoke-StagedNewRecordCheck
    }
    catch {
        Add-FormatProblem "staged docs/報告記録.md の新規recordを検査できませんでした: $($_.Exception.Message)"
    }

    foreach ($problem in $script:formatProblems) {
        Write-Host "[NG] $problem" -ForegroundColor Red
    }
    foreach ($problem in $script:missingProblems) {
        Write-Host "[NG] $problem" -ForegroundColor Red
    }
    if ($script:formatProblems.Count -gt 0 -or $script:missingProblems.Count -gt 0) {
        Write-Host '[HELP] Roadmap-Claim: none = roadmap全体の残件・進捗・完了を主張しない局所報告です。「5検査すべて」のような局所全数はnoneのままです。' -ForegroundColor Yellow
        Write-Host '[HELP] Roadmap-Claim: whole = roadmap/release全体の残件・進捗・全件を述べる報告です。staged正本から生成したschema=1 Roadmap-Snapshotを付け、進捗行を書く場合はexactなRoadmap-Progressを付けます。' -ForegroundColor Yellow
        Write-Host '[HELP] Roadmap-Claim: bounded = 対象IDを限定する報告です。schema=1 Roadmap-Snapshotと、ID・total・checked・uncheckedが一致するRoadmap-Boundsを付けます。' -ForegroundColor Yellow
    }
    if ($script:formatProblems.Count -gt 0) { exit 2 }
    if ($script:missingProblems.Count -gt 0) { exit 1 }
    exit 0
}

if (-not (Test-Path -LiteralPath $effectiveReportPath -PathType Leaf)) {
    Add-MissingProblem "docs/報告記録.md がありません。"
}
else {
    try {
        $script:snapshot = Get-CurrentRoadmapSnapshot
        $policyPath = Join-Path $PSScriptRoot "roadmap-status-policy.json"
        $policy = (Read-Utf8Text $policyPath) | ConvertFrom-Json
        if (-not [datetime]::TryParseExact(
            [string]$policy.report_gate_enforce_on_or_after,
            "yyyy-MM-dd HH:mm",
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::AssumeLocal,
            [ref]$script:reportGateTimestamp
        )) {
            throw "roadmap policyのreport_gate_enforce_on_or_afterを読めません"
        }
        # 本文解析とimmutable suffix hashは、同じ1回のraw byte snapshotを使う。
        # 別々にreadして途中の置換・追記を片方だけ見落とす形にしない。
        $legacySuffixInfo = Get-ImmutableSuffixInfo `
            -Path $effectiveReportPath `
            -BoundaryHeader $script:legacyBoundaryHeader
        $content = [string]$legacySuffixInfo.Text
        $lines = [regex]::Split($content, "\r\n|\n|\r")
        $legacyBoundaryMatches = New-Object System.Collections.Generic.List[int]
        for ($boundaryIndex = 0; $boundaryIndex -lt $lines.Count; $boundaryIndex++) {
            if ([string]::Equals($lines[$boundaryIndex], $script:legacyBoundaryHeader, [StringComparison]::Ordinal)) {
                $legacyBoundaryMatches.Add($boundaryIndex)
            }
        }
        if ($legacyBoundaryMatches.Count -ne 1) {
            throw "報告記録の旧履歴境界を1行に特定できません (実際: $($legacyBoundaryMatches.Count)行)。削除・複製・改名では施行を無効化できません。"
        }
        $script:legacyBoundaryLineIndex = $legacyBoundaryMatches[0]
        # 承認されたimmutable suffixは、改行を含む境界先頭byteからEOFまでの
        # raw UTF-8 bytesを固定する。text再構成でCRLF/LF差を消してはならない。
        $actualLegacySuffixSha256 = $legacySuffixInfo.Sha256
        if (-not [string]::Equals($actualLegacySuffixSha256, $script:legacySuffixSha256, [StringComparison]::Ordinal)) {
            throw "報告記録の旧履歴suffix hashが固定値と一致しません。境界以下へbackdate記録を挿入したり、過去記録を改変したりできません: actual=$actualLegacySuffixSha256 expected=$($script:legacySuffixSha256)"
        }
        $records = New-Object System.Collections.Generic.List[object]
        $reportLastWriteTime = (Get-Item -LiteralPath $effectiveReportPath -Force).LastWriteTime

        # boundaryからEOFまでは、raw UTF-8 bytesの固定hashが改行を含む
        # 内容全体を保護する。施行前の概算時刻も理由を含む事実としてbyte単位で残し、
        # 厳格な見出し・claim・時刻順の検査はboundaryより上の新recordだけへ適用する。
        # boundary自身は新recordのbackdateを防ぐ時刻anchorとして1件だけ解析する。
        for ($lineIndex = 0; $lineIndex -le $script:legacyBoundaryLineIndex; $lineIndex++) {
            $line = $lines[$lineIndex]
            if (-not $line.StartsWith("## ", [System.StringComparison]::Ordinal)) {
                continue
            }

            $match = $headerPattern.Match($line)
            if (-not $match.Success) {
                Add-FormatProblem "$($lineIndex + 1)行目の見出しが書式に合いません: $line"
                continue
            }

            $dateText = $match.Groups["date"].Value
            $recordDate = [datetime]::MinValue
            if (-not [datetime]::TryParseExact(
                $dateText,
                "yyyy-MM-dd",
                [System.Globalization.CultureInfo]::InvariantCulture,
                [System.Globalization.DateTimeStyles]::None,
                [ref]$recordDate
            )) {
                Add-FormatProblem "$($lineIndex + 1)行目の日付が実在しません: $dateText"
                continue
            }

            $recordTimestamp = [datetime]::MinValue
            $timestampText = "{0} {1}" -f $dateText, $match.Groups["time"].Value
            if (-not [datetime]::TryParseExact(
                $timestampText,
                "yyyy-MM-dd HH:mm",
                [System.Globalization.CultureInfo]::InvariantCulture,
                [System.Globalization.DateTimeStyles]::AssumeLocal,
                [ref]$recordTimestamp
            )) {
                Add-FormatProblem "Report heading timestamp cannot be read at line $($lineIndex + 1): $timestampText"
                continue
            }

            $records.Add([PSCustomObject]@{
                LineIndex = $lineIndex
                Date      = $recordDate.Date
                Timestamp = $recordTimestamp
                Header    = $line
                EndLineIndex = -1
                BodyLines = @()
                BodyText = ""
            })
        }

        if ($records.Count -eq 0) {
            Add-MissingProblem "報告記録が1件もありません。"
        }
        else {
            for ($recordIndex = 0; $recordIndex -lt $records.Count; $recordIndex++) {
                $record = $records[$recordIndex]
                $nextLineIndex = $lines.Count
                if ($recordIndex + 1 -lt $records.Count) {
                    $nextLineIndex = $records[$recordIndex + 1].LineIndex
                }
                $bodyLines = if ($nextLineIndex -gt $record.LineIndex + 1) {
                    @($lines[($record.LineIndex + 1)..($nextLineIndex - 1)])
                }
                else { @() }
                $record.EndLineIndex = $nextLineIndex
                $record.BodyLines = $bodyLines
                $record.BodyText = $bodyLines -join "`n"
                if (-not (Test-RecordHasBody $lines $record.LineIndex $nextLineIndex)) {
                    Add-FormatProblem "$($record.LineIndex + 1)行目の記録は見出しだけで、本文がありません: $($record.Header)"
                }
            }

            $newRecords = @($records | Where-Object { $_.LineIndex -lt $script:legacyBoundaryLineIndex })
            $latestNewRecordLineIndex = if ($newRecords.Count -gt 0) { $newRecords[0].LineIndex } else { -1 }
            $seenNewRecordHashes = @{}
            foreach ($record in $newRecords) {
                $recordHash = Get-CanonicalRecordSha256 -Record $record
                if ($seenNewRecordHashes.ContainsKey($recordHash)) {
                    Add-FormatProblem "$($record.LineIndex + 1)行目の施行後recordは同じ本文の重複です。過去recordの複製を新しい根拠として再利用できません。"
                }
                else {
                    $seenNewRecordHashes[$recordHash] = $true
                }
                Test-RoadmapClaimRecord -Record $record -RequireCurrentSnapshot ($record.LineIndex -eq $latestNewRecordLineIndex)
            }

            # 旧履歴suffixの内部は固定hashで保護し、施行後recordだけを分単位の
            # 厳密降順にする。境界recordをanchorへ含めることで、先頭へ同日内の
            # 古い時刻や同一時刻を差し込んでも、正しいRoadmap-Claimだけでは通らない。
            $chronologyRecords = New-Object System.Collections.Generic.List[object]
            foreach ($record in $newRecords) {
                $chronologyRecords.Add($record)
            }
            $legacyBoundaryRecord = @($records | Where-Object { $_.LineIndex -eq $script:legacyBoundaryLineIndex })
            if ($legacyBoundaryRecord.Count -ne 1) {
                throw "報告記録の旧履歴境界を時刻順のanchorとして1件に特定できません (実際: $($legacyBoundaryRecord.Count)件)。"
            }
            $chronologyRecords.Add($legacyBoundaryRecord[0])
            for ($recordIndex = 1; $recordIndex -lt $chronologyRecords.Count; $recordIndex++) {
                $newerRecord = $chronologyRecords[$recordIndex - 1]
                $olderRecord = $chronologyRecords[$recordIndex]
                if ($olderRecord.Timestamp -ge $newerRecord.Timestamp) {
                    Add-FormatProblem "施行後記録の時刻が厳密降順ではありません: $($newerRecord.LineIndex + 1)行目 $($newerRecord.Timestamp.ToString('yyyy-MM-dd HH:mm')) の後に、$($olderRecord.LineIndex + 1)行目 $($olderRecord.Timestamp.ToString('yyyy-MM-dd HH:mm')) があります。同一時刻も使えません。"
                }
            }

            for ($recordIndex = 1; $recordIndex -lt $records.Count; $recordIndex++) {
                $newerRecord = $records[$recordIndex - 1]
                $olderRecord = $records[$recordIndex]
                if ($olderRecord.Date -gt $newerRecord.Date) {
                    Add-FormatProblem "日付が降順ではありません: $($newerRecord.Date.ToString('yyyy-MM-dd')) の後に $($olderRecord.Date.ToString('yyyy-MM-dd')) があります。"
                }
            }

            foreach ($record in $newRecords) {
                if ($record.Timestamp -gt $reportLastWriteTime) {
                    Add-FormatProblem "Report heading at line $($record.LineIndex + 1) is later than the file update time: heading=$($record.Timestamp.ToString('yyyy-MM-dd HH:mm')), file=$($reportLastWriteTime.ToString('yyyy-MM-dd HH:mm:ss'))"
                }
            }

            $global:LASTEXITCODE = 0
            $latestSourceCommitDateLines = @(& git -C $root log -1 --format=%cs -- apps crates)
            $gitStatus = $LASTEXITCODE
            $latestSourceCommitDateText = if ($latestSourceCommitDateLines.Count -gt 0) {
                ([string]$latestSourceCommitDateLines[0]).Trim()
            }
            else {
                ""
            }
            if ($gitStatus -ne 0) {
                Add-FormatProblem "apps/ または crates/ の最新コミット日を取得できませんでした (git の終了コード: $gitStatus)。"
            }
            elseif ($latestSourceCommitDateText.Length -eq 0) {
                Write-Host "[OK] apps/ または crates/ を変更したコミットはまだありません。"
            }
            else {
                $latestSourceCommitDate = [datetime]::MinValue
                if (-not [datetime]::TryParseExact(
                    $latestSourceCommitDateText,
                    "yyyy-MM-dd",
                    [System.Globalization.CultureInfo]::InvariantCulture,
                    [System.Globalization.DateTimeStyles]::None,
                    [ref]$latestSourceCommitDate
                )) {
                    Add-FormatProblem "git が返した最新コミット日を読めません: $latestSourceCommitDateText"
                }
                else {
                    $latestReportDate = $records[0].Date
                    $minimumReportDate = $latestSourceCommitDate.Date.AddDays(-$AllowedDelayDays)
                    if ($latestReportDate -lt $minimumReportDate) {
                        Add-MissingProblem "最新の報告日 $($latestReportDate.ToString('yyyy-MM-dd')) が、apps/ または crates/ の最新コミット日 $($latestSourceCommitDate.ToString('yyyy-MM-dd')) より $AllowedDelayDays 日を超えて古いです。"
                    }
                    else {
                        Write-Host "[OK] 最新の報告日 $($latestReportDate.ToString('yyyy-MM-dd')) は、apps/ または crates/ の最新コミット日 $($latestSourceCommitDate.ToString('yyyy-MM-dd')) に対して許容範囲内です。"
                    }
                }
            }
        }
    }
    catch {
        Add-FormatProblem "docs/報告記録.md を検査できませんでした: $($_.Exception.Message)"
    }
}

foreach ($problem in $script:formatProblems) {
    Write-Host "[NG] $problem" -ForegroundColor Red
}
foreach ($problem in $script:missingProblems) {
    Write-Host "[NG] $problem" -ForegroundColor Red
}

if ($script:formatProblems.Count -gt 0) {
    exit 2
}
if ($script:missingProblems.Count -gt 0) {
    exit 1
}

Write-Host "[OK] 利用者への報告記録の検査に合格しました。" -ForegroundColor Green
exit 0
