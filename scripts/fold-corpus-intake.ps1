# ほかの折り紙ソフトの見本（施策8 外部corpus）を受け入れる台本。
#
# 目的:
#   利用者が置いた `.fold` から、機械的に決まる値だけを取り出して
#   `crates/ori3-export/tests/fixtures/fold/corpus/manifest.json` へ登録する。
#   人が決める値（対応範囲内/外の分類、根拠、出所URL、利用条件）は
#   決定ファイルから読むだけで、この台本は1つも推測しない。
#
# 使い方（統括が実行する）:
#   1) 置かれたファイルの一覧と照合値を見る（読み取りだけ・既定）
#      powershell -NoProfile -ExecutionPolicy Bypass -File scripts\fold-corpus-intake.ps1
#   2) 新しく置かれたファイル用の決定ファイルの下書きを出す
#      ... -File scripts\fold-corpus-intake.ps1 -EmitTemplate <出力先.json>
#   3) 決定ファイルを人が埋めてから manifest へ登録する
#      ... -File scripts\fold-corpus-intake.ps1 -Register <決定ファイル.json>
#
# 守ること:
#   - 分類・根拠・利用条件を台本が作らない。埋まっていなければ登録を拒否する。
#   - 追跡済みの raw ファイルの中身を書き換えない。読むだけである。
#   - `byte_length` と `sha256` は、ディスク上の生バイトをそのまま数える。
#     `.gitattributes` に `-text` が無いと Windows の取り出しで CRLF へ変わり、
#     記録した値と一致しなくなるので、その設定の有無を先に確かめる。

[CmdletBinding(DefaultParameterSetName = "Report")]
param(
    [Parameter(ParameterSetName = "Report")]
    [switch]$Report,

    [Parameter(ParameterSetName = "EmitTemplate", Mandatory = $true)]
    [string]$EmitTemplate,

    [Parameter(ParameterSetName = "Register", Mandatory = $true)]
    [string]$Register,

    [string]$RepoRoot
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$script:Placeholder = "__DECIDE__"
$script:ApprovedLicense = "LicenseRef-ORIGAMI3-User-Authorized-Samples-2026-08-26"
$script:CorpusRelative = "crates/ori3-export/tests/fixtures/fold/corpus"
$script:AttributePattern = "crates/ori3-export/tests/fixtures/fold/corpus/external"
# 4番目の出所は利用者の決定(2026-08-29)でORIPAから origamimagiro/flat-folder へ
# 差し替えた。ORIPAの配布物に .fold が無いためである。
# 出所ごとの枠は実際に手に入る数に合わせてある。公式 edemaine/FOLD が公開する
# 見本のうち、この道具が読める版(1.1以上)は2件だけで、残る3件は古い版である。
# その分を flat-folder が持ち、4出所・合計30件は変えていない。
$script:SourceQuotas = [ordered]@{
    official          = 2
    flat_folder       = 12
    oriedita          = 8
    origami_simulator = 8
}

function Resolve-Root {
    param([string]$Path)
    if ([string]::IsNullOrWhiteSpace($Path)) {
        $Path = Split-Path -Parent $PSScriptRoot
    }
    $resolved = (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path
    return [System.IO.Path]::GetFullPath($resolved).TrimEnd([char[]]@('\', '/'))
}

function Get-SlotId {
    param([string]$Source, [int]$Index)
    # directory名は _ 区切り、id と file名は - 区切りという既存の書き方に合わせる。
    $prefix = switch ($Source) {
        "origami_simulator" { "origami-simulator" }
        "flat_folder" { "flat-folder" }
        default { $Source }
    }
    return ("{0}-{1:d2}" -f $prefix, $Index)
}

function Get-ReservedSlots {
    $slots = New-Object System.Collections.Generic.List[object]
    foreach ($source in $script:SourceQuotas.Keys) {
        for ($index = 1; $index -le $script:SourceQuotas[$source]; $index++) {
            $id = Get-SlotId $source $index
            $slots.Add([pscustomobject]@{
                Id     = $id
                Source = $source
                Path   = "external/$source/$id.fold"
            })
        }
    }
    return $slots
}

function Get-FileSha256Hex {
    param([string]$LiteralPath)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $stream = New-Object System.IO.FileStream(
        $LiteralPath,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::Read
    )
    try {
        $hash = $sha.ComputeHash($stream)
    }
    finally {
        $stream.Dispose()
        $sha.Dispose()
    }
    return ([System.BitConverter]::ToString($hash)).Replace("-", "").ToLowerInvariant()
}

function Test-CorpusGitAttribute {
    param([string]$Root)
    $path = Join-Path $Root ".gitattributes"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        return $false
    }
    foreach ($line in [System.IO.File]::ReadAllLines($path)) {
        $trimmed = $line.Trim()
        if ($trimmed.StartsWith("#")) { continue }
        if ($trimmed -like "*$script:AttributePattern*" -and $trimmed -match '(^|\s)-text(\s|$)') {
            return $true
        }
    }
    return $false
}

function Get-RawFacts {
    param([string]$Root, [string]$RelativePath)

    $full = Join-Path $Root (Join-Path $script:CorpusRelative $RelativePath)
    if (-not (Test-Path -LiteralPath $full -PathType Leaf)) {
        return $null
    }
    $bytes = [System.IO.File]::ReadAllBytes($full)
    $text = $script:Utf8NoBom.GetString($bytes)

    $carriageReturns = 0
    foreach ($byte in $bytes) { if ($byte -eq 13) { $carriageReturns++ } }

    $fileSpec = $null
    $parseError = $null
    try {
        $document = $text | ConvertFrom-Json -ErrorAction Stop
        if ($null -ne $document.PSObject.Properties["file_spec"]) {
            $fileSpec = [double]$document.file_spec
        }
    }
    catch {
        $parseError = $_.Exception.Message
    }

    $item = Get-Item -LiteralPath $full -Force
    return [pscustomobject]@{
        FullPath        = $full
        ByteLength      = [int64]$bytes.Length
        Sha256          = Get-FileSha256Hex $full
        FileSpec        = $fileSpec
        CarriageReturns = $carriageReturns
        LastWriteUtc    = $item.LastWriteTimeUtc.ToString("yyyy-MM-ddTHH:mm:ssZ")
        JsonError       = $parseError
    }
}

function Read-Manifest {
    param([string]$Root)
    $path = Join-Path $Root (Join-Path $script:CorpusRelative "manifest.json")
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "manifest.json が見つかりません: $path"
    }
    $text = $script:Utf8NoBom.GetString([System.IO.File]::ReadAllBytes($path))
    return [pscustomobject]@{
        Path = $path
        Json = $text | ConvertFrom-Json -ErrorAction Stop
    }
}

function Get-ManifestEntryMap {
    param($Manifest)
    $map = @{}
    foreach ($entry in @($Manifest.Json.entries)) {
        $map[[string]$entry.id] = $entry
    }
    return $map
}

function Write-ReportTable {
    param([string]$Root)

    $manifest = Read-Manifest $Root
    $entries = Get-ManifestEntryMap $manifest
    $attributeOk = Test-CorpusGitAttribute $Root

    Write-Host ""
    Write-Host "外部corpus 受け入れ状況" -ForegroundColor Cyan
    Write-Host "  corpus: $(Join-Path $Root $script:CorpusRelative)"
    Write-Host "  manifest schema_version: $($manifest.Json.schema_version)"
    Write-Host "  classification_policy.frozen_at_utc: $($manifest.Json.classification_policy.frozen_at_utc)"
    if ($attributeOk) {
        Write-Host "  .gitattributes の -text: あり（取り出しでCRLFへ変わらない）" -ForegroundColor Green
    }
    else {
        Write-Host "  .gitattributes の -text: なし。Windowsの取り出しで改行がCRLFへ変わり、記録したbyte数・SHA-256と一致しなくなります。" -ForegroundColor Red
        Write-Host "    追記する行: $script:AttributePattern/** -text"
    }
    Write-Host ""

    $format = "{0,-22} {1,-18} {2,-7} {3,-9} {4,-10} {5,-8} {6}"
    Write-Host ($format -f "id", "source", "置いた", "file_spec", "byte数", "manifest", "備考")
    Write-Host ("-" * 100)

    $summary = [ordered]@{
        present = 0; missing = 0; registered = 0; mismatched = 0; crlf = 0
        spec_1_1 = 0; spec_1_2 = 0; spec_other = 0
    }
    $missingBySource = [ordered]@{}
    foreach ($source in $script:SourceQuotas.Keys) { $missingBySource[$source] = 0 }

    foreach ($slot in Get-ReservedSlots) {
        $facts = Get-RawFacts $Root $slot.Path
        $entry = if ($entries.ContainsKey($slot.Id)) { $entries[$slot.Id] } else { $null }
        $notes = New-Object System.Collections.Generic.List[string]

        if ($null -eq $facts) {
            $summary.missing++
            $missingBySource[$slot.Source]++
            Write-Host ($format -f $slot.Id, $slot.Source, "未", "-", "-", $(if ($entry) { "あり" } else { "なし" }), $(if ($entry) { "manifestにあるのに実体が無い" } else { "" }))
            continue
        }

        $summary.present++
        if ($facts.CarriageReturns -gt 0) {
            $summary.crlf++
            $notes.Add("CR $($facts.CarriageReturns) 個")
        }
        if ($null -ne $facts.JsonError) { $notes.Add("JSONとして読めない") }
        switch ($facts.FileSpec) {
            1.1 { $summary.spec_1_1++ }
            1.2 { $summary.spec_1_2++ }
            default { $summary.spec_other++; $notes.Add("file_spec が 1.1/1.2 以外") }
        }

        $manifestState = "なし"
        if ($null -ne $entry) {
            $summary.registered++
            $manifestState = "あり"
            if ([int64]$entry.byte_length -ne $facts.ByteLength) {
                $summary.mismatched++
                $notes.Add("byte数 記録$($entry.byte_length)/実体$($facts.ByteLength)")
            }
            if ([string]$entry.sha256 -ne $facts.Sha256) {
                $notes.Add("SHA-256 不一致")
            }
            if ([string]$entry.path -ne $slot.Path) {
                $notes.Add("path が予約と違う")
            }
        }

        Write-Host ($format -f $slot.Id, $slot.Source, "済", $facts.FileSpec, $facts.ByteLength, $manifestState, ($notes -join " / "))
    }

    Write-Host ""
    Write-Host "集計" -ForegroundColor Cyan
    Write-Host "  置かれている: $($summary.present) / 30    未着: $($summary.missing)"
    Write-Host "  manifest 登録済み: $($summary.registered)    照合値の不一致: $($summary.mismatched)    CRLF混入: $($summary.crlf)"
    Write-Host "  file_spec 1.1: $($summary.spec_1_1)    1.2: $($summary.spec_1_2)    その他: $($summary.spec_other)"
    Write-Host ""
    Write-Host "改善計画 §12.6-1 が求める内訳（30件中）" -ForegroundColor Cyan
    $need11 = [Math]::Max(0, 16 - $summary.spec_1_1)
    Write-Host "  FOLD 1.1 を16件以上: 現在 $($summary.spec_1_1) 件。あと $need11 件必要。"
    Write-Host "  合計30件         : 現在 $($summary.present) 件。あと $($summary.missing) 件必要。"
    Write-Host "  未着の枠: " -NoNewline
    Write-Host (($missingBySource.Keys | ForEach-Object { "$_ $($missingBySource[$_])件" }) -join " / ")
    Write-Host "  FOLD 1.2 は外部見本では数えません（利用者の決定 2026-08-29）。"
    Write-Host "  自前で書き出した1.2を読み戻して一致することで満たし、"
    Write-Host "  その担保は fold_internal_samples.rs の100回連続の往復です。"
    Write-Host "  参考: 置かれている中の file_spec 1.2 は $($summary.spec_1_2) 件です。"
    Write-Host ""
}

function New-DecisionTemplate {
    param([string]$Root, [string]$OutputPath)

    $manifest = Read-Manifest $Root
    $entries = Get-ManifestEntryMap $manifest
    $items = New-Object System.Collections.Generic.List[object]

    foreach ($slot in Get-ReservedSlots) {
        if ($entries.ContainsKey($slot.Id)) { continue }
        $facts = Get-RawFacts $Root $slot.Path
        if ($null -eq $facts) { continue }

        $items.Add([ordered]@{
            id                    = $slot.Id
            source                = $slot.Source
            "_measured_byte_length" = $facts.ByteLength
            "_measured_sha256"      = $facts.Sha256
            "_measured_file_spec"   = $facts.FileSpec
            generator             = $script:Placeholder
            generator_version     = $script:Placeholder
            source_uri            = $script:Placeholder
            classification        = [ordered]@{
                expected           = $script:Placeholder
                basis              = $script:Placeholder
                unsupported_paths  = @()
            }
            observed              = [ordered]@{
                result   = $script:Placeholder
                observed_at_utc = $script:Placeholder
                method   = $script:Placeholder
                warnings = @()
                errors   = @()
            }
            rights                = [ordered]@{
                content_spdx        = $script:ApprovedLicense
                content_evidence    = $script:Placeholder
                rights_holder       = $script:Placeholder
                authorization_date  = $script:Placeholder
                authorization_scope = $script:Placeholder
                reviewer            = $script:Placeholder
                reviewed_on         = $script:Placeholder
            }
        })
    }

    if ($items.Count -eq 0) {
        Write-Host "manifest に無い新しいファイルはありません。下書きは作りません。" -ForegroundColor Yellow
        return
    }

    $template = [ordered]@{
        "_readme" = @(
            "$script:Placeholder を全て埋めてから -Register で渡してください。",
            "byte_length・sha256・path・source_file_last_write_utc は台本が実体から取り直すので書きません。",
            "classification は製品を動かす前に、生のJSONと classification_policy.rules だけを見て決めます。",
            "observed は製品を動かした結果を code と path だけで書きます。散文は書けません。"
        )
        entries   = $items
    }
    $json = $template | ConvertTo-Json -Depth 12
    [System.IO.File]::WriteAllText([System.IO.Path]::GetFullPath($OutputPath), $json, $script:Utf8NoBom)
    Write-Host "下書きを書きました（$($items.Count) 件）: $OutputPath" -ForegroundColor Green
}

function Assert-Filled {
    param($Value, [string]$Label)
    if ($null -eq $Value) { throw "$Label が空です" }
    $text = [string]$Value
    if ($text.Trim() -eq "" -or $text -eq $script:Placeholder) {
        throw "$Label が未記入です（$script:Placeholder のままです）"
    }
    if ($text.Trim().ToUpperInvariant() -eq "NOASSERTION") {
        throw "$Label に NOASSERTION は使えません"
    }
    return $text
}

function Assert-Utc {
    param([string]$Value, [string]$Label)
    if (-not ($Value.Contains("T") -and $Value.EndsWith("Z"))) {
        throw "$Label は UTC の時刻（…T…Z）で書いてください: $Value"
    }
    return $Value
}

function Convert-IssueList {
    param($Issues, [string]$Label)
    $result = New-Object System.Collections.Generic.List[object]
    foreach ($issue in @($Issues)) {
        $code = Assert-Filled $issue.code "$Label.code"
        $path = Assert-Filled $issue.path "$Label.path"
        if (-not $path.StartsWith('$')) { throw "$Label.path は JSON path（先頭が `$`）で書いてください: $path" }
        $item = [ordered]@{ code = $code; path = $path }
        if ($null -ne $issue.PSObject.Properties["value"]) {
            $value = $issue.value
            if ($value -isnot [int] -and $value -isnot [long] -and $value -isnot [double] -and $value -isnot [decimal]) {
                throw "$Label.value は数値だけです"
            }
            $item["value"] = $value
        }
        $result.Add($item)
    }
    return $result
}

function Invoke-Register {
    param([string]$Root, [string]$DecisionsPath)

    if (-not (Test-CorpusGitAttribute $Root)) {
        throw ".gitattributes に `"$script:AttributePattern/** -text`" がありません。先に足してください。CRLFへ変わると記録した SHA-256 と実体が一致しません。"
    }

    $manifest = Read-Manifest $Root
    $existing = Get-ManifestEntryMap $manifest
    $policyFrozenAt = [string]$manifest.Json.classification_policy.frozen_at_utc
    $slots = @{}
    foreach ($slot in Get-ReservedSlots) { $slots[$slot.Id] = $slot }

    $decisionsText = $script:Utf8NoBom.GetString([System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $DecisionsPath).Path))
    $decisions = $decisionsText | ConvertFrom-Json -ErrorAction Stop

    $added = New-Object System.Collections.Generic.List[object]
    $seenSha = New-Object "System.Collections.Generic.HashSet[string]"
    foreach ($entry in @($manifest.Json.entries)) { [void]$seenSha.Add([string]$entry.sha256) }

    foreach ($decision in @($decisions.entries)) {
        $id = Assert-Filled $decision.id "id"
        if (-not $slots.ContainsKey($id)) { throw "予約していない id です: $id" }
        if ($existing.ContainsKey($id)) { throw "すでに manifest にある id です: $id" }
        $slot = $slots[$id]

        $facts = Get-RawFacts $Root $slot.Path
        if ($null -eq $facts) { throw "実体がありません: $($slot.Path)" }
        if ($facts.CarriageReturns -gt 0) {
            throw "$id に CR が $($facts.CarriageReturns) 個あります。取り出しで改行が変わった疑いがあるので登録しません。"
        }
        if ($null -ne $facts.JsonError) { throw "$id は JSON として読めません: $($facts.JsonError)" }
        if ($null -eq $facts.FileSpec) { throw "$id に file_spec がありません" }
        if (-not $seenSha.Add($facts.Sha256)) { throw "$id の SHA-256 が別のentryと同じです（同じファイルの重複）" }

        $expected = Assert-Filled $decision.classification.expected "$id.classification.expected"
        if ($expected -ne "supported" -and $expected -ne "unsupported") {
            throw "$id.classification.expected は supported か unsupported です: $expected"
        }
        $unsupportedPaths = @($decision.classification.unsupported_paths)
        foreach ($path in $unsupportedPaths) {
            if (-not ([string]$path).StartsWith('$')) { throw "$id.classification.unsupported_paths は JSON path です: $path" }
        }
        if ($expected -eq "supported" -and $unsupportedPaths.Count -gt 0) {
            throw "$id は supported なので unsupported_paths を空にしてください"
        }
        if ($expected -eq "unsupported" -and $unsupportedPaths.Count -eq 0) {
            throw "$id は unsupported なので unsupported_paths を1つ以上書いてください"
        }

        $observedResult = Assert-Filled $decision.observed.result "$id.observed.result"
        $observedErrors = Convert-IssueList $decision.observed.errors "$id.observed.errors"
        $observedWarnings = Convert-IssueList $decision.observed.warnings "$id.observed.warnings"
        if ($observedResult -eq "supported" -and $observedErrors.Count -gt 0) {
            throw "$id.observed は supported なので errors を空にしてください"
        }
        if ($observedResult -eq "unsupported" -and $observedErrors.Count -eq 0) {
            throw "$id.observed は unsupported なので errors を1つ以上書いてください"
        }
        if ($observedResult -ne "supported" -and $observedResult -ne "unsupported") {
            throw "$id.observed.result は supported か unsupported です: $observedResult"
        }

        $rights = [ordered]@{
            content_spdx                      = Assert-Filled $decision.rights.content_spdx "$id.rights.content_spdx"
            content_evidence                  = Assert-Filled $decision.rights.content_evidence "$id.rights.content_evidence"
            rights_holder                     = Assert-Filled $decision.rights.rights_holder "$id.rights.rights_holder"
            authorization_date                = Assert-Filled $decision.rights.authorization_date "$id.rights.authorization_date"
            authorization_scope               = Assert-Filled $decision.rights.authorization_scope "$id.rights.authorization_scope"
            generator_license_used_for_content = $false
            reviewer                          = Assert-Filled $decision.rights.reviewer "$id.rights.reviewer"
            reviewed_on                       = Assert-Filled $decision.rights.reviewed_on "$id.rights.reviewed_on"
            redistribution_allowed            = $true
        }
        if ($rights.content_spdx -ne $script:ApprovedLicense) {
            throw "$id.rights.content_spdx は $script:ApprovedLicense だけです"
        }

        $added.Add([ordered]@{
            id                        = $id
            source                    = $slot.Source
            generator                 = Assert-Filled $decision.generator "$id.generator"
            generator_version         = Assert-Filled $decision.generator_version "$id.generator_version"
            source_uri                = Assert-Filled $decision.source_uri "$id.source_uri"
            source_file_last_write_utc = Assert-Utc $facts.LastWriteUtc "$id.source_file_last_write_utc"
            path                      = $slot.Path
            byte_length               = $facts.ByteLength
            sha256                    = $facts.Sha256
            classification            = [ordered]@{
                expected          = $expected
                frozen_at_utc     = Assert-Utc $policyFrozenAt "classification_policy.frozen_at_utc"
                basis             = Assert-Filled $decision.classification.basis "$id.classification.basis"
                unsupported_paths = $unsupportedPaths
            }
            observed                  = [ordered]@{
                result   = $observedResult
                observed_at_utc = Assert-Utc (Assert-Filled $decision.observed.observed_at_utc "$id.observed.observed_at_utc") "$id.observed.observed_at_utc"
                method   = Assert-Filled $decision.observed.method "$id.observed.method"
                excluded_from_frozen_classification = $true
                warnings = $observedWarnings
                errors   = $observedErrors
            }
            rights                    = $rights
        })
    }

    if ($added.Count -eq 0) {
        Write-Host "登録する新しいentryがありません。manifest は変えません。" -ForegroundColor Yellow
        return
    }

    $allEntries = New-Object System.Collections.Generic.List[object]
    foreach ($entry in @($manifest.Json.entries)) { $allEntries.Add($entry) }
    foreach ($entry in $added) { $allEntries.Add([pscustomobject]$entry) }

    $order = @{}
    $rank = 0
    foreach ($slot in Get-ReservedSlots) { $order[$slot.Id] = $rank; $rank++ }
    $sorted = $allEntries | Sort-Object { $order[[string]$_.id] }

    $manifest.Json.entries = @($sorted)
    $manifest.Json.tranche_status = "partial_$($sorted.Count)_of_reserved_30"
    if ($sorted.Count -eq 30) { $manifest.Json.tranche_status = "complete_30_of_reserved_30" }

    $json = $manifest.Json | ConvertTo-Json -Depth 20
    $temporary = "$($manifest.Path).$PID.tmp"
    [System.IO.File]::WriteAllText($temporary, ($json -replace "`r`n", "`n"), $script:Utf8NoBom)
    Move-Item -LiteralPath $temporary -Destination $manifest.Path -Force
    Write-Host "manifest へ $($added.Count) 件を登録しました。合計 $($sorted.Count) 件。" -ForegroundColor Green
    Write-Host "次に必ず実行してください:" -ForegroundColor Cyan
    Write-Host "  cargo test -p ori3-export --test fold_external_corpus_intake"
}

$root = Resolve-Root $RepoRoot
switch ($PSCmdlet.ParameterSetName) {
    "EmitTemplate" { New-DecisionTemplate $root $EmitTemplate }
    "Register" { Invoke-Register $root $Register }
    default { Write-ReportTable $root }
}
