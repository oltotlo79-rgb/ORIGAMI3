# ORIGAMI3 CI再現検査スクリプト (Windows PowerShell 5.1対応)
# 現在のHEADだけを新しいフォルダへ複製し、CIの全ジョブと同じコマンドを同じ順で実行する。

[CmdletBinding()]
param(
    # 複製だけを壊して失敗経路を検証するためのテスト専用スイッチ。
    [switch]$InjectMissingIgnoredReferenceForTest,

    # 複製検証そのものの退行確認専用。どちらも新規複製の中だけを変更する。
    [ValidateSet("None", "Normal", "MissingGit", "IgnoredFile")]
    [string]$CloneValidationTestCase = "None"
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$repoRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$reproRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "verification\ci-repro"))
$sourceName = "source-$([Guid]::NewGuid().ToString('N'))"
$sourceRoot = [IO.Path]::GetFullPath((Join-Path $reproRoot $sourceName))
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

function Get-DescendantDirectoriesSafely {
    param([string]$Root)

    Assert-NoReparsePoint @($Root)
    $pending = New-Object 'System.Collections.Generic.Stack[string]'
    $directories = New-Object 'System.Collections.Generic.List[object]'
    $pending.Push($Root)
    while ($pending.Count -gt 0) {
        $current = $pending.Pop()
        foreach ($item in @(Get-ChildItem -LiteralPath $current -Force)) {
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "再解析ポイントが一時複製の中にあります: $($item.FullName)"
            }
            if ($item.PSIsContainer) {
                $directories.Add($item)
                $pending.Push($item.FullName)
            }
        }
    }
    return $directories.ToArray()
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

function Get-JobRunSteps {
    param([string]$WorkflowPath, [string]$JobName)

    $lines = @(Get-Content -LiteralPath $WorkflowPath -Encoding UTF8)
    if (@($lines | Where-Object { $_ -match '^defaults:\s*(?:#.*)?$' }).Count -gt 0) {
        throw "ci.yml のworkflow共通defaultsには未対応です。check-ci.ps1 を同期してください"
    }
    $jobStart = -1
    $escapedJobName = [Regex]::Escape($JobName)
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match "^  ${escapedJobName}:\s*(?:#.*)?$") {
            if ($jobStart -ne -1) {
                throw "ci.yml に jobs.$JobName が複数あります"
            }
            $jobStart = $i
        }
    }
    if ($jobStart -eq -1) {
        throw "ci.yml に jobs.$JobName が見つかりません"
    }

    $jobEnd = $lines.Count
    for ($i = $jobStart + 1; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^  [A-Za-z0-9_-]+:\s*(?:#.*)?$') {
            $jobEnd = $i
            break
        }
    }

    $stepsStart = -1
    for ($i = $jobStart + 1; $i -lt $jobEnd; $i++) {
        if ($lines[$i] -match '^    steps:\s*(?:#.*)?$') {
            $stepsStart = $i
            break
        }
    }
    if ($stepsStart -eq -1) {
        throw "ci.yml の jobs.$JobName.steps が見つかりません"
    }

    for ($i = $jobStart + 1; $i -lt $jobEnd; $i++) {
        if ($lines[$i] -match '^    defaults:\s*(?:#.*)?$') {
            throw "ci.yml の jobs.$JobName.defaults には未対応です。check-ci.ps1 を同期してください"
        }
        if ($lines[$i] -match '^    (if|continue-on-error):') {
            throw "ci.yml の jobs.$JobName.$($Matches[1]) には未対応です。check-ci.ps1 を同期してください"
        }
        if ($lines[$i] -match '^        (if|continue-on-error|timeout-minutes):') {
            throw "ci.yml のステップ固有 $($Matches[1]) には未対応です。check-ci.ps1 を同期してください"
        }
    }

    $steps = New-Object 'System.Collections.Generic.List[object]'
    $current = $null

    for ($i = $stepsStart + 1; $i -lt $jobEnd; $i++) {
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
        throw "ci.yml の jobs.$JobName に run ステップがありません"
    }
    return $steps.ToArray()
}

function Get-WorkflowJobNames {
    param([string]$WorkflowPath)

    $lines = @(Get-Content -LiteralPath $WorkflowPath -Encoding UTF8)
    $jobsStart = -1
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^jobs:\s*(?:#.*)?$') {
            if ($jobsStart -ne -1) {
                throw "ci.yml に jobs が複数あります"
            }
            $jobsStart = $i
        }
    }
    if ($jobsStart -eq -1) {
        throw "ci.yml に jobs が見つかりません"
    }

    $jobNames = New-Object 'System.Collections.Generic.List[string]'
    for ($i = $jobsStart + 1; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^[^\s#][^:]*:\s*(?:#.*)?$') {
            break
        }
        if ($lines[$i] -match '^  ([A-Za-z0-9_-]+):\s*(?:#.*)?$') {
            $jobNames.Add($Matches[1])
        }
    }
    if ($jobNames.Count -eq 0) {
        throw "ci.yml の jobs にジョブがありません"
    }
    return $jobNames.ToArray()
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

function Resolve-ExternalCommand {
    param([string]$Executable)

    $commands = @(Get-Command -Name $Executable -CommandType Application -ErrorAction Stop)
    if ($commands.Count -eq 0) {
        throw "外部プログラムが見つかりません: $Executable"
    }
    return [string]$commands[0].Source
}

function Invoke-ExternalCapture {
    param([string]$Executable, [string[]]$Arguments)

    $commandPath = Resolve-ExternalCommand $Executable
    $previousErrorActionPreference = $ErrorActionPreference
    $hasNativeExitPreference = Test-Path -LiteralPath Variable:\PSNativeCommandUseErrorActionPreference
    if ($hasNativeExitPreference) {
        $previousNativeExitPreference = $PSNativeCommandUseErrorActionPreference
    }
    try {
        # Windows PowerShellは外部プログラムのstderrをErrorRecordへ変換することがある。
        # stderrは画面へそのまま流し、標準出力だけを受け取る。成否は直後の終了コードだけで決める。
        $ErrorActionPreference = "Continue"
        if ($hasNativeExitPreference) {
            $PSNativeCommandUseErrorActionPreference = $false
        }
        $global:LASTEXITCODE = [int]::MinValue
        $rawOutput = @(& $commandPath @Arguments)
        $commandExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
        if ($hasNativeExitPreference) {
            $PSNativeCommandUseErrorActionPreference = $previousNativeExitPreference
        }
    }

    if ($commandExitCode -eq [int]::MinValue) {
        throw "外部プログラムを起動できませんでした: $Executable"
    }
    return [pscustomobject]@{
        ExitCode = [int]$commandExitCode
        Output = [string[]]@($rawOutput | ForEach-Object { [string]$_ })
    }
}

function Write-ExternalOutput {
    param([object]$Result)

    foreach ($line in $Result.Output) {
        Write-Host $line
    }
}

function Get-SingleOutputLine {
    param([object]$Result, [string]$Description)

    $lines = @($Result.Output | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($lines.Count -ne 1) {
        throw "$Description の出力が1行ではありません: $($lines -join '; ')"
    }
    return $lines[0].Trim()
}

function Assert-ValidClone {
    param(
        [string]$CloneRoot,
        [string]$ExpectedHead,
        [string]$EmptyExcludesFile
    )

    $cloneGit = Join-Path $CloneRoot ".git"
    if (-not (Test-Path -LiteralPath $cloneGit -PathType Container)) {
        throw "複製先に .git がありません: $cloneGit"
    }
    Assert-NoReparsePoint @($CloneRoot, $cloneGit)
    Write-Host "[OK] 複製先に .git がある"

    $topLevelResult = Invoke-ExternalCapture git @(
        "-C", $CloneRoot,
        "-c", "core.excludesFile=$EmptyExcludesFile",
        "rev-parse", "--show-toplevel"
    )
    if ($topLevelResult.ExitCode -ne 0) {
        $script:failureExitCode = $topLevelResult.ExitCode
        Write-ExternalOutput $topLevelResult
        throw "複製先のリポジトリルート確認に失敗しました (終了コード: $($topLevelResult.ExitCode))"
    }
    $actualTopLevel = [IO.Path]::GetFullPath((Get-SingleOutputLine $topLevelResult "複製先のリポジトリルート確認"))
    if (-not [string]::Equals($actualTopLevel, $CloneRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "複製先ではなく別のリポジトリを参照しています: $actualTopLevel"
    }

    $headResult = Invoke-ExternalCapture git @(
        "-C", $CloneRoot,
        "-c", "core.excludesFile=$EmptyExcludesFile",
        "rev-parse", "--verify", "HEAD"
    )
    if ($headResult.ExitCode -ne 0) {
        $script:failureExitCode = $headResult.ExitCode
        Write-ExternalOutput $headResult
        throw "複製先のHEAD確認に失敗しました (終了コード: $($headResult.ExitCode))"
    }
    $actualHead = Get-SingleOutputLine $headResult "複製先のHEAD確認"
    if ($actualHead -cne $ExpectedHead) {
        throw "複製先のHEADが本体と一致しません (本体: $ExpectedHead, 複製: $actualHead)"
    }
    Write-Host "[OK] 複製先のHEADが本体と一致する: $actualHead"

    $statusResult = Invoke-ExternalCapture git @(
        "-C", $CloneRoot,
        "-c", "core.excludesFile=$EmptyExcludesFile",
        "status", "--porcelain", "--untracked-files=all"
    )
    if ($statusResult.ExitCode -ne 0) {
        $script:failureExitCode = $statusResult.ExitCode
        Write-ExternalOutput $statusResult
        throw "複製先の作業ツリー確認に失敗しました (終了コード: $($statusResult.ExitCode))"
    }
    $statusLines = @($statusResult.Output | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($statusLines.Count -ne 0) {
        throw "複製先の作業ツリーが空ではありません: $($statusLines -join '; ')"
    }
    Write-Host "[OK] 複製先の git status --porcelain が空である"

    $forbiddenDirectoryNames = @("verification", "scratchpad", "target", "node_modules")
    $gitPrefix = $cloneGit.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    $forbiddenDirectories = New-Object 'System.Collections.Generic.List[string]'
    foreach ($directory in @(Get-DescendantDirectoriesSafely $CloneRoot)) {
        if ($directory.FullName.StartsWith($gitPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        if ($forbiddenDirectoryNames -contains $directory.Name) {
            $relativePath = $directory.FullName.Substring($CloneRoot.Length).TrimStart('\', '/')
            $forbiddenDirectories.Add($relativePath.Replace('\', '/'))
        }
    }
    if ($forbiddenDirectories.Count -ne 0) {
        throw "複製先に無視対象のフォルダがあります: $($forbiddenDirectories -join '; ')"
    }
    Write-Host "[OK] 複製先に verification/、scratchpad/、target/、node_modules/ が無い"
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
    $commandPath = Resolve-ExternalCommand $Executable
    $commandExitCode = 1
    $invocationError = $null
    Push-Location $WorkingDirectory
    try {
        $previousErrorActionPreference = $ErrorActionPreference
        $hasNativeExitPreference = Test-Path -LiteralPath Variable:\PSNativeCommandUseErrorActionPreference
        if ($hasNativeExitPreference) {
            $previousNativeExitPreference = $PSNativeCommandUseErrorActionPreference
        }
        try {
            # cargo等は進捗をstderrへ出す。表示はそのまま流し、成否は終了コードだけで判定する。
            $ErrorActionPreference = "Continue"
            if ($hasNativeExitPreference) {
                $PSNativeCommandUseErrorActionPreference = $false
            }
            $global:LASTEXITCODE = [int]::MinValue
            & $commandPath @Arguments
            $commandExitCode = $LASTEXITCODE
        }
        catch {
            $invocationError = $_
        }
        finally {
            $ErrorActionPreference = $previousErrorActionPreference
            if ($hasNativeExitPreference) {
                $PSNativeCommandUseErrorActionPreference = $previousNativeExitPreference
            }
        }
    }
    finally {
        Pop-Location
    }

    if ($null -ne $invocationError) {
        $script:failureExitCode = 1
        throw "$Name を起動できませんでした: $($invocationError.Exception.Message)"
    }
    if ($commandExitCode -eq [int]::MinValue) {
        $script:failureExitCode = 1
        throw "$Name を起動できませんでした: 外部プログラムから終了コードが返りませんでした"
    }
    if ($commandExitCode -ne 0) {
        $script:failureExitCode = $commandExitCode
        throw "$Name が失敗しました (終了コード: $commandExitCode)"
    }
}

$expectedChecksSteps = @(
    [pscustomobject]@{ Command = "npm ci"; WorkingDirectory = "apps/desktop"; Executable = "npm"; Arguments = @("ci") },
    [pscustomobject]@{ Command = "cargo test --workspace"; WorkingDirectory = "."; Executable = "cargo"; Arguments = @("test", "--workspace") },
    [pscustomobject]@{ Command = "cargo clippy --workspace --all-targets -- -D warnings"; WorkingDirectory = "."; Executable = "cargo"; Arguments = @("clippy", "--workspace", "--all-targets", "--", "-D", "warnings") },
    [pscustomobject]@{ Command = "npm run build"; WorkingDirectory = "apps/desktop"; Executable = "npm"; Arguments = @("run", "build") },
    [pscustomobject]@{ Command = "npm run lint"; WorkingDirectory = "apps/desktop"; Executable = "npm"; Arguments = @("run", "lint") },
    [pscustomobject]@{ Command = "npm run test"; WorkingDirectory = "apps/desktop"; Executable = "npm"; Arguments = @("run", "test") }
)
$expectedPerformanceSteps = @(
    [pscustomobject]@{ Command = "cargo test --release -p ori3-soft --test perf_soft -- --nocapture"; WorkingDirectory = "."; Executable = "cargo"; Arguments = @("test", "--release", "-p", "ori3-soft", "--test", "perf_soft", "--", "--nocapture") },
    [pscustomobject]@{ Command = "cargo test --release -p ori3-rigid --test perf_miura -- --nocapture"; WorkingDirectory = "."; Executable = "cargo"; Arguments = @("test", "--release", "-p", "ori3-rigid", "--test", "perf_miura", "--", "--nocapture") }
)
$expectedSteps = @($expectedChecksSteps) + @($expectedPerformanceSteps)
$totalStages = $expectedSteps.Count + 2

try {
    $verificationRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "verification"))
    $expectedReproRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "verification\ci-repro"))
    $actualSourceParent = [IO.Path]::GetFullPath((Split-Path -Parent $sourceRoot))
    if (-not [string]::Equals($reproRoot, $expectedReproRoot, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals($actualSourceParent, $expectedReproRoot, [StringComparison]::OrdinalIgnoreCase) -or
        $sourceName -notmatch '^source-[0-9a-f]{32}$') {
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
        throw "新規の複製先が既に存在します。古い内容は再利用しません: $sourceRoot"
    }
    New-Item -ItemType Directory -Path $cacheRoot -Force | Out-Null
    Assert-NoReparsePoint @($cacheRoot, $cargoTarget)
    $utf8NoBom = New-Object Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($sentinelPath, "", $utf8NoBom)
    $sentinelCreated = $true

    $repoGitDirectory = Join-Path $repoRoot ".git"
    $repoHeadResult = Invoke-ExternalCapture git @(
        "-c", "safe.directory=$repoRoot",
        "-c", "safe.directory=$repoGitDirectory",
        "-C", $repoRoot,
        "rev-parse", "--verify", "HEAD"
    )
    if ($repoHeadResult.ExitCode -ne 0) {
        $script:failureExitCode = $repoHeadResult.ExitCode
        Write-ExternalOutput $repoHeadResult
        throw "本体のHEAD確認に失敗しました (終了コード: $($repoHeadResult.ExitCode))"
    }
    $repoHead = Get-SingleOutputLine $repoHeadResult "本体のHEAD確認"

    $cloneResult = Invoke-ExternalCapture git @(
        "-c", "safe.directory=$repoRoot",
        "-c", "safe.directory=$repoGitDirectory",
        "-c", "core.excludesFile=$sentinelPath",
        "clone", "--no-hardlinks", "--", $repoRoot, $sourceRoot
    )
    Write-ExternalOutput $cloneResult
    if ($cloneResult.ExitCode -ne 0) {
        $script:failureExitCode = $cloneResult.ExitCode
        throw "HEADの複製に失敗しました (終了コード: $($cloneResult.ExitCode))"
    }

    if ($CloneValidationTestCase -eq "MissingGit") {
        $testGitDirectory = Join-Path $sourceRoot ".git"
        Assert-NoReparsePoint @($sourceRoot, $testGitDirectory)
        Remove-Item -LiteralPath $testGitDirectory -Recurse -Force
        if (Test-Path -LiteralPath $testGitDirectory) {
            throw "[TEST] 複製先の .git を削除できませんでした: $testGitDirectory"
        }
        Write-Host "[TEST] 新規複製の中だけで .git を削除しました" -ForegroundColor Yellow
    }
    elseif ($CloneValidationTestCase -eq "IgnoredFile") {
        $testIgnoredDirectory = Join-Path $sourceRoot "target"
        $testIgnoredFile = Join-Path $testIgnoredDirectory "check-ci-probe.tmp"
        New-Item -ItemType Directory -Path $testIgnoredDirectory | Out-Null
        [IO.File]::WriteAllText($testIgnoredFile, "複製検証用", $utf8NoBom)
        Write-Host "[TEST] 新規複製の中だけに無視対象 target/check-ci-probe.tmp を置きました" -ForegroundColor Yellow
    }

    Assert-ValidClone -CloneRoot $sourceRoot -ExpectedHead $repoHead -EmptyExcludesFile $sentinelPath
    if ($CloneValidationTestCase -eq "Normal") {
        Write-Host "[TEST OK] 正常な新規複製は4確認すべてに合格しました" -ForegroundColor Green
        $script:failureExitCode = 0
    }
    else {
        Write-Stage 2 $totalStages "ci.yml の checks / performance と実行定義を同期確認"
        $workflowPath = Join-Path $sourceRoot ".github\workflows\ci.yml"
        $actualJobNames = @(Get-WorkflowJobNames $workflowPath)
        $expectedJobNames = @("checks", "performance")
        if ($actualJobNames.Count -ne $expectedJobNames.Count -or
            @(Compare-Object -ReferenceObject $expectedJobNames -DifferenceObject $actualJobNames).Count -ne 0) {
            throw "ci.yml のジョブ一覧が変わりました (ci.yml: $($actualJobNames -join ', '), check-ci.ps1: $($expectedJobNames -join ', '))"
        }
        $ciChecksSteps = @(Get-JobRunSteps $workflowPath "checks")
        $ciPerformanceSteps = @(Get-JobRunSteps $workflowPath "performance")
        Assert-CiStepsMatch -Actual $ciChecksSteps -Expected $expectedChecksSteps
        Assert-CiStepsMatch -Actual $ciPerformanceSteps -Expected $expectedPerformanceSteps
        $ciSteps = @($ciChecksSteps) + @($ciPerformanceSteps)

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
        Write-Host "[OK] HEADの内容だけでCI checks / performance両ジョブの全検査に合格しました" -ForegroundColor Green
        $script:failureExitCode = 0
    }
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
    if (Test-Path -LiteralPath $sourceRoot) {
        $cleanupParent = [IO.Path]::GetFullPath((Split-Path -Parent $sourceRoot))
        $sourceItem = Get-Item -LiteralPath $sourceRoot -Force
        if (-not [string]::Equals($cleanupParent, $reproRoot, [StringComparison]::OrdinalIgnoreCase) -or
            $sourceName -notmatch '^source-[0-9a-f]{32}$' -or
            ($sourceItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            Write-Host "[NG] 一時複製の削除前安全確認に失敗しました: $sourceRoot" -ForegroundColor Red
            $script:failureExitCode = 1
        }
        else {
            try {
                [void](Get-DescendantDirectoriesSafely $sourceRoot)
                Remove-Item -LiteralPath $sourceRoot -Recurse -Force
                if (Test-Path -LiteralPath $sourceRoot) {
                    throw "削除後もフォルダが残っています"
                }
            }
            catch {
                Write-Host "[NG] 一時複製を削除できませんでした: $($_.Exception.Message)" -ForegroundColor Red
                $script:failureExitCode = 1
            }
        }
    }
    if ($sentinelCreated) {
        if (-not (Test-Path -LiteralPath $sentinelPath -PathType Leaf)) {
            Write-Host "[NG] 一時除外設定が通常ファイルではありません: $sentinelPath" -ForegroundColor Red
            $script:failureExitCode = 1
        }
        else {
            $sentinelItem = Get-Item -LiteralPath $sentinelPath -Force
            if (($sentinelItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                Write-Host "[NG] 一時除外設定が再解析ポイントへ置き換わっています: $sentinelPath" -ForegroundColor Red
                $script:failureExitCode = 1
            }
            else {
                try {
                    Remove-Item -LiteralPath $sentinelPath -Force
                    if (Test-Path -LiteralPath $sentinelPath) {
                        throw "削除後もファイルが残っています"
                    }
                }
                catch {
                    Write-Host "[NG] 一時除外設定を削除できませんでした: $($_.Exception.Message)" -ForegroundColor Red
                    $script:failureExitCode = 1
                }
            }
        }
    }
    $stopwatch.Stop()
    Write-Host ("所要時間: {0:hh\:mm\:ss}" -f $stopwatch.Elapsed)
}

exit $script:failureExitCode
