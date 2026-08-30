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
$runtimePath = Join-Path $repositoryRoot "scratchpad\watch-agents.runtime.json"
$latestOutputPath = Join-Path $repositoryRoot "scratchpad\watch-agents.latest.log"
$lockPath = Join-Path $repositoryRoot "scratchpad\watch-agents.lock"
$script:AssertionCount = 0
$helperProcess = $null
$script:OwnedWatcherProcesses = New-Object System.Collections.Generic.List[object]

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

function ConvertTo-ProcessArgumentString {
    param([Parameter(Mandatory = $true)][string[]]$Values)

    $parts = foreach ($value in $Values) {
        $escaped = [regex]::Replace($value, '(\\*)"', '$1$1\"')
        $trailingBackslashes = [regex]::Match($escaped, '\\*$').Value
        $escaped = $escaped + $trailingBackslashes
        '"' + $escaped + '"'
    }
    return ($parts -join ' ')
}

function Start-ContinuousWatcher {
    param(
        [Parameter(Mandatory = $true)][string]$PowerShellPath,
        [Parameter(Mandatory = $true)][string]$ConfigPath
    )

    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = $PowerShellPath
    $startInfo.Arguments = ConvertTo-ProcessArgumentString -Values @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $scriptPath,
        "-DefinitionPath", $ConfigPath,
        "-RepositoryRoot", $repositoryRoot,
        "-IntervalMinutes", "10",
        "-StaleAfterMinutes", "40"
    )
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        $process.Dispose()
        throw "継続監視processを起動できません"
    }
    $script:OwnedWatcherProcesses.Add($process)
    return $process
}

function Stop-OwnedWatcherProcesses {
    foreach ($process in $script:OwnedWatcherProcesses) {
        try {
            if (-not $process.HasExited) {
                # この試験が起動したwatch-agents.ps1だけを、保持したProcess objectで終了する。
                $process.Kill()
                [void]$process.WaitForExit(10000)
            }
        }
        catch {
            # cleanupは後続の安全な一時領域検査へ進める。
        }
        finally {
            $process.Dispose()
        }
    }
    $script:OwnedWatcherProcesses.Clear()
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

    Write-Output "[1/4] 更新があれば稼働、40分更新が無ければ停滞と判定する"
    $result = Invoke-Watcher $powerShellCommand.Source $definitionPath $stdoutPath $stderrPath
    if ($result.ExitCode -ne 0) {
        Write-Output $result.Output
    }
    Assert-Equal $result.ExitCode 0 "停滞は監視結果でありスクリプト失敗にしないこと"
    Assert-Contains $result.Output "間隔=10分 / 停滞閾値=40分" "既定の間隔と閾値を表示すること"
    Assert-Contains $result.Output "[稼働] fresh-report" "報告書が新しければ稼働と判定すること"
    Assert-Contains $result.Output "[稼働] fresh-source" "許可ソースが新しければ稼働と判定すること"
    Assert-Contains $result.Output "[停滞] stale-agent" "全成果物が40分より古ければ停滞と判定すること"
    Assert-True (-not (Test-Path -LiteralPath $runtimePath)) "-Onceはruntime stateを作らないこと"
    Assert-True (-not (Test-Path -LiteralPath $latestOutputPath)) "-Onceは固定latest outputを作らないこと"
    Assert-True (-not (Test-Path -LiteralPath $lockPath)) "-Onceはsingleton lockを作らないこと"

    Write-Output "[2/4] 既定動作は表示だけで、ファイルもprocessも変更しない"
    Assert-Contains $result.Output "動作=表示のみ（プロセス停止=0 / ファイル変更=0）" "非破壊の既定動作を表示すること"
    Assert-Contains $result.Output ("PID={0}" -f $helperProcess.Id) "テスト実行ファイルのPIDを補助表示すること"
    Assert-Contains $result.Output "CommandLine=" "補助processのコマンド行を表示すること"
    Assert-True (-not $helperProcess.HasExited) "監視スクリプトがテスト実行processを停止しないこと"
    Assert-True (-not $result.Output.Contains(("PID={0}" -f $PID))) "監視を呼び出したPowerShell自身を補助test processに数えないこと"

    $tokens = $null
    $parseErrors = $null
    $watcherAst = [Management.Automation.Language.Parser]::ParseFile(
        (Resolve-Path -LiteralPath $scriptPath),
        [ref]$tokens,
        [ref]$parseErrors
    )
    Assert-Equal @($parseErrors).Count 0 "production watcherの構文を解析できること"
    $kindFunctions = @($watcherAst.FindAll({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
            $node.Name -eq "Get-ProcessKind"
    }, $true))
    Assert-Equal $kindFunctions.Count 1 "productionのGet-ProcessKindが1件だけ存在すること"
    $classificationProbe = @'
$probeProcess = [pscustomobject]@{
    Name = "powershell.exe"
    ExecutablePath = "C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
    CommandLine = "powershell.exe -Command Get-CimInstance where C:\self-match\target\debug\deps\not-a-real-test.exe"
}
Get-ProcessKind -Process $probeProcess
'@
    $classification = & ([ScriptBlock]::Create($kindFunctions[0].Extent.Text + $classificationProbe))
    Assert-True ($null -eq $classification) "CommandLineでtest exeを言及するPowerShellをテストに数えないこと"
    foreach ($path in $watchedPaths) {
        $after = Get-FileFingerprint $path
        Assert-Equal $after.Length $before[$path].Length "監視対象の長さを変えないこと: $path"
        Assert-Equal $after.LastWriteTicks $before[$path].LastWriteTicks "監視対象の更新時刻を変えないこと: $path"
        Assert-Equal $after.Hash $before[$path].Hash "監視対象の内容を変えないこと: $path"
    }

    Write-Output "[3/4] 存在しない監視パスを日本語で監視不能と知らせる"
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

    Write-Output "[4/4] 継続監視は固定UTF-8 runtimeを発行し、二重起動を拒否する"
    [IO.File]::WriteAllText(
        $definitionPath,
        (([ordered]@{
            agents = @(
                [ordered]@{
                    name = "continuous-agent"
                    reportPath = "scratchpad/fresh-report.md"
                    sourcePaths = @("src/fresh-report")
                }
            )
        }) | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )
    $continuous = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source -ConfigPath $definitionPath
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline -and
        (-not (Test-Path -LiteralPath $runtimePath -PathType Leaf) -or
         -not (Test-Path -LiteralPath $latestOutputPath -PathType Leaf))) {
        if ($continuous.HasExited) {
            break
        }
        Start-Sleep -Milliseconds 100
    }
    if ($continuous.HasExited) {
        $continuousStdout = $continuous.StandardOutput.ReadToEnd()
        $continuousStderr = $continuous.StandardError.ReadToEnd()
        Write-Output ("継続監視の早期終了: exit={0}`nstdout:`n{1}`nstderr:`n{2}" -f
            $continuous.ExitCode, $continuousStdout, $continuousStderr)
    }
    Assert-True (-not $continuous.HasExited) "継続監視がruntime発行後も稼働していること"
    Assert-True (Test-Path -LiteralPath $runtimePath -PathType Leaf) "固定runtime stateを発行すること"
    Assert-True (Test-Path -LiteralPath $latestOutputPath -PathType Leaf) "固定latest outputを発行すること"
    Assert-True (Test-Path -LiteralPath $lockPath -PathType Leaf) "固定singleton lockを発行すること"

    $strictUtf8 = [Text.UTF8Encoding]::new($false, $true)
    $latestText = $strictUtf8.GetString([IO.File]::ReadAllBytes($latestOutputPath))
    Assert-Contains $latestText "[停滞監視]" "latest outputがUTF-8で読めること"
    Assert-Contains $latestText "[走査完了]" "latest outputが完了scanだけを表すこと"
    $runtimeState = [IO.File]::ReadAllText($runtimePath, [Text.Encoding]::UTF8) | ConvertFrom-Json
    Assert-Equal ([int]$runtimeState.pid) $continuous.Id "runtime stateがwatcher PIDを記録すること"
    Assert-Equal ([string]$runtimeState.mode) "continuous" "runtime stateが継続modeを記録すること"
    Assert-Equal ([int]$runtimeState.intervalMinutes) 10 "runtime stateが10分間隔を記録すること"
    Assert-Equal ([int]$runtimeState.staleAfterMinutes) 40 "runtime stateが40分閾値を記録すること"
    Assert-Equal ([string]$runtimeState.outputPath) ([IO.Path]::GetFullPath($latestOutputPath)) "runtime stateが固定output実パスを記録すること"
    Assert-True (-not [string]::IsNullOrWhiteSpace([string]$runtimeState.outputSha256)) "runtime stateがoutput hashを記録すること"
    Assert-True (-not [string]::IsNullOrWhiteSpace([string]$runtimeState.definitionSha256)) "runtime stateがdefinition hashを記録すること"

    $lockHeld = $false
    try {
        $lockProbe = New-Object IO.FileStream($lockPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        $lockProbe.Dispose()
    }
    catch [IO.IOException] {
        $nativeCode = ($_.Exception.GetBaseException().HResult -band 0xFFFF)
        $lockHeld = ($nativeCode -eq 32 -or $nativeCode -eq 33)
    }
    Assert-True $lockHeld "継続監視processがsingleton lockを保持すること"

    $duplicate = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source -ConfigPath $definitionPath
    $duplicateExited = $duplicate.WaitForExit(10000)
    Assert-True $duplicateExited "二重起動が待ち続けず終了すること"
    if ($duplicateExited) {
        Assert-True ($duplicate.ExitCode -ne 0) "二重起動が非0で拒否されること"
    }

    Write-Output ("watch-agents self-test passed: 4 cases, {0} assertions" -f $script:AssertionCount)
}
finally {
    Stop-OwnedWatcherProcesses
    if ($null -ne $helperProcess) {
        [void]$helperProcess.WaitForExit(15000)
        $helperProcess.Dispose()
    }
    Remove-TestSandbox
}
