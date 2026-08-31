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

function Wait-ForWatcherRuntime {
    param(
        [Parameter(Mandatory = $true)]$Process,
        [int]$TimeoutSeconds = 20
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($Process.HasExited) {
            $stdout = $Process.StandardOutput.ReadToEnd()
            $stderr = $Process.StandardError.ReadToEnd()
            throw "継続監視がruntime発行前に終了しました: exit=$($Process.ExitCode)`nstdout:`n$stdout`nstderr:`n$stderr"
        }
        if (Test-Path -LiteralPath $runtimePath -PathType Leaf) {
            try {
                $runtimeStream = New-Object IO.FileStream(
                    $runtimePath,
                    [IO.FileMode]::Open,
                    [IO.FileAccess]::Read,
                    ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
                )
                try {
                    $runtimeReader = New-Object IO.StreamReader(
                        $runtimeStream,
                        [Text.UTF8Encoding]::new($false, $true),
                        $false
                    )
                    try {
                        $state = $runtimeReader.ReadToEnd() | ConvertFrom-Json
                    }
                    finally {
                        $runtimeReader.Dispose()
                    }
                }
                finally {
                    $runtimeStream.Dispose()
                }
                if ([int]$state.pid -eq $Process.Id -and [int]$state.scanSequence -ge 1) {
                    return $state
                }
            }
            catch {
                # atomic replaceの境界で旧stateを読んだ場合は、同じproduction出力を再読する。
            }
        }
        Start-Sleep -Milliseconds 100
    }
    throw "継続監視のruntime発行を${TimeoutSeconds}秒以内に確認できません: PID=$($Process.Id)"
}

function Stop-OwnedWatcherProcess {
    param([Parameter(Mandatory = $true)]$Process)

    if (-not $Process.HasExited) {
        # この試験がProcess objectを保持するwatcherだけを止める。production watcherには停止処理を足さない。
        $Process.Kill()
        if (-not $Process.WaitForExit(10000)) {
            throw "試験所有watcherが10秒以内に終了しません: PID=$($Process.Id)"
        }
    }
}

function ConvertTo-TestCanonicalPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    return [IO.Path]::GetFullPath($Path).Replace(
        [IO.Path]::AltDirectorySeparatorChar,
        [IO.Path]::DirectorySeparatorChar
    )
}

function ConvertTo-TestBase64Utf8 {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    return [Convert]::ToBase64String([Text.UTF8Encoding]::new($false).GetBytes($Text))
}

function Get-TestSha256HexFromText {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Text)
        return ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace("-", "").ToLowerInvariant()
    }
    finally {
        $sha.Dispose()
    }
}

function Get-TestAgentKey {
    param(
        [Parameter(Mandatory = $true)][string]$ReportPath,
        [Parameter(Mandatory = $true)][string[]]$SourcePaths
    )

    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("version=1")
    $lines.Add("reportPath={0}" -f (ConvertTo-TestBase64Utf8 -Text (ConvertTo-TestCanonicalPath -Path $ReportPath)))
    foreach ($sourcePath in $SourcePaths) {
        $lines.Add("sourcePath={0}" -f (ConvertTo-TestBase64Utf8 -Text (ConvertTo-TestCanonicalPath -Path $sourcePath)))
    }
    return Get-TestSha256HexFromText -Text ($lines.ToArray() -join "`n")
}

function Get-TestProblemDigest {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Problems)

    $lines = foreach ($problem in $Problems) {
        "path={0}`tproblem={1}" -f
            (ConvertTo-TestBase64Utf8 -Text (ConvertTo-TestCanonicalPath -Path ([string]$problem.Path))),
            (ConvertTo-TestBase64Utf8 -Text ([string]$problem.Problem))
    }
    return Get-TestSha256HexFromText -Text (@($lines) -join "`n")
}

function Get-TestIncidentId {
    param(
        [Parameter(Mandatory = $true)][string]$AgentKey,
        [Parameter(Mandatory = $true)][string]$Status,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$LatestPath,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$LatestWriteUtc,
        [Parameter(Mandatory = $true)][string]$ProblemDigest
    )

    return Get-TestSha256HexFromText -Text (@(
            "version=1"
            "agentKey=$AgentKey"
            "status=$Status"
            "latestPath=$(ConvertTo-TestBase64Utf8 -Text $LatestPath)"
            "latestWriteUtc=$LatestWriteUtc"
            "problemDigest=$ProblemDigest"
        ) -join "`n")
}

function Get-TestAgentStatesCanonicalText {
    param([Parameter(Mandatory = $true)][object[]]$AgentStates)

    $lines = foreach ($state in $AgentStates) {
        @(
            "agentKey=$([string]$state.agentKey)"
            "name=$(ConvertTo-TestBase64Utf8 -Text ([string]$state.name))"
            "status=$([string]$state.status)"
            "latestPath=$(ConvertTo-TestBase64Utf8 -Text ([string]$state.latestPath))"
            "latestWriteUtc=$([string]$state.latestWriteUtc)"
            "problemDigest=$([string]$state.problemDigest)"
            "incidentId=$([string]$state.incidentId)"
        ) -join "`t"
    }
    return @($lines) -join "`n"
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
    $old = $now.AddMinutes(-61)

    $freshReport = Join-Path $repositoryRoot "scratchpad\fresh-report.md"
    $freshReportSource = Join-Path $repositoryRoot "src\fresh-report\value.rs"
    $freshSourceReport = Join-Path $repositoryRoot "scratchpad\fresh-source.md"
    $freshSource = Join-Path $repositoryRoot "src\fresh-source\value.rs"
    $freshSourceTieFirst = Join-Path $repositoryRoot "src\fresh-source\a.rs"
    $staleReport = Join-Path $repositoryRoot "scratchpad\stale.md"
    $staleSource = Join-Path $repositoryRoot "src\stale\value.rs"
    $futureReport = Join-Path $repositoryRoot "scratchpad\future.md"
    $futureSource = Join-Path $repositoryRoot "src\future\value.rs"
    $futureWrite = $now.AddMinutes(10)
    New-TestFile $freshReport $fresh "fresh report"
    New-TestFile $freshReportSource $old "old source"
    New-TestFile $freshSourceReport $old "old report"
    New-TestFile $freshSource $fresh "fresh source"
    New-TestFile $freshSourceTieFirst $fresh "same-time source selected by canonical path"
    New-TestFile $staleReport $old "old report"
    New-TestFile $staleSource $old "old source"
    New-TestFile $futureReport $futureWrite "future report"
    New-TestFile $futureSource $fresh "ordinary source"

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
        $freshSource, $freshSourceTieFirst, $staleReport, $staleSource,
        $futureReport, $futureSource, $definitionPath
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

    Write-Output "[1/8] 更新があれば稼働、61分更新が無ければ停滞と判定する"
    $result = Invoke-Watcher $powerShellCommand.Source $definitionPath $stdoutPath $stderrPath
    if ($result.ExitCode -ne 0) {
        Write-Output $result.Output
    }
    Assert-Equal $result.ExitCode 0 "停滞は監視結果でありスクリプト失敗にしないこと"
    Assert-Contains $result.Output "間隔=10分 / 停滞閾値=40分" "既定の間隔と閾値を表示すること"
    Assert-Contains $result.Output "[稼働] fresh-report" "報告書が新しければ稼働と判定すること"
    Assert-Contains $result.Output "[稼働] fresh-source" "許可ソースが新しければ稼働と判定すること"
    Assert-Contains $result.Output $freshSourceTieFirst "directory内が同時刻ならcanonical path昇順のfileを選ぶこと"
    Assert-True (-not $result.Output.Contains($freshSource)) "同時刻tieで列挙順依存の後方fileを選ばないこと"
    Assert-Contains $result.Output "[停滞] stale-agent" "全成果物が40分より古ければ停滞と判定すること"
    Assert-Contains $result.Output "AGENT_WATCH_STATUS schema=2 total=3 active=2 stalled=1 unmonitorable=0 states_sha256=" "-Onceも対象件数とstatus件数を機械可読で表示すること"
    Assert-Contains $result.Output "AGENT_WATCH_INCIDENT schema=2 status=stalled incident=" "-Onceも停滞incidentを機械可読で表示すること"
    Assert-True (-not (Test-Path -LiteralPath $runtimePath)) "-Onceはruntime stateを作らないこと"
    Assert-True (-not (Test-Path -LiteralPath $latestOutputPath)) "-Onceは固定latest outputを作らないこと"
    Assert-True (-not (Test-Path -LiteralPath $lockPath)) "-Onceはsingleton lockを作らないこと"

    Write-Output "[2/8] 既定動作は表示だけで、ファイルもprocessも変更しない"
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

    Write-Output "[3/8] 存在しない監視パスを日本語で監視不能と知らせる"
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
    Assert-Contains $result.Output "AGENT_WATCH_STATUS schema=2 total=1 active=0 stalled=0 unmonitorable=1 states_sha256=" "監視不能をsummary件数へ反映すること"
    Assert-Contains $result.Output "AGENT_WATCH_INCIDENT schema=2 status=unmonitorable incident=" "監視不能incidentを表示すること"

    Write-Output "[4/8] production継続監視が61分無変化をschema 2の停滞incidentとして発行する"
    [IO.File]::WriteAllText(
        $definitionPath,
        (([ordered]@{
            agents = @(
                [ordered]@{
                    name = "stale-agent"
                    reportPath = "scratchpad/stale.md"
                    sourcePaths = @("src/stale", "src/fresh-report")
                }
            )
        }) | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )
    $continuousInputs = @($staleReport, $staleSource, $freshReportSource, $definitionPath)
    $continuousBefore = @{}
    foreach ($path in $continuousInputs) {
        $continuousBefore[$path] = Get-FileFingerprint $path
    }
    $continuous = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source -ConfigPath $definitionPath
    $runtimeState = Wait-ForWatcherRuntime -Process $continuous
    Assert-True (-not $continuous.HasExited) "継続監視がruntime発行後も稼働していること"
    Assert-True (Test-Path -LiteralPath $runtimePath -PathType Leaf) "固定runtime stateを発行すること"
    Assert-True (Test-Path -LiteralPath $latestOutputPath -PathType Leaf) "固定latest outputを発行すること"
    Assert-True (Test-Path -LiteralPath $lockPath -PathType Leaf) "固定singleton lockを発行すること"

    $strictUtf8 = [Text.UTF8Encoding]::new($false, $true)
    $latestText = $strictUtf8.GetString([IO.File]::ReadAllBytes($latestOutputPath))
    Assert-Contains $latestText "[停滞監視]" "latest outputがUTF-8で読めること"
    Assert-Contains $latestText "[走査完了]" "latest outputが完了scanだけを表すこと"
    Assert-Equal ([int]$runtimeState.schemaVersion) 2 "runtime schema 2を発行すること"
    Assert-Equal ([int]$runtimeState.pid) $continuous.Id "runtime stateがwatcher PIDを記録すること"
    Assert-Equal ([string]$runtimeState.mode) "continuous" "runtime stateが継続modeを記録すること"
    Assert-Equal ([int]$runtimeState.intervalMinutes) 10 "runtime stateが10分間隔を記録すること"
    Assert-Equal ([int]$runtimeState.staleAfterMinutes) 40 "runtime stateが40分閾値を記録すること"
    Assert-Equal ([string]$runtimeState.outputPath) ([IO.Path]::GetFullPath($latestOutputPath)) "runtime stateが固定output実パスを記録すること"
    Assert-Equal ([string]$runtimeState.outputSha256) ((Get-FileFingerprint $latestOutputPath).Hash.ToLowerInvariant()) "runtime stateがlatest outputの実hashを記録すること"
    Assert-True (-not [string]::IsNullOrWhiteSpace([string]$runtimeState.definitionSha256)) "runtime stateがdefinition hashを記録すること"
    Assert-Equal ([int]$runtimeState.agentCount) 1 "runtimeが監視対象総数を記録すること"
    Assert-Equal ([int]$runtimeState.activeCount) 0 "61分古い入力をactiveに数えないこと"
    Assert-Equal ([int]$runtimeState.stalledCount) 1 "61分古い入力をstalledに数えること"
    Assert-Equal ([int]$runtimeState.unmonitorableCount) 0 "読める古い入力をunmonitorableに数えないこと"
    $stalledStates = @($runtimeState.agentStates)
    Assert-Equal $stalledStates.Count 1 "全担当のstructured stateを発行すること"
    $stalledState = $stalledStates[0]
    Assert-Equal ([string]$stalledState.status) "stalled" "61分古い担当のmachine statusがstalledであること"
    $orderedSourcePaths = @(
        (Join-Path $repositoryRoot "src\stale"),
        (Join-Path $repositoryRoot "src\fresh-report")
    )
    $expectedAgentKey = Get-TestAgentKey -ReportPath $staleReport -SourcePaths $orderedSourcePaths
    Assert-Equal ([string]$stalledState.agentKey) $expectedAgentKey "agentKeyをreportPathと定義順sourcePathsから再計算できること"
    $reversedAgentKey = Get-TestAgentKey -ReportPath $staleReport -SourcePaths @(
        $orderedSourcePaths[1],
        $orderedSourcePaths[0]
    )
    Assert-True ($reversedAgentKey -cne $expectedAgentKey) "sourcePathsの定義順をagentKeyへ含めること"
    Assert-Equal ([string]$stalledState.problemDigest) (Get-TestProblemDigest -Problems @()) "問題0件も固定digestで表すこと"
    Assert-Equal ([string]$stalledState.latestPath) (ConvertTo-TestCanonicalPath -Path $staleReport) "同時刻ではcanonical path昇順でlatestを決定すること"
    $expectedIncident = Get-TestIncidentId `
        -AgentKey $expectedAgentKey `
        -Status "stalled" `
        -LatestPath ([string]$stalledState.latestPath) `
        -LatestWriteUtc ([string]$stalledState.latestWriteUtc) `
        -ProblemDigest ([string]$stalledState.problemDigest)
    Assert-Equal ([string]$stalledState.incidentId) $expectedIncident "停滞incidentをscan時刻なしのcanonical入力から再計算できること"
    Assert-True ([regex]::IsMatch([string]$stalledState.incidentId, '^[0-9a-f]{64}$')) "停滞incidentがlowercase SHA-256であること"
    $stalledCanonical = Get-TestAgentStatesCanonicalText -AgentStates $stalledStates
    $expectedStatesSha = Get-TestSha256HexFromText -Text $stalledCanonical
    Assert-Equal ([string]$runtimeState.agentStatesSha256) $expectedStatesSha "agentStates hashを固定field順・UTF-8・LFから再計算できること"
    $expectedSummary = "AGENT_WATCH_STATUS schema=2 total=1 active=0 stalled=1 unmonitorable=0 states_sha256=$expectedStatesSha"
    Assert-Contains $latestText $expectedSummary "latest outputのsummaryをruntime件数とhashに一致させること"
    Assert-Contains $latestText ("AGENT_WATCH_INCIDENT schema=2 status=stalled incident={0} agent_key={1}" -f $expectedIncident, $expectedAgentKey) "latest outputへ停滞incident IDを出すこと"
    foreach ($path in $continuousInputs) {
        $after = Get-FileFingerprint $path
        Assert-Equal $after.Length $continuousBefore[$path].Length "継続監視が入力長を変えないこと: $path"
        Assert-Equal $after.LastWriteTicks $continuousBefore[$path].LastWriteTicks "継続監視が入力時刻を変えないこと: $path"
        Assert-Equal $after.Hash $continuousBefore[$path].Hash "継続監視が入力内容を変えないこと: $path"
    }

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
    Stop-OwnedWatcherProcess -Process $continuous

    Write-Output "[5/8] 同じ停滞episodeはprocess・scanが変わっても同じincident IDになる"
    $repeat = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source -ConfigPath $definitionPath
    $repeatRuntime = Wait-ForWatcherRuntime -Process $repeat
    $repeatState = @($repeatRuntime.agentStates)[0]
    Assert-True ([string]$repeatRuntime.instanceId -cne [string]$runtimeState.instanceId) "再起動したwatcherのinstanceIdが変わること"
    Assert-Equal ([string]$repeatState.incidentId) $expectedIncident "同じ停滞episodeでincident IDを固定すること"
    Assert-Equal ([string]$repeatRuntime.agentStatesSha256) $expectedStatesSha "scan時刻とsequenceをagentStates hashへ含めないこと"
    Stop-OwnedWatcherProcess -Process $repeat

    Write-Output "[6/8] 成果更新後はactiveとなり、旧停滞incidentをruntimeとlatest outputから失効させる"
    $progressTime = [DateTime]::UtcNow
    New-TestFile $staleReport $progressTime "progress after stalled episode"
    $active = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source -ConfigPath $definitionPath
    $activeRuntime = Wait-ForWatcherRuntime -Process $active
    $activeState = @($activeRuntime.agentStates)[0]
    Assert-Equal ([int]$activeRuntime.activeCount) 1 "成果更新後をactiveに数えること"
    Assert-Equal ([int]$activeRuntime.stalledCount) 0 "成果更新後をstalledに残さないこと"
    Assert-Equal ([int]$activeRuntime.unmonitorableCount) 0 "成果更新後をunmonitorableにしないこと"
    Assert-Equal ([string]$activeState.status) "active" "成果更新後のstatusがactiveであること"
    Assert-Equal ([string]$activeState.agentKey) $expectedAgentKey "成果更新で担当identityを変えないこと"
    Assert-Equal ([string]$activeState.incidentId) "" "activeにはincident IDを持たせないこと"
    Assert-True ([string]$activeRuntime.agentStatesSha256 -cne $expectedStatesSha) "成果更新をagentStates hashへ反映すること"
    $activeLatestText = $strictUtf8.GetString([IO.File]::ReadAllBytes($latestOutputPath))
    Assert-Contains $activeLatestText "AGENT_WATCH_STATUS schema=2 total=1 active=1 stalled=0 unmonitorable=0 states_sha256=" "成果更新後のsummaryがactiveを示すこと"
    Assert-True (-not $activeLatestText.Contains($expectedIncident)) "成果更新後のlatest outputに旧incidentを残さないこと"
    Stop-OwnedWatcherProcess -Process $active

    Write-Output "[7/8] 監視不能にも安定incidentを付け、件数・状態hashの改変を検出可能にする"
    [IO.File]::WriteAllText(
        $definitionPath,
        (([ordered]@{
            agents = @(
                [ordered]@{
                    name = "missing-agent"
                    reportPath = "scratchpad/stale.md"
                    sourcePaths = @("src/does-not-exist")
                }
            )
        }) | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )
    $unmonitorable = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source -ConfigPath $definitionPath
    $unmonitorableRuntime = Wait-ForWatcherRuntime -Process $unmonitorable
    $unmonitorableStates = @($unmonitorableRuntime.agentStates)
    $unmonitorableState = $unmonitorableStates[0]
    Assert-Equal ([int]$unmonitorableRuntime.activeCount) 0 "監視不能をactiveに数えないこと"
    Assert-Equal ([int]$unmonitorableRuntime.stalledCount) 0 "監視不能をstalledに数えないこと"
    Assert-Equal ([int]$unmonitorableRuntime.unmonitorableCount) 1 "監視不能件数を1と記録すること"
    Assert-Equal ([string]$unmonitorableState.status) "unmonitorable" "不存在pathのmachine statusがunmonitorableであること"
    $missingPath = Join-Path $repositoryRoot "src\does-not-exist"
    $expectedMissingDigest = Get-TestProblemDigest -Problems @(
        [pscustomobject]@{ Path = $missingPath; Problem = "存在しません" }
    )
    Assert-Equal ([string]$unmonitorableState.problemDigest) $expectedMissingDigest "監視不能理由のpathと本文をdigestへ含めること"
    $expectedUnmonitorableIncident = Get-TestIncidentId `
        -AgentKey ([string]$unmonitorableState.agentKey) `
        -Status "unmonitorable" `
        -LatestPath ([string]$unmonitorableState.latestPath) `
        -LatestWriteUtc ([string]$unmonitorableState.latestWriteUtc) `
        -ProblemDigest $expectedMissingDigest
    Assert-Equal ([string]$unmonitorableState.incidentId) $expectedUnmonitorableIncident "監視不能incidentをcanonical入力から再計算できること"
    $unmonitorableCanonical = Get-TestAgentStatesCanonicalText -AgentStates $unmonitorableStates
    $unmonitorableStatesSha = Get-TestSha256HexFromText -Text $unmonitorableCanonical
    Assert-Equal ([string]$unmonitorableRuntime.agentStatesSha256) $unmonitorableStatesSha "監視不能stateも全体hashへ含めること"
    $tamperedCanonical = $unmonitorableCanonical.Replace("status=unmonitorable", "status=active")
    Assert-True ((Get-TestSha256HexFromText -Text $tamperedCanonical) -cne [string]$unmonitorableRuntime.agentStatesSha256) "statusを書き換えるとstate hashが一致しないこと"
    $unmonitorableLatestText = $strictUtf8.GetString([IO.File]::ReadAllBytes($latestOutputPath))
    Assert-Contains $unmonitorableLatestText ("AGENT_WATCH_STATUS schema=2 total=1 active=0 stalled=0 unmonitorable=1 states_sha256={0}" -f $unmonitorableStatesSha) "監視不能summaryをruntimeと一致させること"
    Assert-Contains $unmonitorableLatestText ("AGENT_WATCH_INCIDENT schema=2 status=unmonitorable incident={0}" -f $expectedUnmonitorableIncident) "監視不能incident IDをlatest outputへ出すこと"
    Stop-OwnedWatcherProcess -Process $unmonitorable

    Write-Output "[8/8] 判定時刻より2分を超えて未来のmtimeはactiveにせず、安定した監視不能incidentにする"
    [IO.File]::WriteAllText(
        $definitionPath,
        (([ordered]@{
            agents = @(
                [ordered]@{
                    name = "future-agent"
                    reportPath = "scratchpad/future.md"
                    sourcePaths = @("src/future")
                }
            )
        }) | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )
    $futureInputs = @($futureReport, $futureSource, $definitionPath)
    $futureBefore = @{}
    foreach ($path in $futureInputs) {
        $futureBefore[$path] = Get-FileFingerprint $path
    }
    $futureWatcher = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source -ConfigPath $definitionPath
    $futureRuntime = Wait-ForWatcherRuntime -Process $futureWatcher
    $futureStates = @($futureRuntime.agentStates)
    $futureState = $futureStates[0]
    Assert-Equal ([int]$futureRuntime.activeCount) 0 "未来mtimeをactiveに数えないこと"
    Assert-Equal ([int]$futureRuntime.stalledCount) 0 "未来mtimeをstalledに数えないこと"
    Assert-Equal ([int]$futureRuntime.unmonitorableCount) 1 "未来mtimeをunmonitorableに数えること"
    Assert-Equal ([string]$futureState.status) "unmonitorable" "未来mtimeのmachine statusをunmonitorableにすること"
    Assert-Equal ([string]$futureState.latestPath) (ConvertTo-TestCanonicalPath -Path $futureReport) "未来mtimeの対象fileをlatestPathへ記録すること"
    $recordedFutureWriteUtc = (Get-Item -LiteralPath $futureReport).LastWriteTimeUtc.ToString(
        "o",
        [Globalization.CultureInfo]::InvariantCulture
    )
    $futureProblemText = "更新時刻が許容範囲の2分を超えて未来です: lastWriteUtc=$recordedFutureWriteUtc"
    $expectedFutureProblemDigest = Get-TestProblemDigest -Problems @(
        [pscustomobject]@{ Path = $futureReport; Problem = $futureProblemText }
    )
    Assert-Equal ([string]$futureState.problemDigest) $expectedFutureProblemDigest "problemへ判定時刻を入れず、対象mtimeだけを固定すること"
    $expectedFutureAgentKey = Get-TestAgentKey `
        -ReportPath $futureReport `
        -SourcePaths @((Join-Path $repositoryRoot "src\future"))
    $expectedFutureIncident = Get-TestIncidentId `
        -AgentKey $expectedFutureAgentKey `
        -Status "unmonitorable" `
        -LatestPath ([string]$futureState.latestPath) `
        -LatestWriteUtc ([string]$futureState.latestWriteUtc) `
        -ProblemDigest $expectedFutureProblemDigest
    Assert-Equal ([string]$futureState.incidentId) $expectedFutureIncident "未来mtimeのincidentをcanonical入力から再計算できること"
    $futureLatestText = $strictUtf8.GetString([IO.File]::ReadAllBytes($latestOutputPath))
    Assert-Contains $futureLatestText "[監視不能] future-agent" "未来mtimeを人向け表示でも監視不能にすること"
    Assert-True (-not $futureLatestText.Contains("[稼働] future-agent")) "未来mtimeを人向け表示で稼働にしないこと"
    Assert-Contains $futureLatestText $futureProblemText "未来mtime問題を現在時刻なしの固定文で表示すること"
    Assert-Contains $futureLatestText ("AGENT_WATCH_INCIDENT schema=2 status=unmonitorable incident={0}" -f $expectedFutureIncident) "未来mtimeのincident IDをlatest outputへ出すこと"
    Stop-OwnedWatcherProcess -Process $futureWatcher

    $futureRepeat = Start-ContinuousWatcher -PowerShellPath $powerShellCommand.Source -ConfigPath $definitionPath
    $futureRepeatRuntime = Wait-ForWatcherRuntime -Process $futureRepeat
    $futureRepeatState = @($futureRepeatRuntime.agentStates)[0]
    Assert-True ([string]$futureRepeatRuntime.instanceId -cne [string]$futureRuntime.instanceId) "未来mtime再検査が別watcher instanceであること"
    Assert-Equal ([string]$futureRepeatState.problemDigest) $expectedFutureProblemDigest "scan時刻が変わっても未来mtime problemDigestを固定すること"
    Assert-Equal ([string]$futureRepeatState.incidentId) $expectedFutureIncident "scan時刻が変わっても未来mtime incidentを固定すること"
    Stop-OwnedWatcherProcess -Process $futureRepeat
    foreach ($path in $futureInputs) {
        $after = Get-FileFingerprint $path
        Assert-Equal $after.Length $futureBefore[$path].Length "未来mtime検査が入力長を変えないこと: $path"
        Assert-Equal $after.LastWriteTicks $futureBefore[$path].LastWriteTicks "未来mtime検査が入力時刻を変えないこと: $path"
        Assert-Equal $after.Hash $futureBefore[$path].Hash "未来mtime検査が入力内容を変えないこと: $path"
    }

    Write-Output ("watch-agents self-test passed: 8 cases, {0} assertions" -f $script:AssertionCount)
}
finally {
    Stop-OwnedWatcherProcesses
    if ($null -ne $helperProcess) {
        [void]$helperProcess.WaitForExit(15000)
        $helperProcess.Dispose()
    }
    Remove-TestSandbox
}
