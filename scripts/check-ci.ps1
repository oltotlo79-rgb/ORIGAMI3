# ORIGAMI3 CI再現検査スクリプト (Windows PowerShell 5.1対応)
# 現在のHEADだけを複製し、CIの checks ジョブと同じコマンドを同じ順で実行する。

[CmdletBinding()]
param(
    # 複製だけを壊して失敗経路を検証するためのテスト専用スイッチ。
    [switch]$InjectMissingIgnoredReferenceForTest
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$repoRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$reproRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "verification\ci-repro"))
$sourceRoot = [IO.Path]::GetFullPath((Join-Path $reproRoot "source"))
$cacheRoot = [IO.Path]::GetFullPath((Join-Path $reproRoot "cache"))
$cargoTarget = [IO.Path]::GetFullPath((Join-Path $cacheRoot "cargo-target"))
$lockPath = Join-Path $reproRoot "check-ci.lock"
$sentinelName = ".ignored-source-probe-$([Guid]::NewGuid().ToString('N'))"
$sentinelPath = Join-Path $reproRoot $sentinelName
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
$lockStream = $null
$sentinelCreated = $false
$script:failureExitCode = 1

function Write-Stage {
    param([int]$Number, [int]$Total, [string]$Name)

    Write-Host ""
    Write-Host "=== ($Number/$Total) $Name ===" -ForegroundColor Cyan
}

function Assert-NoReparsePoint {
    param([string[]]$Paths)

    foreach ($pathToCheck in $Paths) {
        if (-not (Test-Path -LiteralPath $pathToCheck)) {
            continue
        }
        $item = Get-Item -LiteralPath $pathToCheck -Force
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "再解析ポイントは複製・キャッシュ先に使えません: $pathToCheck"
        }
    }
}

function ConvertFrom-SimpleYamlValue {
    param([string]$Value)

    $result = $Value.Trim()
    if ($result.Length -ge 2) {
        if ($result[0] -eq '"' -and $result[$result.Length - 1] -eq '"') {
            return $result.Substring(1, $result.Length - 2)
        }
        if ($result[0] -eq "'" -and $result[$result.Length - 1] -eq "'") {
            return $result.Substring(1, $result.Length - 2).Replace("''", "'")
        }
    }
    return $result
}

function Add-RunStepIfPresent {
    param(
        [System.Collections.Generic.List[object]]$Steps,
        [AllowNull()][hashtable]$Step
    )

    if ($null -eq $Step -or $null -eq $Step.Run) {
        return
    }
    if ([string]::IsNullOrWhiteSpace([string]$Step.Name)) {
        throw "ci.yml の run ステップに name がありません"
    }
    $Steps.Add([pscustomobject]@{
        Name = [string]$Step.Name
        WorkingDirectory = [string]$Step.WorkingDirectory
        Command = [string]$Step.Run
    })
}

function Get-ChecksRunSteps {
    param([string]$WorkflowPath)

    $lines = @(Get-Content -LiteralPath $WorkflowPath -Encoding UTF8)
    if (@($lines | Where-Object { $_ -match '^defaults:\s*(?:#.*)?$' }).Count -gt 0) {
        throw "ci.yml のworkflow共通defaultsには未対応です。check-ci.ps1 を同期してください"
    }
    $checksStart = -1
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^  checks:\s*(?:#.*)?$') {
            if ($checksStart -ne -1) {
                throw "ci.yml に jobs.checks が複数あります"
            }
            $checksStart = $i
        }
    }
    if ($checksStart -eq -1) {
        throw "ci.yml に jobs.checks が見つかりません"
    }

    $checksEnd = $lines.Count
    for ($i = $checksStart + 1; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^  [A-Za-z0-9_-]+:\s*(?:#.*)?$') {
            $checksEnd = $i
            break
        }
    }

    $stepsStart = -1
    for ($i = $checksStart + 1; $i -lt $checksEnd; $i++) {
        if ($lines[$i] -match '^    steps:\s*(?:#.*)?$') {
            $stepsStart = $i
            break
        }
    }
    if ($stepsStart -eq -1) {
        throw "ci.yml の jobs.checks.steps が見つかりません"
    }

    for ($i = $checksStart + 1; $i -lt $checksEnd; $i++) {
        if ($lines[$i] -match '^    defaults:\s*(?:#.*)?$') {
            throw "ci.yml の jobs.checks.defaults には未対応です。check-ci.ps1 を同期してください"
        }
        if ($lines[$i] -match '^    (if|continue-on-error):') {
            throw "ci.yml の jobs.checks.$($Matches[1]) には未対応です。check-ci.ps1 を同期してください"
        }
        if ($lines[$i] -match '^        (if|continue-on-error|timeout-minutes):') {
            throw "ci.yml のステップ固有 $($Matches[1]) には未対応です。check-ci.ps1 を同期してください"
        }
    }

    $steps = New-Object 'System.Collections.Generic.List[object]'
    $current = $null

    for ($i = $stepsStart + 1; $i -lt $checksEnd; $i++) {
        $line = $lines[$i]

        if ($line -match '^      -\s+(.+)$') {
            Add-RunStepIfPresent -Steps $steps -Step $current
            $current = @{
                Name = ""
                WorkingDirectory = "."
                Run = $null
            }
            $firstProperty = $Matches[1]
            if ($firstProperty -match '^name:\s*(.+)$') {
                $current.Name = ConvertFrom-SimpleYamlValue $Matches[1]
            }
            elseif ($firstProperty -match '^run:\s*(.*)$') {
                $runValue = $Matches[1].Trim()
                if ([string]::IsNullOrWhiteSpace($runValue) -or $runValue -in @('|', '>', '|-', '>-', '|+', '>+')) {
                    throw "ci.yml の複数行 run には未対応です。check-ci.ps1 を同期してください"
                }
                $current.Run = ConvertFrom-SimpleYamlValue $runValue
            }
            elseif ($firstProperty -notmatch '^uses:\s*.+$') {
                throw "ci.yml のステップ先頭プロパティ '$firstProperty' には未対応です。check-ci.ps1 を同期してください"
            }
            continue
        }

        if ($null -eq $current) {
            continue
        }

        if ($line -match '^        name:\s*(.+)$') {
            $current.Name = ConvertFrom-SimpleYamlValue $Matches[1]
        }
        elseif ($line -match '^        working-directory:\s*(.+)$') {
            $current.WorkingDirectory = ConvertFrom-SimpleYamlValue $Matches[1]
        }
        elseif ($line -match '^        run:\s*(.*)$') {
            if ($null -ne $current.Run) {
                throw "ci.yml の1ステップに run が複数あります"
            }
            $runValue = $Matches[1].Trim()
            if ([string]::IsNullOrWhiteSpace($runValue) -or $runValue -in @('|', '>', '|-', '>-', '|+', '>+')) {
                throw "ci.yml の複数行 run には未対応です。check-ci.ps1 を同期してください"
            }
            $current.Run = ConvertFrom-SimpleYamlValue $runValue
        }
        elseif ($line -match '^        (shell|env|timeout-minutes):') {
            throw "ci.yml のステップ固有 $($Matches[1]) には未対応です。check-ci.ps1 を同期してください"
        }
    }
    Add-RunStepIfPresent -Steps $steps -Step $current

    if ($steps.Count -eq 0) {
        throw "ci.yml の jobs.checks に run ステップがありません"
    }
    return $steps.ToArray()
}

function Normalize-RelativePath {
    param([string]$Path)

    $normalized = $Path.Trim().Replace('\', '/').TrimEnd('/')
    if ([string]::IsNullOrWhiteSpace($normalized)) {
        return "."
    }
    return $normalized
}

function Assert-CiStepsMatch {
    param([object[]]$Actual, [object[]]$Expected)

    if ($Actual.Count -ne $Expected.Count) {
        throw "ci.yml のrunステップ数が変わりました (ci.yml: $($Actual.Count), check-ci.ps1: $($Expected.Count))"
    }

    for ($i = 0; $i -lt $Expected.Count; $i++) {
        $actualDirectory = Normalize-RelativePath $Actual[$i].WorkingDirectory
        $expectedDirectory = Normalize-RelativePath $Expected[$i].WorkingDirectory
        if ($Actual[$i].Command -cne $Expected[$i].Command -or $actualDirectory -cne $expectedDirectory) {
            throw "ci.yml のrunステップ $($i + 1) が不一致です (ci.yml: '$($Actual[$i].Command)' at '$actualDirectory', check-ci.ps1: '$($Expected[$i].Command)' at '$expectedDirectory')"
        }
    }
}

function Invoke-CheckedCommand {
    param(
        [int]$StageNumber,
        [int]$TotalStages,
        [string]$Name,
        [string]$WorkingDirectory,
        [string]$Executable,
        [string[]]$Arguments
    )

    Write-Stage $StageNumber $TotalStages $Name
    $global:LASTEXITCODE = 0
    Push-Location $WorkingDirectory
    try {
        try {
            & $Executable @Arguments
        }
        catch {
            $script:failureExitCode = 1
            throw "$Name を起動できませんでした: $($_.Exception.Message)"
        }
        $commandExitCode = $LASTEXITCODE
    }
    finally {
        Pop-Location
    }

    if ($commandExitCode -ne 0) {
        $script:failureExitCode = $commandExitCode
        throw "$Name が失敗しました (終了コード: $commandExitCode)"
    }
}

$expectedSteps = @(
    [pscustomobject]@{ Command = "npm ci"; WorkingDirectory = "apps/desktop"; Executable = "npm"; Arguments = @("ci") },
    [pscustomobject]@{ Command = "cargo test --workspace"; WorkingDirectory = "."; Executable = "cargo"; Arguments = @("test", "--workspace") },
    [pscustomobject]@{ Command = "cargo clippy --workspace --all-targets -- -D warnings"; WorkingDirectory = "."; Executable = "cargo"; Arguments = @("clippy", "--workspace", "--all-targets", "--", "-D", "warnings") },
    [pscustomobject]@{ Command = "npm run build"; WorkingDirectory = "apps/desktop"; Executable = "npm"; Arguments = @("run", "build") },
    [pscustomobject]@{ Command = "npm run lint"; WorkingDirectory = "apps/desktop"; Executable = "npm"; Arguments = @("run", "lint") },
    [pscustomobject]@{ Command = "npm run test"; WorkingDirectory = "apps/desktop"; Executable = "npm"; Arguments = @("run", "test") }
)
$totalStages = $expectedSteps.Count + 2

try {
    $verificationRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "verification"))
    $expectedReproRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "verification\ci-repro"))
    $expectedSourceRoot = [IO.Path]::GetFullPath((Join-Path $expectedReproRoot "source"))
    if (-not [string]::Equals($reproRoot, $expectedReproRoot, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals($sourceRoot, $expectedSourceRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "複製先の安全確認に失敗しました: $sourceRoot"
    }

    $gitignorePath = Join-Path $repoRoot ".gitignore"
    $gitignoreLines = @(Get-Content -LiteralPath $gitignorePath -Encoding UTF8)
    if (-not @($gitignoreLines | Where-Object { $_.Trim() -eq "/verification/" }).Count) {
        throw "verification/ が .gitignore 対象ではありません"
    }

    Assert-NoReparsePoint @($verificationRoot, $reproRoot, $sourceRoot, $cacheRoot, $cargoTarget, $lockPath)
    New-Item -ItemType Directory -Path $reproRoot -Force | Out-Null
    Assert-NoReparsePoint @($verificationRoot, $reproRoot, $sourceRoot, $cacheRoot, $cargoTarget, $lockPath)

    try {
        $lockStream = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    }
    catch {
        throw "別の check-ci.ps1 が実行中です: $($_.Exception.Message)"
    }

    Write-Stage 1 $totalStages "HEADを複製し、無視対象が混入していないことを確認"
    if (Test-Path -LiteralPath $sourceRoot) {
        Remove-Item -LiteralPath $sourceRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path $cacheRoot -Force | Out-Null
    Assert-NoReparsePoint @($cacheRoot, $cargoTarget)
    $utf8NoBom = New-Object Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($sentinelPath, "", $utf8NoBom)
    $sentinelCreated = $true

    $global:LASTEXITCODE = 0
    try {
        $repoGitDirectory = Join-Path $repoRoot ".git"
        & git `
            -c "safe.directory=$repoRoot" `
            -c "safe.directory=$repoGitDirectory" `
            -c "core.excludesFile=$sentinelPath" `
            clone --no-hardlinks --quiet -- $repoRoot $sourceRoot
    }
    catch {
        throw "HEADの複製を開始できませんでした: $($_.Exception.Message)"
    }
    if ($LASTEXITCODE -ne 0) {
        $script:failureExitCode = $LASTEXITCODE
        throw "HEADの複製に失敗しました (終了コード: $LASTEXITCODE)"
    }

    $copiedSentinel = Join-Path $sourceRoot "verification\ci-repro\$sentinelName"
    if (Test-Path -LiteralPath $copiedSentinel) {
        throw "無視対象ファイルがHEAD複製へ混入しました: $copiedSentinel"
    }

    $global:LASTEXITCODE = 0
    $cloneStatus = @(& git -C $sourceRoot -c "core.excludesFile=$sentinelPath" status --porcelain=v1 --untracked-files=all --ignored 2>&1)
    if ($LASTEXITCODE -ne 0) {
        $script:failureExitCode = $LASTEXITCODE
        throw "複製先の状態確認に失敗しました (終了コード: $LASTEXITCODE)"
    }
    if ($cloneStatus.Count -ne 0) {
        throw "複製先にコミット外のファイルがあります: $($cloneStatus -join '; ')"
    }

    $global:LASTEXITCODE = 0
    & git -C $sourceRoot -c "core.excludesFile=$sentinelPath" check-ignore --no-index --quiet "verification/ci-repro/.probe"
    if ($LASTEXITCODE -ne 0) {
        throw "HEAD内の .gitignore で verification/ci-repro/ が無視対象になっていません"
    }

    Write-Stage 2 $totalStages "ci.yml の jobs.checks と実行定義を同期確認"
    $workflowPath = Join-Path $sourceRoot ".github\workflows\ci.yml"
    $ciSteps = @(Get-ChecksRunSteps $workflowPath)
    Assert-CiStepsMatch -Actual $ciSteps -Expected $expectedSteps

    if ($InjectMissingIgnoredReferenceForTest) {
        $testTarget = Join-Path $sourceRoot "apps\desktop\src-tauri\src\lib.rs"
        $utf8NoBom = New-Object Text.UTF8Encoding($false)
        $missingReference = "`r`n" + 'const _CI_REPRO_MISSING: &str = include_str!("../../../../verification/ci-repro-missing.rs");' + "`r`n"
        [IO.File]::AppendAllText(
            $testTarget,
            $missingReference,
            $utf8NoBom
        )
        Write-Host "[TEST] 複製先だけに無視対象ファイルの欠損参照を注入しました" -ForegroundColor Yellow
    }

    $env:CARGO_TERM_COLOR = "always"
    $env:CARGO_TARGET_DIR = $cargoTarget

    for ($i = 0; $i -lt $expectedSteps.Count; $i++) {
        $step = $expectedSteps[$i]
        $workingDirectory = if ((Normalize-RelativePath $step.WorkingDirectory) -eq ".") {
            $sourceRoot
        }
        else {
            Join-Path $sourceRoot ($step.WorkingDirectory.Replace('/', '\'))
        }
        $heading = "$($ciSteps[$i].Name): $($step.Command)"
        Invoke-CheckedCommand `
            -StageNumber ($i + 3) `
            -TotalStages $totalStages `
            -Name $heading `
            -WorkingDirectory $workingDirectory `
            -Executable $step.Executable `
            -Arguments $step.Arguments
    }

    Write-Host ""
    Write-Host "[OK] HEADの内容だけでCI checksジョブの全検査に合格しました" -ForegroundColor Green
    $script:failureExitCode = 0
}
catch {
    Write-Host ""
    Write-Host "[NG] $($_.Exception.Message)" -ForegroundColor Red
    if ($script:failureExitCode -eq 0) {
        $script:failureExitCode = 1
    }
}
finally {
    if ($null -ne $lockStream) {
        $lockStream.Dispose()
    }
    if ($sentinelCreated -and (Test-Path -LiteralPath $sentinelPath -PathType Leaf)) {
        $sentinelItem = Get-Item -LiteralPath $sentinelPath -Force
        if (($sentinelItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) {
            Remove-Item -LiteralPath $sentinelPath -Force
        }
    }
    $stopwatch.Stop()
    Write-Host ("所要時間: {0:hh\:mm\:ss}" -f $stopwatch.Elapsed)
}

exit $script:failureExitCode
