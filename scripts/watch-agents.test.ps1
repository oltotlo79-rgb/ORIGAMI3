[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$scriptPath = Join-Path $PSScriptRoot "watch-agents.ps1"
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-watch-agents-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [IO.Path]::GetFullPath((Join-Path $tempBase $sandboxName))
$repositoryRoot = Join-Path $sandboxRoot "repo"
$definitionPath = Join-Path $sandboxRoot "agents.json"
$stdoutPath = Join-Path $sandboxRoot "stdout.txt"
$stderrPath = Join-Path $sandboxRoot "stderr.txt"
$script:AssertionCount = 0
$helperProcess = $null

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

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)][AllowNull()]$Actual,
        [Parameter(Mandatory = $true)][AllowNull()]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $script:AssertionCount += 1
    if ($Actual -ne $Expected) {
        throw "ASSERTION FAILED: $Message (expected=$Expected, actual=$Actual)"
    }
}

function Assert-Contains {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $script:AssertionCount += 1
    if (-not $Text.Contains($Expected)) {
        throw "ASSERTION FAILED: $Message (missing='$Expected')`n$Text"
    }
}

function New-TestFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][DateTime]$LastWriteTimeUtc,
        [Parameter(Mandatory = $true)][string]$Content
    )

    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $Path))
    [IO.File]::WriteAllText($Path, $Content, [Text.UTF8Encoding]::new($false))
    [IO.File]::SetLastWriteTimeUtc($Path, $LastWriteTimeUtc)
}

function Get-FileFingerprint {
    param([Parameter(Mandatory = $true)][string]$Path)

    $item = Get-Item -LiteralPath $Path
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $stream = [IO.File]::OpenRead($Path)
        try {
            $hash = [BitConverter]::ToString($sha.ComputeHash($stream)).Replace("-", "")
        }
        finally {
            $stream.Dispose()
        }
    }
    finally {
        $sha.Dispose()
    }
    [pscustomobject]@{
        Length = $item.Length
        LastWriteTicks = $item.LastWriteTimeUtc.Ticks
        Hash = $hash
    }
}

function Invoke-Watcher {
    param(
        [Parameter(Mandatory = $true)][string]$PowerShellPath,
        [Parameter(Mandatory = $true)][string]$ConfigPath,
        [Parameter(Mandatory = $true)][string]$OutPath,
        [Parameter(Mandatory = $true)][string]$ErrPath
    )

    $arguments = @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $scriptPath,
        "-DefinitionPath", $ConfigPath,
        "-RepositoryRoot", $repositoryRoot,
        "-Once"
    )
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        & $PowerShellPath @arguments 1> $OutPath 2> $ErrPath
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }

    $parts = New-Object System.Collections.Generic.List[string]
    if (Test-Path -LiteralPath $OutPath -PathType Leaf) {
        $parts.Add([IO.File]::ReadAllText($OutPath))
    }
    if (Test-Path -LiteralPath $ErrPath -PathType Leaf) {
        $parts.Add([IO.File]::ReadAllText($ErrPath))
    }
    [pscustomobject]@{
        ExitCode = $exitCode
        Output = ($parts -join "`n")
    }
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $sandboxRoot)) {
        return
    }
    $resolved = [IO.Path]::GetFullPath($sandboxRoot).TrimEnd([char[]]"\/")
    $parent = [IO.Path]::GetDirectoryName($resolved)
    $leaf = [IO.Path]::GetFileName($resolved)
    if (($parent -ne $tempBase) -or
        (-not [regex]::IsMatch($leaf, '^ori3-watch-agents-test-[0-9a-f]{32}$', [Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
        throw "安全でない一時領域の削除を拒否しました: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
    throw "検査本体が見つかりません: $scriptPath"
}
$powerShellCommand = Get-Command powershell.exe, pwsh.exe, pwsh -ErrorAction SilentlyContinue | Select-Object -First 1
if ($null -eq $powerShellCommand) {
    throw "隔離検査を起動するPowerShellが見つかりません"
}

[void][IO.Directory]::CreateDirectory($repositoryRoot)
try {
    $now = [DateTime]::UtcNow
    $fresh = $now.AddMinutes(-5)
    $old = $now.AddMinutes(-55)

    $freshReport = Join-Path $repositoryRoot "scratchpad\fresh-report.md"
    $freshReportSource = Join-Path $repositoryRoot "src\fresh-report\value.rs"
    $freshSourceReport = Join-Path $repositoryRoot "scratchpad\fresh-source.md"
    $freshSource = Join-Path $repositoryRoot "src\fresh-source\value.rs"
    $staleReport = Join-Path $repositoryRoot "scratchpad\stale.md"
    $staleSource = Join-Path $repositoryRoot "src\stale\value.rs"
    New-TestFile $freshReport $fresh "fresh report"
    New-TestFile $freshReportSource $old "old source"
    New-TestFile $freshSourceReport $old "old report"
    New-TestFile $freshSource $fresh "fresh source"
    New-TestFile $staleReport $old "old report"
    New-TestFile $staleSource $old "old source"

    $definition = [ordered]@{
        agents = @(
            [ordered]@{
                name = "fresh-report"
                reportPath = "scratchpad/fresh-report.md"
                sourcePaths = @("src/fresh-report")
            },
            [ordered]@{
                name = "fresh-source"
                reportPath = "scratchpad/fresh-source.md"
                sourcePaths = @("src/fresh-source")
            },
            [ordered]@{
                name = "stale-agent"
                reportPath = "scratchpad/stale.md"
                sourcePaths = @("src/stale")
            }
        )
    }
    [IO.File]::WriteAllText(
        $definitionPath,
        ($definition | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )

    $watchedPaths = @(
        $freshReport, $freshReportSource, $freshSourceReport,
        $freshSource, $staleReport, $staleSource, $definitionPath
    )
    $before = @{}
    foreach ($path in $watchedPaths) {
        $before[$path] = Get-FileFingerprint $path
    }

    $helperPath = Join-Path $sandboxRoot "ori3-target-watch-test\debug\deps\watch-agents-fixture-tests.exe"
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $helperPath))
    [IO.File]::Copy((Join-Path $env:SystemRoot "System32\cmd.exe"), $helperPath)
    $helperProcess = Start-Process -FilePath $helperPath -ArgumentList '/d /c "ping.exe -n 8 127.0.0.1 >nul"' -WindowStyle Hidden -PassThru
    Start-Sleep -Milliseconds 200
    Assert-True (-not $helperProcess.HasExited) "補助processのfixtureが監視中に実行されていること"

    Write-Output "[1/3] 更新があれば稼働、40分更新が無ければ停滞と判定する"
    $result = Invoke-Watcher $powerShellCommand.Source $definitionPath $stdoutPath $stderrPath
    if ($result.ExitCode -ne 0) {
        Write-Output $result.Output
    }
    Assert-Equal $result.ExitCode 0 "停滞は監視結果でありスクリプト失敗にしないこと"
    Assert-Contains $result.Output "間隔=10分 / 停滞閾値=40分" "既定の間隔と閾値を表示すること"
    Assert-Contains $result.Output "[稼働] fresh-report" "報告書が新しければ稼働と判定すること"
    Assert-Contains $result.Output "[稼働] fresh-source" "許可ソースが新しければ稼働と判定すること"
    Assert-Contains $result.Output "[停滞] stale-agent" "全成果物が40分より古ければ停滞と判定すること"

    Write-Output "[2/3] 既定動作は表示だけで、ファイルもprocessも変更しない"
    Assert-Contains $result.Output "動作=表示のみ（プロセス停止=0 / ファイル変更=0）" "非破壊の既定動作を表示すること"
    Assert-Contains $result.Output ("PID={0}" -f $helperProcess.Id) "テスト実行ファイルのPIDを補助表示すること"
    Assert-Contains $result.Output "CommandLine=" "補助processのコマンド行を表示すること"
    Assert-True (-not $helperProcess.HasExited) "監視スクリプトがテスト実行processを停止しないこと"
    foreach ($path in $watchedPaths) {
        $after = Get-FileFingerprint $path
        Assert-Equal $after.Length $before[$path].Length "監視対象の長さを変えないこと: $path"
        Assert-Equal $after.LastWriteTicks $before[$path].LastWriteTicks "監視対象の更新時刻を変えないこと: $path"
        Assert-Equal $after.Hash $before[$path].Hash "監視対象の内容を変えないこと: $path"
    }

    Write-Output "[3/3] 存在しない監視パスを日本語で監視不能と知らせる"
    $missingDefinition = [ordered]@{
        agents = @(
            [ordered]@{
                name = "missing-agent"
                reportPath = "scratchpad/fresh-report.md"
                sourcePaths = @("src/does-not-exist")
            }
        )
    }
    [IO.File]::WriteAllText(
        $definitionPath,
        ($missingDefinition | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )
    $result = Invoke-Watcher $powerShellCommand.Source $definitionPath $stdoutPath $stderrPath
    Assert-Equal $result.ExitCode 0 "不存在パスも表示だけで知らせること"
    Assert-Contains $result.Output "[監視不能] missing-agent" "監視不能の担当名を表示すること"
    Assert-Contains $result.Output "存在しません" "不存在理由を日本語で表示すること"

    Write-Output ("watch-agents self-test passed: 3 cases, {0} assertions" -f $script:AssertionCount)
}
finally {
    if ($null -ne $helperProcess) {
        [void]$helperProcess.WaitForExit(15000)
        $helperProcess.Dispose()
    }
    Remove-TestSandbox
}
