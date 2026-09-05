param(
    [switch]$Update,
    [switch]$Check,
    # ロードマップ本文は触れず、台帳・手動受入・報告だけを再生成する。
    # 状態の食い違いを調査するときに使う。
    [switch]$WriteTraceability,
    # 統括・担当者の追記を残したまま、台帳と手動受入だけを再生成する。
    [switch]$PreserveReport,
    # 現在入力からの再生成bytesと保存済み3成果物を比較する。状態差とは独立に使う。
    [switch]$CheckTraceability,
    # 検査名台帳の見出し `# definition-tree-sha256=` だけを、いま計算した値で
    # 書き直す唯一の入口。値は -CheckTraceability が照合するのと同じ計算経路
    # (同じ関数・同じ正規化)から出すので、人が hash を手で書かない。
    # 通常の検査・生成経路からは呼ばない(§10.7.6: 通常検査が追跡対象を
    # 書き換えない)。他のmodeとは同時指定できない。
    [switch]$WriteTestNamesHash,
    # 自己試験用。通常は docs/traceability を使う。
    [string]$TraceabilityPath = "",
    # 自己試験用。通常は正本と追跡対象の検査名台帳を使う。
    [string]$RoadmapInputPath = "",
    [string]$TestNamesInputPath = "",
    [string]$TestSourceRoot = "",
    [string]$ExecutionContractRoot = ""
)

# 施策7-D2〜D10: 実装ロードマップのcheckboxと、実在する検査又は手動受入を
# 同じ入力から再生成・監査する。PowerShell 5.1でも動くように書く。
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$roadmapPath = if ([string]::IsNullOrWhiteSpace($RoadmapInputPath)) {
    Join-Path $repoRoot "docs/implementation-roadmap.md"
}
else {
    [IO.Path]::GetFullPath($RoadmapInputPath)
}
$progressPath = Join-Path $repoRoot "docs/progress.md"
$testNamesPath = if ([string]::IsNullOrWhiteSpace($TestNamesInputPath)) {
    Join-Path $repoRoot "docs/traceability/roadmap-evidence-test-names.txt"
}
else {
    [IO.Path]::GetFullPath($TestNamesInputPath)
}
$testSourceRoot = if ([string]::IsNullOrWhiteSpace($TestSourceRoot)) {
    $repoRoot
}
else {
    [IO.Path]::GetFullPath($TestSourceRoot)
}
$executionContractRoot = if ([string]::IsNullOrWhiteSpace($ExecutionContractRoot)) {
    $repoRoot
}
else {
    [IO.Path]::GetFullPath($ExecutionContractRoot)
}
$traceabilityDir = if ([string]::IsNullOrWhiteSpace($TraceabilityPath)) {
    Join-Path $repoRoot "docs/traceability"
}
else {
    [IO.Path]::GetFullPath($TraceabilityPath)
}
$jsonPath = Join-Path $traceabilityDir "roadmap-links.json"
$markdownPath = Join-Path $traceabilityDir "roadmap-links.md"
$manualPath = Join-Path $traceabilityDir "manual-acceptance.md"
$reportPath = Join-Path $repoRoot "scratchpad/doc-link-7d-report.md"

if ($Check -and $CheckTraceability) { throw "-Check と -CheckTraceability は同時指定できません" }
if (($Update -or $WriteTraceability) -and ($Check -or $CheckTraceability)) { throw "書込と検査は同時指定できません" }
if ($WriteTestNamesHash -and ($Update -or $WriteTraceability -or $Check -or $CheckTraceability)) {
    throw "-WriteTestNamesHash は単独で実行してください"
}

function Get-CurrentRoadmapSnapshot {
    $snapshotScript = Join-Path $PSScriptRoot "get-roadmap-status.ps1"
    if (-not (Test-Path -LiteralPath $snapshotScript -PathType Leaf)) {
        throw "ロードマップsnapshot生成器がありません: $snapshotScript"
    }
    $powershellExe = (Get-Process -Id $PID).Path
    $global:LASTEXITCODE = 0
    $output = @(& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $snapshotScript -RoadmapPath $roadmapPath -Format Json)
    $snapshotExit = $LASTEXITCODE
    if ($snapshotExit -ne 0) { throw "ロードマップsnapshot生成に失敗しました (終了コード: $snapshotExit)" }
    if ($output.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$output[0])) {
        throw "ロードマップsnapshotがJSON 1行を返しませんでした (行数: $($output.Count))"
    }
    $snapshot = [string]$output[0] | ConvertFrom-Json
    if ([int]$snapshot.schema -ne 1 -or [string]$snapshot.scope -ne "whole" -or [bool]$snapshot.partial -or
        [int]$snapshot.audited -ne [int]$snapshot.total -or [int]$snapshot.unclassified -ne 0 -or
        [int]$snapshot.checked + [int]$snapshot.unchecked -ne [int]$snapshot.total) {
        throw "ロードマップsnapshotの全件会計が不正です"
    }
    return $snapshot
}

function Get-BytesSha256([byte[]]$Bytes) {
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha256.ComputeHash($Bytes))).Replace("-", "").ToLowerInvariant() }
    finally { $sha256.Dispose() }
}

function Test-ArtifactBytes {
    param([string]$Label, [string]$Path, [byte[]]$ExpectedBytes)

    $expectedHash = Get-BytesSha256 $ExpectedBytes
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        Write-Host "[STALE] $Label がありません: $Path expected_sha256=$expectedHash"
        return $false
    }
    $actualBytes = [IO.File]::ReadAllBytes($Path)
    $actualHash = Get-BytesSha256 $actualBytes
    $firstDifference = -1
    $limit = [Math]::Min($ExpectedBytes.Length, $actualBytes.Length)
    for ($index = 0; $index -lt $limit; $index++) {
        if ($ExpectedBytes[$index] -ne $actualBytes[$index]) { $firstDifference = $index; break }
    }
    if ($firstDifference -lt 0 -and $ExpectedBytes.Length -ne $actualBytes.Length) { $firstDifference = $limit }
    if ($firstDifference -ge 0) {
        Write-Host "[STALE] $Label bytes不一致: offset=$firstDifference expected_length=$($ExpectedBytes.Length) actual_length=$($actualBytes.Length) expected_sha256=$expectedHash actual_sha256=$actualHash"
        return $false
    }
    Write-Host "[FRESH] $Label bytes一致: length=$($actualBytes.Length) sha256=$actualHash"
    return $true
}

$roadmapSnapshot = Get-CurrentRoadmapSnapshot

foreach ($requiredPath in @($roadmapPath, $progressPath, $testNamesPath)) {
    if (-not (Test-Path -LiteralPath $requiredPath)) {
        throw "必須入力がありません: $requiredPath"
    }
}

$expectedCheckboxCounts = [ordered]@{
    M0 = 11
    M1 = 38
    M2 = 70
    M3 = 45
    M4 = 19
    M5 = 1
    ADDITIONAL = 2
}

# ここに書く名前は追跡対象の検査名台帳に実在するものだけである。
# 台帳はリポジトリ全検査ではなく、この割当集合だけを対象とすることを明記する。
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

# Task単位の既定割当だけでは細目の受入条件を十分に表せない項目は、link IDごとに
# 網羅する検査を明示する。ここへ書く名前も検査名台帳に実在しなければならない。
# link ID割当は手動受入の語句分類より優先し、UI文言を含んでも自動検査で立証済みの
# 項目を手動証拠へ退行させない。
$linkTests = @{
    "ADDITIONAL.FOLD-ALL.C01" = @(
        "fold_all::tests::targets_use_mountain_positive_valley_negative_and_skip_non_hinges"
        "fold_all::tests::invalid_inputs_are_errors_but_a_calculated_pose_has_no_layer_order"
        "src/components/FoldAllPreview.dom.test.tsx > 全部いっぺんに折ってみる画面 > 既存パネルの入口から0〜100%のつまみと記録でない約束を常時表示する"
        "src/components/FoldAllPreview.dom.test.tsx > 全部いっぺんに折ってみる画面 > 0・25・50・75・100%と計算待ちの間ずっと仮の形で手順ではないと示す"
        "src/store/foldAllPreview.test.ts > 全部の折り目をいっぺんに動かす一時表示 > 保存しても一斉形や手順を記録せず、専用表示を続ける"
        "src/store/foldAllPreview.test.ts > 全部の折り目をいっぺんに動かす一時表示 > Undoは作品履歴を進めず、通常表示へ戻るだけ"
        "src/store/foldAllPreview.test.ts > 全部の折り目をいっぺんに動かす一時表示 > Redoは作品履歴を進めず、通常表示へ戻るだけ"
        "commands::tests::pose_commands_match_the_cross_runtime_diagonal_fixtures"
        "src/store/foldAllPreview.savedFile.test.ts > 一斉表示中の73%を実ファイルへ保存せず、新しいbackendで開き直しても手順・履歴に現れない"
    )
    "M2.T2-8.C01" = @(
        "autosave::tests::autosave_worker_waits_thirty_seconds_and_still_skips_clean_documents"
        "autosave::tests::autosave_skips_clean_document_and_writes_untitled_to_app_data"
        "autosave::tests::clean_exit_discards_but_dirty_exit_keeps_the_autosave"
    )
    "M2.T2-8.C02" = @(
        "src/App.dom.test.tsx > startup recovery check > checks recovery exactly once after preparing the new document"
        "autosave::tests::recovery_wire_uses_null_for_no_choices_and_requires_both_internal_fields"
        "autosave::tests::recovery_choices_are_newest_first_and_keep_the_fourth_as_overflow"
        "src/components/RecoveryDialog.dom.test.tsx > 復旧ダイアログ > 残っていれば理由と選択肢を日本語で出す"
    )
    "M2.T2-8.C04" = @(
        "autosave::tests::clean_exit_never_discards_another_process_active_snapshot"
        "autosave::tests::forced_process_kill_then_new_process_restores_exact_document"
        "autosave::tests::restore_recovers_the_same_document"
        "autosave::tests::two_real_processes_keep_both_documents_when_autosave_transactions_overlap"
    )
    "M2.T2-8.C05" = @(
        "store::tests::saving_own_opened_document_twice_after_edits_does_not_conflict"
        "store::tests::two_real_processes_reject_a_stale_explicit_save_and_keep_the_first_save"
    )
    "M3.T3-4.C28" = @("compare_two_ways_on_allowed_corpus_anchors")
    "M4.T4-5.C03" = @("diagram_pdf_and_svg_create_nonempty_structured_files")
}

# 完了印を付けた自動証拠は、Task既定割当・本文語句分類に委ねず、link IDごとに
# ここで検査名を明示する。未着手([ ])は対象外であり、新規作業を不要に止めない。
# 同一検査が複数細目を立証する場合だけ、IDを同じ割当にまとめる。
$completedAutomaticLinkTests = @(
    [pscustomobject]@{ LinkIds = @("M1.T1-1.C01", "M1.T1-1.C02"); TestNames = @("test_document_json_roundtrip") }
    [pscustomobject]@{ LinkIds = @("M1.T1-10.C01"); TestNames = @("yakko_double_blintz_folds_flat_to_half_square") }
    [pscustomobject]@{ LinkIds = @("M1.T1-2.C01", "M1.T1-2.C02"); TestNames = @("test_seg_intersection_crossing") }
    [pscustomobject]@{ LinkIds = @("M1.T1-3.C01", "M1.T1-3.C02"); TestNames = @("extraction_is_deterministic") }
    [pscustomobject]@{ LinkIds = @("M1.T1-4.C01", "M1.T1-4.C02", "M1.T1-4.C03"); TestNames = @("store::tests::add_segment_undo_redo_roundtrip") }
    [pscustomobject]@{ LinkIds = @("M1.T1-5.C01", "M1.T1-5.C02", "M1.T1-5.C03"); TestNames = @("src/store/ipcQueue.test.ts > createSerialQueue > 前の要求が完了するまで次の要求を開始しない(発行順に直列実行)") }
    [pscustomobject]@{ LinkIds = @("M1.T1-6.C01"); TestNames = @("src/lib/alignPick.test.ts > 線分の交点 > 十字に交わる2本の交点を返す") }
    [pscustomobject]@{ LinkIds = @("M1.T1-7.C01", "M1.T1-7.C02"); TestNames = @("loop_free_cp_converges_immediately") }
    [pscustomobject]@{ LinkIds = @("M1.T1-8.C01", "M1.T1-8.C02", "M1.T1-8.C03"); TestNames = @("warm_start_converges_faster_to_same_solution") }
    [pscustomobject]@{ LinkIds = @("M2.T2-0.C01", "M2.T2-0.C02", "M2.T2-0.C03", "M2.T2-0.C04"); TestNames = @("yakko_hinge_20_sweep_stays_within_frame_budget") }
    [pscustomobject]@{ LinkIds = @("M2.T2-1.C01", "M2.T2-1.C02"); TestNames = @("half_folded_square_is_mirror_pair_with_right_face_on_top") }
    [pscustomobject]@{ LinkIds = @("M2.T2-2.C01", "M2.T2-2.C02"); TestNames = @("folding_only_top_layer_keeps_lower_layers_in_place") }
    [pscustomobject]@{ LinkIds = @("M2.T2-3.C01", "M2.T2-3.C02", "M2.T2-3.C04"); TestNames = @("replay_twice_is_bit_identical") }
    [pscustomobject]@{ LinkIds = @("M2.T2-5.C02"); TestNames = @("store::tests::cushion_then_cupboard_fold_only_with_fold_through") }
    [pscustomobject]@{ LinkIds = @("M2.T2-6.C01", "M2.T2-6.C02", "M2.T2-6.C03", "M2.T2-6.C05"); TestNames = @("inside_reverse_on_four_layer_flap") }
    [pscustomobject]@{ LinkIds = @("M2.T2-6b.C01", "M2.T2-6b.C02", "M2.T2-6b.C03", "M2.T2-6b.C04"); TestNames = @("sim011_completeness_table_and_generic_routes_are_permanent") }
    [pscustomobject]@{ LinkIds = @("M2.T2-6c.C06"); TestNames = @("src/lib/layerMotion.test.ts > 汎用層操作の入力 > 既存折り目のReflectをregionなし・Keepへ変換する") }
    [pscustomobject]@{ LinkIds = @("M3.T3-2.C01", "M3.T3-2.C02"); TestNames = @("packing_quality_baseline_1005_runs") }
    [pscustomobject]@{ LinkIds = @("M3.T3-3.C01", "M3.T3-3.C02", "M3.T3-3.C03"); TestNames = @("depth_three_branching_skeleton_packs_and_generates_valid_cp") }
    [pscustomobject]@{ LinkIds = @("M3.T3-4.C05", "M3.T3-4.C06", "M3.T3-4.C07", "M3.T3-4.C08", "M3.T3-4.C09", "M3.T3-4.C10", "M3.T3-4.C11", "M3.T3-4.C12", "M3.T3-4.C13", "M3.T3-4.C14", "M3.T3-4.C15", "M3.T3-4.C16", "M3.T3-4.C17", "M3.T3-4.C18", "M3.T3-4.C19", "M3.T3-4.C20", "M3.T3-4.C21", "M3.T3-4.C24", "M3.T3-4.C25", "M3.T3-4.C26", "M3.T3-4.C27", "M3.T3-4.C29", "M3.T3-4.C30", "M3.T3-4.C31", "M3.T3-4.C32", "M3.T3-4.C33", "M3.T3-4.C34", "M3.T3-4.C35", "M3.T3-4.C36"); TestNames = @("proposal_matrix_contract") }
    [pscustomobject]@{ LinkIds = @("M4.T4-1.C01", "M4.T4-1.C02"); TestNames = @("open_sink_turns_the_tip_of_the_preliminary_base_inside_out") }
    [pscustomobject]@{ LinkIds = @("M4.T4-2.C01", "M4.T4-2.C02"); TestNames = @("twist_works_on_a_triangle_and_rejects_only_undefined_input") }
    [pscustomobject]@{ LinkIds = @("M4.T4-3.C01"); TestNames = @("cp_svg::tests::each_edge_kind_has_its_own_style") }
    [pscustomobject]@{ LinkIds = @("M4.T4-4.C01", "M4.T4-4.C02"); TestNames = @("manual::tests::representative_json_makes_four_page_pdf_and_two_toc_items") }
    [pscustomobject]@{ LinkIds = @("M4.T4-5.C01", "M4.T4-5.C02"); TestNames = @("pdf::tests::seven_steps_make_a_cover_and_two_pages") }
    [pscustomobject]@{ LinkIds = @("M4.T4-6.C01"); TestNames = @("traditional_frog_has_required_techniques_and_replays_connected_twice") }
)
foreach ($assignment in $completedAutomaticLinkTests) {
    foreach ($linkId in @($assignment.LinkIds)) {
        if ($linkTests.ContainsKey($linkId)) { throw "完了済み自動証拠のlink ID割当が重複しています: $linkId" }
        $linkTests[$linkId] = @($assignment.TestNames)
    }
}

# 承認済みのC04/C05は既存C02/C03の前へ追記されたが、IDは正本上の既存IDを保つ。
# 順序を自由化せず、このTaskだけの5件の順序を固定してinline markerとの一致照合を維持する。
$taskLinkIdOrder = @{
    "M2|Task 2-8" = @(
        "M2.T2-8.C01"
        "M2.T2-8.C04"
        "M2.T2-8.C05"
        "M2.T2-8.C02"
        "M2.T2-8.C03"
    )
    # 2026-09-05: 一斉折りの節へNFR-002の実機確認(C02)を新設した。ADDITIONALは
    # 以前ID採番を1つに固定しており、2件目のcheckboxを足すと必ず
    # 「inline linkが台帳と一致しません」で止まった。他のTaskと同じ固定順序表へ
    # 移し、正本の並び順どおりにC01・C02を割り当てる。既存C01のIDは動かない。
    "ADDITIONAL|Fold All" = @(
        "ADDITIONAL.FOLD-ALL.C01"
        "ADDITIONAL.FOLD-ALL.C02"
    )
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
    "MANUAL.M3.T3-4.C01.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "提案ウィザード3画面" }
    "MANUAL.M3.T3-4.C02.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "提案ウィザードの起動位置" }
    "MANUAL.M4.T4-3.C02.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "展開図書き出しダイアログ" }
    "MANUAL.M4.T4-5.C03.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "手順図書き出しダイアログ" }
    "MANUAL.ADDITIONAL.FOLD-ALL.C02.SCREEN-ACCEPTANCE" = [pscustomobject]@{ Class = "X"; Subject = "一斉折りの仮表示の速さ(NFR-002)" }
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

# 2026-09-05: 受入担当2名が専用CDP枠で実測し手で書いた7件。生成器の表へ
# 取り込み、再生成しても同じ本文が出るようにする(§10.7.6)。文言は
# docs/traceability/manual-acceptance.md から1字も変えていない。
$manuallyRecordedAcceptance = [ordered]@{
    "MANUAL.M1.T1-10.C02.SCREEN-ACCEPTANCE" = @(
        '1. 2026-09-05に専用CDP枠で実行済み。実行本体: `scratchpad/acceptance-2026-09-05/driver-481-redo.mjs`（`apps/desktop/tests-live/doc-link-b1-cdp-support.mjs`の`connectDesktop`/`evaluate`/`restoreBlank`を使う。exit=0）。'
        '2. 実測結果: 白紙へ**描いて**やっこさんを折った。正本`crates/ori3-rigid/tests/fixtures/check-yakko.ori3`と同じ折り目を谷8ストローク・山8ストロークで引き、頂点4→**20**・辺4→**36**（正本と一致）、平らにたためない点**0**・平坦条件の違反**0**・警告**0**。「全部いっぺんに折ってみる」100%で面17・外形0.5×0.5・**厚み0**（平らに畳めた）。表示はすべて日本語で、常設4区画（ツールレール1・展開図1・3D1・コンテキストパネル1）は不変。'
        '3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に手順0・道具「選択」・開いているdialog 0の白紙へ復元した。記録と実測値は`scratchpad/acceptance-2026-09-05/MANUAL.M1.T1-10.C02-redo.stdout.log`、画像は同フォルダの`MANUAL.M1.T1-10.C02-redo-1-blank.png`〜`-4-folded.png`にある。'
        '4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。'
    )
    "MANUAL.M2.T2-6b.C05.SCREEN-ACCEPTANCE" = @(
        '1. 2026-09-05に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-pull-cdp.mjs`（exit=0、`M2.T2-6b.C05 VERIFY PASSED`）。'
        '2. 実測結果: つかんで動かす操作で手順がちょうど1件増え（1→2）、折り目の辺が36→51へ増え、道具は「折る」のまま、離した後の掴みは解除（`grab.active=false`）、増えた手順「2 単純折り」がタイムラインにある。fixtureは`crates/ori3-rigid/tests/fixtures/check-yakko.ori3`、正規化座標(0.50,0.50)→(0.65,0.50)。'
        '3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に手順0・道具「選択」・開いているdialog 0の白紙へ復元した。記録と実測値は`scratchpad/acceptance-2026-09-05/MANUAL.M2.T2-6b.C05.SCREEN-ACCEPTANCE.stdout.log`にある。'
        '4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。'
    )
    "MANUAL.M2.T2-6c.C08.SCREEN-ACCEPTANCE" = @(
        '1. 2026-09-05に専用CDP枠で実行済み。このcheckboxの成果物は「詰まった箇所を`docs/progress.md`に記録」であり、**所見は`docs/progress.md`の「2026-09-05 - 説明なしで座布団折りから鶴の基本形まで折れるかを実機で確かめ、詰まった箇所を記録した」の節**にある。'
        '2. 実測結果の要約: 座布団折りは説明なしで折れた（頂点8・辺12、平らにたためない点0・警告0、100%で面5・厚み0）。詰まりは2件で、①次に折る技法を画面が案内しない（技法一覧は9種の名前とヒント「左の一覧から技法を選んでください」だけ）②平らに畳めないときに、山谷を変えるべき折り目を画面が名指ししない（案内は「平らにたためない場所があります」だけ）。鶴の基本形には到達しなかった。操作不能・英語表示・表示崩れは0件。'
        '3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に手順0・道具「選択」・開いているdialog 0の白紙へ復元した。記録と実測値は`scratchpad/acceptance-2026-09-05/MANUAL.M2.T2-6c.C08.stdout.log`、画像は同フォルダの`MANUAL.M2.T2-6c.C08-A1-zabuton-cp.png`〜`-C1-technique-menu.png`にある。'
        '4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。'
    )
    "MANUAL.M2.T2-7.C03.SCREEN-ACCEPTANCE" = @(
        '1. 2026-09-05に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-penetration-cdp.mjs`（exit=0、`M2.T2-7.C03 VERIFY PASSED`）。'
        '2. 実測結果: 面が交差するfixture（`crates/ori3-layers/tests/fixtures/penetration-warning.ori3`）で、警告バッジがちょうど1個、疑わしい折り目の案内がちょうど1個、capture APIの警告数1、バッジのclassは`status-badge`だけで`error`を含まず、バッジの文言は日本語の「警告 1」。'
        '3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に手順0・道具「選択」・開いているdialog 0の白紙へ復元した。記録と実測値は`scratchpad/acceptance-2026-09-05/MANUAL.M2.T2-7.C03.SCREEN-ACCEPTANCE.stdout.log`にある。'
        '4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。'
    )
    "MANUAL.M2.T2-9.C02.SCREEN-ACCEPTANCE" = @(
        '1. 2026-09-05に専用CDP枠で実行済み。実行本体: `scratchpad/acceptance-2026-09-05/driver-752-redo.mjs`（exit=0）。作品は`apps/desktop/tests-live/fixtures/traditional-crane-full.ori3`（正本CP 頂点56・辺114、手順3。`crates/ori3-layers/tests/acceptance_crane.rs`の`crane()`と同じ辺ID群で正本の一括collapse 1手を3手へ分けたもの）。'
        '2. 実測結果: 手順0〜3を1つずつ進めて**鶴が完成した**（札は「折る前」「1 単純折り」「2 花弁折り」「3 中割り折り」で全て日本語。面59、3D画像に翼・首・頭・尾が見える）。展開図の内側の頂点id=10を(0.5,0.6659)→(0.56,0.7059)へドラッグして修正すると、「再生」の後に完成形が変わり（面座標のチェックサム96091.5→91406.8、外接箱も変化）、画面上部に日本語で「指定を優先し、いちばん近い形で追従中」と出て**追従した**。操作不能・英語表示・表示崩れは0件で、常設4区画は不変。'
        '3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に手順0・道具「選択」・開いているdialog 0の白紙へ復元した。記録と実測値は`scratchpad/acceptance-2026-09-05/MANUAL.M2.T2-9.C02-redo.stdout.log`、画像は同フォルダの`MANUAL.M2.T2-9.C02-redo-1-opened.png`〜`-4-after-replay.png`にある。手順2（鳥の基本形）では日本語で「この折り方だと紙が突き抜けています」「指定した角度に近い形を表示しています（閉包RMS 1.655e-12）」と出て操作は止まらない（`-redo-chip-step-2.png`）。'
        '4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。'
    )
    "MANUAL.M3.T3-4.C04.COMMIT-PUSH" = @(
        '1. 2026-09-05に実行済み。`docs/implementation-roadmap.md` の `M3.T3-4.C04` と同じTaskを確認した。'
        '2. 明示対応commit: `dbb2a6b`（題名: 骨格を指定して展開図を提案してもらう画面を追加）。統括が`git log --grep`で実測し、リモート本線`origin/main`（当時のHEAD `dfd3c59`）の祖先であることを確認した。'
        '3. 画面部分も確認済み: 出っぱりを既定4本から6本へ増やして頭1・尾1・足4を指定でき、提案の3画面（骨格→候補→確認）を通り、候補4件の説明は日本語、「この展開図を使う」で適用してdialog 0・展開図（頂点29・辺64・手順1）が入り、そのまま「谷」「折る」道具へ進めた。常設4区画は提案中も適用後も不変。記録は`scratchpad/claude-acceptance-report.md`の796の節と`scratchpad/acceptance-2026-09-05/MANUAL.M3.T3-4.C04.stdout.log`、画像は同フォルダの`MANUAL.M3.T3-4.C04-1-skeleton.png`〜`-5-editable.png`にある。'
        '4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。'
    )
    "MANUAL.M4.T4-6.C02.SCREEN-ACCEPTANCE" = @(
        '1. 2026-09-05に専用CDP枠で実行済み。実行本体: `scratchpad/acceptance-2026-09-05/driver-945-redo.mjs`（exit=0）。作品は`apps/desktop/tests-live/fixtures/frog.ori3`（頂点141・辺280、手順14。`crates/ori3-layers/tests/acceptance_frog.rs`の`frog()`が折る伝承のカエル）。'
        '2. 実測結果: 手順0〜14を1つずつ進めて**カエルが完成した**（札は「1 単純折り」「2 単純折り」「3〜8 開いてつぶす」「9 花弁折り」「10〜13 中割り折り」「14 段折り」で全て日本語、面140、警告0）。「書き出し」→「折り図(PDF)」→「保存先を選んで書き出す」でPDFを書き出し、画面に日本語で「保存しました:frog-diagram.pdf」と出た。書き出したPDFを開いて目視した: **4ページ・A4（595.28×841.89pt＝210×297mm）・427,473バイト**、1ページ目は表紙「折り図／できあがりの形(全14手順)／紙の大きさ 100×100mm」でカエルの完成形（足4本）の絵、2〜4ページは1ページ6コマ・番号1〜14の日本語の手順（山は赤・谷は青・矢印つき、ページ番号「2ページ」〜「4ページ」）。操作不能・英語表示・表示崩れは0件。'
        '3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に手順0・道具「選択」・開いているdialog 0の白紙へ復元した。記録と実測値は`scratchpad/acceptance-2026-09-05/MANUAL.M4.T4-6.C02-redo.stdout.log`、書き出したPDFは`%TEMP%\ori3-acceptance-2026-09-05\frog-diagram.pdf`（SHA-256 `AD4CCBD41DD0E219D1B68053C8A18C9759E7BA289A4DD263B4C91AC638D22536`）、画像は同フォルダの`MANUAL.M4.T4-6.C02-redo-1-opened.png`〜`-4-export-saved.png`と`-pdf-page1.png`〜`-pdf-page4.png`にある。'
        '4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。'
    )
    "MANUAL.ADDITIONAL.FOLD-ALL.C02.SCREEN-ACCEPTANCE" = @(
        '1. 2026-09-05に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-fold-all-latency-cdp.mjs`（exit=0、`ADDITIONAL.FOLD-ALL.C02 VERIFY PASSED`）。ほかの測定を全て止めた静かな状態で3回実行した（同時に走る`cargo`・`rustc`・test実行ファイルはいずれも0件）。'
        '2. 実測結果: NFR-002の2点を3回とも満たした。**ソルバー1回の最大は17.6 / 5.1 / 17.7 ms**（上限33 ms以内）、**3D更新は43.838 / 38.872 / 45.899 回/秒**（下限30回/秒以上）。「全部いっぺんに折ってみる」のつまみへ10・20…100%の10入力を送って往復時間を採り、続けて1秒間つまみを動かして`data-applied-percent`の変化回数を数えた。要件でない「入力から画面反映まで」の時間は合否に使わず参考値として出すだけにしている。'
        '3. PID・実行ファイルSHA-256を照合した（実行ファイルSHA-256 `4BF0DC2268CB7001AED90F852EC5AF228A2EF365FAD82DB4347271EB20DE2FD6`、HEAD `dfd3c59`の同梱版）。測定のあいだだけ`window.fetch`を包み、終了時に必ず元へ戻す。記録と実測値は`scratchpad/acceptance-2026-09-05/M2.T2-6b.FOLD-ALL-LATENCY-quiet-1.log`〜`-quiet-3.log`にある（ID新設前の仮IDのままのファイル名）。'
        '4. 同じ条件で再実行するときも、ソルバー1回が33 msを超えるか、更新が30回/秒を下回れば不合格にする。上限・下限はNFR-002の数値そのままで、緩めない。'
    )
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

function Get-BracedDefinitionRange([string]$Text, [int]$StartIndex, [string]$Label) {
    $openingIndex = $Text.IndexOf('{', $StartIndex)
    if ($openingIndex -lt 0) { throw "$Label の本体開始を読めません" }
    $depth = 0
    for ($characterIndex = $openingIndex; $characterIndex -lt $Text.Length; $characterIndex++) {
        if ($Text[$characterIndex] -eq '{') { $depth++ }
        elseif ($Text[$characterIndex] -eq '}') {
            $depth--
            if ($depth -eq 0) {
                return [pscustomobject]@{
                    StartIndex = $StartIndex
                    OpeningIndex = $openingIndex
                    EndIndex = $characterIndex
                    Text = $Text.Substring($StartIndex, $characterIndex - $StartIndex + 1).TrimEnd()
                }
            }
            if ($depth -lt 0) { break }
        }
    }
    throw "$Label の本体終端を読めません"
}

function Get-BracedDefinition([string]$Text, [int]$StartIndex, [string]$Label) {
    return [string](Get-BracedDefinitionRange $Text $StartIndex $Label).Text
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
$testInventoryBytes = [System.IO.File]::ReadAllBytes($testNamesPath)
$testInventoryHash = Get-BytesSha256 $testInventoryBytes
$testInventoryLines = [System.IO.File]::ReadAllLines($testNamesPath, [System.Text.Encoding]::UTF8)
if ($testInventoryLines.Count -lt 4 -or
    $testInventoryLines[0] -cne '# ORIGAMI3-ROADMAP-EVIDENCE-TEST-NAMES schema=2') {
    throw "検査名台帳のschema見出しがありません: $testNamesPath"
}
$inventoryScopeMatch = [regex]::Match(
    $testInventoryLines[1],
    '^# scope=roadmap-mapped names=(?<names>\d+) source-files=(?<files>\d+) repository-test-total=not-claimed$'
)
if (-not $inventoryScopeMatch.Success) {
    throw "検査名台帳の対象範囲・件数表示が不正です: $testNamesPath"
}
$inventoryDefinitionHashMatch = [regex]::Match($testInventoryLines[2], '^# definition-tree-sha256=(?<sha>[0-9a-f]{64})$')
if (-not $inventoryDefinitionHashMatch.Success) {
    throw "検査名台帳のtest definition hash表示が不正です: $testNamesPath"
}
$testInventoryDataLines = @($testInventoryLines | Where-Object {
    -not [string]::IsNullOrWhiteSpace($_) -and -not $_.StartsWith('#', [StringComparison]::Ordinal)
})
$testInventoryEntries = @(
    foreach ($inventoryLine in $testInventoryDataLines) {
        $entryMatch = [regex]::Match($inventoryLine, '^(?<name>.+) \| (?<path>(?:apps|crates)/[A-Za-z0-9_.\-/]+)(?: \| (?<mode>active-default|ignored-explicit))?$')
        if (-not $entryMatch.Success) { throw "検査名台帳のentry書式が不正です: $inventoryLine" }
        $relativeSourcePath = [string]$entryMatch.Groups['path'].Value
        if ($relativeSourcePath.Contains('..') -or $relativeSourcePath.Contains('\')) {
            throw "検査名台帳のsource pathが不正です: $relativeSourcePath"
        }
        $sourcePath = [IO.Path]::GetFullPath((Join-Path $testSourceRoot $relativeSourcePath))
        $sourceRootPrefix = [IO.Path]::GetFullPath($testSourceRoot).TrimEnd([char[]]'\/') + [IO.Path]::DirectorySeparatorChar
        if (-not $sourcePath.StartsWith($sourceRootPrefix, [StringComparison]::OrdinalIgnoreCase) -or
            -not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
            throw "検査名台帳のsourceがありません: $relativeSourcePath"
        }
        [pscustomobject][ordered]@{
            name = [string]$entryMatch.Groups['name'].Value
            relative_path = $relativeSourcePath
            full_path = $sourcePath
            execution_mode = if ($entryMatch.Groups['mode'].Success) { [string]$entryMatch.Groups['mode'].Value } else { 'active-default' }
        }
    }
)
$testInventoryNames = @($testInventoryEntries | ForEach-Object { $_.name })
$activeDefaultTestCount = @($testInventoryEntries | Where-Object { $_.execution_mode -eq 'active-default' }).Count
$ignoredExplicitTestCount = @($testInventoryEntries | Where-Object { $_.execution_mode -eq 'ignored-explicit' }).Count
$inventoryDuplicates = @($testInventoryNames | Group-Object | Where-Object { $_.Count -ne 1 })
if ($inventoryDuplicates.Count -ne 0) {
    throw "検査名台帳に重複があります: $($inventoryDuplicates.Name -join ', ')"
}
$testInventoryEntryByName = @{}
foreach ($entry in $testInventoryEntries) {
    $testInventoryEntryByName[[string]$entry.name] = $entry
}
$declaredInventoryCount = [int]$inventoryScopeMatch.Groups['names'].Value
if ($declaredInventoryCount -ne $testInventoryNames.Count) {
    throw "検査名台帳の表示件数が実数と不一致です: declared=$declaredInventoryCount actual=$($testInventoryNames.Count)"
}
$sourceRelativePaths = @($testInventoryEntries | ForEach-Object { $_.relative_path } | Sort-Object -Unique)
$declaredSourceFileCount = [int]$inventoryScopeMatch.Groups['files'].Value
if ($declaredSourceFileCount -ne $sourceRelativePaths.Count) {
    throw "検査名台帳のsource file表示件数が実数と不一致です: declared=$declaredSourceFileCount actual=$($sourceRelativePaths.Count)"
}
$definitionManifestLines = New-Object System.Collections.Generic.List[string]
foreach ($entry in $testInventoryEntries) {
    $sourceLines = [IO.File]::ReadAllLines($entry.full_path, [Text.Encoding]::UTF8)
    $sourceText = $sourceLines -join "`n"
    $canonicalDefinitionLines = New-Object System.Collections.Generic.List[string]
    if ($entry.relative_path.EndsWith('.rs', [StringComparison]::Ordinal)) {
        $rustName = [string]$entry.name
        $lastSeparator = $rustName.LastIndexOf('::', [StringComparison]::Ordinal)
        if ($lastSeparator -ge 0) { $rustName = $rustName.Substring($lastSeparator + 2) }
        $signaturePattern = '^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+' + [regex]::Escape($rustName) + '\s*\('
        $signatureIndexes = @(
            for ($sourceLineIndex = 0; $sourceLineIndex -lt $sourceLines.Count; $sourceLineIndex++) {
                if ($sourceLines[$sourceLineIndex] -match $signaturePattern) { $sourceLineIndex }
            }
        )
        if ($rustName -notmatch '^[A-Za-z_][A-Za-z0-9_]*$' -or $signatureIndexes.Count -ne 1) {
            throw "検査名台帳のRust test定義がsourceにありません: $($entry.name) -> $($entry.relative_path)"
        }
        $signatureIndex = [int]$signatureIndexes[0]
        $testAttribute = $null
        $hasIgnoredAttribute = $false
        $hasConditionalAttribute = $false
        $outerAttributeLines = @()
        $attributeFloor = 0
        for ($attributeIndex = $signatureIndex - 1; $attributeIndex -ge $attributeFloor; $attributeIndex--) {
            $attributeCandidate = $sourceLines[$attributeIndex].Trim()
            if ($attributeCandidate.Length -eq 0 -or $attributeCandidate.StartsWith('//', [StringComparison]::Ordinal)) {
                continue
            }
            if (-not $attributeCandidate.StartsWith('#[', [StringComparison]::Ordinal)) {
                break
            }
            $outerAttributeLines = @($attributeCandidate) + $outerAttributeLines
            if ($attributeCandidate -match '^#\[(?:cfg|cfg_attr)\b') {
                $hasConditionalAttribute = $true
            }
            if ($attributeCandidate -match '^#\[(?:ignore|cfg_attr\([^]]*\bignore\b)') {
                $hasIgnoredAttribute = $true
            }
            if ($attributeCandidate -match '^#\[(?:[A-Za-z0-9_]+::)?test(?:\([^]]*\))?\]$') {
                $testAttribute = $attributeCandidate
            }
        }
        if ($null -eq $testAttribute) {
            throw "検査名台帳のRust test属性がありません: $($entry.name) -> $($entry.relative_path)"
        }
        if ($hasConditionalAttribute) {
            throw "検査名台帳のRust testに条件付き属性があります: $($entry.name) -> $($entry.relative_path) attributes=$($outerAttributeLines -join ',')"
        }
        foreach ($outerAttributeLine in $outerAttributeLines) {
            $canonicalDefinitionLines.Add([string]$outerAttributeLine)
        }
        if ($hasIgnoredAttribute) {
            if ($entry.execution_mode -ne 'ignored-explicit' -or
                $entry.relative_path -notmatch '^crates/(?<crate>[A-Za-z0-9_-]+)/tests/(?<target>[A-Za-z0-9_-]+)\.rs$') {
                throw "検査名台帳のRust test属性がactiveではありません: $($entry.name) -> $($entry.relative_path) mode=$($entry.execution_mode) ignored=True"
            }
            $crateName = [string]$Matches['crate']
            $testTarget = [string]$Matches['target']
            $ignoredCommand = "cargo test --release -p $crateName --test $testTarget -- --ignored --nocapture"
            $workflowContractPath = Join-Path $executionContractRoot '.github/workflows/ci.yml'
            $checkCiContractPath = Join-Path $executionContractRoot 'scripts/check-ci.ps1'
            $rulesContractPath = Join-Path $executionContractRoot 'docs/rules/03-品質ゲート.md'
            foreach ($contractPath in @($workflowContractPath, $checkCiContractPath, $rulesContractPath)) {
                if (-not (Test-Path -LiteralPath $contractPath -PathType Leaf)) {
                    throw "ignored testの実行契約がありません: $contractPath"
                }
            }
            $workflowContractLines = [IO.File]::ReadAllLines($workflowContractPath, [Text.Encoding]::UTF8)
            $checkCiContractLines = [IO.File]::ReadAllLines($checkCiContractPath, [Text.Encoding]::UTF8)
            $rulesContractLines = [IO.File]::ReadAllLines($rulesContractPath, [Text.Encoding]::UTF8)
            $workflowExecutionLines = @($workflowContractLines | Where-Object { $_ -match '^\s*run:\s*' + [regex]::Escape($ignoredCommand) + '\s*$' })
            $checkCiExecutionLines = @($checkCiContractLines | Where-Object {
                $_.Contains('Command = "' + $ignoredCommand + '"') -and
                $_.Contains('Arguments = @("test", "--release", "-p", "' + $crateName + '", "--test", "' + $testTarget + '", "--", "--ignored", "--nocapture")')
            })
            $rulesExecutionLines = @($rulesContractLines | Where-Object { $_.Contains('`' + $ignoredCommand + '`') })
            if ($workflowExecutionLines.Count -ne 1 -or $checkCiExecutionLines.Count -ne 1 -or $rulesExecutionLines.Count -ne 1) {
                throw "ignored testの明示実行がCI/check-ci/品質規約の3経路で一意ではありません: $($entry.name) workflow=$($workflowExecutionLines.Count) check_ci=$($checkCiExecutionLines.Count) rules=$($rulesExecutionLines.Count)"
            }
            $canonicalDefinitionLines.Add("execution_mode=ignored-explicit command=$ignoredCommand")
            $canonicalDefinitionLines.Add($workflowExecutionLines[0].Trim())
            $canonicalDefinitionLines.Add($checkCiExecutionLines[0].Trim())
            $canonicalDefinitionLines.Add($rulesExecutionLines[0].Trim())
        }
        elseif ($entry.execution_mode -ne 'active-default') {
            throw "検査名台帳のexecution modeとRust属性が不一致です: $($entry.name) mode=$($entry.execution_mode) ignored=False"
        }
        $signatureMatches = @([regex]::Matches($sourceText, '(?m)' + $signaturePattern))
        if ($signatureMatches.Count -ne 1) {
            throw "検査名台帳のRust test本体開始が一意ではありません: $($entry.name) -> $($entry.relative_path)"
        }
        $rustDefinition = Get-BracedDefinition $sourceText $signatureMatches[0].Index "Rust test $($entry.name)"
        $canonicalDefinitionLines.Add($rustDefinition)
        # Rustの本体抽出だけを証拠にすると、文字列・コメント内の波括弧を構文上の
        # 終端と誤認した場合に、その後ろのassert変更を見逃し得る。改行をLFへ
        # 正規化した実source全体もdefinition treeへ入れ、抽出器の誤認時もfail-closedにする。
        $rustSourceBytes = [Text.Encoding]::UTF8.GetBytes($sourceText + "`n")
        $canonicalDefinitionLines.Add("rust_source_sha256=$(Get-BytesSha256 $rustSourceBytes)")
    }
    else {
        if ($entry.execution_mode -ne 'active-default') {
            throw "画面testに未対応のexecution modeです: $($entry.name) mode=$($entry.execution_mode)"
        }
        $segments = @($entry.name -split ' > ')
        $expectedRelativePath = $entry.relative_path.Substring('apps/desktop/'.Length)
        if ($segments.Count -lt 2 -or -not [string]::Equals($segments[0], $expectedRelativePath, [StringComparison]::Ordinal)) {
            throw "検査名台帳の画面test pathが名前と一致しません: $($entry.name) -> $($entry.relative_path)"
        }
        $declarationIndexes = New-Object System.Collections.Generic.List[int]
        for ($segmentIndex = 1; $segmentIndex -lt $segments.Count; $segmentIndex++) {
            $segment = $segments[$segmentIndex]
            $declarationPattern = '\b(?:describe|it|test)\s*\(\s*["'']' + [regex]::Escape($segment) + '["'']'
            $matchingLineIndexes = @(
                for ($sourceLineIndex = 0; $sourceLineIndex -lt $sourceLines.Count; $sourceLineIndex++) {
                    $trimmedSourceLine = $sourceLines[$sourceLineIndex].Trim()
                    if ($sourceLines[$sourceLineIndex] -match $declarationPattern -and
                        -not $trimmedSourceLine.StartsWith('//', [StringComparison]::Ordinal) -and
                        -not $trimmedSourceLine.StartsWith('*', [StringComparison]::Ordinal)) {
                        $sourceLineIndex
                    }
                }
            )
            if ($matchingLineIndexes.Count -ne 1) {
                throw "検査名台帳の画面test宣言がsourceにありません: $segment -> $($entry.relative_path)"
            }
            $declarationIndexes.Add([int]$matchingLineIndexes[0])
        }
        for ($declarationIndex = 1; $declarationIndex -lt $declarationIndexes.Count; $declarationIndex++) {
            if ($declarationIndexes[$declarationIndex] -le $declarationIndexes[$declarationIndex - 1]) {
                throw "検査名台帳の画面test suite順が不正です: $($entry.name) -> $($entry.relative_path)"
            }
        }
        $declarationStartIndexes = New-Object System.Collections.Generic.List[int]
        foreach ($declarationLineIndex in $declarationIndexes) {
            $declarationStartIndex = 0
            for ($sourceLineIndex = 0; $sourceLineIndex -lt $declarationLineIndex; $sourceLineIndex++) {
                $declarationStartIndex += $sourceLines[$sourceLineIndex].Length + 1
            }
            $declarationStartIndexes.Add($declarationStartIndex)
        }
        for ($declarationIndex = 0; $declarationIndex -lt $declarationIndexes.Count - 1; $declarationIndex++) {
            $suiteRange = Get-BracedDefinitionRange $sourceText $declarationStartIndexes[$declarationIndex] "画面test suite $($segments[$declarationIndex + 1])"
            $childStartIndex = $declarationStartIndexes[$declarationIndex + 1]
            if ($childStartIndex -le $suiteRange.OpeningIndex -or $childStartIndex -ge $suiteRange.EndIndex) {
                throw "検査名台帳の画面testが指定suite内にありません: $($entry.name) -> $($entry.relative_path)"
            }
            $suiteRegistrationPrefix = $sourceText.Substring(
                $declarationStartIndexes[$declarationIndex],
                $childStartIndex - $declarationStartIndexes[$declarationIndex]
            ).TrimEnd()
            $canonicalDefinitionLines.Add($suiteRegistrationPrefix)
        }
        $targetLineIndex = $declarationIndexes[$declarationIndexes.Count - 1]
        $targetStartIndex = $declarationStartIndexes[$declarationStartIndexes.Count - 1]
        $screenDefinition = Get-BracedDefinition $sourceText $targetStartIndex "画面test $($entry.name)"
        $canonicalDefinitionLines.Add($screenDefinition)
        $screenSourceBytes = [Text.Encoding]::UTF8.GetBytes($sourceText + "`n")
        $canonicalDefinitionLines.Add("screen_source_sha256=$(Get-BytesSha256 $screenSourceBytes)")
    }
    $definitionBytes = [Text.Encoding]::UTF8.GetBytes(($canonicalDefinitionLines.ToArray() -join "`n") + "`n")
    $definitionManifestLines.Add("$($entry.name)`t$($entry.relative_path)`t$($entry.execution_mode)`t$(Get-BytesSha256 $definitionBytes)")
}
$definitionManifestBytes = [Text.Encoding]::UTF8.GetBytes(($definitionManifestLines.ToArray() -join "`n") + "`n")
$testDefinitionTreeHash = Get-BytesSha256 $definitionManifestBytes
$declaredDefinitionTreeHash = [string]$inventoryDefinitionHashMatch.Groups['sha'].Value
if ($WriteTestNamesHash) {
    # 見出しの64桁だけを、いま計算した値のASCII bytesへ置き換える。
    # 他のbyteは1つも触らないので、改行の種類・並び・件数表示は保存される。
    $inventoryMarkerBytes = [Text.Encoding]::ASCII.GetBytes('# definition-tree-sha256=')
    $inventoryMarkerOffsets = New-Object System.Collections.Generic.List[int]
    for ($scanIndex = 0; $scanIndex -le $testInventoryBytes.Length - ($inventoryMarkerBytes.Length + 64); $scanIndex++) {
        $matched = $true
        for ($markerIndex = 0; $markerIndex -lt $inventoryMarkerBytes.Length; $markerIndex++) {
            if ($testInventoryBytes[$scanIndex + $markerIndex] -ne $inventoryMarkerBytes[$markerIndex]) { $matched = $false; break }
        }
        if ($matched) { $inventoryMarkerOffsets.Add($scanIndex) }
    }
    if ($inventoryMarkerOffsets.Count -ne 1) {
        throw "検査名台帳のtest definition hash見出しをbytes内で1箇所に特定できません (実際: $($inventoryMarkerOffsets.Count)箇所)"
    }
    if ([string]::Equals($declaredDefinitionTreeHash, $testDefinitionTreeHash, [StringComparison]::Ordinal)) {
        Write-Host "[FRESH] 検査名台帳のtest definition hashは既に現在定義と一致しています: $testDefinitionTreeHash"
        exit 0
    }
    # hashは作業ツリーの現在の中身から出る。未コミットの検査sourceがあると、
    # commit済みだけを見るCIの値と食い違う(§10.1)。黙って焼き付けないよう、
    # 対象の未コミット分を実名で表示してから書く。commitの直前にもう一度
    # このmodeを回すこと。
    $dirtySourcePaths = @()
    try {
        $global:LASTEXITCODE = 0
        $statusOutput = @(& git -C $repoRoot --no-optional-locks status --porcelain --untracked-files=no -- @($sourceRelativePaths) 2>$null)
        if ($LASTEXITCODE -eq 0) {
            $dirtySourcePaths = @($statusOutput | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) })
        }
    }
    catch { $dirtySourcePaths = @() }
    if ($dirtySourcePaths.Count -ne 0) {
        Write-Host "[DIRTY] 未コミットの検査sourceを含む値です ($($dirtySourcePaths.Count)件)。commit直前に再実行してください:"
        foreach ($dirtySourcePath in $dirtySourcePaths) { Write-Host "  $dirtySourcePath" }
    }
    $newHashBytes = [Text.Encoding]::ASCII.GetBytes($testDefinitionTreeHash)
    if ($newHashBytes.Length -ne 64) { throw "計算したtest definition hashが64桁ではありません: $testDefinitionTreeHash" }
    $updatedInventoryBytes = New-Object byte[] ($testInventoryBytes.Length)
    [Array]::Copy($testInventoryBytes, $updatedInventoryBytes, $testInventoryBytes.Length)
    [Array]::Copy($newHashBytes, 0, $updatedInventoryBytes, $inventoryMarkerOffsets[0] + $inventoryMarkerBytes.Length, 64)
    [IO.File]::WriteAllBytes($testNamesPath, $updatedInventoryBytes)
    Write-Host "[WRITE] 検査名台帳のtest definition hashを更新しました:`nold=$declaredDefinitionTreeHash`nnew=$testDefinitionTreeHash`ndefinitions=$($definitionManifestLines.Count) files=$($sourceRelativePaths.Count)"
    exit 0
}
if (-not [string]::Equals($declaredDefinitionTreeHash, $testDefinitionTreeHash, [StringComparison]::Ordinal)) {
    throw "検査名台帳のtest definition hashが現在定義と不一致です:`ndeclared=$declaredDefinitionTreeHash`nactual=$testDefinitionTreeHash`ndefinitions=$($definitionManifestLines.Count) files=$($sourceRelativePaths.Count)"
}
$testInventory = ($testInventoryNames -join "`n") + "`n"
$mappedTestNames = @(
    foreach ($value in $taskTests.Values) {
        foreach ($name in @($value)) { [string]$name }
    }
    foreach ($value in $linkTests.Values) {
        foreach ($name in @($value)) { [string]$name }
    }
    foreach ($value in $scopeFallbackTests.Values) {
        foreach ($name in @($value)) { [string]$name }
    }
) | Sort-Object -Unique
$missingMappedNames = @($mappedTestNames | Where-Object { $testInventoryNames -cnotcontains $_ })
$extraInventoryNames = @($testInventoryNames | Where-Object { $mappedTestNames -cnotcontains $_ })
if ($missingMappedNames.Count -ne 0 -or $extraInventoryNames.Count -ne 0) {
    throw "検査名台帳と全割当の集合が不一致です: mapped=$($mappedTestNames.Count) inventory=$($testInventoryNames.Count) missing=$($missingMappedNames -join ', ') extra=$($extraInventoryNames -join ', ')"
}
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
    if ($line -match '^## M[0-9](?:\s|:)' -or
        [string]::Equals($line, '## 3. マイルストーン完了時の共通チェック', [StringComparison]::Ordinal)) {
        $scope = $null
        $task = $null
        $linkTask = $null
    }
    if ($line -match '^## (M[0-5])(?:\s|:)') {
        $scope = $Matches[1]
        $task = $null
        $linkTask = $null
    }
    if ([string]::Equals($line, '### 全部の折り目を一斉に折る一時表示', [StringComparison]::Ordinal)) {
        # 追加目標のうち、完了済みのfold-allだけを証拠台帳へ昇格する。
        # 直前のFOLD-IOは引き続き明示対象外であり、汎用的に追加目標を取り込まない。
        $scope = "ADDITIONAL"
        $task = "Fold All"
        $linkTask = $task
    }
    elseif ($line -match '^### (Task [0-9]+-[0-9A-Za-z]+)') {
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

        if ($scope -notin @("M1", "M2", "M3", "M4", "M5", "ADDITIONAL")) {
            $updatedLines.Add($line)
            continue
        }
        if ($null -eq $task -or $null -eq $linkTask) { throw "Task又は作業見出しを判定できません: line $($index + 1)" }

        # link IDと既存の証拠名は、ロードマップ本文に書かれた既存markerと一致させる。
        # 一方、進捗照合は下の$task（作業18ならWork 18）だけで行う。
        $taskKey = "$scope|$linkTask"
        # ADDITIONALは承認済みの節だけを受け入れる。採番そのものは他のscopeと同じ
        # 共通経路($taskOrdinals + $taskLinkIdOrder)で行う。以前はここでIDを
        # "ADDITIONAL.FOLD-ALL.C01"へ固定していたため、同じ節へ2件目のcheckboxを
        # 足すと導出IDがC01のまま重なり、必ず「inline linkが台帳と一致しません」で
        # 止まった(2026-09-05に実測)。
        if ($scope -eq "ADDITIONAL" -and $taskKey -ne "ADDITIONAL|Fold All") {
            throw "未承認の追加目標を証拠台帳へ取り込めません: $taskKey"
        }
        if (-not $taskOrdinals.ContainsKey($taskKey)) { $taskOrdinals[$taskKey] = 0 }
        $taskOrdinals[$taskKey] = [int]$taskOrdinals[$taskKey] + 1
        $taskNumber = ([regex]::Match($linkTask, 'Task (?<number>[0-9]+-[0-9A-Za-z]+)')).Groups['number'].Value
        if ($taskLinkIdOrder.ContainsKey($taskKey)) {
            $orderedIds = @($taskLinkIdOrder[$taskKey])
            if ($taskOrdinals[$taskKey] -gt $orderedIds.Count) { throw "Taskの固定link ID順序を超えました: $taskKey" }
            $linkId = [string]$orderedIds[$taskOrdinals[$taskKey] - 1]
        }
        else {
            $linkId = "$scope.T$taskNumber.C{0:D2}" -f $taskOrdinals[$taskKey]
        }
        $manualKind = Get-ManualKind $checkboxText
        $testName = $null
        $testNames = @()
        $manualId = $null
        $commitProof = $null
        $commitHash = $null
        $commitSubject = $null
        $commitMainAncestor = $null
        $testMappingOrigin = $null
        if ($linkTests.ContainsKey($linkId)) {
            $testNames = @($linkTests[$linkId])
            $testNames = @($testNames | ForEach-Object { [string]$_ } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
            if ($testNames.Count -eq 0) { throw "検査名を割り当てられません: $linkId" }
            $testName = $testNames[0]
            $evidenceId = "TEST.$linkId"
            $evidenceType = "test"
            $testMappingOrigin = "link-id"
        }
        elseif ($null -ne $manualKind) {
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
            $testMappingOrigin = "task-default-or-scope-fallback"
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
            test_mapping_origin = $testMappingOrigin
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
                '\s+— \[証拠:[^\]]+\]\(traceability/roadmap-links\.md#[^)]+\) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=[^\s]+ evidence=[^\s>]+ -->',
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

# M6はcheckboxがない。D9で指定された受入基準を、証拠link checkbox監査から分離して登録する。
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
    test_mapping_origin = $null
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
$checkedAutomaticRecords = @($checkboxRecords | Where-Object {
    $_.checkbox_state -eq "checked" -and $_.evidence_type -eq "test"
})
$checkedAutomaticWithoutExplicitLinkTests = @($checkedAutomaticRecords | Where-Object {
    $_.test_mapping_origin -ne "link-id"
})
if ($checkedAutomaticWithoutExplicitLinkTests.Count -ne 0) {
    throw "完了済み自動証拠にlink ID単位の明示検査割当がありません: $($checkedAutomaticWithoutExplicitLinkTests.id -join ', ')"
}
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
        # 完了済み自動証拠では、存在だけでなく実行形態も明示的に再確認する。
        # active-default以外を許すのは、上でCI/check-ci/品質規約の3経路を照合した
        # ignored-explicitだけである。未実行のskipや存在しない検査はここへ到達できない。
        if ($record.checkbox_state -eq "checked") {
            $entry = $testInventoryEntryByName[$testName]
            if ($null -eq $entry -or $entry.execution_mode -notin @("active-default", "ignored-explicit")) {
                throw "完了済み自動証拠の検査が実行可能な台帳entryではありません: $($record.id) -> $testName"
            }
        }
    }
}
if ($checkboxRecords.Count -ne [int]$roadmapSnapshot.evidence_linked) {
    throw "証拠link checkbox数がsnapshotと不一致です: records=$($checkboxRecords.Count) snapshot=$($roadmapSnapshot.evidence_linked)"
}
foreach ($scopeName in $expectedCheckboxCounts.Keys) {
    if ($scopeCounts[$scopeName] -ne $expectedCheckboxCounts[$scopeName]) {
        throw "$scopeName checkbox数が不一致です: expected=$($expectedCheckboxCounts[$scopeName]) actual=$($scopeCounts[$scopeName])"
    }
}
if ($duplicateIds.Count -ne 0) { throw "link ID重複があります: $($duplicateIds.Name -join ', ')" }
if ($unresolvedRecords.Count -ne 0) { throw "未接続linkがあります: $($unresolvedRecords.id -join ', ')" }

$canonicalRecords = @($records | Sort-Object id)
$explicitOutsideItems = @($roadmapSnapshot.items | Where-Object { $_.source_kind -eq "explicit_outside" } | Sort-Object id | ForEach-Object {
    [ordered]@{
        id = [string]$_.id
        line_number = [int]$_.line_number
        state = [string]$_.state
        text_sha256 = [string]$_.text_sha256
    }
})
if ($checkboxRecords.Count + $explicitOutsideItems.Count -ne [int]$roadmapSnapshot.total) {
    throw "証拠linkと明示対象外の会計が全体件数と一致しません: linked=$($checkboxRecords.Count) outside=$($explicitOutsideItems.Count) total=$($roadmapSnapshot.total)"
}
$jsonDocument = [ordered]@{
    schema = 2
    generated_by = "scripts/doc-link-audit.ps1"
    roadmap_snapshot = [ordered]@{
        schema = [int]$roadmapSnapshot.schema
        sha256 = [string]$roadmapSnapshot.roadmap_sha256
        policy_sha256 = [string]$roadmapSnapshot.policy_sha256
        total = [int]$roadmapSnapshot.total
        audited = [int]$roadmapSnapshot.audited
        checked = [int]$roadmapSnapshot.checked
        unchecked = [int]$roadmapSnapshot.unchecked
    }
    test_name_inventory = [ordered]@{
        path = "docs/traceability/roadmap-evidence-test-names.txt"
        schema = 2
        scope = "roadmap-mapped"
        audited = [int]$testInventoryNames.Count
        mapped = [int]$mappedTestNames.Count
        source_files = [int]$sourceRelativePaths.Count
        definition_tree_sha256 = $testDefinitionTreeHash
        execution_modes = [ordered]@{
            active_default = [int]$activeDefaultTestCount
            ignored_explicit = [int]$ignoredExplicitTestCount
        }
        repository_test_total = $null
        sha256 = $testInventoryHash
    }
    traceability_accounting = [ordered]@{
        linked_checkbox_count = [int]$checkboxRecords.Count
        explicit_outside_count = [int]$explicitOutsideItems.Count
        unclassified_count = [int]$roadmapSnapshot.unclassified
        accounted_total = [int]($checkboxRecords.Count + $explicitOutsideItems.Count)
    }
    explicit_outside = $explicitOutsideItems
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
$generatedHash = Get-BytesSha256 $jsonBytes

function ConvertTo-MarkdownCell([string]$Value) {
    if ($null -eq $Value) { return "—" }
    return $Value.Replace("|", "\\|").Replace("`r", " ").Replace("`n", " ")
}

$markdown = New-Object System.Collections.Generic.List[string]
$markdown.Add("# ロードマップ証拠リンク")
$markdown.Add("")
$markdown.Add('この台帳は `scripts/doc-link-audit.ps1` が生成する。各checkboxの本文にある証拠リンクと同じIDを持ち、検査名は `docs/traceability/roadmap-evidence-test-names.txt` の対象限定台帳で照合する。これはリポジトリ全検査の件数を主張しない。')
$markdown.Add("")
$markdown.Add("- ロードマップ全体: $($roadmapSnapshot.audited)/$($roadmapSnapshot.total)件 (checked=$($roadmapSnapshot.checked), unchecked=$($roadmapSnapshot.unchecked))")
$markdown.Add("- 証拠台帳対象: $($checkboxRecords.Count)/$($roadmapSnapshot.total)件")
$markdown.Add("- 明示対象外: $($explicitOutsideItems.Count)件")
$markdown.Add("- 検査名台帳対象: $($testInventoryNames.Count)/$($mappedTestNames.Count)件（roadmap-mapped、リポジトリ全検査数は主張しない）")
$markdown.Add("- 検査定義対象: $($testInventoryNames.Count)/$($mappedTestNames.Count)件、source $($sourceRelativePaths.Count)/$($sourceRelativePaths.Count)ファイル、definition tree SHA-256: ``$testDefinitionTreeHash``")
$markdown.Add("- 実行モード: active-default=${activeDefaultTestCount}件、ignored-explicit=${ignoredExplicitTestCount}件（後者はCI・check-ci・品質規約の明示実行を照合）")
$markdown.Add("- 検査名台帳SHA-256: ``$testInventoryHash``")
$markdown.Add("- ロードマップSHA-256: ``$($roadmapSnapshot.roadmap_sha256)``")
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
$manualMarkdown.Add("- ロードマップSHA-256: ``$($roadmapSnapshot.roadmap_sha256)``")
$manualMarkdown.Add("- 検査名台帳SHA-256: ``$testInventoryHash``（roadmap-mapped $($testInventoryNames.Count)/$($mappedTestNames.Count)件、source $($sourceRelativePaths.Count)/$($sourceRelativePaths.Count)ファイル、definition tree ``$testDefinitionTreeHash``、リポジトリ全検査数は主張しない）")
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
    if ($manuallyRecordedAcceptance.Contains($record.manual_id)) {
        foreach ($recordedLine in $manuallyRecordedAcceptance[$record.manual_id]) {
            $manualMarkdown.Add($recordedLine)
        }
    }
    elseif ($record.manual_id -match "COMMIT-PUSH$") {
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
        if ($record.manual_id -eq "MANUAL.M2.T2-6c.C07.SCREEN-ACCEPTANCE") {
            $manualMarkdown.Add("1. 担当: このDOM検査を実装していないレビュー担当者（画面の目視確認をした人とも別の人）。")
            $manualMarkdown.Add("2. 対象: ``apps/desktop/src/components/Viewer3D/Viewer3D.dom.test.tsx``、``ViewerOperationHint.dom.test.tsx``、``PaperActionTip.dom.test.tsx``、``apps/desktop/src/components/OperationSteps.dom.test.tsx``、``apps/desktop/src/lib/layerMotion.test.ts``、およびテスト設定。プレビュー表示、操作理由、ドラッグの開始・移動・終了、手順表示のそれぞれが、画面部品を実際に組み立てた検査で確認されているかを読む。")
            $manualMarkdown.Add("3. 実行: ``cd apps/desktop; npm.cmd run test -- --run src/components/Viewer3D/Viewer3D.dom.test.tsx src/components/Viewer3D/ViewerOperationHint.dom.test.tsx src/components/Viewer3D/PaperActionTip.dom.test.tsx src/components/OperationSteps.dom.test.tsx src/lib/layerMotion.test.ts``。実行結果で全対象がpassし、skipが0であることを確認する。")
            $manualMarkdown.Add("4. 合格: 各観点に対応するtest名・実行結果・確認日を記録する。入力変換だけ、test名だけ、又は画面の見た目だけでは合格にしない。少なくとも1本はDOM上のpointer down/move/upを通し、少なくとも1本はプレビュー又は操作理由のDOMを確認していなければ不合格とする。")
            $manualMarkdown.Add("5. 不合格: 対応する検査が無い、skipがある、又は上のコマンドが失敗したときは、文書の状態を変えず不足した観点と実パスを統括へ報告する。")
        }
        elseif ($record.manual_id -eq "MANUAL.M4.T4-5.C03.SCREEN-ACCEPTANCE") {
            $manualMarkdown.Add("1. 担当: 受入担当者（必要なら利用者）。折り鶴ではない、保存済みで手順を2件以上含む作品を1つ開く。所要時間の目安は10分。")
            $manualMarkdown.Add("2. 画面上部の「書き出し」を開き、「折り図(PDF)」を選ぶ。保存先は空の作業用フォルダーを選び、保存する。保存後、そのフォルダーに``.pdf``ファイルがちょうど1個あり、サイズが0より大きく、通常のPDF閲覧ソフトで開けることを確認する。")
            $manualMarkdown.Add("3. 同じ作品で再び「書き出し」を開き、「折り図(ページごとのSVG)」を選ぶ。PDFとは別の空の作業用フォルダーを選び、保存する。``.svg``ファイルが手順数以上あり、すべてサイズが0より大きく、少なくとも最初と最後のファイルを開いて図と手順番号が表示されることを確認する。")
            $manualMarkdown.Add("4. 合格: 2種類の選択肢をそれぞれ選べ、保存操作後に上記のファイル数・拡張子・サイズ・内容を満たす。確認日、作品の手順数、各フォルダーのファイル名とサイズ、確認者を記録する。")
            $manualMarkdown.Add("5. 不合格: 選択肢が無い、保存画面へ進めない、保存が失敗する、ファイル数・拡張子・サイズ・表示のいずれかが条件を満たさない場合は、状態を変えずに統括へ報告する。")
        }
        elseif ($completedCdpAcceptance.Contains($record.manual_id)) {
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

$lineEnding = [Environment]::NewLine
$utf8NoBom = New-Object Text.UTF8Encoding($false)
$markdownText = ($markdown.ToArray() -join $lineEnding) + $lineEnding
$manualText = ($manualMarkdown.ToArray() -join $lineEnding) + $lineEnding
$markdownBytes = $utf8NoBom.GetBytes($markdownText)
$manualBytes = $utf8NoBom.GetBytes($manualText)

$report = New-Object System.Collections.Generic.List[string]
$report.Add("# 施策7-D2〜D10 文書リンク監査")
$report.Add("")
$report.Add("生成日時: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss K')")
$report.Add("")
$report.Add("## 検査名一覧の取得")
$report.Add("")
$report.Add("- 取込先: ``docs/traceability/roadmap-evidence-test-names.txt``（roadmap-mapped $($testInventoryNames.Count)/$($mappedTestNames.Count)件。リポジトリ全検査数は主張しない）")
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
$report.Add("| checkbox総数 | $($checkboxRecords.Count) / $($roadmapSnapshot.evidence_linked) |")
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
    [System.IO.File]::WriteAllBytes($jsonPath, $jsonBytes)
    [System.IO.File]::WriteAllBytes($markdownPath, $markdownBytes)
    [System.IO.File]::WriteAllBytes($manualPath, $manualBytes)
    if (-not $PreserveReport) {
        [System.IO.File]::WriteAllLines($reportPath, $report, (New-Object System.Text.UTF8Encoding($false)))
    }
}

$artifactFresh = $true
if ($Check -or $CheckTraceability) {
    if (Test-Path -LiteralPath $jsonPath -PathType Leaf) {
        try {
            $savedLedger = [IO.File]::ReadAllText($jsonPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
            $savedRoadmapHash = [string]$savedLedger.roadmap_snapshot.sha256
            if (-not [string]::Equals($savedRoadmapHash, [string]$roadmapSnapshot.roadmap_sha256, [StringComparison]::Ordinal)) {
                Write-Host "[STALE] roadmap snapshot hash不一致: saved=$savedRoadmapHash current=$($roadmapSnapshot.roadmap_sha256)"
                $artifactFresh = $false
            }
            $savedInventoryHash = [string]$savedLedger.test_name_inventory.sha256
            if (-not [string]::Equals($savedInventoryHash, $testInventoryHash, [StringComparison]::Ordinal)) {
                Write-Host "[STALE] test inventory hash不一致: saved=$savedInventoryHash current=$testInventoryHash"
                $artifactFresh = $false
            }
        }
        catch {
            Write-Host "[STALE] roadmap-links.json のsnapshot metadataを読めません: $($_.Exception.Message)"
            $artifactFresh = $false
        }
    }
}
if ($Check -or $CheckTraceability -or $Update -or $WriteTraceability) {
    if (-not (Test-ArtifactBytes "roadmap-links.json" $jsonPath $jsonBytes)) { $artifactFresh = $false }
    if (-not (Test-ArtifactBytes "roadmap-links.md" $markdownPath $markdownBytes)) { $artifactFresh = $false }
    if (-not (Test-ArtifactBytes "manual-acceptance.md" $manualPath $manualBytes)) { $artifactFresh = $false }
}

Write-Output "roadmap_accounted=$($roadmapSnapshot.audited)/$($roadmapSnapshot.total) checked=$($roadmapSnapshot.checked) unchecked=$($roadmapSnapshot.unchecked) traceability_linked=$($checkboxRecords.Count)/$($roadmapSnapshot.total) explicit_outside=$($explicitOutsideItems.Count) unclassified=$($roadmapSnapshot.unclassified) test_inventory_scope=roadmap-mapped test_inventory_audited=$($testInventoryNames.Count)/$($mappedTestNames.Count) test_source_files=$($sourceRelativePaths.Count)/$($sourceRelativePaths.Count) test_definition_tree_sha256=$testDefinitionTreeHash test_execution_active=$activeDefaultTestCount test_execution_ignored_explicit=$ignoredExplicitTestCount repository_test_total=not-claimed test_inventory_sha256=$testInventoryHash test=$($testRecords.Count) manual=$($manualRecords.Count) unlinked=$($unresolvedRecords.Count) duplicate=$($duplicateIds.Count) progress_disagreement=$($statusDisagreements.Count) unchecked_with_completed_task=$(@($checkboxRecords | Where-Object { $_.progress_state -eq 'unchecked-but-progress-task-exists' }).Count) commit_evidence_verified=$($verifiedCommitEvidence.Count) commit_evidence_unverified=$($unverifiedCommitEvidence.Count) unchecked_with_test=$($uncheckedWithTests.Count) reverse_progress_disagreement=$($reverseDisagreements.Count) hash=$generatedHash"
if (-not $artifactFresh) {
    Write-Output "TRACEABILITY-STALE: saved artifacts do not match current roadmap snapshot"
    if ($Check -or $CheckTraceability) { exit 2 }
    throw "書き込んだ証拠台帳bytesが再生成結果と一致しません"
}
if (($statusDisagreements.Count + $reverseDisagreements.Count) -ne 0) {
    Write-Output "STATUS-MISMATCH: roadmap status was preserved; see scratchpad/doc-link-7d-report.md"
    if ($Check) { exit 2 }
}
if ($CheckTraceability) { exit 0 }
