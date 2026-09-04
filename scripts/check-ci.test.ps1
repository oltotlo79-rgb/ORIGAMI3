[CmdletBinding()]
param(
    [string]$CheckerPath
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
$script:AssertionCount = 0
$script:Utf8NoBom = New-Object Text.UTF8Encoding($false)

if ([string]::IsNullOrWhiteSpace($CheckerPath)) {
    $CheckerPath = Join-Path $PSScriptRoot "check-ci.ps1"
}
$CheckerPath = [IO.Path]::GetFullPath($CheckerPath)
$RepositoryRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$SandboxName = "ori3-check-ci-contract-test-{0}" -f [Guid]::NewGuid().ToString("N")
$SandboxRoot = [IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetTempPath()) $SandboxName))
$FixturePaths = @(
    "docs/rules/03-品質ゲート.md",
    "docs/rules/05-リリース.md",
    "docs/traceability/roadmap-evidence-test-names.txt",
    "scripts/check.ps1",
    "scripts/check-receipt.ps1",
    "scripts/check-release-ready.ps1",
    "scripts/check-roadmap-governance.ps1",
    "scripts/doc-link-audit.ps1",
    "scripts/hooks/pre-commit",
    ".github/workflows/ci.yml",
    "apps/desktop/src-tauri/src/surface_order_sa_endpoint_heavy.rs",
    "apps/desktop/src-tauri/src/surface_order_acceptance.rs"
)
$RepositorySourceHashes = @{}
foreach ($relativePath in $FixturePaths) {
    $RepositorySourceHashes[$relativePath] = (Get-FileHash -LiteralPath (Join-Path $RepositoryRoot $relativePath) -Algorithm SHA256).Hash
}

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $script:AssertionCount += 1
    if (-not $Condition) {
        throw "ASSERTION FAILED: $Message"
    }
}

function New-CaseFixture {
    param([Parameter(Mandatory = $true)][string]$Name)

    $caseRoot = [IO.Path]::GetFullPath((Join-Path $SandboxRoot $Name))
    foreach ($relativePath in $FixturePaths) {
        $source = Join-Path $RepositoryRoot $relativePath
        $target = Join-Path $caseRoot $relativePath
        [void][IO.Directory]::CreateDirectory((Split-Path -Parent $target))
        [IO.File]::Copy($source, $target, $true)
    }
    return $caseRoot
}

function Set-ExactReplacement {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Before,
        [Parameter(Mandatory = $true)][string]$After
    )

    $text = [IO.File]::ReadAllText($Path, $script:Utf8NoBom)
    $count = [regex]::Matches($text, [regex]::Escape($Before)).Count
    Assert-True ($count -eq 1) "故障注入前の文字列は厳密に1件であること: $Path (count=$count)"
    [IO.File]::WriteAllText($Path, $text.Replace($Before, $After), $script:Utf8NoBom)
}

function Invoke-IsolatedChecker {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$PowerShellPath
    )

    $global:LASTEXITCODE = 0
    $output = @(& $PowerShellPath -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $CheckerPath -StaticContractOnly -StaticContractRoot $Root 2>&1)
    return [pscustomobject]@{
        ExitCode = $LASTEXITCODE
        Output = (($output | ForEach-Object { [string]$_ }) -join "`n")
    }
}

function New-GovernanceProductionFixture {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [string]$FailingScript = "",
        [switch]$UseActualCiContractTest
    )

    $fixtureRoot = [IO.Path]::GetFullPath((Join-Path $SandboxRoot "production-$Name"))
    $scriptsRoot = Join-Path $fixtureRoot "scripts"
    [void][IO.Directory]::CreateDirectory($scriptsRoot)
    $governancePath = Join-Path $scriptsRoot "check-roadmap-governance.ps1"
    if ($UseActualCiContractTest) {
        foreach ($relativePath in $FixturePaths) {
            $source = Join-Path $RepositoryRoot $relativePath
            $target = Join-Path $fixtureRoot $relativePath
            [void][IO.Directory]::CreateDirectory((Split-Path -Parent $target))
            [IO.File]::Copy($source, $target, $true)
        }
        foreach ($ciContractScript in @("scripts/check-ci.ps1", "scripts/check-ci.test.ps1")) {
            $source = Join-Path $RepositoryRoot $ciContractScript
            $target = Join-Path $fixtureRoot $ciContractScript
            [IO.File]::Copy($source, $target, $true)
        }
    }
    else {
        [IO.File]::Copy((Join-Path $RepositoryRoot "scripts/check-roadmap-governance.ps1"), $governancePath, $true)
    }
    $stageScripts = @(
        "get-roadmap-status.ps1",
        "doc-link-audit.ps1",
        "check-report-log.ps1",
        "get-roadmap-status.test.ps1",
        "generate-roadmap-links.test.ps1",
        "doc-link-audit.test.ps1",
        "check-report-log.test.ps1",
        "check-release-ready.test.ps1",
        "watch-agents.test.ps1",
        "hooks/check-agent-watch.test.ps1",
        "check-ci.test.ps1"
    )
    $stubTemplate = @'
[CmdletBinding()]
param(
    [string]$Format,
    [switch]$CheckTraceability,
    [Parameter(ValueFromRemainingArguments = $true)][object[]]$RemainingArguments
)
$stageRoot = if ((Split-Path -Leaf $PSScriptRoot) -ceq "hooks") { Split-Path -Parent $PSScriptRoot } else { $PSScriptRoot }
$markerPath = Join-Path $stageRoot "stage-events.txt"
$stageName = if ((Split-Path -Leaf $PSScriptRoot) -ceq "hooks") { "hooks/$($MyInvocation.MyCommand.Name)" } else { $MyInvocation.MyCommand.Name }
[IO.File]::AppendAllText($markerPath, "$stageName`n", (New-Object Text.UTF8Encoding($false)))
exit __EXIT_CODE__
'@
    foreach ($stageScript in $stageScripts) {
        if ($UseActualCiContractTest -and $stageScript -ceq "check-ci.test.ps1") {
            continue
        }
        $exitCode = if ($stageScript -ceq $FailingScript) { 17 } else { 0 }
        $stubPath = Join-Path $scriptsRoot $stageScript
        [void][IO.Directory]::CreateDirectory((Split-Path -Parent $stubPath))
        [IO.File]::WriteAllText($stubPath, $stubTemplate.Replace("__EXIT_CODE__", [string]$exitCode), $script:Utf8NoBom)
    }
    return [pscustomobject]@{
        CheckerPath = $governancePath
        Root = $fixtureRoot
        MarkerPath = Join-Path $scriptsRoot "stage-events.txt"
        StageScripts = [string[]]$stageScripts
    }
}

function Invoke-IsolatedGovernance {
    param(
        [Parameter(Mandatory = $true)]$Fixture,
        [Parameter(Mandatory = $true)][string]$PowerShellPath
    )

    $previousErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        $output = @(& $PowerShellPath -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Fixture.CheckerPath 2>&1)
        $governanceExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorAction
    }
    return [pscustomobject]@{
        ExitCode = $governanceExitCode
        Output = (($output | ForEach-Object { [string]$_ }) -join "`n")
    }
}

function Assert-GovernanceStageOrder {
    param(
        [Parameter(Mandatory = $true)]$Fixture,
        [Parameter(Mandatory = $true)][string]$Message
    )

    Assert-True (Test-Path -LiteralPath $Fixture.MarkerPath -PathType Leaf) "$Message markerを残すこと"
    $actual = @(Get-Content -LiteralPath $Fixture.MarkerPath -Encoding UTF8)
    Assert-True ($actual.Count -eq $Fixture.StageScripts.Count) "$Message 11 stageすべてを実invokeすること(actual=$($actual.Count))"
    for ($index = 0; $index -lt $Fixture.StageScripts.Count; $index++) {
        Assert-True ($actual[$index] -ceq $Fixture.StageScripts[$index]) "$Message stage $($index + 1)を予定順にinvokeすること"
    }
}

function Assert-Result {
    param(
        [Parameter(Mandatory = $true)]$Result,
        [Parameter(Mandatory = $true)][bool]$ShouldSucceed,
        [Parameter(Mandatory = $true)][string]$ExpectedOutput,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $exitMatches = if ($ShouldSucceed) { $Result.ExitCode -eq 0 } else { $Result.ExitCode -ne 0 }
    $outputMatches = $Result.Output.Contains($ExpectedOutput)
    if (-not $exitMatches -or -not $outputMatches) {
        Write-Host $Result.Output
    }
    Assert-True $exitMatches "$Message (exit=$($Result.ExitCode))"
    Assert-True $outputMatches "$Message (missing='$ExpectedOutput')"
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $SandboxRoot)) {
        return
    }
    $resolved = [IO.Path]::GetFullPath($SandboxRoot).TrimEnd([char[]]"\/")
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
    $parent = [IO.Path]::GetDirectoryName($resolved)
    $leaf = [IO.Path]::GetFileName($resolved)
    if ($parent -ne $tempRoot -or
        -not [regex]::IsMatch($leaf, '^ori3-check-ci-contract-test-[0-9a-f]{32}$', [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        throw "安全でない一時領域の削除を拒否しました: $resolved"
    }
    $item = Get-Item -LiteralPath $resolved -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "再解析ポイントの一時領域は削除しません: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

if (-not (Test-Path -LiteralPath $CheckerPath -PathType Leaf)) {
    throw "検査本体が見つかりません: $CheckerPath"
}
$powerShellCommand = Get-Command powershell.exe, powershell, pwsh -ErrorAction SilentlyContinue | Select-Object -First 1
if ($null -eq $powerShellCommand) {
    throw "隔離検査を起動するPowerShellが見つかりません"
}
$PowerShellPath = $powerShellCommand.Source

[void][IO.Directory]::CreateDirectory($SandboxRoot)
try {
    Write-Host "[1/32] 正しい8契約とCI 3ジョブ定義は成功する"
    $caseRoot = New-CaseFixture "valid"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $true "checked=8/8 violations=0 warnings=0" "正しい静的契約を拒否しないこと"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 8 / 8")) "8契約すべてを検出対象として報告すること"
    Assert-True (-not $result.Output.Contains("[NG]")) "正しいfixtureへNGを出さないこと"
    Assert-True (-not $result.Output.Contains("ORIGAMI3_CI_CONTRACT_FAIL_OPEN")) "正しいfixtureをfail-openにしないこと"

    Write-Host "[2/32] C01: check.ps1からno-fail-fastを外すと検出する"
    $caseRoot = New-CaseFixture "c01-check"
    Set-ExactReplacement (Join-Path $caseRoot "scripts/check.ps1") '"test", "--workspace", "--no-fail-fast", "--",' '"test", "--workspace", "--",'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C01]" "check.ps1のargv差を検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 8 / 8")) "C01故障でも8契約を走査すること"

    Write-Host "[3/32] C02: receipt正本からno-fail-fastを外すと検出する"
    $caseRoot = New-CaseFixture "c02-receipt"
    Set-ExactReplacement (Join-Path $caseRoot "scripts/check-receipt.ps1") '"test", "--workspace", "--no-fail-fast", "--",' '"test", "--workspace", "--",'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C02]" "receipt正本のargv差を検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 8 / 8")) "C02故障でも8契約を走査すること"

    Write-Host "[4/32] C03: pre-commit直接fallbackからno-fail-fastを外すと検出する"
    $caseRoot = New-CaseFixture "c03-pre-commit"
    Set-ExactReplacement (Join-Path $caseRoot "scripts/hooks/pre-commit") 'cargo test --workspace --no-fail-fast -- --skip' 'cargo test --workspace -- --skip'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C03]" "pre-commit fallbackのargv差を検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 8 / 8")) "C03故障でも8契約を走査すること"

    Write-Host "[5/32] C04: CI checksからno-fail-fastを外すと検出する"
    $caseRoot = New-CaseFixture "c04-ci-checks"
    Set-ExactReplacement (Join-Path $caseRoot ".github/workflows/ci.yml") 'cargo test --workspace --no-fail-fast -- --skip surface_order_179_999' 'cargo test --workspace -- --skip surface_order_179_999'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C04]" "CI checksのargv差を検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 8 / 8")) "C04故障でも8契約を走査すること"

    Write-Host "[6/32] C05: #13へignoreを戻すと検出する"
    $caseRoot = New-CaseFixture "c05-active-13"
    Set-ExactReplacement (Join-Path $caseRoot "apps/desktop/src-tauri/src/surface_order_sa_endpoint_heavy.rs") "#[test]`nfn surface_order_179_999_to_180_all_110_creases" "#[test]`n#[ignore]`nfn surface_order_179_999_to_180_all_110_creases"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C05]" "#13の再ignoreを検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 8 / 8")) "C05故障でも8契約を走査すること"

    Write-Host "[7/32] C06: #14へignoreを戻すと検出する"
    $caseRoot = New-CaseFixture "c06-active-14"
    Set-ExactReplacement (Join-Path $caseRoot "apps/desktop/src-tauri/src/surface_order_acceptance.rs") "#[test]`nfn surface_order_exact_endpoint_is_rank_stable_for_previous_19" "#[test]`n#[ignore]`nfn surface_order_exact_endpoint_is_rank_stable_for_previous_19"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C06]" "#14の再ignoreを検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 8 / 8")) "C06故障でも8契約を走査すること"

    Write-Host "[8/32] C07: proposal matrix PerformanceをCIから変えると検出する"
    $caseRoot = New-CaseFixture "c07-matrix"
    Set-ExactReplacement (Join-Path $caseRoot ".github/workflows/ci.yml") 'run-proposal-matrix.ps1 -Mode Performance' 'run-proposal-matrix.ps1 -Mode PerformanceProbe'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C07]" "proposal matrixのCI欠落を検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 8 / 8")) "C07故障でも8契約を走査すること"

    Write-Host "[9/32] C08: 独立staticがroadmap governance step削除を検出する"
    $caseRoot = New-CaseFixture "c08-roadmap-governance"
    Set-ExactReplacement (Join-Path $caseRoot ".github/workflows/ci.yml") 'powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check-roadmap-governance.ps1' 'powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check-roadmap-governance-MISSING.ps1'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "独立staticがroadmap governance呼出しの欠落を検出すること"
    Assert-True ($result.Output.Contains("static_call=True, governance_call=False")) "独立staticが残った状態でgovernance削除を拒否すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 8 / 8")) "C08故障でも8契約を走査すること"

    Write-Host "[10/32] C08: governance本体から実invokeを消すと検出する"
    $caseRoot = New-CaseFixture "c08-governance-invoke-removed"
    Set-ExactReplacement `
        (Join-Path $caseRoot "scripts/check-roadmap-governance.ps1") `
        '& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $scriptPath @checkArguments' `
        'Write-Host "stage invoke was removed"'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "governance本体の実invoke削除を検出すること"

    Write-Host "[11/32] C08: governance本体がwatch-agents stageをskipすると検出する"
    $caseRoot = New-CaseFixture "c08-governance-watch-stage-skip"
    Set-ExactReplacement `
        (Join-Path $caseRoot "scripts/check-roadmap-governance.ps1") `
        '    $check = $checks[$index]' `
        ('    $check = $checks[$index]' + [Environment]::NewLine + '    if ($check.Script -ceq "watch-agents.test.ps1") { continue }')
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "governance本体のwatch-agents stage skipを検出すること"
    Assert-True ($result.Output.Contains("body_hash=False")) "watch-agents stage skipをnormalized body hashで拒否すること"

    Write-Host "[12/32] C08: script名コメント・偽receipt・exit 0だけの本体を拒否する"
    $caseRoot = New-CaseFixture "c08-governance-fake-receipt"
    $fakeGovernance = @'
# get-roadmap-status.ps1 doc-link-audit.ps1 check-report-log.ps1
# get-roadmap-status.test.ps1 generate-roadmap-links.test.ps1 doc-link-audit.test.ps1
# check-report-log.test.ps1 check-release-ready.test.ps1 watch-agents.test.ps1
# hooks/check-agent-watch.test.ps1 check-ci.test.ps1
Write-Host "ROADMAP_GOVERNANCE_STAGES planned=11 begun=11 invoked=11 ended=11 failures=0"
exit 0
'@
    [IO.File]::WriteAllText((Join-Path $caseRoot "scripts/check-roadmap-governance.ps1"), $fakeGovernance, $script:Utf8NoBom)
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "偽receiptだけのgovernance本体を拒否すること"

    Write-Host "[13/32] C08: checkout fetch-depth 1を拒否する"
    $caseRoot = New-CaseFixture "c08-checkout-shallow-one"
    Set-ExactReplacement (Join-Path $caseRoot ".github/workflows/ci.yml") '          fetch-depth: 0' '          fetch-depth: 1'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "checks jobのshallow checkoutを拒否すること"
    Assert-True ($result.Output.Contains("checkout_full_history=False")) "fetch-depth 1の拒否理由を表示すること"

    Write-Host "[14/32] C08: checkout fetch-depth指定の削除を拒否する"
    $caseRoot = New-CaseFixture "c08-checkout-depth-removed"
    Set-ExactReplacement (Join-Path $caseRoot ".github/workflows/ci.yml") '          fetch-depth: 0' '          # fetch-depth intentionally removed'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "checks jobのfetch-depth欠落を拒否すること"
    Assert-True ($result.Output.Contains("checkout_full_history=False")) "fetch-depth欠落の拒否理由を表示すること"

    Write-Host "[15/32] C08: 本体のci.yml(LF・checkout1件・fetch-depth: 0・コメント無し行)をそのままcheckout_full_historyが真と判定すること"
    $caseRoot = New-CaseFixture "c08-checkout-fetch-depth-lf-positive"
    $ciPath = Join-Path $caseRoot ".github/workflows/ci.yml"
    Assert-True ((([IO.File]::ReadAllText($ciPath, $script:Utf8NoBom)) -notmatch "`r`n")) "複製直後のci.ymlがLFのままであること(byte copyの前提確認)"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $true "checked=8/8 violations=0 warnings=0" "本体のci.yml(LF)をcheckout_full_historyが正しく真と判定すること"
    Assert-True (-not $result.Output.Contains("checkout_full_history=False")) "LFのci.ymlでcheckout_full_historyを偽にしないこと"

    Write-Host "[16/32] C08: ci.ymlをCRLFへ変換した複製でもcheckout_full_historyが真と判定すること(git clone --no-hardlinksがcore.autocrlf=trueでCRLF化する複製経路の再発防止)"
    $caseRoot = New-CaseFixture "c08-checkout-fetch-depth-crlf-positive"
    $ciPath = Join-Path $caseRoot ".github/workflows/ci.yml"
    $crlfText = ([IO.File]::ReadAllText($ciPath, $script:Utf8NoBom)).Replace("`r`n", "`n").Replace("`r", "`n").Replace("`n", "`r`n")
    Assert-True ($crlfText -match "`r`n") "変換後のci.ymlがCRLFへ変わっていること(fixtureの前提確認)"
    [IO.File]::WriteAllText($ciPath, $crlfText, $script:Utf8NoBom)
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $true "checked=8/8 violations=0 warnings=0" "CRLF化したci.ymlでもcheckout_full_historyを偽陰性にしないこと"
    Assert-True (-not $result.Output.Contains("checkout_full_history=False")) "CRLFのci.ymlでcheckout_full_historyを偽にしないこと"

    Write-Host "[17/32] C08: 追跡対象の検査名台帳が欠落すればfail-closedする"
    $caseRoot = New-CaseFixture "c08-test-inventory-missing"
    $inventoryPath = Join-Path $caseRoot "docs/traceability/roadmap-evidence-test-names.txt"
    [IO.File]::Delete($inventoryPath)
    Assert-True (-not (Test-Path -LiteralPath $inventoryPath)) "新しい検査名台帳の欠落fixtureを作れること"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "追跡対象の検査名台帳が無ければ止まること"

    Write-Host "[18/32] C08: doc-link-auditが旧scratchpad台帳へ戻れば拒否する"
    $caseRoot = New-CaseFixture "c08-doc-link-old-inventory"
    Set-ExactReplacement `
        (Join-Path $caseRoot "scripts/doc-link-audit.ps1") `
        'Join-Path $repoRoot "docs/traceability/roadmap-evidence-test-names.txt"' `
        'Join-Path $repoRoot "scratchpad/doc-link-testnames.txt"'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "doc-link-auditの旧scratchpad台帳への逆戻りを検出すること"
    Assert-True ($result.Output.Contains("doc_inventory=False")) "doc-link台帳接続の拒否理由を表示すること"

    Write-Host "[19/32] C08: rules/05の第6段から完了関門を外すと拒否する"
    $caseRoot = New-CaseFixture "c08-release-rules-stage6"
    Set-ExactReplacement `
        (Join-Path $caseRoot "docs/rules/05-リリース.md") `
        'scripts/get-roadmap-status.ps1 -Format Report -RequireComplete' `
        'scripts/get-roadmap-status.ps1 -Format Report -RequireComplete-MISSING'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "rules/05の第6段完了関門の欠落を検出すること"
    Assert-True ($result.Output.Contains("release_rules=False")) "第6段規約の拒否理由を表示すること"

    Write-Host "[20/32] C08: rules/05の6段receiptを5段へ変えると拒否する"
    $caseRoot = New-CaseFixture "c08-release-rules-receipt"
    Set-ExactReplacement `
        (Join-Path $caseRoot "docs/rules/05-リリース.md") `
        'RELEASE_STAGES planned=6 begun=6 ended=6' `
        'RELEASE_STAGES planned=5 begun=5 ended=5'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "rules/05の6段receipt差を検出すること"
    Assert-True ($result.Output.Contains("release_rules=False")) "6段receipt規約の拒否理由を表示すること"

    Write-Host "[21/32] C08: release本体のRequireComplete実invokeを消すと拒否する"
    $caseRoot = New-CaseFixture "c08-release-roadmap-invoke"
    Set-ExactReplacement `
        (Join-Path $caseRoot "scripts/check-release-ready.ps1") `
        '& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $snapshotScript -Format Report -RequireComplete 2>&1' `
        'Write-Host "roadmap completion invoke was removed"'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "release本体のRequireComplete実invoke削除を検出すること"
    Assert-True ($result.Output.Contains("release_roadmap=False")) "完了関門実invokeの拒否理由を表示すること"

    Write-Host "[22/32] C08: release本体のCheckTraceability実invokeを消すと拒否する"
    $caseRoot = New-CaseFixture "c08-release-traceability-invoke"
    Set-ExactReplacement `
        (Join-Path $caseRoot "scripts/check-release-ready.ps1") `
        '& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $docLinkAudit -CheckTraceability' `
        'Write-Host "traceability invoke was removed"'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "release本体のCheckTraceability実invoke削除を検出すること"
    Assert-True ($result.Output.Contains("release_traceability=False")) "証拠台帳実invokeの拒否理由を表示すること"

    Write-Host "[23/32] C08: release本体のplanned stageを5件へ減らすと拒否する"
    $caseRoot = New-CaseFixture "c08-release-planned-five"
    Set-ExactReplacement `
        (Join-Path $caseRoot "scripts/check-release-ready.ps1") `
        '    "利用者への報告記録がリリース日と同じこと",' `
        '    "利用者への報告記録がリリース日と同じこと"'
    Set-ExactReplacement `
        (Join-Path $caseRoot "scripts/check-release-ready.ps1") `
        '    "ロードマップ全件snapshotと証拠台帳"' `
        '    # planned stage 6 was removed'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "release本体のplanned=6欠落を検出すること"
    Assert-True ($result.Output.Contains("release_planned6=False")) "planned=6の拒否理由を表示すること"

    Write-Host "[24/32] production形: governance本体が11 stageを実invokeしてreceiptを集約する"
    $governanceFixture = New-GovernanceProductionFixture "all-pass"
    $result = Invoke-IsolatedGovernance $governanceFixture $PowerShellPath
    Assert-Result $result $true "ROADMAP_GOVERNANCE_STAGES planned=11 begun=11 invoked=11 ended=11 failures=0" "11 stage成功時の集約receiptを確認すること"
    Assert-True (@([regex]::Matches($result.Output, '(?m)^ROADMAP_GOVERNANCE_STAGE ')).Count -eq 11) "成功時にstage receiptを11件出すこと"
    Assert-GovernanceStageOrder $governanceFixture "成功時"

    Write-Host "[25/32] production形: 1 stage exit 17でも残りを実invokeし最終exitを非0にする"
    $governanceFixture = New-GovernanceProductionFixture "one-failure" "check-report-log.ps1"
    $result = Invoke-IsolatedGovernance $governanceFixture $PowerShellPath
    Assert-Result $result $false "[NG] report claim evidence failed (exit=17)" "stage失敗の終了コードを集約すること"
    Assert-True ($result.Output.Contains("ROADMAP_GOVERNANCE_STAGES planned=11 begun=11 invoked=11 ended=11 failures=1")) "失敗時も11 stage完走と失敗数1をreceiptへ出すこと"
    Assert-True (@([regex]::Matches($result.Output, '(?m)^ROADMAP_GOVERNANCE_STAGE ')).Count -eq 11) "失敗時にもstage receiptを11件出すこと"
    Assert-GovernanceStageOrder $governanceFixture "失敗時"

    Write-Host "[26/32] current_statusのstep内target差を厳密同期が検出する"
    $caseRoot = New-CaseFixture "current-status-target"
    Set-ExactReplacement (Join-Path $caseRoot ".github/workflows/ci.yml") 'RUNNER_TEMP\ori3-target-docs7b' 'RUNNER_TEMP\ori3-target-docs7c'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "runステップ 1 が不一致" "current_statusのstep内target差を検出すること"

    Write-Host "[27/32] 必須規約の読取エラーはC08でfail-closedする"
    $caseRoot = New-CaseFixture "fail-open"
    $rulesPath = Join-Path $caseRoot "docs/rules/03-品質ゲート.md"
    [IO.File]::Delete($rulesPath)
    Assert-True (-not (Test-Path -LiteralPath $rulesPath)) "fail-open故障入力で規約ファイルを削除できること"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "roadmap governanceの必須規約を読めなければ止まること"

    Write-Host "[28/32] job-level runner contextを追加すると検出する"
    $caseRoot = New-CaseFixture "job-level-runner-context"
    $jobEnvironment = '      CARGO_TERM_COLOR: never' + [Environment]::NewLine + '      CARGO_TARGET_DIR: ${{ runner.temp }}\ori3-target-docs7b'
    Set-ExactReplacement (Join-Path $caseRoot ".github/workflows/ci.yml") '      CARGO_TERM_COLOR: never' $jobEnvironment
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "job-level envにrunner contextを書けません" "job-level runner contextを拒否すること"

    Write-Host "[29/32] 未対応の複数行runでgovernanceを確認不能ならfail-closedする"
    $caseRoot = New-CaseFixture "unsupported-yaml-run"
    Set-ExactReplacement `
        (Join-Path $caseRoot ".github/workflows/ci.yml") `
        '        run: powershell -NoProfile -ExecutionPolicy Bypass -File crates/ori3-propose/tests/run-proposal-matrix.ps1 -Mode Performance' `
        '        run: |'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C08]" "CI run一覧を解析できなければroadmap governanceを未確認で通さないこと"

    Write-Host "[30/32] production形: governance第11段が独立static step削除を検出する"
    $governanceFixture = New-GovernanceProductionFixture "independent-static-removed" -UseActualCiContractTest
    Set-ExactReplacement `
        (Join-Path $governanceFixture.Root ".github/workflows/ci.yml") `
        'powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check-ci.ps1 -StaticContractOnly' `
        'powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check-ci-MISSING.ps1 -StaticContractOnly'
    $result = Invoke-IsolatedGovernance $governanceFixture $PowerShellPath
    Assert-Result $result $false "[NG] CI contract negative tests failed (exit=1)" "governance第11段が独立static step削除を検出すること"
    Assert-True ($result.Output.Contains("static_call=False, governance_call=True")) "governanceが残った状態で独立static削除を拒否すること"
    Assert-True ($result.Output.Contains("ROADMAP_GOVERNANCE_STAGE number=11 planned=11 script=check-ci.test.ps1 invoked=1 exit=1")) "第11段の実invokeと失敗終了コードを表示すること"
    Assert-True ($result.Output.Contains("ROADMAP_GOVERNANCE_STAGES planned=11 begun=11 invoked=11 ended=11 failures=1")) "独立static削除でもgovernance 11段を完走すること"

    Write-Host "[31/32] 静的契約内部のAST例外をC00 violationとしてfail-closedする"
    $caseRoot = New-CaseFixture "internal-exception-fail-closed"
    Set-ExactReplacement `
        (Join-Path $caseRoot "scripts/check-release-ready.ps1") `
        'Write-Stage 6 "ロードマップ全件snapshotと証拠台帳"' `
        'Write-Stage 6 "unterminated'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C00]" "静的契約内部のAST解析例外を非0にすること"
    Assert-True ($result.Output.Contains("violations=1 warnings=0")) "内部例外をwarningではなくviolationへ集約すること"
    Assert-True (-not $result.Output.Contains("ORIGAMI3_CI_CONTRACT_FAIL_OPEN")) "内部例外をfail-open表示しないこと"

    Write-Host "[32/32] warning-onlyの必須source欠落もcallerがfail-closedする"
    $caseRoot = New-CaseFixture "warning-only-fail-closed"
    $warningOnlyPath = Join-Path $caseRoot "apps/desktop/src-tauri/src/surface_order_sa_endpoint_heavy.rs"
    [IO.File]::Delete($warningOnlyPath)
    Assert-True (-not (Test-Path -LiteralPath $warningOnlyPath)) "warning-only故障入力でC05 sourceを削除できること"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "violations=0 warnings=1" "warning-onlyでもStaticContractOnly callerを非0にすること"
    Assert-True ($result.Output.Contains("ORIGAMI3_CI_CONTRACT_WARNING")) "warning-onlyの読取失敗を表示すること"
    Assert-True ($result.Output.Contains("fail-closedで拒否しました")) "warning-onlyをcallerが拒否した理由を表示すること"

    $repositorySourcesUnchanged = $true
    foreach ($relativePath in $FixturePaths) {
        $currentHash = (Get-FileHash -LiteralPath (Join-Path $RepositoryRoot $relativePath) -Algorithm SHA256).Hash
        if ($currentHash -cne $RepositorySourceHashes[$relativePath]) {
            $repositorySourcesUnchanged = $false
            break
        }
    }
    Assert-True $repositorySourcesUnchanged "隔離検査が本体fixtureを書き換えないこと"

    Write-Host "[OK] check-ci静的契約とgovernance production形の隔離テスト: 32/32件、$script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
