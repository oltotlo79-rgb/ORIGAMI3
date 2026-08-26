param(
    [switch]$Update,
    [switch]$Check,
    # ロードマップ本文は触れず、台帳・手動受入・報告だけを再生成する。
    # 状態の食い違いを調査するときに使う。
    [switch]$WriteTraceability,
    # 統括・担当者の追記を残したまま、台帳と手動受入だけを再生成する。
    [switch]$PreserveReport
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
    "M4|Task 4-6" = @("traditional_frog_has_required_techniques_and_replays_connected_twice")
}

$scopeFallbackTests = @{
    M1 = "test_document_json_roundtrip"
    M2 = "full_replay_folds_flat_and_layers_are_a_permutation"
    M3 = "proposal_matrix_contract"
    M4 = "the_frog_is_deterministic"
    # M5.T5-1.C01は保存形式だけでなく、手順位置・鶴・水風船の再生までを同時に
    # 立証する。1つの検査名だけへ縮約せず、実在する4検査を明示対応する。
    M5 = @(
        "finish_soft_round_trips_three_values_only_with_measured_tolerance"
        "finish_soft_replay_uses_the_latest_completed_pose_at_each_position"
        "crane_replays_with_finish_soft_on_and_off_three_times_without_penetration"
        "balloon_replays_with_finish_soft_on_and_off_three_times_without_penetration"
    )
}

# 施策7の状態差B1で、実行記録がまだない手動受入を区分する。
# Xは将来CDPで画面操作・文字列・画素を機械確認できるもの、Yは検査の意味を
# 人が読んで判断するもの、Zは紙を折る実物比較が必要なものとして扱う。
$b1ManualAcceptanceClassification = [ordered]@{
    "MANUAL.M2.T2-6b.C05.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "つまんで動かす操作とツールレール" }
    "MANUAL.M2.T2-6b.C06.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "技法サブメニュー9種" }
    "MANUAL.M2.T2-6c.C01.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "層のずらし表示" }
    "MANUAL.M2.T2-6c.C02.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "つかんで動かす操作" }
    "MANUAL.M2.T2-6c.C03.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "実行前プレビュー" }
    "MANUAL.M2.T2-6c.C04.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "状態と操作理由の表示" }
    "MANUAL.M2.T2-6c.C05.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "技法の自動判定と記録" }
    "MANUAL.M2.T2-6c.C07.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "Y"; Subject = "DOM検査基盤と主要経路の検査" }
    "MANUAL.M2.T2-7.C01.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "4種類の作図補助" }
    "MANUAL.M2.T2-7.C02.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "局所平坦違反の橙表示" }
    "MANUAL.M2.T2-7.C03.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "めり込み警告バッジ" }
    "MANUAL.M2.T2-8.C02.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "復旧ダイアログ" }
    "MANUAL.M3.T3-4.C01.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "提案ウィザード3画面" }
    "MANUAL.M3.T3-4.C02.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "提案ウィザードの起動位置" }
    "MANUAL.M4.T4-3.C02.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "展開図書き出しダイアログ" }
    "MANUAL.M4.T4-5.C03.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "手順図書き出しダイアログ" }
}

# 専用CDP枠で実行済みのX受入。通常のnpm検査名一覧には含めず、実機・fixture・
# 復元を伴う手動受入IDの実施記録として再生成する。
$completedCdpAcceptance = [ordered]@{
    "MANUAL.M2.T2-6b.C06.SCREEN-ACCEPTANCE" = "技法9種の名称と順序が完全一致"
    "MANUAL.M2.T2-6c.C01.SCREEN-ACCEPTANCE" = "固定1280×860で主要3層が各14,000物理画素以上、層重心間80画素以上。固定drag/wheel後も同条件、視点差50画素以上"
    "MANUAL.M2.T2-6c.C02.SCREEN-ACCEPTANCE" = "通常dragの対象面13、Shift dragの対象面17、両方とも手順をちょうど1件追加"
    "MANUAL.M2.T2-6c.C03.SCREEN-ACCEPTANCE" = "通常dragのプレビュー多角形13・線分49、Shift dragの多角形17・線分61、release後grab inactive"
    "MANUAL.M2.T2-6c.C04.SCREEN-ACCEPTANCE" = "通常時と途中step時の操作ヒント各1件、標準修飾キー名以外の英字語0"
    "MANUAL.M2.T2-7.C01.SCREEN-ACCEPTANCE" = "作図4種各1、等分4、角度22.5°、補助線画素の増分が角度4,000・垂線45・等分55・二等分20以上"
    "MANUAL.M2.T2-7.C02.SCREEN-ACCEPTANCE" = "違反fixtureの橙(#ff8c00、RGB距離12以内)画素412、合格境界320以上"
    "MANUAL.M3.T3-4.C01.SCREEN-ACCEPTANCE" = "skeleton/candidates/confirm各1回、候補4件、違反数文4件、適用後dialog 0"
    "MANUAL.M3.T3-4.C02.SCREEN-ACCEPTANCE" = "提案前後でツールレール・展開図・3D・下部パネルが各1"
    "MANUAL.M4.T4-3.C02.SCREEN-ACCEPTANCE" = "書出しradio 4、PNG長辺1024、補助線checkboxを両状態へ切替"
}

$completedCdpAcceptanceScripts = @{
    "MANUAL.M2.T2-6c.C01.SCREEN-ACCEPTANCE" = "apps/desktop/tests-live/doc-link-b1-remaining-cdp.mjs"
    "MANUAL.M2.T2-6c.C02.SCREEN-ACCEPTANCE" = "apps/desktop/tests-live/doc-link-b1-grab-cdp.mjs"
    "MANUAL.M2.T2-6c.C03.SCREEN-ACCEPTANCE" = "apps/desktop/tests-live/doc-link-b1-grab-cdp.mjs"
    "MANUAL.M2.T2-7.C01.SCREEN-ACCEPTANCE" = "apps/desktop/tests-live/doc-link-b1-remaining-cdp.mjs"
    "MANUAL.M2.T2-7.C02.SCREEN-ACCEPTANCE" = "apps/desktop/tests-live/doc-link-b1-remaining-cdp.mjs"
}

# Task番号だけで「実装完了」と「コミット→プッシュ」を対応させない。
# 状態差として検出されたCOMMIT-PUSH 12件は、実コミットとorigin/main祖先をここで
# 明示対応させる。題名が要件を短く表現している場合も、実際のsubjectを台帳へ残す。
$commitPushEvidence = [ordered]@{
    "MANUAL.M2.T2-6c.C09.COMMIT-PUSH" = "85b8ca42b473f16312c4431b880bf569f48538f9"
    "MANUAL.M2.T2-7.C04.COMMIT-PUSH" = "dfd5ca03dce87fa2ae6cfff5cb05aba5b527d478"
    "MANUAL.M2.T2-9.C03.COMMIT-PUSH" = "f00628a8d365a01a71421cfeb32467e77bb75ebd"
    "MANUAL.M3.T3-1.C01.COMMIT-PUSH" = "6ce06fb3ac7cb21bf694a12af2db7a1871710f67"
    "MANUAL.M3.T3-2.C03.COMMIT-PUSH" = "8532fb2dc74fb8cf606569ca6a00ca212677c1c5"
    "MANUAL.M3.T3-3.C04.COMMIT-PUSH" = "e66e15152b347e7e0db1a77e7927fa7c83cc5d5d"
    "MANUAL.M4.T4-1.C03.COMMIT-PUSH" = "e2a4dff1ce092417e3ff722a082d010a738efabf"
    "MANUAL.M4.T4-2.C03.COMMIT-PUSH" = "98e94ad293beb1b81c4c66acead9ff8d47248171"
    "MANUAL.M4.T4-3.C03.COMMIT-PUSH" = "8ad7be3511f64dce20c8aaa8b4b1a897e4d5d656"
    "MANUAL.M4.T4-4.C03.COMMIT-PUSH" = "1b1a0e650cd373fc0a877d7fb133452f767739ba"
    "MANUAL.M4.T4-5.C04.COMMIT-PUSH" = "eb1c2c5904ebe67c15d2e2331c9533cddf91705c"
    "MANUAL.M4.T4-6.C03.COMMIT-PUSH" = "7c49536e8807074751cebd7852f801b5f24dd79b"
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

function Get-CommitPushProof([string]$ManualId) {
    if (-not $commitPushEvidence.Contains($ManualId)) { return $null }
    $commit = [string]$commitPushEvidence[$ManualId]
    $subject = @(& git -C $repoRoot show -s --format=%s $commit)
    if ($LASTEXITCODE -ne 0 -or $subject.Count -ne 1) {
        throw "COMMIT-PUSH対応のcommitを読めません: $ManualId -> $commit"
    }
    & git -C $repoRoot merge-base --is-ancestor $commit origin/main
    $onMain = $LASTEXITCODE -eq 0
    return [pscustomobject]@{
        hash = $commit
        subject = [string]$subject[0]
        origin_main_ancestor = $onMain
    }
}

function Test-ListedName([string]$Inventory, [string]$Name) {
    return [regex]::IsMatch($Inventory, "(?m)^" + [regex]::Escape($Name) + "(?:\: test)?\s*$")
}

function Get-ProgressCompletionTaskNumbers([string]$Text) {
    # 進捗の日時見出しだけを完了記録として扱う。本文中のTask番号や既知の問題を
    # 完了扱いにしない。また「Task 3-1/3-2」「Task 4-4 / 4-5」を両方拾う。
    $numbers = New-Object 'System.Collections.Generic.HashSet[string]'
    foreach ($line in ($Text -split "`r?`n")) {
        if ($line -notmatch '^## ') { continue }
        foreach ($match in [regex]::Matches($line, 'Task\s+(?<number>[0-9]+-[0-9A-Za-z]+)(?<following>(?:\s*/\s*[0-9]+-[0-9A-Za-z]+)*)')) {
            [void]$numbers.Add($match.Groups['number'].Value)
            foreach ($following in [regex]::Matches($match.Groups['following'].Value, '[0-9]+-[0-9A-Za-z]+')) {
                [void]$numbers.Add($following.Value)
            }
        }
        if ($line -match 'Task\s+M5(?:$|[^0-9A-Za-z])') {
            [void]$numbers.Add('M5')
        }
    }
    return $numbers
}

$roadmapLines = [System.IO.File]::ReadAllLines($roadmapPath, [System.Text.Encoding]::UTF8)
$progressText = [System.IO.File]::ReadAllText($progressPath, [System.Text.Encoding]::UTF8)
$testInventory = [System.IO.File]::ReadAllText($testNamesPath, [System.Text.Encoding]::UTF8)
$records = New-Object System.Collections.Generic.List[object]
$scope = $null
# $taskは現在の実際の見出し、$linkTaskは既存inline link IDを保つTask見出し。
# 作業見出しに入っても、Task番号を進捗照合へ持ち越してはならない。
$task = $null
$linkTask = $null
$taskOrdinals = @{}
$updatedLines = New-Object System.Collections.Generic.List[string]
$progressCompletedTaskNumbers = Get-ProgressCompletionTaskNumbers $progressText

for ($index = 0; $index -lt $roadmapLines.Count; $index++) {
    $line = $roadmapLines[$index]
    if ($line -match '^## M[0-9](?:\s|:)') {
        $scope = $null
        $task = $null
        $linkTask = $null
    }
    if ($line -match '^## (M[0-5])(?:\s|:)') {
        $scope = $Matches[1]
        $task = $null
        $linkTask = $null
    }
    if ($line -match '^### (Task [0-9]+-[0-9A-Za-z]+)') {
        $task = $Matches[1]
        $linkTask = $task
    }
    elseif ($line -match '^### 作業(?<number>[0-9]+(?:・[0-9]+)*)') {
        $task = "Work $($Matches['number'])"
    }
    elseif ($line -match '^### ') {
        # Taskでも作業でもない見出しの下にcheckboxがあれば、直前のTaskを使わず
        # 明示的に失敗させる。
        $task = $null
        $linkTask = $null
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
                commit_hash = $null
                commit_subject = $null
                commit_main_ancestor = $null
                progress_state = "historical-link-in-M0-evidence-table"
                progress_task_applicable = $false
                progress_task_id = $null
                progress_task_recorded = $null
            })
            $updatedLines.Add($line)
            continue
        }

        if ($scope -notin @("M1", "M2", "M3", "M4", "M5")) {
            $updatedLines.Add($line)
            continue
        }
        if ($null -eq $task -or $null -eq $linkTask) { throw "Task又は作業見出しを判定できません: line $($index + 1)" }

        # link IDと既存の証拠名は、ロードマップ本文に書かれた既存markerと一致させる。
        # 一方、進捗照合は下の$task（作業18ならWork 18）だけで行う。
        $taskKey = "$scope|$linkTask"
        if (-not $taskOrdinals.ContainsKey($taskKey)) { $taskOrdinals[$taskKey] = 0 }
        $taskOrdinals[$taskKey] = [int]$taskOrdinals[$taskKey] + 1
        $taskNumber = ([regex]::Match($linkTask, 'Task (?<number>[0-9]+-[0-9A-Za-z]+)')).Groups['number'].Value
        $linkId = "$scope.T$taskNumber.C{0:D2}" -f $taskOrdinals[$taskKey]
        $manualKind = Get-ManualKind $checkboxText
        $testName = $null
        $testNames = @()
        $manualId = $null
        $commitProof = $null
        $commitHash = $null
        $commitSubject = $null
        $commitMainAncestor = $null
        if ($null -ne $manualKind) {
            $manualId = "MANUAL.$linkId.$($manualKind.ToUpperInvariant())"
            $evidenceId = $manualId
            $evidenceType = "manual"
            if ($manualKind -eq "commit-push") {
                $commitProof = Get-CommitPushProof $manualId
                if ($null -ne $commitProof) {
                    $commitHash = $commitProof.hash
                    $commitSubject = $commitProof.subject
                    $commitMainAncestor = $commitProof.origin_main_ancestor
                }
            }
        }
        else {
            if ($taskTests.ContainsKey($taskKey)) { $testNames = @($taskTests[$taskKey]) }
            else { $testNames = @($scopeFallbackTests[$scope]) }
            $testNames = @($testNames | ForEach-Object { [string]$_ } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
            if ($testNames.Count -eq 0) { throw "検査名を割り当てられません: $linkId" }
            # 互換性のため先頭名は従来のtest_nameにも残し、全件はtest_namesに保存する。
            $testName = $testNames[0]
            $evidenceId = "TEST.$linkId"
            $evidenceType = "test"
        }

        # [ ]なのに同じTaskの完了見出しがあるものだけを要確認にする。
        # 作業見出しにはTask番号を持ち越さず、対応する進捗Taskが存在しないものとして扱う。
        $progressTaskApplicable = $task -match '^Task '
        $progressTaskId = $null
        $progressTaskMatch = $false
        if ($progressTaskApplicable) {
            $progressTaskId = ([regex]::Match($task, 'Task (?<number>[0-9]+-[0-9A-Za-z]+)')).Groups['number'].Value
            $progressTaskMatch = $progressCompletedTaskNumbers.Contains($progressTaskId)
        }
        if ($scope -eq "M5") {
            $progressTaskId = 'M5'
            $progressTaskApplicable = $true
            $progressTaskMatch = $progressCompletedTaskNumbers.Contains($progressTaskId)
        }
        $progressState = "consistent"
        $explicitManualConfirmation = $checkboxText -match "手動確認|実機確認"
        # 本文に手動確認を含むものは、進捗Taskとの状態差ではなく受入未実施として残す。
        # それ以外のCOMMIT-PUSHだけが、今回のC分類どおり明示commit対応を必要とする。
        $requiresExplicitCommitProof = $manualKind -eq "commit-push" -and $checkboxState -eq "unchecked" -and $progressTaskMatch -and -not $explicitManualConfirmation
        if ($requiresExplicitCommitProof) {
            # 履歴証拠はTask番号では進捗と結ばない。明示したcommitの本線祖先だけを使う。
            $progressTaskApplicable = $false
            if ($null -eq $commitProof) {
                $progressState = "commit-evidence-mapping-required"
            }
            elseif (-not $commitProof.origin_main_ancestor) {
                $progressState = "commit-evidence-not-on-origin-main"
            }
            else {
                $progressState = "commit-evidence-verified"
            }
        }
        elseif ($checkboxState -eq "unchecked" -and $progressTaskMatch -and -not $explicitManualConfirmation) {
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
            test_names = $testNames
            manual_id = $manualId
            commit_hash = $commitHash
            commit_subject = $commitSubject
            commit_main_ancestor = $commitMainAncestor
            progress_state = $progressState
            progress_task_applicable = $progressTaskApplicable
            progress_task_id = $progressTaskId
            progress_task_recorded = $progressTaskMatch
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
    test_names = @()
    manual_id = "MANUAL.M6.ACCEPTANCE.C01.FULL-ACCEPTANCE"
    commit_hash = $null
    commit_subject = $null
    commit_main_ancestor = $null
    progress_state = "manual-acceptance-required"
    progress_task_applicable = $false
    progress_task_id = $null
    progress_task_recorded = $null
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
$statusDisagreements = @($checkboxRecords | Where-Object {
    $_.progress_state -in @(
        "unchecked-but-progress-task-exists",
        "commit-evidence-mapping-required",
        "commit-evidence-not-on-origin-main"
    )
})
$verifiedCommitEvidence = @($checkboxRecords | Where-Object { $_.progress_state -eq "commit-evidence-verified" })
$unverifiedCommitEvidence = @($checkboxRecords | Where-Object {
    $_.progress_state -in @("commit-evidence-mapping-required", "commit-evidence-not-on-origin-main")
})
$uncheckedWithTests = @($checkboxRecords | Where-Object { $_.progress_state -eq "unchecked-with-test-link" })
# 逆方向: ロードマップは完了だが、進捗の日時見出しに同じTaskの完了記録がない。
# 作業見出しは進捗Taskと一対一に対応しないため、ここでは数えない。
$reverseDisagreements = @($checkboxRecords | Where-Object {
    $_.checkbox_state -eq "checked" -and $_.progress_task_applicable -and -not $_.progress_task_recorded
})

foreach ($record in $testRecords) {
    foreach ($testName in @($record.test_names)) {
        if (-not (Test-ListedName $testInventory $testName)) {
            throw "保存済み一覧に無い検査名です: $($record.id) -> $testName"
        }
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
    $evidence = if ($record.evidence_type -eq "test") { "自動 ``$($record.test_names -join ' / ')``" } elseif ($record.evidence_type -eq "manual") { "手動 ``$($record.manual_id)``" } else { "既存 ``$($record.evidence_id)``" }
    $markdown.Add(('| <a id="roadmap-evidence-{0}"></a>`{1}` | {2} | {3} | {4} |' -f $slug, $record.id, (ConvertTo-MarkdownCell $evidence), $record.checkbox_state, $record.progress_state))
}

$manualMarkdown = New-Object System.Collections.Generic.List[string]
$manualMarkdown.Add("# 手動受入手順")
$manualMarkdown.Add("")
$manualMarkdown.Add('この文書のIDは `roadmap-links.json` の手動証拠と1対1で対応する。実施者はID、日付、結果、確認した画面又は履歴を記録する。担当者はアプリを起動せず、画面確認は統括が同梱版で行う。')
$manualMarkdown.Add("")
$manualMarkdown.Add("## B1未実施受入の自動化可否（2026-08-26）")
$manualMarkdown.Add("")
$manualMarkdown.Add('XはCDPで画面操作、表示文字列、画素又は領域の有無を確認できる。Yは検査が主張する範囲を人が読んで判断する。Z（実際に紙を折る比較が必要な項目）は、この16件にはない。')
$manualMarkdown.Add("")
$manualMarkdown.Add("| ID | 区分 | 確認対象 |")
$manualMarkdown.Add("|---|---|---|")
foreach ($manualId in $b1ManualAcceptanceClassification.Keys) {
    $classification = $b1ManualAcceptanceClassification[$manualId]
    $manualMarkdown.Add("| ``$manualId`` | $($classification.Class) | $($classification.Subject) |")
}
$manualMarkdown.Add("")
$manualMarkdown.Add("### X: CDP自動化の共通手順")
$manualMarkdown.Add("")
$manualMarkdown.Add('1. 専用の検査環境で同梱版を1つだけ起動し、CDP接続後に該当IDの操作を再現する。')
$manualMarkdown.Add('2. 指定された文字列、要素領域、状態ごとのスクリーンショットを取得し、期待する画素領域又は文字列と比較する。')
$manualMarkdown.Add('3. ID、操作列、取得画像、比較結果を保存する。1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。')
$manualMarkdown.Add("")
$manualMarkdown.Add("### Y: 人が判断する手順")
$manualMarkdown.Add("")
$manualMarkdown.Add('#### `MANUAL.M2.T2-6c.C07.SCREEN-ACCEPTANCE`')
$manualMarkdown.Add('1. 担当: 画面検査の担当者とは別のレビュー担当者。')
$manualMarkdown.Add('2. `apps/desktop/src/lib/layerMotion.test.ts` とテスト設定を読み、jsdomとTesting Libraryの基盤、およびプレビュー・ヒント・ドラッグの主要経路を検査する実在testがあることを確認する。')
$manualMarkdown.Add('3. 担当者が指定する検査名一覧の取得又は対象test実行の結果を確認し、ID、確認日、確認したtest名、結果を記録する。画面上の見た目だけ、又はtest名だけでは合格にしない。')
foreach ($record in ($manualRecords | Sort-Object manual_id)) {
    $manualMarkdown.Add("")
    $manualMarkdown.Add("## $($record.manual_id)")
    if ($record.manual_id -match "COMMIT-PUSH$") {
        $manualMarkdown.Add("1. ``docs/implementation-roadmap.md`` の ``$($record.id)`` と同じTaskを確認する。")
        if ($null -ne $record.commit_hash) {
            $manualMarkdown.Add("2. 明示対応commit: ``$($record.commit_hash)``（題名: $($record.commit_subject)）。")
            $manualMarkdown.Add("3. ``git merge-base --is-ancestor $($record.commit_hash) origin/main`` の確認結果: ``$($record.commit_main_ancestor)``。")
            $manualMarkdown.Add("4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。")
        }
        else {
            $manualMarkdown.Add("2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。")
            $manualMarkdown.Add("3. 題名・確認日・結果を記録し、確認不能なら合格にしない。")
        }
    }
    elseif ($record.manual_id -match "SCREEN-ACCEPTANCE$") {
        if ($completedCdpAcceptance.Contains($record.manual_id)) {
            $cdpScript = if ($completedCdpAcceptanceScripts.ContainsKey($record.manual_id)) { $completedCdpAcceptanceScripts[$record.manual_id] } else { "apps/desktop/tests-live/doc-link-b1-cdp.mjs" }
            $manualMarkdown.Add("1. 2026-08-26に専用CDP枠で実行済み。実行本体: ``$cdpScript``。")
            $manualMarkdown.Add("2. 実測結果: $($completedCdpAcceptance[$record.manual_id])。")
            $manualMarkdown.Add("3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に指定作品、道具、dialog、capture属性、viewportを復元した。")
            $manualMarkdown.Add("4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。")
        }
        else {
            $manualMarkdown.Add("1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。")
            $manualMarkdown.Add("2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。")
            $manualMarkdown.Add("3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。")
        }
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
$report.Add("| 逆方向: roadmap完了・進捗Task記録なし | $($reverseDisagreements.Count) |")
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
if ($statusDisagreements.Count -eq 0) {
    $report.Add("- なし")
}
else {
    foreach ($record in ($statusDisagreements | Sort-Object id)) {
        $reportEvidence = if ($record.evidence_type -eq "test") { $record.test_names -join ' / ' } elseif ($record.evidence_type -eq "manual") { $record.manual_id } else { $record.evidence_id }
        $report.Add("- ``$($record.id)``: roadmap=$($record.checkbox_state), progress=$($record.progress_state), evidence=``$reportEvidence``")
    }
}
$report.Add("")
$report.Add("## 未完了だが実在testへ結んだ項目（進捗状態差ではない）")
$report.Add("")
if ($uncheckedWithTests.Count -eq 0) {
    $report.Add("- なし")
}
else {
    foreach ($record in ($uncheckedWithTests | Sort-Object id)) {
        $report.Add("- ``$($record.id)``: roadmap=unchecked, progress=record-not-applicable, evidence=``$($record.test_name)``")
    }
}
$report.Add("")
$report.Add("## M6受入")
$report.Add("")
$report.Add("- ``$($records | Where-Object { $_.id -eq 'M6.ACCEPTANCE.C01' } | Select-Object -ExpandProperty manual_id)``: 手順は ``docs/traceability/manual-acceptance.md`` に記載。")
$report.Add("")
$report.Add("## 逆方向の照合（roadmap完了・進捗Task記録なし）")
$report.Add("")
if ($reverseDisagreements.Count -eq 0) {
    $report.Add("- なし")
}
else {
    foreach ($record in ($reverseDisagreements | Sort-Object id)) {
        $report.Add("- ``$($record.id)``: roadmap=checked, progressTask=$($record.progress_task_id), progress=record-not-found")
    }
}

if ($Update -or $WriteTraceability) {
    New-Item -ItemType Directory -Force -Path $traceabilityDir | Out-Null
    if ($Update) {
        [System.IO.File]::WriteAllLines($roadmapPath, $updatedLines, (New-Object System.Text.UTF8Encoding($false)))
    }
    [System.IO.File]::WriteAllText($jsonPath, $jsonText + [Environment]::NewLine, (New-Object System.Text.UTF8Encoding($false)))
    [System.IO.File]::WriteAllLines($markdownPath, $markdown, (New-Object System.Text.UTF8Encoding($false)))
    [System.IO.File]::WriteAllLines($manualPath, $manualMarkdown, (New-Object System.Text.UTF8Encoding($false)))
    if (-not $PreserveReport) {
        [System.IO.File]::WriteAllLines($reportPath, $report, (New-Object System.Text.UTF8Encoding($false)))
    }
}

Write-Output "checkbox=$($checkboxRecords.Count)/182 test=$($testRecords.Count) manual=$($manualRecords.Count) unlinked=$($unresolvedRecords.Count) duplicate=$($duplicateIds.Count) progress_disagreement=$($statusDisagreements.Count) regressed_to_unstarted=$($statusDisagreements.Count) commit_evidence_verified=$($verifiedCommitEvidence.Count) commit_evidence_unverified=$($unverifiedCommitEvidence.Count) unchecked_with_test=$($uncheckedWithTests.Count) reverse_progress_disagreement=$($reverseDisagreements.Count) hash=$generatedHash"
if (($statusDisagreements.Count + $reverseDisagreements.Count) -ne 0) {
    Write-Output "STATUS-MISMATCH: roadmap status was preserved; see scratchpad/doc-link-7d-report.md"
    if ($Check) { exit 2 }
}
