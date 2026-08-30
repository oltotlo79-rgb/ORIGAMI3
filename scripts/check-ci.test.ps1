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
    "scripts/check.ps1",
    "scripts/check-receipt.ps1",
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
    Write-Host "[1/12] 正しい7契約とCI 3ジョブ定義は成功する"
    $caseRoot = New-CaseFixture "valid"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $true "checked=7/7 violations=0 warnings=0" "正しい静的契約を拒否しないこと"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 7 / 7")) "7契約すべてを検出対象として報告すること"
    Assert-True (-not $result.Output.Contains("[NG]")) "正しいfixtureへNGを出さないこと"
    Assert-True (-not $result.Output.Contains("ORIGAMI3_CI_CONTRACT_FAIL_OPEN")) "正しいfixtureをfail-openにしないこと"

    Write-Host "[2/12] C01: check.ps1からno-fail-fastを外すと検出する"
    $caseRoot = New-CaseFixture "c01-check"
    Set-ExactReplacement (Join-Path $caseRoot "scripts/check.ps1") '"test", "--workspace", "--no-fail-fast", "--",' '"test", "--workspace", "--",'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C01]" "check.ps1のargv差を検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 7 / 7")) "C01故障でも7契約を走査すること"

    Write-Host "[3/12] C02: receipt正本からno-fail-fastを外すと検出する"
    $caseRoot = New-CaseFixture "c02-receipt"
    Set-ExactReplacement (Join-Path $caseRoot "scripts/check-receipt.ps1") '"test", "--workspace", "--no-fail-fast", "--",' '"test", "--workspace", "--",'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C02]" "receipt正本のargv差を検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 7 / 7")) "C02故障でも7契約を走査すること"

    Write-Host "[4/12] C03: pre-commit直接fallbackからno-fail-fastを外すと検出する"
    $caseRoot = New-CaseFixture "c03-pre-commit"
    Set-ExactReplacement (Join-Path $caseRoot "scripts/hooks/pre-commit") 'cargo test --workspace --no-fail-fast -- --skip' 'cargo test --workspace -- --skip'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C03]" "pre-commit fallbackのargv差を検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 7 / 7")) "C03故障でも7契約を走査すること"

    Write-Host "[5/12] C04: CI checksからno-fail-fastを外すと検出する"
    $caseRoot = New-CaseFixture "c04-ci-checks"
    Set-ExactReplacement (Join-Path $caseRoot ".github/workflows/ci.yml") 'cargo test --workspace --no-fail-fast -- --skip surface_order_179_999' 'cargo test --workspace -- --skip surface_order_179_999'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C04]" "CI checksのargv差を検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 7 / 7")) "C04故障でも7契約を走査すること"

    Write-Host "[6/12] C05: #13へignoreを戻すと検出する"
    $caseRoot = New-CaseFixture "c05-active-13"
    Set-ExactReplacement (Join-Path $caseRoot "apps/desktop/src-tauri/src/surface_order_sa_endpoint_heavy.rs") "#[test]`nfn surface_order_179_999_to_180_all_110_creases" "#[test]`n#[ignore]`nfn surface_order_179_999_to_180_all_110_creases"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C05]" "#13の再ignoreを検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 7 / 7")) "C05故障でも7契約を走査すること"

    Write-Host "[7/12] C06: #14へignoreを戻すと検出する"
    $caseRoot = New-CaseFixture "c06-active-14"
    Set-ExactReplacement (Join-Path $caseRoot "apps/desktop/src-tauri/src/surface_order_acceptance.rs") "#[test]`nfn surface_order_exact_endpoint_is_rank_stable_for_previous_19" "#[test]`n#[ignore]`nfn surface_order_exact_endpoint_is_rank_stable_for_previous_19"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C06]" "#14の再ignoreを検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 7 / 7")) "C06故障でも7契約を走査すること"

    Write-Host "[8/12] C07: proposal matrix PerformanceをCIから変えると検出する"
    $caseRoot = New-CaseFixture "c07-matrix"
    Set-ExactReplacement (Join-Path $caseRoot ".github/workflows/ci.yml") 'run-proposal-matrix.ps1 -Mode Performance' 'run-proposal-matrix.ps1 -Mode PerformanceProbe'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "[NG][C07]" "proposal matrixのCI欠落を検出すること"
    Assert-True ($result.Output.Contains("GATE_DRIFT_DETECTED 7 / 7")) "C07故障でも7契約を走査すること"

    Write-Host "[9/12] current_statusのstep内target差を厳密同期が検出する"
    $caseRoot = New-CaseFixture "current-status-target"
    Set-ExactReplacement (Join-Path $caseRoot ".github/workflows/ci.yml") 'RUNNER_TEMP\ori3-target-docs7b' 'RUNNER_TEMP\ori3-target-docs7c'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "runステップ 1 が不一致" "current_statusのstep内target差を検出すること"

    Write-Host "[10/12] 内部読取エラーは警告してfail-openする"
    $caseRoot = New-CaseFixture "fail-open"
    $rulesPath = Join-Path $caseRoot "docs/rules/03-品質ゲート.md"
    [IO.File]::Delete($rulesPath)
    Assert-True (-not (Test-Path -LiteralPath $rulesPath)) "fail-open故障入力で規約ファイルを削除できること"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $true "ORIGAMI3_CI_CONTRACT_FAIL_OPEN" "内部読取エラーは警告して通すこと"

    Write-Host "[11/12] job-level runner contextを追加すると検出する"
    $caseRoot = New-CaseFixture "job-level-runner-context"
    $jobEnvironment = '      CARGO_TERM_COLOR: never' + [Environment]::NewLine + '      CARGO_TARGET_DIR: ${{ runner.temp }}\ori3-target-docs7b'
    Set-ExactReplacement (Join-Path $caseRoot ".github/workflows/ci.yml") '      CARGO_TERM_COLOR: never' $jobEnvironment
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $false "job-level envにrunner contextを書けません" "job-level runner contextを拒否すること"

    Write-Host "[12/12] 未対応の複数行runは警告してfail-openする"
    $caseRoot = New-CaseFixture "unsupported-yaml-run"
    Set-ExactReplacement `
        (Join-Path $caseRoot ".github/workflows/ci.yml") `
        '        run: powershell -NoProfile -ExecutionPolicy Bypass -File crates/ori3-propose/tests/run-proposal-matrix.ps1 -Mode Performance' `
        '        run: |'
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-Result $result $true "ORIGAMI3_CI_CONTRACT_FAIL_OPEN" "未対応YAMLは警告して通すこと"

    $repositorySourcesUnchanged = $true
    foreach ($relativePath in $FixturePaths) {
        $currentHash = (Get-FileHash -LiteralPath (Join-Path $RepositoryRoot $relativePath) -Algorithm SHA256).Hash
        if ($currentHash -cne $RepositorySourceHashes[$relativePath]) {
            $repositorySourcesUnchanged = $false
            break
        }
    }
    Assert-True $repositorySourcesUnchanged "隔離検査が本体fixtureを書き換えないこと"

    Write-Host "[OK] check-ci静的契約の隔離テスト: 12/12件、$script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
