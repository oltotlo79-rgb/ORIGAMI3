# doc-link-audit.ps1 のbyte freshness自己試験。
# 本番roadmapから本番scriptが生成した3成果物を基準にし、そのbytesを変異させる。

$ErrorActionPreference = "Stop"
$sut = Join-Path $PSScriptRoot "doc-link-audit.ps1"
$snapshotSut = Join-Path $PSScriptRoot "get-roadmap-status.ps1"
$repoRoot = Split-Path -Parent $PSScriptRoot
$productionRoadmap = Join-Path $repoRoot "docs/implementation-roadmap.md"
$productionInventory = Join-Path $repoRoot "docs/traceability/roadmap-evidence-test-names.txt"
$powershellExe = (Get-Process -Id $PID).Path
$tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$tempName = "ori3-doc-link-audit-test-" + [Guid]::NewGuid().ToString("N")
$tempRoot = [IO.Path]::GetFullPath((Join-Path $tempParent $tempName))
$script:assertions = 0

function Assert-True([bool]$Condition, [string]$Message) {
    $script:assertions++
    if (-not $Condition) { throw "[TEST NG] $Message" }
}

function Invoke-Ps1([string]$ScriptPath, [string[]]$Arguments) {
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        $output = @(& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $ScriptPath @Arguments 2>&1)
        return [pscustomobject]@{ ExitCode = $LASTEXITCODE; Text = ($output -join "`n") }
    }
    finally { $ErrorActionPreference = $previous }
}

function Invoke-Sut([string[]]$Arguments) {
    return Invoke-Ps1 $sut $Arguments
}

function Assert-Exit($Result, [int]$Expected, [string]$Name, [string]$Diagnostic = "") {
    Assert-True ($Result.ExitCode -eq $Expected) "$Name exit expected=$Expected actual=$($Result.ExitCode)`n$($Result.Text)"
    if (-not [string]::IsNullOrWhiteSpace($Diagnostic)) {
        Assert-True ($Result.Text -match [regex]::Escape($Diagnostic)) "$Name diagnostic '$Diagnostic' missing`n$($Result.Text)"
    }
}

function Flip-MiddleByte([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    $offset = [Math]::Floor($bytes.Length / 2)
    $bytes[$offset] = $bytes[$offset] -bxor 1
    [IO.File]::WriteAllBytes($Path, $bytes)
}

function Restore-Artifacts([hashtable]$Baseline) {
    foreach ($name in $Baseline.Keys) {
        [IO.File]::WriteAllBytes((Join-Path $tempRoot $name), [byte[]]$Baseline[$name])
    }
}

function Test-IsReparsePoint([IO.FileSystemInfo]$Item) {
    return (($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq [IO.FileAttributes]::ReparsePoint)
}

# junction(reparse point)自身だけを外す。中身は辿らない。
# Windows PowerShell 5.1 の `Remove-Item` は、中身のあるjunctionに対して
# -Recurse 無しだと NullReferenceException を投げる(この作業機で実測:
# 5.1.26100.9168、FullyQualifiedErrorId=System.NullReferenceException,
# Microsoft.PowerShell.Commands.RemoveItemCommand)。
# `[IO.Directory]::Delete` は1引数版が非再帰で、reparse pointを辿らないため、
# junction先(本体のdocs/.git/apps/crates/.github)には触れずに外せる。
function Remove-JunctionLink([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) { return }
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if ($null -eq $item) { return }
    if (-not (Test-IsReparsePoint $item)) {
        throw "expected a junction, found a real directory: $Path"
    }
    [IO.Directory]::Delete($Path)
}

# reparse pointを一切辿らずに木を消す。`Remove-Item -Recurse` を使わないのは、
# junctionを辿って本体を消す版が報告されているためで、この関数は再解析点に
# 出会ったら中身へ入らずreparse point自身だけを外す。
function Remove-TreeWithoutFollowingLinks([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) { return }
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if ($null -eq $item) { return }
    if (Test-IsReparsePoint $item) {
        [IO.Directory]::Delete($Path)
        return
    }
    if (($item.Attributes -band [IO.FileAttributes]::Directory) -ne [IO.FileAttributes]::Directory) {
        [IO.File]::SetAttributes($Path, [IO.FileAttributes]::Normal)
        [IO.File]::Delete($Path)
        return
    }
    foreach ($childPath in [IO.Directory]::GetFileSystemEntries($Path)) {
        Remove-TreeWithoutFollowingLinks $childPath
    }
    [IO.Directory]::Delete($Path)
}

function Remove-TestRoot {
    if (-not (Test-Path -LiteralPath $tempRoot)) { return }
    $resolved = [IO.Path]::GetFullPath($tempRoot).TrimEnd([char[]]"\/")
    if ([IO.Path]::GetDirectoryName($resolved) -ne $tempParent -or
        [IO.Path]::GetFileName($resolved) -notmatch '^ori3-doc-link-audit-test-[0-9a-f]{32}$') {
        throw "unsafe self-test cleanup refused: $resolved"
    }
    Remove-TreeWithoutFollowingLinks $resolved
}

[void][IO.Directory]::CreateDirectory($tempRoot)
try {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $global:LASTEXITCODE = 0
    $snapshotOutput = @(& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $snapshotSut -Format Json 2>&1)
    $snapshotExit = $LASTEXITCODE
    $ErrorActionPreference = $previous
    Assert-True ($snapshotExit -eq 0 -and $snapshotOutput.Count -eq 1) "production snapshotを取得できません"
    $snapshot = [string]$snapshotOutput[0] | ConvertFrom-Json
    $sutSource = [IO.File]::ReadAllText($sut, [Text.Encoding]::UTF8)
    Assert-True (-not $sutSource.Contains('scratchpad/doc-link-testnames.txt')) "production監査が未追跡scratchpad入力へ依存しています"
    Assert-True ($sutSource.Contains('docs/traceability/roadmap-evidence-test-names.txt')) "production監査が追跡対象の検査名台帳を参照していません"

    $write = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot)
    Assert-Exit $write 0 "production generation"
    Assert-True (([regex]::Matches($write.Text, '\[FRESH\]')).Count -eq 3) "write後の3成果物再読込がありません`n$($write.Text)"
    Assert-True ($write.Text -match "roadmap_accounted=$($snapshot.audited)/$($snapshot.total) checked=$($snapshot.checked) unchecked=$($snapshot.unchecked)") "全件の会計表示がありません"
    Assert-True ($write.Text -match "traceability_linked=$($snapshot.evidence_linked)/$($snapshot.total) explicit_outside=$($snapshot.explicit_outside) unclassified=0") "link+対象外の会計表示がありません"
    $generatedLedger = [IO.File]::ReadAllText((Join-Path $tempRoot "roadmap-links.json"), [Text.Encoding]::UTF8) | ConvertFrom-Json
    Assert-True ([int]$generatedLedger.traceability_accounting.linked_checkbox_count -eq 186 -and [int]$generatedLedger.traceability_accounting.explicit_outside_count -eq 1) "追加目標昇格後の186+1会計が生成台帳にありません"
    $outsideIds = @($generatedLedger.explicit_outside | ForEach-Object { [string]$_.id })
    Assert-True ($outsideIds.Count -eq 1 -and $outsideIds[0] -eq 'ADDITIONAL.FOLD-IO.C01') "FOLD-IO以外を明示対象外へ残しています: $($outsideIds -join ', ')"
    $foldAllRecords = @($generatedLedger.records | Where-Object { $_.id -eq 'ADDITIONAL.FOLD-ALL.C01' })
    Assert-True ($foldAllRecords.Count -eq 1 -and [string]$foldAllRecords[0].scope -eq 'ADDITIONAL' -and [string]$foldAllRecords[0].checkbox_state -eq 'checked' -and [string]$foldAllRecords[0].evidence_id -eq 'TEST.ADDITIONAL.FOLD-ALL.C01' -and [string]$foldAllRecords[0].test_mapping_origin -eq 'link-id') "fold-all追加目標が完了済みの明示link-ID自動証拠として生成されません"
    $expectedFoldAllTests = @(
        'fold_all::tests::targets_use_mountain_positive_valley_negative_and_skip_non_hinges'
        'fold_all::tests::invalid_inputs_are_errors_but_a_calculated_pose_has_no_layer_order'
        'src/components/FoldAllPreview.dom.test.tsx > 全部いっぺんに折ってみる画面 > 既存パネルの入口から0〜100%のつまみと記録でない約束を常時表示する'
        'src/components/FoldAllPreview.dom.test.tsx > 全部いっぺんに折ってみる画面 > 0・25・50・75・100%と計算待ちの間ずっと仮の形で手順ではないと示す'
        'src/store/foldAllPreview.test.ts > 全部の折り目をいっぺんに動かす一時表示 > 保存しても一斉形や手順を記録せず、専用表示を続ける'
        'src/store/foldAllPreview.test.ts > 全部の折り目をいっぺんに動かす一時表示 > Undoは作品履歴を進めず、通常表示へ戻るだけ'
        'src/store/foldAllPreview.test.ts > 全部の折り目をいっぺんに動かす一時表示 > Redoは作品履歴を進めず、通常表示へ戻るだけ'
        'commands::tests::pose_commands_match_the_cross_runtime_diagonal_fixtures'
        'src/store/foldAllPreview.savedFile.test.ts > 一斉表示中の73%を実ファイルへ保存せず、新しいbackendで開き直しても手順・履歴に現れない'
    )
    Assert-True ((@($foldAllRecords[0].test_names) -join "`n") -ceq ($expectedFoldAllTests -join "`n")) "fold-allの9検査割当又は順序が一致しません"
    Assert-True (@($generatedLedger.records | Where-Object { $_.id -eq 'ADDITIONAL.FOLD-IO.C01' }).Count -eq 0) "FOLD-IOを証拠recordへ取り込んでいます"
    $inventoryCount = [int]$generatedLedger.test_name_inventory.audited
    $mappedCount = [int]$generatedLedger.test_name_inventory.mapped
    $sourceFileCount = [int]$generatedLedger.test_name_inventory.source_files
    $activeExecutionCount = [int]$generatedLedger.test_name_inventory.execution_modes.active_default
    $ignoredExecutionCount = [int]$generatedLedger.test_name_inventory.execution_modes.ignored_explicit
    Assert-True ($write.Text -match "test_inventory_scope=roadmap-mapped test_inventory_audited=$inventoryCount/$mappedCount test_source_files=$sourceFileCount/$sourceFileCount test_definition_tree_sha256=[0-9a-f]{64} test_execution_active=$activeExecutionCount test_execution_ignored_explicit=$ignoredExecutionCount repository_test_total=not-claimed test_inventory_sha256=[0-9a-f]{64}") "対象限定の検査名台帳件数/source/実行モード/hash表示がありません"
    Assert-True ($inventoryCount -gt 0 -and $inventoryCount -eq $mappedCount -and $sourceFileCount -gt 0 -and $activeExecutionCount + $ignoredExecutionCount -eq $inventoryCount -and $ignoredExecutionCount -gt 0 -and [string]$generatedLedger.test_name_inventory.definition_tree_sha256 -match '^[0-9a-f]{64}$' -and [string]$generatedLedger.test_name_inventory.repository_test_total -eq '') "生成台帳が検査名全割当・execution mode・definition tree hash・全体非主張を保持していません"
    $uncheckedWithCompletedTaskCount = @($generatedLedger.records | Where-Object { $_.progress_state -eq 'unchecked-but-progress-task-exists' }).Count
    Assert-True ($write.Text -match "unchecked_with_completed_task=$uncheckedWithCompletedTaskCount") "完了進捗がある未チェック項目の件数表示がありません"
    Assert-True (-not $write.Text.Contains('regressed_to_unstarted=')) "状態差を印の後退と誤認させる旧表示が残っています"

    $names = @("roadmap-links.json", "roadmap-links.md", "manual-acceptance.md")
    $baseline = @{}
    foreach ($name in $names) { $baseline[$name] = [IO.File]::ReadAllBytes((Join-Path $tempRoot $name)) }

    $fresh = Invoke-Sut @("-CheckTraceability", "-TraceabilityPath", $tempRoot)
    Assert-Exit $fresh 0 "fresh artifacts"
    Assert-True (([regex]::Matches($fresh.Text, '\[FRESH\]')).Count -eq 3) "fresh checkが3成果物を対象にしていません"

    foreach ($name in $names) {
        Restore-Artifacts $baseline
        Flip-MiddleByte (Join-Path $tempRoot $name)
        $mutated = Invoke-Sut @("-CheckTraceability", "-TraceabilityPath", $tempRoot)
        Assert-Exit $mutated 2 "mutated $name" "[STALE] $name bytes不一致"
        Assert-True ($mutated.Text -match 'offset=\d+ .*expected_sha256=[0-9a-f]{64} actual_sha256=[0-9a-f]{64}') "$name のoffset/hash診断がありません`n$($mutated.Text)"
    }

    Restore-Artifacts $baseline
    $missingPath = Join-Path $tempRoot "roadmap-links.md"
    Remove-Item -LiteralPath $missingPath -Force
    $missing = Invoke-Sut @("-CheckTraceability", "-TraceabilityPath", $tempRoot)
    Assert-Exit $missing 2 "missing artifact" "[STALE] roadmap-links.md がありません"

    Restore-Artifacts $baseline
    foreach ($name in $names) { Flip-MiddleByte (Join-Path $tempRoot $name) }
    $sameOldGeneration = Invoke-Sut @("-CheckTraceability", "-TraceabilityPath", $tempRoot)
    Assert-Exit $sameOldGeneration 2 "three synchronized stale artifacts" "TRACEABILITY-STALE"
    Assert-True (([regex]::Matches($sameOldGeneration.Text, '\[STALE\] (?:roadmap-links\.json|roadmap-links\.md|manual-acceptance\.md) bytes不一致')).Count -eq 3) "3成果物を同時に古くしても全3件を検出していません"

    Restore-Artifacts $baseline
    $missingInventory = Join-Path $tempRoot "missing-test-inventory.txt"
    $missingInventoryResult = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestNamesInputPath", $missingInventory)
    Assert-Exit $missingInventoryResult 1 "missing clean-checkout inventory" "必須入力がありません"

    $invalidInventory = Join-Path $tempRoot "invalid-test-inventory.txt"
    $inventoryText = [IO.File]::ReadAllText($productionInventory, [Text.Encoding]::UTF8)
    $inventoryText = $inventoryText.Replace('autosave::tests::restore_recovers_the_same_document', 'autosave::tests::autosave_keeps_path_and_dirty_flag')
    [IO.File]::WriteAllText($invalidInventory, $inventoryText, (New-Object Text.UTF8Encoding($false)))
    $invalidInventoryResult = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestNamesInputPath", $invalidInventory)
    Assert-Exit $invalidInventoryResult 1 "inventory mapping drift" "検査名台帳"

    Restore-Artifacts $baseline
    $sourceFixtureRoot = Join-Path $tempRoot "source-root"
    $inventoryDataLines = @([IO.File]::ReadAllLines($productionInventory, [Text.Encoding]::UTF8) | Where-Object { -not $_.StartsWith('#') -and $_ -match ' \| ' })
    $fixtureSourcePaths = @($inventoryDataLines | ForEach-Object { [regex]::Match($_, ' \| (?<path>(?:apps|crates)/[A-Za-z0-9_.\-/]+)(?: \| (?:active-default|ignored-explicit))?$').Groups['path'].Value } | Sort-Object -Unique)
    foreach ($relativeSourcePath in $fixtureSourcePaths) {
        $destination = Join-Path $sourceFixtureRoot $relativeSourcePath
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
        [IO.File]::WriteAllBytes($destination, [IO.File]::ReadAllBytes((Join-Path $repoRoot $relativeSourcePath)))
    }
    $sourceFixtureBaseline = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot)
    Assert-Exit $sourceFixtureBaseline 0 "test source fixture baseline"

    $executionContractFixtureRoot = Join-Path $tempRoot "execution-contract-root"
    foreach ($relativeContractPath in @('.github/workflows/ci.yml', 'scripts/check-ci.ps1', 'docs/rules/03-品質ゲート.md')) {
        $contractDestination = Join-Path $executionContractFixtureRoot $relativeContractPath
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($contractDestination))
        [IO.File]::WriteAllBytes($contractDestination, [IO.File]::ReadAllBytes((Join-Path $repoRoot $relativeContractPath)))
    }
    $executionContractBaseline = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot, "-ExecutionContractRoot", $executionContractFixtureRoot)
    Assert-Exit $executionContractBaseline 0 "ignored test execution contract fixture baseline"
    $workflowFixture = Join-Path $executionContractFixtureRoot '.github/workflows/ci.yml'
    $workflowText = [IO.File]::ReadAllText($workflowFixture, [Text.Encoding]::UTF8)
    $ignoredCommand = 'cargo test --release -p ori3-propose --test perf_packing -- --ignored --nocapture'
    $brokenWorkflowText = $workflowText.Replace($ignoredCommand, $ignoredCommand + '-removed')
    Assert-True (-not [string]::Equals($workflowText, $brokenWorkflowText, [StringComparison]::Ordinal)) "ignored testのCI実行削除fixtureがproduction workflowを変異できません"
    [IO.File]::WriteAllText($workflowFixture, $brokenWorkflowText, (New-Object Text.UTF8Encoding($false)))
    $ignoredExecutionRemoved = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot, "-ExecutionContractRoot", $executionContractFixtureRoot)
    Assert-Exit $ignoredExecutionRemoved 1 "ignored test CI execution removed" "ignored testの明示実行がCI/check-ci/品質規約の3経路で一意ではありません"
    $autosaveFixture = Join-Path $sourceFixtureRoot "apps/desktop/src-tauri/src/autosave.rs"
    $autosaveText = [IO.File]::ReadAllText($autosaveFixture, [Text.Encoding]::UTF8)
    $mutatedAutosaveText = $autosaveText.Replace('fn restore_recovers_the_same_document()', 'fn restore_recovers_the_same_document_removed()')
    Assert-True (-not [string]::Equals($autosaveText, $mutatedAutosaveText, [StringComparison]::Ordinal)) "source test削除fixtureがproduction sourceを変異できません"
    $autosaveText = $mutatedAutosaveText
    [IO.File]::WriteAllText($autosaveFixture, $autosaveText, (New-Object Text.UTF8Encoding($false)))
    $sourceDefinitionRemoved = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot)
    Assert-Exit $sourceDefinitionRemoved 1 "mapped source test removed" "Rust test定義がsourceにありません"

    [IO.File]::WriteAllBytes($autosaveFixture, [IO.File]::ReadAllBytes((Join-Path $repoRoot "apps/desktop/src-tauri/src/autosave.rs")))
    $autosaveText = [IO.File]::ReadAllText($autosaveFixture, [Text.Encoding]::UTF8)
    $ignoredAutosaveText = [regex]::Replace(
        $autosaveText,
        '(?m)^(?<indent>\s*)#\[test\](?<newline>\r?\n)(?<fnindent>\s*)fn restore_recovers_the_same_document\(\)',
        '${indent}#[test]${newline}${indent}#[ignore]${newline}${fnindent}fn restore_recovers_the_same_document()',
        1
    )
    Assert-True (-not [string]::Equals($autosaveText, $ignoredAutosaveText, [StringComparison]::Ordinal)) "Rust ignore fixtureが対象test属性を変異できません"
    [IO.File]::WriteAllText($autosaveFixture, $ignoredAutosaveText, (New-Object Text.UTF8Encoding($false)))
    $ignoredRustTest = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot)
    Assert-Exit $ignoredRustTest 1 "mapped Rust test ignored" "Rust test属性がactiveではありません"

    [IO.File]::WriteAllBytes($autosaveFixture, [IO.File]::ReadAllBytes((Join-Path $repoRoot "apps/desktop/src-tauri/src/autosave.rs")))
    $autosaveText = [IO.File]::ReadAllText($autosaveFixture, [Text.Encoding]::UTF8)
    $unmarkedAutosaveText = [regex]::Replace(
        $autosaveText,
        '(?m)^\s*#\[test\]\r?\n(?=\s*fn restore_recovers_the_same_document\(\))',
        '',
        1
    )
    Assert-True (-not [string]::Equals($autosaveText, $unmarkedAutosaveText, [StringComparison]::Ordinal)) "Rust test属性削除fixtureが対象を変異できません"
    [IO.File]::WriteAllText($autosaveFixture, $unmarkedAutosaveText, (New-Object Text.UTF8Encoding($false)))
    $unmarkedRustTest = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot)
    Assert-Exit $unmarkedRustTest 1 "mapped Rust test attribute removed with nearby tests" "Rust test属性がありません"

    [IO.File]::WriteAllBytes($autosaveFixture, [IO.File]::ReadAllBytes((Join-Path $repoRoot "apps/desktop/src-tauri/src/autosave.rs")))
    $autosaveText = [IO.File]::ReadAllText($autosaveFixture, [Text.Encoding]::UTF8)
    $disabledAutosaveText = [regex]::Replace(
        $autosaveText,
        '(?m)^(?<indent>\s*)#\[test\](?<newline>\r?\n)(?<fnindent>\s*)fn restore_recovers_the_same_document\(\)',
        '${indent}#[cfg(any())]${newline}${indent}// cfg-padding-01${newline}${indent}// cfg-padding-02${newline}${indent}// cfg-padding-03${newline}${indent}// cfg-padding-04${newline}${indent}// cfg-padding-05${newline}${indent}// cfg-padding-06${newline}${indent}// cfg-padding-07${newline}${indent}// cfg-padding-08${newline}${indent}// cfg-padding-09${newline}${indent}// cfg-padding-10${newline}${indent}// cfg-padding-11${newline}${indent}// cfg-padding-12${newline}${indent}// cfg-padding-13${newline}${indent}#[test]${newline}${fnindent}fn restore_recovers_the_same_document()',
        1
    )
    Assert-True (-not [string]::Equals($autosaveText, $disabledAutosaveText, [StringComparison]::Ordinal)) "Rust cfg fixtureが対象testを変異できません"
    [IO.File]::WriteAllText($autosaveFixture, $disabledAutosaveText, (New-Object Text.UTF8Encoding($false)))
    $disabledRustTest = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot)
    Assert-Exit $disabledRustTest 1 "mapped Rust test disabled by cfg" "Rust testに条件付き属性があります"

    [IO.File]::WriteAllBytes($autosaveFixture, [IO.File]::ReadAllBytes((Join-Path $repoRoot "apps/desktop/src-tauri/src/autosave.rs")))
    $autosaveText = [IO.File]::ReadAllText($autosaveFixture, [Text.Encoding]::UTF8)
    $vacuousAutosaveText = $autosaveText.Replace('assert!(s.is_dirty(), "まだ保存していない内容なので未保存扱い");', 'assert!(true, "証拠を消した偽検査");')
    Assert-True (-not [string]::Equals($autosaveText, $vacuousAutosaveText, [StringComparison]::Ordinal)) "Rust test本体fixtureが対象assertを変異できません"
    [IO.File]::WriteAllText($autosaveFixture, $vacuousAutosaveText, (New-Object Text.UTF8Encoding($false)))
    $vacuousRustTest = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot)
    Assert-Exit $vacuousRustTest 1 "mapped Rust test body made vacuous" "test definition hashが現在定義と不一致"

    [IO.File]::WriteAllBytes($autosaveFixture, [IO.File]::ReadAllBytes((Join-Path $repoRoot "apps/desktop/src-tauri/src/autosave.rs")))
    $autosaveText = [IO.File]::ReadAllText($autosaveFixture, [Text.Encoding]::UTF8)
    $braceInStringText = $autosaveText.Replace(
        'fn restore_recovers_the_same_document() {',
        "fn restore_recovers_the_same_document() {`r`n        let _ = `"}`"; // naive brace scanner must not define the trust boundary"
    )
    Assert-True (-not [string]::Equals($autosaveText, $braceInStringText, [StringComparison]::Ordinal)) "Rust文字列brace fixtureがproduction sourceを変異できません"
    [IO.File]::WriteAllText($autosaveFixture, $braceInStringText, (New-Object Text.UTF8Encoding($false)))
    $braceInventory = Join-Path $tempRoot "brace-test-inventory.txt"
    [IO.File]::WriteAllBytes($braceInventory, [IO.File]::ReadAllBytes($productionInventory))
    $braceInventoryProbe = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot, "-TestNamesInputPath", $braceInventory)
    Assert-Exit $braceInventoryProbe 1 "Rust string brace fixture needs a matching definition baseline" "test definition hashが現在定義と不一致"
    $braceHashMatch = [regex]::Match($braceInventoryProbe.Text, 'actual=(?<sha>[0-9a-f]{64})')
    Assert-True $braceHashMatch.Success "Rust文字列brace fixtureのdefinition hashを取得できません`n$($braceInventoryProbe.Text)"
    $braceInventoryText = [IO.File]::ReadAllText($braceInventory, [Text.Encoding]::UTF8)
    $braceInventoryText = [regex]::Replace(
        $braceInventoryText,
        '(?m)^# definition-tree-sha256=[0-9a-f]{64}$',
        '# definition-tree-sha256=' + $braceHashMatch.Groups['sha'].Value,
        1
    )
    [IO.File]::WriteAllText($braceInventory, $braceInventoryText, (New-Object Text.UTF8Encoding($false)))
    $braceBaseline = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot, "-TestNamesInputPath", $braceInventory)
    Assert-Exit $braceBaseline 0 "Rust string brace fixture baseline"
    $vacuousAfterBraceText = $braceInStringText.Replace(
        'assert!(s.is_dirty(), "まだ保存していない内容なので未保存扱い");',
        'assert!(true, "braceより後ろの証拠を消した偽検査");'
    )
    Assert-True (-not [string]::Equals($braceInStringText, $vacuousAfterBraceText, [StringComparison]::Ordinal)) "Rust文字列brace後方assert fixtureが対象assertを変異できません"
    [IO.File]::WriteAllText($autosaveFixture, $vacuousAfterBraceText, (New-Object Text.UTF8Encoding($false)))
    $vacuousAfterBrace = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot, "-TestNamesInputPath", $braceInventory)
    Assert-Exit $vacuousAfterBrace 1 "mapped Rust assertion after string brace made vacuous" "test definition hashが現在定義と不一致"

    [IO.File]::WriteAllBytes($autosaveFixture, [IO.File]::ReadAllBytes((Join-Path $repoRoot "apps/desktop/src-tauri/src/autosave.rs")))
    $alignPickFixture = Join-Path $sourceFixtureRoot "apps/desktop/src/lib/alignPick.test.ts"
    $alignPickText = [IO.File]::ReadAllText($alignPickFixture, [Text.Encoding]::UTF8)
    $skippedAlignPickText = $alignPickText.Replace('it("十字に交わる2本の交点を返す",', 'it.skip("十字に交わる2本の交点を返す",')
    Assert-True (-not [string]::Equals($alignPickText, $skippedAlignPickText, [StringComparison]::Ordinal)) "Vitest skip fixtureが対象test宣言を変異できません"
    [IO.File]::WriteAllText($alignPickFixture, $skippedAlignPickText, (New-Object Text.UTF8Encoding($false)))
    $skippedScreenTest = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot)
    Assert-Exit $skippedScreenTest 1 "mapped screen test skipped" "画面test宣言がsourceにありません"

    [IO.File]::WriteAllBytes($alignPickFixture, [IO.File]::ReadAllBytes((Join-Path $repoRoot "apps/desktop/src/lib/alignPick.test.ts")))
    $alignPickText = [IO.File]::ReadAllText($alignPickFixture, [Text.Encoding]::UTF8)
    $vacuousAlignPickText = $alignPickText.Replace('expect(x).toEqual([0, 0]);', 'expect(true).toBe(true);')
    Assert-True (-not [string]::Equals($alignPickText, $vacuousAlignPickText, [StringComparison]::Ordinal)) "画面test本体fixtureが対象expectを変異できません"
    [IO.File]::WriteAllText($alignPickFixture, $vacuousAlignPickText, (New-Object Text.UTF8Encoding($false)))
    $vacuousScreenTest = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot)
    Assert-Exit $vacuousScreenTest 1 "mapped screen test body made vacuous" "test definition hashが現在定義と不一致"

    [IO.File]::WriteAllBytes($alignPickFixture, [IO.File]::ReadAllBytes((Join-Path $repoRoot "apps/desktop/src/lib/alignPick.test.ts")))
    $alignPickText = [IO.File]::ReadAllText($alignPickFixture, [Text.Encoding]::UTF8)
    $unregisteredAlignPickText = $alignPickText.Replace(
        'describe("線分の交点", () => {',
        "describe(`"線分の交点`", () => {`r`n  return; // 対象testを登録しない"
    )
    Assert-True (-not [string]::Equals($alignPickText, $unregisteredAlignPickText, [StringComparison]::Ordinal)) "画面test未登録fixtureがsuiteを変異できません"
    [IO.File]::WriteAllText($alignPickFixture, $unregisteredAlignPickText, (New-Object Text.UTF8Encoding($false)))
    $unregisteredScreenTest = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot)
    Assert-Exit $unregisteredScreenTest 1 "mapped screen test not registered by suite return" "test definition hashが現在定義と不一致"

    [IO.File]::WriteAllBytes($alignPickFixture, [IO.File]::ReadAllBytes((Join-Path $repoRoot "apps/desktop/src/lib/alignPick.test.ts")))
    $alignPickText = [IO.File]::ReadAllText($alignPickFixture, [Text.Encoding]::UTF8)
    $movedAlignPickText = $alignPickText.Replace(
        '  it("十字に交わる2本の交点を返す", () => {',
        "});`r`n`r`ndescribe(`"別suite`", () => {`r`n  it(`"十字に交わる2本の交点を返す`", () => {"
    )
    Assert-True (-not [string]::Equals($alignPickText, $movedAlignPickText, [StringComparison]::Ordinal)) "画面test suite外移動fixtureが対象を変異できません"
    [IO.File]::WriteAllText($alignPickFixture, $movedAlignPickText, (New-Object Text.UTF8Encoding($false)))
    $movedScreenTest = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-TestSourceRoot", $sourceFixtureRoot)
    Assert-Exit $movedScreenTest 1 "mapped screen test moved outside named suite" "画面testが指定suite内にありません"

    Restore-Artifacts $baseline
    $roadmapFixture = Join-Path $tempRoot "implementation-roadmap.md"
    [IO.File]::WriteAllBytes($roadmapFixture, [IO.File]::ReadAllBytes($productionRoadmap))
    $fixtureBaseline = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-RoadmapInputPath", $roadmapFixture)
    Assert-Exit $fixtureBaseline 0 "roadmap source fixture baseline"
    $roadmapFixtureText = [IO.File]::ReadAllText($roadmapFixture, [Text.Encoding]::UTF8)
    $renamedFoldAllHeading = $roadmapFixtureText.Replace(
        '### 全部の折り目を一斉に折る一時表示',
        '### 全部の折り目を一斉に折る一時表示（未承認の別見出し）'
    )
    Assert-True (-not [string]::Equals($roadmapFixtureText, $renamedFoldAllHeading, [StringComparison]::Ordinal)) "fold-all専用見出しの負例を作れません"
    [IO.File]::WriteAllText($roadmapFixture, $renamedFoldAllHeading, (New-Object Text.UTF8Encoding($false)))
    $renamedFoldAll = Invoke-Sut @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $tempRoot, "-RoadmapInputPath", $roadmapFixture)
    Assert-Exit $renamedFoldAll 1 "renamed fold-all additional heading" "証拠link checkbox数がsnapshotと不一致です"
    [IO.File]::WriteAllBytes($roadmapFixture, [IO.File]::ReadAllBytes($productionRoadmap))
    [IO.File]::AppendAllText($roadmapFixture, "`r`n<!-- roadmap-source-hash-negative-test -->`r`n", (New-Object Text.UTF8Encoding($false)))
    $sourceDrift = Invoke-Sut @("-CheckTraceability", "-TraceabilityPath", $tempRoot, "-RoadmapInputPath", $roadmapFixture)
    Assert-Exit $sourceDrift 2 "roadmap source changed with saved ledger" "roadmap snapshot hash不一致"
    Assert-True (([regex]::Matches($sourceDrift.Text, '\[STALE\] (?:roadmap-links\.json|roadmap-links\.md|manual-acceptance\.md) bytes不一致')).Count -eq 3) "roadmap sourceだけの変更で3成果物すべてを古いと判定していません`n$($sourceDrift.Text)"

    # 2026-09-05: 検査名台帳の見出し `# definition-tree-sha256=` を書き直す
    # 唯一の入口 -WriteTestNamesHash。値は -CheckTraceability が照合するのと
    # 同じ計算経路から出るので、人がhashを手で書かない。書込先は必ず
    # -TestNamesInputPath で渡した複製にし、追跡対象の台帳へは触らない。
    Restore-Artifacts $baseline
    $hashWriteInventory = Join-Path $tempRoot "write-hash-inventory.txt"
    $productionInventoryBytes = [IO.File]::ReadAllBytes($productionInventory)
    [IO.File]::WriteAllBytes($hashWriteInventory, $productionInventoryBytes)
    $staleHash = '0123456789abcdef' * 4
    $hashWriteText = [IO.File]::ReadAllText($hashWriteInventory, [Text.Encoding]::UTF8)
    $staleInventoryText = [regex]::Replace($hashWriteText, '(?m)^# definition-tree-sha256=[0-9a-f]{64}$', '# definition-tree-sha256=' + $staleHash, 1)
    Assert-True (-not [string]::Equals($hashWriteText, $staleInventoryText, [StringComparison]::Ordinal)) "台帳hashの古い値を作れません"
    [IO.File]::WriteAllText($hashWriteInventory, $staleInventoryText, (New-Object Text.UTF8Encoding($false)))
    $staleInventoryBytes = [IO.File]::ReadAllBytes($hashWriteInventory)

    # 負例: 古い見出しのままでは -CheckTraceability が止める(この入口が
    # 検査を弱めていないことの確認)。
    $staleInventoryCheck = Invoke-Sut @("-CheckTraceability", "-TraceabilityPath", $tempRoot, "-TestNamesInputPath", $hashWriteInventory)
    Assert-Exit $staleInventoryCheck 1 "stale definition hash still blocks -CheckTraceability" "test definition hashが現在定義と不一致"
    $expectedHashMatch = [regex]::Match($staleInventoryCheck.Text, 'actual=(?<sha>[0-9a-f]{64})')
    Assert-True $expectedHashMatch.Success "現在定義のdefinition hashを取得できません`n$($staleInventoryCheck.Text)"

    # 正例: 書き直すと exit=0 になり、見出しの64桁だけが現在の値へ変わる。
    $hashWrite = Invoke-Sut @("-WriteTestNamesHash", "-TestNamesInputPath", $hashWriteInventory)
    Assert-Exit $hashWrite 0 "definition hash write" "[WRITE]"
    $writtenInventoryBytes = [IO.File]::ReadAllBytes($hashWriteInventory)
    Assert-True ($writtenInventoryBytes.Length -eq $staleInventoryBytes.Length) "台帳のbyte長が変わりました"
    $changedByteCount = 0
    for ($byteIndex = 0; $byteIndex -lt $writtenInventoryBytes.Length; $byteIndex++) {
        if ($writtenInventoryBytes[$byteIndex] -ne $staleInventoryBytes[$byteIndex]) { $changedByteCount++ }
    }
    Assert-True ($changedByteCount -le 64) "見出しの64桁以外のbyteも変わりました (変化=$changedByteCount)"
    $writtenInventoryText = [IO.File]::ReadAllText($hashWriteInventory, [Text.Encoding]::UTF8)
    Assert-True ($writtenInventoryText -match ('(?m)^# definition-tree-sha256=' + $expectedHashMatch.Groups['sha'].Value + '$')) "現在定義のdefinition hashを書けていません"
    $freshInventoryCheck = Invoke-Sut @("-CheckTraceability", "-TraceabilityPath", $tempRoot, "-TestNamesInputPath", $hashWriteInventory)
    Assert-Exit $freshInventoryCheck 0 "definition hash after write passes -CheckTraceability"

    # 冪等: 一致しているときは書かずに [FRESH] を出して exit=0。
    $hashWriteAgain = Invoke-Sut @("-WriteTestNamesHash", "-TestNamesInputPath", $hashWriteInventory)
    Assert-Exit $hashWriteAgain 0 "definition hash write is idempotent" "[FRESH]"
    $unchangedInventoryBytes = [IO.File]::ReadAllBytes($hashWriteInventory)
    Assert-True ([Convert]::ToBase64String($unchangedInventoryBytes) -ceq [Convert]::ToBase64String($writtenInventoryBytes)) "一致しているのに台帳を書き換えました"

    # 負例: 他のmodeとの同時指定は拒否する(通常の検査・生成経路から台帳を
    # 書けないようにする。§10.7.6)。
    foreach ($conflictingMode in @("-CheckTraceability", "-WriteTraceability", "-Check", "-Update")) {
        $conflict = Invoke-Sut @("-WriteTestNamesHash", $conflictingMode, "-TestNamesInputPath", $hashWriteInventory, "-TraceabilityPath", $tempRoot)
        Assert-Exit $conflict 1 "definition hash write refuses $conflictingMode" "-WriteTestNamesHash は単独で実行してください"
    }
    # 通常の検査経路(governance・CI・check-ci)がこの書込みmodeを呼ばないことを
    # 実ファイルで固定する。
    foreach ($callerRelativePath in @('scripts/check-roadmap-governance.ps1', 'scripts/check-ci.ps1', '.github/workflows/ci.yml')) {
        $callerText = [IO.File]::ReadAllText((Join-Path $repoRoot $callerRelativePath), [Text.Encoding]::UTF8)
        Assert-True (-not $callerText.Contains('WriteTestNamesHash')) "$callerRelativePath が台帳書込みmodeを呼んでいます"
    }

    # 2026-09-05 負例: 片付けの部品がjunctionを辿らず、片付けが失敗しても
    # 本来の判定を隠さないこと。起きた失敗:
    # `Remove-Item -LiteralPath <junction> -Force` がWindows PowerShell 5.1で
    # NullReferenceExceptionを投げ、$ErrorActionPreference="Stop"のためfinallyが
    # 中断し、直後のmanual table負例(Assert-Exit以降)へ一度も到達しないままexit=1になった。
    $linkProbeRoot = Join-Path $tempRoot "junction-cleanup-probe"
    $linkProbeTarget = Join-Path $linkProbeRoot "target"
    $linkProbeTree = Join-Path $linkProbeRoot "tree"
    [void][IO.Directory]::CreateDirectory((Join-Path $linkProbeTarget "nested"))
    [IO.File]::WriteAllText((Join-Path $linkProbeTarget "canary.txt"), "ALIVE")
    [IO.File]::WriteAllText((Join-Path $linkProbeTarget "nested\canary.txt"), "ALIVE")
    [void][IO.Directory]::CreateDirectory($linkProbeTree)
    [IO.File]::WriteAllText((Join-Path $linkProbeTree "own.txt"), "own")
    [void](New-Item -ItemType Junction -Path (Join-Path $linkProbeTree "linked") -Target $linkProbeTarget)

    # Remove-JunctionLinkはjunctionだけを外し、junction先の中身を1つも消さない。
    Remove-JunctionLink (Join-Path $linkProbeTree "linked")
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $linkProbeTree "linked"))) "Remove-JunctionLinkがjunctionを外せません"
    Assert-True (Test-Path -LiteralPath (Join-Path $linkProbeTarget "canary.txt")) "Remove-JunctionLinkがjunction先の中身を消しました"
    Assert-True (Test-Path -LiteralPath (Join-Path $linkProbeTarget "nested\canary.txt")) "Remove-JunctionLinkがjunction先の入れ子まで消しました"

    # junctionを張ったまま木を消しても、junction先(本体に相当)へは入らない。
    [void](New-Item -ItemType Junction -Path (Join-Path $linkProbeTree "linked") -Target $linkProbeTarget)
    Remove-TreeWithoutFollowingLinks $linkProbeTree
    Assert-True (-not (Test-Path -LiteralPath $linkProbeTree)) "Remove-TreeWithoutFollowingLinksが木を消せません"
    Assert-True (Test-Path -LiteralPath (Join-Path $linkProbeTarget "canary.txt")) "木の削除がjunctionを辿って本体側を消しました"
    Assert-True (Test-Path -LiteralPath (Join-Path $linkProbeTarget "nested\canary.txt")) "木の削除がjunction先の入れ子まで消しました"

    # 実ディレクトリを渡したら消さずに拒否し、その失敗が後続の判定を止めないこと。
    $cleanupRefused = $false
    try { $null = $null }
    finally {
        try { Remove-JunctionLink $linkProbeTarget }
        catch { $cleanupRefused = $true }
    }
    $cleanupReachedAfterFailure = $true
    Assert-True $cleanupRefused "Remove-JunctionLinkが実ディレクトリを拒否しません"
    Assert-True (Test-Path -LiteralPath (Join-Path $linkProbeTarget "canary.txt")) "拒否したのに実ディレクトリを消しました"
    Assert-True $cleanupReachedAfterFailure "片付けが失敗すると後続の判定へ到達できません"

    # 片付けのfinally節が、投げる形(Remove-Item・catchなし)へ戻っていないことを実ファイルで固定する。
    $selfText = [IO.File]::ReadAllText($PSCommandPath, [Text.Encoding]::UTF8)
    $cleanupFinally = [regex]::Match($selfText, '(?ms)^    finally \{\r?\n        # junctionは対象の中身を持たない.*?^    \}')
    Assert-True $cleanupFinally.Success "manual table片付けのfinally節を自分の中から取り出せません"
    # コメント行は判定から除く。節の中の説明文が `Remove-Item` という語を含むため
    # (この不具合の経緯そのものを書いてある)、実行される行だけを見る。
    $cleanupFinallyCode = (@($cleanupFinally.Value -split "`r?`n" | Where-Object { -not $_.TrimStart().StartsWith('#') })) -join "`n"
    Assert-True ($cleanupFinallyCode.Contains('Remove-JunctionLink')) "片付けの実行行からRemove-JunctionLinkが消えています"
    Assert-True (-not $cleanupFinallyCode.Contains('Remove-Item')) "片付けのfinally節がRemove-Itemへ戻っています(junctionでNullReferenceException)"
    Assert-True (([regex]::Matches($cleanupFinallyCode, 'try \{ Remove-')).Count -eq 2) "片付けの2手順がtry/catchで包まれていません"
    Remove-TreeWithoutFollowingLinks $linkProbeRoot

    # 2026-09-05: 受入担当2名が手で書いた7件の手動受入記録が
    # $manuallyRecordedAcceptance の表へ取り込まれていることの負例。
    # 1件を表から外した複製で生成すると、その1件だけ定型文へ戻り、
    # 他の6件は手書き本文のまま変わらないことを確かめる
    # (§10.7.6再発防止: 記録が表に無いと再生成のたびに定型文へ戻る)。
    Restore-Artifacts $baseline
    $sutText = [IO.File]::ReadAllText($sut, [Text.Encoding]::UTF8)
    $removedManualId = "MANUAL.M2.T2-6b.C05.SCREEN-ACCEPTANCE"
    $manualTableEntryPattern = '(?ms)^    "' + [regex]::Escape($removedManualId) + '" = @\(.*?\r?\n    \)\r?\n'
    $mutatedSutText = [regex]::Replace($sutText, $manualTableEntryPattern, '', 1)
    Assert-True (-not [string]::Equals($sutText, $mutatedSutText, [StringComparison]::Ordinal)) "手動受入表から $removedManualId を複製で取り除けません"
    Assert-True (-not $mutatedSutText.Contains("`"$removedManualId`" = @(")) "複製に $removedManualId の表項目がまだ残っています"
    Assert-True ($mutatedSutText.Contains('"MANUAL.M2.T2-7.C03.SCREEN-ACCEPTANCE" = @(')) "他のIDの表項目まで複製で消えています"
    # 複製は repoRoot構造ごと必要とする(get-roadmap-status.ps1・
    # roadmap-status-policy.json・docs/progress.md・COMMIT-PUSH証拠のgit操作が
    # $PSScriptRootの兄弟と$repoRootでのgit -C解決へ依存するため)。scripts/だけを
    # 変異させた実体コピーにし、他は本体を指すjunctionにして安全に隔離する。
    # .ps1として実行するため、production同様にBOM付きUTF-8で書く(PowerShell 5.1は
    # BOMが無いソースをsystem codepageで読み、日本語文字列を壊すため)。
    $manualTableFakeRepoRoot = Join-Path $tempRoot "manual-table-mutant-repo"
    if (Test-Path -LiteralPath $manualTableFakeRepoRoot) { Remove-Item -LiteralPath $manualTableFakeRepoRoot -Recurse -Force }
    [void][IO.Directory]::CreateDirectory($manualTableFakeRepoRoot)
    $manualTableLinkedDirs = @("docs", ".git", "apps", "crates", ".github")
    try {
        foreach ($linkedDir in $manualTableLinkedDirs) {
            [void](New-Item -ItemType Junction -Path (Join-Path $manualTableFakeRepoRoot $linkedDir) -Target (Join-Path $repoRoot $linkedDir))
        }
        Copy-Item -LiteralPath (Join-Path $repoRoot "scripts") -Destination (Join-Path $manualTableFakeRepoRoot "scripts") -Recurse -Force
        $manualTableMutantScript = Join-Path $manualTableFakeRepoRoot "scripts\doc-link-audit.ps1"
        [IO.File]::WriteAllText($manualTableMutantScript, $mutatedSutText, (New-Object Text.UTF8Encoding($true)))
        $manualTableMutantOut = Join-Path $tempRoot "manual-table-mutant-out"
        [void][IO.Directory]::CreateDirectory($manualTableMutantOut)
        $manualTableMutantResult = Invoke-Ps1 $manualTableMutantScript @("-WriteTraceability", "-PreserveReport", "-TraceabilityPath", $manualTableMutantOut, "-RoadmapInputPath", $roadmapFixture, "-TestNamesInputPath", $hashWriteInventory)
    }
    finally {
        # junctionは対象の中身を持たないので、個別に(非再帰で)先に外してから
        # 複製したscripts/だけを削除する。親を先に再帰削除すると、古い
        # PowerShellではjunction先(本体のdocs/apps/crates/.git)まで辿って
        # 消しかねないため、この順序を守る。
        # 片付けの失敗で本来の判定(:Assert-Exit以降)を隠さない。2026-09-05に
        # `Remove-Item -LiteralPath <junction> -Force` がNullReferenceExceptionを投げ、
        # $ErrorActionPreference="Stop" のため finally が中断し、manual table負例の
        # 判定へ一度も到達しないまま exit=1 になった。片付けの例外は警告にする。
        if ($manualTableLinkedDirs -and -not [string]::IsNullOrWhiteSpace($manualTableFakeRepoRoot)) {
            foreach ($linkedDir in $manualTableLinkedDirs) {
                $linkedPath = Join-Path $manualTableFakeRepoRoot $linkedDir
                try { Remove-JunctionLink $linkedPath }
                catch { Write-Warning "[CLEANUP] junctionを外せません: $linkedPath : $($_.Exception.Message)" }
            }
            try { Remove-TreeWithoutFollowingLinks $manualTableFakeRepoRoot }
            catch { Write-Warning "[CLEANUP] 偽repoを消せません: $manualTableFakeRepoRoot : $($_.Exception.Message)" }
        }
    }
    Assert-Exit $manualTableMutantResult 0 "manual table mutant generation"
    $mutantManualText = [IO.File]::ReadAllText((Join-Path $manualTableMutantOut "manual-acceptance.md"), [Text.Encoding]::UTF8)
    Assert-True ($mutantManualText.Contains("## $removedManualId`r`n1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。")) "$removedManualId が表から外れても定型文へ戻っていません`n$mutantManualText"
    # 2026-09-05: 生成物の実体は backtick 1個（`apps/...`）である。ここを2個で書くと
    # 否定の表明が常に真になり、手書き本文が残っていても気づけない（空振り）。
    # 実体と同じ1個に直して、実際に検出できる表明にした。
    Assert-True (-not $mutantManualText.Contains('実行本体: `apps/desktop/tests-live/doc-link-b1-pull-cdp.mjs`')) "$removedManualId の手書き本文が複製にまだ残っています"
    # 同上。backtick 2個では生成物と一致せず、この肯定の表明は決して通らなかった。
    Assert-True ($mutantManualText.Contains('実行本体: `apps/desktop/tests-live/doc-link-b1-penetration-cdp.mjs`')) "他のID(M2.T2-7.C03)の手書き本文が巻き添えで消えています"
    Assert-True ($mutantManualText.Contains('骨格を指定して展開図を提案してもらう画面を追加')) "他のID(M3.T3-4.C04)の手書き本文が巻き添えで消えています"
    Assert-True ($mutantManualText.Contains('頂点141・辺280、手順14')) "他のID(M4.T4-6.C02)の手書き本文が巻き添えで消えています"

    Write-Host "[TEST OK] doc-link-audit: $script:assertions assertions"
    exit 0
}
finally {
    Remove-TestRoot
}
