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

function Invoke-Sut([string[]]$Arguments) {
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        $output = @(& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $sut @Arguments 2>&1)
        return [pscustomobject]@{ ExitCode = $LASTEXITCODE; Text = ($output -join "`n") }
    }
    finally { $ErrorActionPreference = $previous }
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

function Remove-TestRoot {
    if (-not (Test-Path -LiteralPath $tempRoot)) { return }
    $resolved = [IO.Path]::GetFullPath($tempRoot).TrimEnd([char[]]"\/")
    if ([IO.Path]::GetDirectoryName($resolved) -ne $tempParent -or
        [IO.Path]::GetFileName($resolved) -notmatch '^ori3-doc-link-audit-test-[0-9a-f]{32}$') {
        throw "unsafe self-test cleanup refused: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
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
    [IO.File]::AppendAllText($roadmapFixture, "`r`n<!-- roadmap-source-hash-negative-test -->`r`n", (New-Object Text.UTF8Encoding($false)))
    $sourceDrift = Invoke-Sut @("-CheckTraceability", "-TraceabilityPath", $tempRoot, "-RoadmapInputPath", $roadmapFixture)
    Assert-Exit $sourceDrift 2 "roadmap source changed with saved ledger" "roadmap snapshot hash不一致"
    Assert-True (([regex]::Matches($sourceDrift.Text, '\[STALE\] (?:roadmap-links\.json|roadmap-links\.md|manual-acceptance\.md) bytes不一致')).Count -eq 3) "roadmap sourceだけの変更で3成果物すべてを古いと判定していません`n$($sourceDrift.Text)"

    Write-Host "[TEST OK] doc-link-audit: $script:assertions assertions"
    exit 0
}
finally {
    Remove-TestRoot
}
