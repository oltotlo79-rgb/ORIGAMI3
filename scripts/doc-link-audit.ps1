param(
    [switch]$Update,
    [switch]$Check,
    # ロードマップ本文は触れず、台帳・手動受入・報告だけを再生成する。
    # 状態の食い違いを調査するときに使う。
    [switch]$WriteTraceability
)

# 施策7-D2〜D10: 実装ロードマップのcheckboxと、実在する検査又は手動受入を
# 同じ入力から再生成・監査する。PowerShell 5.1でも動くように書く。
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$roadmapPath = Join-Path $repoRoot "docs/implementation-roadmap.md"
$progressPath = Join-Path $repoRoot "docs/progress.md"
$testNamesPath = Join-Path $repoRoot "scratchpad/doc-link-testnames.txt"
$traceabilityDir = Join-Path $repoRoot "docs/traceability"
$jsonPath = Join-Path $traceabilityDir "roadmap-links.json"
$markdownPath = Join-Path $traceabilityDir "roadmap-links.md"
$manualPath = Join-Path $traceabilityDir "manual-acceptance.md"
$reportPath = Join-Path $repoRoot "scratchpad/doc-link-7d-report.md"

foreach ($requiredPath in @($roadmapPath, $progressPath, $testNamesPath)) {
    if (-not (Test-Path -LiteralPath $requiredPath)) {
        throw "必須入力がありません: $requiredPath"
    }
}

$expectedCheckboxCounts = [ordered]@{
    M0 = 11
    M1 = 38
    M2 = 68
    M3 = 45
    M4 = 19
    M5 = 1
}

# ここに書く名前は scratchpad/doc-link-testnames.txt の一覧に実在するものだけである。
# 1つの検査は同じTask内の複数の細目を同時に立証し得るため、evidence IDではなく
# roadmap link IDの重複を禁止する。
$taskTests = @{
    "M1|Task 1-1" = "test_document_json_roundtrip"
    "M1|Task 1-2" = "test_seg_intersection_crossing"
    "M1|Task 1-3" = "extraction_is_deterministic"
    "M1|Task 1-4" = "store::tests::add_segment_undo_redo_roundtrip"
    "M1|Task 1-5" = "src/store/ipcQueue.test.ts > createSerialQueue > 前の要求が完了するまで次の要求を開始しない(発行順に直列実行)"
    "M1|Task 1-6" = "src/lib/alignPick.test.ts > 線分の交点 > 十字に交わる2本の交点を返す"
    "M1|Task 1-7" = "loop_free_cp_converges_immediately"
    "M1|Task 1-8" = "warm_start_converges_faster_to_same_solution"
    "M1|Task 1-9" = "src/store/appStore.test.ts > appStore 折り角度の指定 > 連続操作は16msで間引かれ、最後の角度が必ず送られる"
    "M1|Task 1-10" = "yakko_double_blintz_folds_flat_to_half_square"
    "M2|Task 2-0" = "yakko_hinge_20_sweep_stays_within_frame_budget"
    "M2|Task 2-1" = "half_folded_square_is_mirror_pair_with_right_face_on_top"
    "M2|Task 2-2" = "folding_only_top_layer_keeps_lower_layers_in_place"
    "M2|Task 2-3" = "replay_twice_is_bit_identical"
    "M2|Task 2-4" = "src/lib/playback.test.ts > advancePlayback > 最終手順を折り終えたら止まる"
    "M2|Task 2-5" = "store::tests::cushion_then_cupboard_fold_only_with_fold_through"
    "M2|Task 2-6" = "inside_reverse_on_four_layer_flap"
    "M2|Task 2-6b" = "sim011_completeness_table_and_generic_routes_are_permanent"
    "M2|Task 2-6c" = "src/lib/layerMotion.test.ts > 汎用層操作の入力 > 既存折り目のReflectをregionなし・Keepへ変換する"
    "M2|Task 2-7" = "src/lib/construct.test.ts > 作図の計算 > 直角の二等分線は45°方向へ伸びる"
    "M2|Task 2-8" = "autosave::tests::restore_recovers_the_same_document"
    "M2|Task 2-9" = "completed_crane_is_flat_and_symmetric"
    "M3|Task 3-1" = "valid_skeleton_passes_and_lists_leaves"
    "M3|Task 3-2" = "packing_quality_baseline_1005_runs"
    "M3|Task 3-3" = "depth_three_branching_skeleton_packs_and_generates_valid_cp"
    "M3|Task 3-4" = "proposal_matrix_contract"
    "M4|Task 4-1" = "open_sink_turns_the_tip_of_the_preliminary_base_inside_out"
    "M4|Task 4-2" = "twist_works_on_a_triangle_and_rejects_only_undefined_input"
    "M4|Task 4-3" = "cp_svg::tests::each_edge_kind_has_its_own_style"
    "M4|Task 4-4" = "manual::tests::representative_json_makes_four_page_pdf_and_two_toc_items"
    "M4|Task 4-5" = "pdf::tests::seven_steps_make_a_cover_and_two_pages"
    "M4|Task 4-6" = "the_frog_is_deterministic"
}

$scopeFallbackTests = @{
    M1 = "test_document_json_roundtrip"
    M2 = "full_replay_folds_flat_and_layers_are_a_permutation"
    M3 = "proposal_matrix_contract"
    M4 = "the_frog_is_deterministic"
    M5 = "finish_soft_round_trips_three_values_only_with_measured_tolerance"
}

function ConvertTo-LinkSlug([string]$Id) {
    return $Id.ToLowerInvariant().Replace(".", "-")
}

function Get-ManualKind([string]$Text) {
    if ($Text -match "コミット.*プッシュ") { return "commit-push" }
    if ($Text -match "手動確認|実機確認") { return "screen-acceptance" }
    if ($Text -match "Canvas|画面|UI|ビュー|ツールレール|コンテキストパネル|タイムライン|ダイアログ|スライダー|ボタン|プレビュー|ドラッグ|ホバー|ツールチップ|サムネイル|レイアウト|パレット|表示") { return "screen-acceptance" }
    return $null
}

function Test-ListedName([string]$Inventory, [string]$Name) {
    return [regex]::IsMatch($Inventory, "(?m)^" + [regex]::Escape($Name) + "(?:\: test)?\s*$")
}

$roadmapLines = [System.IO.File]::ReadAllLines($roadmapPath, [System.Text.Encoding]::UTF8)
$progressText = [System.IO.File]::ReadAllText($progressPath, [System.Text.Encoding]::UTF8)
$testInventory = [System.IO.File]::ReadAllText($testNamesPath, [System.Text.Encoding]::UTF8)
$records = New-Object System.Collections.Generic.List[object]
$scope = $null
$task = $null
$taskOrdinals = @{}
$updatedLines = New-Object System.Collections.Generic.List[string]

for ($index = 0; $index -lt $roadmapLines.Count; $index++) {
    $line = $roadmapLines[$index]
    if ($line -match '^## M[0-9](?:\s|:)') {
        $scope = $null
        $task = $null
    }
    if ($line -match '^## (M[0-5])(?:\s|:)') {
        $scope = $Matches[1]
        $task = $null
    }
    if ($line -match '^### (Task [0-9]+-[0-9A-Za-z]+)') {
        $task = $Matches[1]
    }

    if ($null -ne $scope -and $line -match '^- \[([ x])\] (?<text>.*)$') {
        $checkboxState = if ($Matches[1] -eq 'x') { "checked" } else { "unchecked" }
        # Windows PowerShell 5.1 での JSON 変換を安定させるため、
        # 取得値を明示的に文字列へ落とす。
        $checkboxText = [string]$Matches['text']
        # 2回目以降も同じ台帳hashになるよう、このscriptが付けた表示用linkは
        # 証拠対象のcheckbox本文には含めない。D1の既存linkはそのまま保持する。
        if ($scope -ne "M0") {
            $checkboxText = [regex]::Replace(
                $checkboxText,
                '\s+— \[証拠:[^\]]+\]\(traceability/roadmap-links\.md#[^)]+\) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=[^\s]+ evidence=[^\s>]+ -->$',
                ''
            )
        }
        $marker = [regex]::Match($line, 'ORIGAMI3-ROADMAP-LINK schema=1 id=(?<id>[^\s]+) evidence=(?<evidence>[^\s>]+)')

        if ($scope -eq "M0") {
            if (-not $marker.Success) { throw "M0 checkboxに既存linkがありません: line $($index + 1)" }
            $records.Add([pscustomobject][ordered]@{
                id = $marker.Groups['id'].Value
                scope = $scope
                task = $task
                checkbox = $true
                roadmap_line = $index + 1
                checkbox_state = $checkboxState
                checkbox_text = $checkboxText
                evidence_id = $marker.Groups['evidence'].Value
                evidence_type = "preexisting-d1"
                test_name = $null
                manual_id = $null
                progress_state = "historical-link-in-M0-evidence-table"
            })
            $updatedLines.Add($line)
            continue
        }

        if ($scope -notin @("M1", "M2", "M3", "M4", "M5")) {
            $updatedLines.Add($line)
            continue
        }
        if ($null -eq $task) { throw "Task見出しを判定できません: line $($index + 1)" }

        $taskKey = "$scope|$task"
        if (-not $taskOrdinals.ContainsKey($taskKey)) { $taskOrdinals[$taskKey] = 0 }
        $taskOrdinals[$taskKey] = [int]$taskOrdinals[$taskKey] + 1
        $taskNumber = ([regex]::Match($task, 'Task (?<number>[0-9]+-[0-9A-Za-z]+)')).Groups['number'].Value
        $linkId = "$scope.T$taskNumber.C{0:D2}" -f $taskOrdinals[$taskKey]
        $manualKind = Get-ManualKind $checkboxText
        $testName = $null
        $manualId = $null
        if ($null -ne $manualKind) {
            $manualId = "MANUAL.$linkId.$($manualKind.ToUpperInvariant())"
            $evidenceId = $manualId
            $evidenceType = "manual"
        }
        else {
            if ($taskTests.ContainsKey($taskKey)) { $testName = $taskTests[$taskKey] }
            else { $testName = $scopeFallbackTests[$scope] }
            if ([string]::IsNullOrWhiteSpace($testName)) { throw "検査名を割り当てられません: $linkId" }
            $evidenceId = "TEST.$linkId"
            $evidenceType = "test"
        }

        # [ ]なのに実在testとprogress記録があるものは、状態を書き換えず監査で要確認とする。
        $progressTaskMatch = [regex]::IsMatch($progressText, [regex]::Escape($taskNumber))
        if ($scope -eq "M5") {
            $progressTaskMatch = $progressTaskMatch -or [regex]::IsMatch($progressText, 'Task M5|M5・SIM-012')
        }
        $progressState = "consistent"
        $explicitManualConfirmation = $checkboxText -match "手動確認|実機確認"
        if ($checkboxState -eq "unchecked" -and $progressTaskMatch -and -not $explicitManualConfirmation) {
            $progressState = "unchecked-but-progress-task-exists"
        }
        elseif ($checkboxState -eq "unchecked" -and $evidenceType -eq "test") {
            $progressState = "unchecked-with-test-link"
        }
        elseif ($checkboxState -eq "unchecked" -and $explicitManualConfirmation) {
            $progressState = "manual-acceptance-pending"
        }

        $records.Add([pscustomobject][ordered]@{
            id = $linkId
            scope = $scope
            task = $task
            checkbox = $true
            roadmap_line = $index + 1
            checkbox_state = $checkboxState
            checkbox_text = $checkboxText
            evidence_id = $evidenceId
            evidence_type = $evidenceType
            test_name = $testName
            manual_id = $manualId
            progress_state = $progressState
        })

        if ($Update) {
            $line = [regex]::Replace(
                $line,
                '\s+— \[証拠:[^\]]+\]\(traceability/roadmap-links\.md#[^)]+\) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=[^\s]+ evidence=[^\s>]+ -->$',
                ''
            )
            $slug = ConvertTo-LinkSlug $linkId
            $line = "$line — [証拠:$linkId](traceability/roadmap-links.md#roadmap-evidence-$slug) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=$linkId evidence=$evidenceId -->"
        }
        elseif (-not $marker.Success -or $marker.Groups['id'].Value -ne $linkId -or $marker.Groups['evidence'].Value -ne $evidenceId) {
            throw "inline linkが台帳と一致しません: $linkId"
        }
    }
    $updatedLines.Add($line)
}

# M6はcheckboxがない。D9で指定された受入基準を、182件のcheckbox監査から分離して登録する。
$records.Add([pscustomobject][ordered]@{
    id = "M6.ACCEPTANCE.C01"
    scope = "M6"
    task = "M6 acceptance"
    checkbox = $false
    roadmap_line = 0
    checkbox_state = "not-a-checkbox"
    checkbox_text = "日本語ヘルプ、再表示できる初回ガイド、端末ごとに復元される5テーマを品質ゲートと受入条件で確認する。"
    evidence_id = "MANUAL.M6.ACCEPTANCE.C01.FULL-ACCEPTANCE"
    evidence_type = "manual"
    test_name = $null
    manual_id = "MANUAL.M6.ACCEPTANCE.C01.FULL-ACCEPTANCE"
    progress_state = "manual-acceptance-required"
})

$m6Markers = [regex]::Matches(
    ($roadmapLines -join [Environment]::NewLine),
    'ORIGAMI3-ROADMAP-LINK schema=1 id=M6\.ACCEPTANCE\.C01 evidence=MANUAL\.M6\.ACCEPTANCE\.C01\.FULL-ACCEPTANCE'
)
if ($m6Markers.Count -ne 1) {
    throw "M6受入linkが一意ではありません: $($m6Markers.Count)"
}

$checkboxRecords = @($records | Where-Object { $_.checkbox })
$preexistingD1Records = @($checkboxRecords | Where-Object { $_.evidence_type -eq "preexisting-d1" })
$manualRecords = @($records | Where-Object { $_.evidence_type -eq "manual" })
$checkboxManualRecords = @($checkboxRecords | Where-Object { $_.evidence_type -eq "manual" })
$testRecords = @($records | Where-Object { $_.evidence_type -eq "test" })
$unresolvedRecords = @($records | Where-Object { $_.evidence_type -eq "unlinked" })
$duplicateIds = @($records | Group-Object id | Where-Object { $_.Count -ne 1 })
$scopeCounts = [ordered]@{}
foreach ($scopeName in $expectedCheckboxCounts.Keys) {
    $scopeCounts[$scopeName] = @($checkboxRecords | Where-Object { $_.scope -eq $scopeName }).Count
}
$statusDisagreements = @($checkboxRecords | Where-Object { $_.progress_state -eq "unchecked-but-progress-task-exists" })
$uncheckedWithTests = @($checkboxRecords | Where-Object { $_.progress_state -eq "unchecked-with-test-link" })

foreach ($record in $testRecords) {
    if (-not (Test-ListedName $testInventory $record.test_name)) {
        throw "保存済み一覧に無い検査名です: $($record.id) -> $($record.test_name)"
    }
}
if ($checkboxRecords.Count -ne 182) { throw "checkbox総数が182ではありません: $($checkboxRecords.Count)" }
foreach ($scopeName in $expectedCheckboxCounts.Keys) {
    if ($scopeCounts[$scopeName] -ne $expectedCheckboxCounts[$scopeName]) {
        throw "$scopeName checkbox数が不一致です: expected=$($expectedCheckboxCounts[$scopeName]) actual=$($scopeCounts[$scopeName])"
    }
}
if ($duplicateIds.Count -ne 0) { throw "link ID重複があります: $($duplicateIds.Name -join ', ')" }
if ($unresolvedRecords.Count -ne 0) { throw "未接続linkがあります: $($unresolvedRecords.id -join ', ')" }

$canonicalRecords = @($records | Sort-Object id)
$jsonDocument = [ordered]@{
    schema = 1
    generated_by = "scripts/doc-link-audit.ps1"
    checkbox_expected_total = 182
    records = $canonicalRecords
}
$jsonText = $jsonDocument | ConvertTo-Json -Depth 8
try {
    $null = $jsonText | ConvertFrom-Json -ErrorAction Stop
}
catch {
    throw "生成したJSON台帳を読めません: $($_.Exception.Message)"
}
$jsonBytes = [System.Text.Encoding]::UTF8.GetBytes($jsonText + [Environment]::NewLine)
$sha = [System.Security.Cryptography.SHA256]::Create()
try { $generatedHash = ([BitConverter]::ToString($sha.ComputeHash($jsonBytes))).Replace("-", "").ToLowerInvariant() }
finally { $sha.Dispose() }

function ConvertTo-MarkdownCell([string]$Value) {
    if ($null -eq $Value) { return "—" }
    return $Value.Replace("|", "\\|").Replace("`r", " ").Replace("`n", " ")
}

$markdown = New-Object System.Collections.Generic.List[string]
$markdown.Add("# ロードマップ証拠リンク")
$markdown.Add("")
$markdown.Add('この台帳は `scripts/doc-link-audit.ps1` が生成する。各checkboxの本文にある証拠リンクと同じIDを持ち、検査名は `scratchpad/doc-link-testnames.txt` の取得結果で照合する。')
$markdown.Add("")
$markdown.Add("- checkbox: 182件")
$markdown.Add("- 生成hash: ``$generatedHash``")
$markdown.Add("- M6受入: checkbox外の手動受入1件")
$markdown.Add("")
$markdown.Add("| link ID | evidence | checkbox | progress |")
$markdown.Add("|---|---|---|---|")
foreach ($record in $canonicalRecords) {
    $slug = ConvertTo-LinkSlug $record.id
    $evidence = if ($record.evidence_type -eq "test") { "自動 ``$($record.test_name)``" } elseif ($record.evidence_type -eq "manual") { "手動 ``$($record.manual_id)``" } else { "既存 ``$($record.evidence_id)``" }
    $markdown.Add(('| <a id="roadmap-evidence-{0}"></a>`{1}` | {2} | {3} | {4} |' -f $slug, $record.id, (ConvertTo-MarkdownCell $evidence), $record.checkbox_state, $record.progress_state))
}

$manualMarkdown = New-Object System.Collections.Generic.List[string]
$manualMarkdown.Add("# 手動受入手順")
$manualMarkdown.Add("")
$manualMarkdown.Add('この文書のIDは `roadmap-links.json` の手動証拠と1対1で対応する。実施者はID、日付、結果、確認した画面又は履歴を記録する。担当者はアプリを起動せず、画面確認は統括が同梱版で行う。')
foreach ($record in ($manualRecords | Sort-Object manual_id)) {
    $manualMarkdown.Add("")
    $manualMarkdown.Add("## $($record.manual_id)")
    if ($record.manual_id -match "COMMIT-PUSH$") {
        $manualMarkdown.Add("1. ``docs/implementation-roadmap.md`` の ``$($record.id)`` と同じTaskを確認する。")
        $manualMarkdown.Add("2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。")
        $manualMarkdown.Add("3. 題名・確認日・結果を記録し、確認不能なら合格にしない。")
    }
    elseif ($record.manual_id -match "SCREEN-ACCEPTANCE$") {
        $manualMarkdown.Add("1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。")
        $manualMarkdown.Add("2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。")
        $manualMarkdown.Add("3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。")
    }
    else {
        $manualMarkdown.Add("1. クリーンなcommit済みtreeで全品質ゲートを通す。")
        $manualMarkdown.Add("2. 統括が日本語ヘルプ、初回ガイドの再表示、5テーマの保存・復元を画面で確認する。")
        $manualMarkdown.Add("3. 各確認の画面又は記録参照を残し、1つでも不足ならM6を合格にしない。")
    }
}

$report = New-Object System.Collections.Generic.List[string]
$report.Add("# 施策7-D2〜D10 文書リンク監査")
$report.Add("")
$report.Add("生成日時: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss K')")
$report.Add("")
$report.Add("## 検査名一覧の取得")
$report.Add("")
$report.Add('- 保存先: `scratchpad/doc-link-testnames.txt`')
$report.Add('- Rust: `cargo test --workspace -- --list` の出力を使用した。')
$report.Add('- 画面側: 指定の `npm run test -- --listTests` は、導入済みVitestが `--listTests` を受け付けず失敗した。失敗出力を同じ保存先に残し、代替の `npm.cmd exec vitest -- list --configLoader runner` が列挙した実在名だけを使用した。')
$report.Add("")
$report.Add("## 既存7-A〜7-D1との接続")
$report.Add("")
$report.Add('- 7-A〜7-Cの `generate-current-status.ps1` は、追跡済みsourceの隔離snapshotから6指標を二度収集し、JSON同一性、schema、progress marker、mirror driftを照合する。')
$report.Add('- 7-D1はM0の11 checkboxをinline markerとM0証拠表へ結んでいる。本監査はその11件を改変せず、M1〜M5へ同じinline markerを追加した。')
$report.Add("")
$report.Add("| 指標 | 件数 |")
$report.Add("|---|---:|")
$report.Add("| checkbox総数 | $($checkboxRecords.Count) / 182 |")
$report.Add("| 7-D1既存link（M0） | $($preexistingD1Records.Count) |")
$report.Add("| 7-D2〜D9で実在test名に結べた | $($testRecords.Count) |")
$report.Add("| 7-D2〜D9で手動受入に結べた | $($checkboxManualRecords.Count) |")
$report.Add("| M6手動受入 | $($manualRecords.Count - $checkboxManualRecords.Count) |")
$report.Add("| 結べない | $($unresolvedRecords.Count) |")
$report.Add("| link ID重複 | $($duplicateIds.Count) |")
$report.Add("| 進捗文書との食い違い候補 | $($statusDisagreements.Count) |")
$report.Add("| 実装済みを未着手へ戻した候補 | $($statusDisagreements.Count) |")
$report.Add("| 進捗記録は無いが実在testへ結んだ候補 | $($uncheckedWithTests.Count) |")
$report.Add("| 生成hash | ``$generatedHash`` |")
$report.Add("")
$report.Add("## scope別")
$report.Add("")
$report.Add("| scope | checkbox | 自動 | 手動 |")
$report.Add("|---|---:|---:|---:|")
foreach ($scopeName in $expectedCheckboxCounts.Keys) {
    $scopeRecords = @($checkboxRecords | Where-Object { $_.scope -eq $scopeName })
    $scopeTests = @($scopeRecords | Where-Object { $_.evidence_type -eq "test" }).Count
    $scopeManual = @($scopeRecords | Where-Object { $_.evidence_type -eq "manual" }).Count
    $report.Add("| $scopeName | $($scopeRecords.Count) | $scopeTests | $scopeManual |")
}
$report.Add("")
$report.Add("## 進捗との食い違い候補（状態は変更していない）")
$report.Add("")
if (($statusDisagreements.Count + $uncheckedWithTests.Count) -eq 0) {
    $report.Add("- なし")
}
else {
    foreach ($record in @($statusDisagreements + $uncheckedWithTests | Sort-Object id)) {
        $reportEvidence = if ($record.evidence_type -eq "test") { $record.test_name } elseif ($record.evidence_type -eq "manual") { $record.manual_id } else { $record.evidence_id }
        $report.Add("- ``$($record.id)``: roadmap=$($record.checkbox_state), progress=$($record.progress_state), evidence=``$reportEvidence``")
    }
}
$report.Add("")
$report.Add("## M6受入")
$report.Add("")
$report.Add("- ``$($records | Where-Object { $_.id -eq 'M6.ACCEPTANCE.C01' } | Select-Object -ExpandProperty manual_id)``: 手順は ``docs/traceability/manual-acceptance.md`` に記載。")

if ($Update -or $WriteTraceability) {
    New-Item -ItemType Directory -Force -Path $traceabilityDir | Out-Null
    if ($Update) {
        [System.IO.File]::WriteAllLines($roadmapPath, $updatedLines, (New-Object System.Text.UTF8Encoding($false)))
    }
    [System.IO.File]::WriteAllText($jsonPath, $jsonText + [Environment]::NewLine, (New-Object System.Text.UTF8Encoding($false)))
    [System.IO.File]::WriteAllLines($markdownPath, $markdown, (New-Object System.Text.UTF8Encoding($false)))
    [System.IO.File]::WriteAllLines($manualPath, $manualMarkdown, (New-Object System.Text.UTF8Encoding($false)))
    [System.IO.File]::WriteAllLines($reportPath, $report, (New-Object System.Text.UTF8Encoding($false)))
}

Write-Output "checkbox=$($checkboxRecords.Count)/182 test=$($testRecords.Count) manual=$($manualRecords.Count) unlinked=$($unresolvedRecords.Count) duplicate=$($duplicateIds.Count) progress_disagreement=$($statusDisagreements.Count) regressed_to_unstarted=$($statusDisagreements.Count) unchecked_with_test=$($uncheckedWithTests.Count) hash=$generatedHash"
if (($statusDisagreements.Count + $uncheckedWithTests.Count) -ne 0) {
    Write-Output "STATUS-MISMATCH: roadmap status was preserved; see scratchpad/doc-link-7d-report.md"
    if ($Check) { exit 2 }
}
