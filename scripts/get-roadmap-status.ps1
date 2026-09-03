# ORIGAMI3 ロードマップの単一 snapshot 生成器 (Windows PowerShell 5.1 対応)
# ロードマップ全チェックボックスを、証拠リンクまたは明示的な対象外項目へ
# 1件ずつ分類する。分類不能・重複・policyずれがあれば snapshot を無効にする。

[CmdletBinding()]
param(
    [string]$RoadmapPath = "",
    [string]$PolicyPath = "",
    [ValidateSet("Text", "Json", "Report")]
    [string]$Format = "Text",
    [switch]$RequireComplete
)

$ErrorActionPreference = "Stop"
$scriptDirectory = [string]$PSScriptRoot
if ([string]::IsNullOrWhiteSpace($scriptDirectory)) {
    $scriptDirectory = Split-Path -Parent ([IO.Path]::GetFullPath([string]$MyInvocation.MyCommand.Path))
}
$root = Split-Path -Parent $scriptDirectory
if ([string]::IsNullOrWhiteSpace($PolicyPath)) {
    $PolicyPath = Join-Path $scriptDirectory "roadmap-status-policy.json"
}

function Read-Utf8Text {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "ファイルが見つかりません: $Path"
    }
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
        throw "UTF-8 BOMは禁止です: $Path"
    }
    $utf8 = New-Object Text.UTF8Encoding($false, $true)
    return $utf8.GetString($bytes)
}

function Get-Sha256HexFromBytes {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace("-", "").ToLowerInvariant()
    }
    finally {
        $sha.Dispose()
    }
}

function Get-Sha256HexFromText {
    param([Parameter(Mandatory = $true)][string]$Text)
    $utf8NoBom = New-Object Text.UTF8Encoding($false)
    return Get-Sha256HexFromBytes -Bytes $utf8NoBom.GetBytes($Text)
}

function Get-Sha256HexFromNormalizedText {
    # gitはテキストファイルをLFで保存する。作業ツリーは core.autocrlf や
    # checkout filterの都合でCRLFへ変換されていたり、CRLFとLFが混在して
    # いたりする(実測: docs/implementation-roadmap.mdが991 CRLF+3 LF混在)。
    # roadmap_sha256 / policy_sha256は「gitが保存するbytes(LF)」を一意に
    # hashするため、CRLF/CRをすべてLFへ正規化してからhashする。これにより
    # 通常経路(作業ツリーの生bytes)・staged経路(git cat-file --filtersの
    # index bytes)・履歴復元経路(過去commitのblobにcheckout filterを適用
    # したbytes)のどれから来たbytesでも、同じ正規化hashへ収束する。
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)
    $normalizedText = $Text.Replace("`r`n", "`n").Replace("`r", "`n")
    return Get-Sha256HexFromText -Text $normalizedText
}

try {
    $policyText = Read-Utf8Text -Path $PolicyPath
    $policy = $policyText | ConvertFrom-Json
    if ([int]$policy.schema -ne 1) {
        throw "roadmap policyのschemaが未対応です: $($policy.schema)"
    }
    if ([string]::IsNullOrWhiteSpace($RoadmapPath)) {
        $relativePath = [string]$policy.roadmap_relative_path
        if ([string]::IsNullOrWhiteSpace($relativePath)) {
            throw "roadmap policyにroadmap_relative_pathがありません"
        }
        $RoadmapPath = Join-Path $root ($relativePath.Replace('/', '\'))
    }

    $roadmapFullPath = [IO.Path]::GetFullPath($RoadmapPath)
    $roadmapBytes = [IO.File]::ReadAllBytes($roadmapFullPath)
    if ($roadmapBytes.Length -ge 3 -and $roadmapBytes[0] -eq 0xEF -and $roadmapBytes[1] -eq 0xBB -and $roadmapBytes[2] -eq 0xBF) {
        throw "UTF-8 BOMは禁止です: $roadmapFullPath"
    }
    $utf8Strict = New-Object Text.UTF8Encoding($false, $true)
    $roadmapText = $utf8Strict.GetString($roadmapBytes)
    $roadmapSha256 = Get-Sha256HexFromNormalizedText -Text $roadmapText
    $policySha256 = Get-Sha256HexFromNormalizedText -Text $policyText

    $outsideByHash = @{}
    $outsideIds = @{}
    foreach ($outside in @($policy.explicit_outside_items)) {
        $outsideId = [string]$outside.id
        $outsideHash = ([string]$outside.text_sha256).ToLowerInvariant()
        if ($outsideId -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]+$') {
            throw "明示対象外IDが不正です: $outsideId"
        }
        if ($outsideHash -notmatch '^[0-9a-f]{64}$') {
            throw "明示対象外のtext_sha256が不正です: $outsideId"
        }
        if ($outsideIds.ContainsKey($outsideId) -or $outsideByHash.ContainsKey($outsideHash)) {
            throw "明示対象外のIDまたはhashが重複しています: $outsideId"
        }
        $outsideIds[$outsideId] = $true
        $outsideByHash[$outsideHash] = $outside
    }
    if ($outsideByHash.Count -eq 0) {
        throw "明示対象外の会計が空です"
    }

    $problems = New-Object System.Collections.Generic.List[string]
    $items = New-Object System.Collections.Generic.List[object]
    $seenIds = @{}
    $matchedOutside = @{}
    $lines = $roadmapText -split "`r?`n"
    for ($index = 0; $index -lt $lines.Count; $index++) {
        $line = [string]$lines[$index]
        if ($line -notmatch '^\s*-\s*\[') {
            continue
        }
        $lineNumber = $index + 1
        $match = [regex]::Match($line, '^- \[(?<state>[ xX])\] (?<text>.+)$')
        if (-not $match.Success) {
            $problems.Add("行${lineNumber}: checkbox書式が厳密形式 '- [ ] ' / '- [x] ' ではありません")
            continue
        }

        $state = if ($match.Groups['state'].Value -match '^[xX]$') { "checked" } else { "unchecked" }
        $checkboxText = [string]$match.Groups['text'].Value
        $textHash = Get-Sha256HexFromText -Text $checkboxText
        $markerCount = ([regex]::Matches($line, 'ORIGAMI3-ROADMAP-LINK')).Count
        $linkMatch = [regex]::Match(
            $line,
            '<!-- ORIGAMI3-ROADMAP-LINK schema=1 id=(?<id>[A-Za-z0-9][A-Za-z0-9._-]*) evidence=(?<evidence>[^\s>]+) -->'
        )
        $sourceKind = "unclassified"
        $id = ""
        if ($markerCount -gt 0) {
            if ($markerCount -ne 1 -or -not $linkMatch.Success) {
                $problems.Add("行${lineNumber}: 証拠リンクmarkerが重複または不正です")
            }
            else {
                $id = [string]$linkMatch.Groups['id'].Value
                if ($seenIds.ContainsKey($id)) {
                    $problems.Add("行${lineNumber}: 証拠IDが重複しています: $id (初出行$($seenIds[$id]))")
                }
                else {
                    $seenIds[$id] = $lineNumber
                    $sourceKind = "evidence_linked"
                }
            }
        }
        elseif ($outsideByHash.ContainsKey($textHash)) {
            $outside = $outsideByHash[$textHash]
            $id = [string]$outside.id
            if ($matchedOutside.ContainsKey($id)) {
                $problems.Add("行${lineNumber}: 明示対象外項目が重複しています: $id")
            }
            else {
                $matchedOutside[$id] = $lineNumber
                $sourceKind = "explicit_outside"
            }
        }
        else {
            $problems.Add("行${lineNumber}: 証拠リンクも明示対象外policyもありません")
        }

        $items.Add([pscustomobject][ordered]@{
            line_number = $lineNumber
            id = $id
            state = $state
            source_kind = $sourceKind
            text_sha256 = $textHash
        })
    }

    foreach ($outside in @($policy.explicit_outside_items)) {
        if (-not $matchedOutside.ContainsKey([string]$outside.id)) {
            $problems.Add("明示対象外項目がロードマップにありません: $($outside.id)")
        }
    }

    $total = $items.Count
    $checked = @($items | Where-Object { $_.state -eq "checked" }).Count
    $unchecked = @($items | Where-Object { $_.state -eq "unchecked" }).Count
    $evidenceLinked = @($items | Where-Object { $_.source_kind -eq "evidence_linked" }).Count
    $explicitOutside = @($items | Where-Object { $_.source_kind -eq "explicit_outside" }).Count
    $unclassified = @($items | Where-Object { $_.source_kind -eq "unclassified" }).Count
    $audited = $evidenceLinked + $explicitOutside
    $m0Count = @($items | Where-Object { $_.source_kind -eq "evidence_linked" -and $_.id -match '^M0\.' }).Count

    if ($checked + $unchecked -ne $total) {
        $problems.Add("checked + unchecked がtotalと一致しません")
    }
    if ($evidenceLinked -ne [int]$policy.expected_evidence_linked) {
        $problems.Add("証拠リンク件数がpolicyと不一致です: actual=$evidenceLinked expected=$($policy.expected_evidence_linked)")
    }
    if ($m0Count -ne [int]$policy.expected_scope_counts.M0) {
        $problems.Add("M0件数がpolicyと不一致です: actual=$m0Count expected=$($policy.expected_scope_counts.M0)")
    }
    if ($explicitOutside -ne $outsideByHash.Count) {
        $problems.Add("明示対象外件数がpolicyと不一致です: actual=$explicitOutside expected=$($outsideByHash.Count)")
    }
    if ($audited -ne $total -or $unclassified -ne 0) {
        $problems.Add("全checkboxを会計できていません: audited=$audited total=$total unclassified=$unclassified")
    }

    if ($problems.Count -gt 0) {
        foreach ($problem in $problems) {
            [Console]::Error.WriteLine("[NG] $problem")
        }
        exit 2
    }

    $reportPercent = [Math]::Round(([double]$checked * 100.0 / [double]$total), 1, [MidpointRounding]::AwayFromZero).ToString('0.0', [Globalization.CultureInfo]::InvariantCulture)
    $reportSnapshotLine = "Roadmap-Snapshot: schema=1 roadmap_sha256=$roadmapSha256 policy_sha256=$policySha256 scope=whole audited=$audited/$total partial=false checked=$checked unchecked=$unchecked evidence_linked=$evidenceLinked explicit_outside=$explicitOutside unclassified=$unclassified"
    $reportProgressLine = "Roadmap-Progress: checked=$checked total=$total percent=$reportPercent"
    $snapshot = [pscustomobject][ordered]@{
        schema = 1
        roadmap_sha256 = $roadmapSha256
        policy_sha256 = $policySha256
        scope = "whole"
        total = $total
        audited = $audited
        checked = $checked
        unchecked = $unchecked
        evidence_linked = $evidenceLinked
        explicit_outside = $explicitOutside
        unclassified = $unclassified
        partial = $false
        report_snapshot_line = $reportSnapshotLine
        report_progress_line = $reportProgressLine
        scopes = [pscustomobject][ordered]@{
            M0 = $m0Count
            evidence_linked = $evidenceLinked
            whole = $total
        }
        items = $items.ToArray()
    }

    if ($Format -eq "Json") {
        Write-Output ($snapshot | ConvertTo-Json -Depth 8 -Compress)
    }
    elseif ($Format -eq "Report") {
        Write-Output $reportSnapshotLine
        Write-Output $reportProgressLine
    }
    else {
        Write-Output ("ROADMAP_STATUS schema=1 roadmap_sha256={0} policy_sha256={1} scope=whole audited={2}/{3} partial=false checked={4} unchecked={5} evidence_linked={6} explicit_outside={7} unclassified={8}" -f `
            $roadmapSha256, $policySha256, $audited, $total, $checked, $unchecked, $evidenceLinked, $explicitOutside, $unclassified)
    }

    if ($RequireComplete -and $unchecked -gt 0) {
        [Console]::Error.WriteLine("[NG] ロードマップに未チェックが残っています: $unchecked/$total")
        exit 1
    }
    exit 0
}
catch {
    [Console]::Error.WriteLine("[NG] ロードマップsnapshotを生成できません: $($_.Exception.Message)")
    if (-not [string]::IsNullOrWhiteSpace([string]$_.ScriptStackTrace)) {
        [Console]::Error.WriteLine([string]$_.ScriptStackTrace)
    }
    exit 2
}
