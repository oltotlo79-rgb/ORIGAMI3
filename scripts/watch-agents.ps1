<#
.SYNOPSIS
担当ごとの成果物更新と、長時間処理の補助情報を表示します。

.DESCRIPTION
JSON定義にある報告書と許可ソースの最新更新時刻を監視します。既定は10分間隔の
継続監視で、40分以上どの成果物にも更新が無ければ停滞と表示します。このスクリプトは
表示だけを行い、processの停止やファイルの作成・変更・削除は行いません。

定義ファイルの例:
{
  "agents": [
    {
      "name": "担当A",
      "reportPath": "scratchpad/task-a-report.md",
      "sourcePaths": ["crates/example/src", "docs/example.md"]
    }
  ]
}

相対パスは RepositoryRoot を基準に解決します。絶対パスも指定できます。

.PARAMETER DefinitionPath
担当と監視対象を記したJSONファイルです。

.PARAMETER Once
1回だけ判定して終了します。隔離試験や定点確認に使います。指定しなければ継続監視です。
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$DefinitionPath,

    [string]$RepositoryRoot = "",

    [ValidateRange(1, 1440)]
    [int]$IntervalMinutes = 10,

    [ValidateRange(1, 10080)]
    [int]$StaleAfterMinutes = 40,

    [switch]$Once
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $scriptDirectory = [string]$PSScriptRoot
    if ([string]::IsNullOrWhiteSpace($scriptDirectory)) {
        $invocationPath = [string]$MyInvocation.MyCommand.Path
        if (-not [string]::IsNullOrWhiteSpace($invocationPath)) {
            $scriptDirectory = Split-Path -Parent ([IO.Path]::GetFullPath($invocationPath))
        }
    }
    if ([string]::IsNullOrWhiteSpace($scriptDirectory)) {
        throw "RepositoryRoot was not supplied and the script directory could not be determined."
    }
    $RepositoryRoot = Split-Path -Parent $scriptDirectory
}

function Resolve-WatchPath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$BasePath
    )

    if ([IO.Path]::IsPathRooted($Path)) {
        return [IO.Path]::GetFullPath($Path)
    }
    return [IO.Path]::GetFullPath((Join-Path $BasePath $Path))
}

function Get-WatchedPathState {
    param(
        [Parameter(Mandatory = $true)][string]$ConfiguredPath,
        [Parameter(Mandatory = $true)][string]$BasePath,
        [Parameter(Mandatory = $true)][bool]$MustBeFile
    )

    $fullPath = Resolve-WatchPath -Path $ConfiguredPath -BasePath $BasePath
    if (-not (Test-Path -LiteralPath $fullPath)) {
        return [pscustomobject]@{
            ConfiguredPath = $ConfiguredPath
            FullPath = $fullPath
            Exists = $false
            LatestPath = $null
            LastWriteTimeUtc = $null
            Bytes = $null
            Problem = "存在しません"
        }
    }

    $item = Get-Item -LiteralPath $fullPath -Force
    if ($MustBeFile -and -not ($item -is [IO.FileInfo])) {
        return [pscustomobject]@{
            ConfiguredPath = $ConfiguredPath
            FullPath = $fullPath
            Exists = $true
            LatestPath = $null
            LastWriteTimeUtc = $null
            Bytes = $null
            Problem = "報告書にファイルでないパスが指定されています"
        }
    }

    if ($item -is [IO.FileInfo]) {
        return [pscustomobject]@{
            ConfiguredPath = $ConfiguredPath
            FullPath = $fullPath
            Exists = $true
            LatestPath = $item.FullName
            LastWriteTimeUtc = $item.LastWriteTimeUtc
            Bytes = $item.Length
            Problem = $null
        }
    }

    try {
        $latest = Get-ChildItem -LiteralPath $fullPath -File -Recurse -Force -ErrorAction Stop |
            Sort-Object LastWriteTimeUtc -Descending |
            Select-Object -First 1
    }
    catch {
        return [pscustomobject]@{
            ConfiguredPath = $ConfiguredPath
            FullPath = $fullPath
            Exists = $true
            LatestPath = $null
            LastWriteTimeUtc = $null
            Bytes = $null
            Problem = "配下を走査できません: $($_.Exception.Message)"
        }
    }

    if ($null -eq $latest) {
        return [pscustomobject]@{
            ConfiguredPath = $ConfiguredPath
            FullPath = $fullPath
            Exists = $true
            LatestPath = $null
            LastWriteTimeUtc = $null
            Bytes = $null
            Problem = "配下に監視できるファイルがありません"
        }
    }

    return [pscustomobject]@{
        ConfiguredPath = $ConfiguredPath
        FullPath = $fullPath
        Exists = $true
        LatestPath = $latest.FullName
        LastWriteTimeUtc = $latest.LastWriteTimeUtc
        Bytes = $latest.Length
        Problem = $null
    }
}

function Get-ProcessKind {
    param([Parameter(Mandatory = $true)]$Process)

    $name = ([string]$Process.Name).ToLowerInvariant()
    if ($name -eq "cargo" -or $name -eq "cargo.exe") {
        return "cargo"
    }
    if ($name -eq "rustc" -or $name -eq "rustc.exe") {
        return "rustc"
    }

    $locationText = (([string]$Process.ExecutablePath) + " " + ([string]$Process.CommandLine))
    if ([regex]::IsMatch(
            $locationText,
            '[\\/][^\\/]*target[^\\/]*[\\/](?:debug|release)[\\/]deps[\\/][^\\/\s"]+\.exe',
            [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        return "テスト"
    }
    return $null
}

function Get-SupplementalProcesses {
    $rawProcesses = New-Object System.Collections.Generic.List[object]
    $warning = $null
    try {
        foreach ($process in @(Get-CimInstance Win32_Process -ErrorAction Stop)) {
            $cpuSeconds = $null
            try {
                $cpuSeconds = [Math]::Round(
                    (([Convert]::ToDouble($process.KernelModeTime) + [Convert]::ToDouble($process.UserModeTime)) / 10000000.0),
                    3
                )
            }
            catch {
                $cpuSeconds = $null
            }
            $rawProcesses.Add([pscustomobject]@{
                Name = [string]$process.Name
                ExecutablePath = [string]$process.ExecutablePath
                CommandLine = [string]$process.CommandLine
                ProcessId = [int]$process.ProcessId
                CpuSeconds = $cpuSeconds
            })
        }
    }
    catch {
        $warning = "Win32_Processを取得できないためGet-Processへ切り替えました: $($_.Exception.Message)"
        try {
            foreach ($process in @(Get-Process -ErrorAction Stop)) {
                $executablePath = $null
                try {
                    $executablePath = [string]$process.Path
                }
                catch {
                    $executablePath = $null
                }
                $cpuSeconds = $null
                try {
                    $cpuSeconds = [Math]::Round([double]$process.CPU, 3)
                }
                catch {
                    $cpuSeconds = $null
                }
                $rawProcesses.Add([pscustomobject]@{
                    Name = [string]$process.Name
                    ExecutablePath = $executablePath
                    CommandLine = "<取得不可: Win32_Processを利用できません>"
                    ProcessId = [int]$process.Id
                    CpuSeconds = $cpuSeconds
                })
            }
        }
        catch {
            return [pscustomobject]@{
                Processes = @()
                Warning = "process情報を取得できません: $($_.Exception.Message)"
            }
        }
    }

    $results = New-Object System.Collections.Generic.List[object]
    foreach ($process in $rawProcesses) {
        $kind = Get-ProcessKind -Process $process
        if ($null -eq $kind) {
            continue
        }

        $commandLine = [string]$process.CommandLine
        if ([string]::IsNullOrWhiteSpace($commandLine)) {
            $commandLine = "<取得不可>"
        }
        $results.Add([pscustomobject]@{
            Kind = $kind
            ProcessId = [int]$process.ProcessId
            CpuSeconds = $process.CpuSeconds
            CommandLine = $commandLine
        })
    }

    return [pscustomobject]@{
        Processes = @($results | Sort-Object Kind, ProcessId)
        Warning = $warning
    }
}

function Format-WatchTime {
    param(
        [AllowNull()]$TimeUtc,
        [Parameter(Mandatory = $true)][DateTime]$NowUtc
    )

    if ($null -eq $TimeUtc) {
        return "<取得不可>"
    }
    $time = [DateTime]$TimeUtc
    $ageMinutes = [Math]::Max(0.0, [Math]::Round(($NowUtc - $time).TotalMinutes, 1))
    return "{0:yyyy-MM-dd HH:mm:ss}Z（{1}分前）" -f $time, $ageMinutes
}

function Assert-AgentDefinition {
    param([Parameter(Mandatory = $true)]$Agent)

    $propertyNames = @($Agent.PSObject.Properties.Name)
    if (($propertyNames -notcontains "name") -or [string]::IsNullOrWhiteSpace([string]$Agent.name)) {
        throw "担当定義の name が空です"
    }
    if (($propertyNames -notcontains "reportPath") -or [string]::IsNullOrWhiteSpace([string]$Agent.reportPath)) {
        throw "担当 '$($Agent.name)' の reportPath が空です"
    }
    if ($propertyNames -notcontains "sourcePaths") {
        throw "担当 '$($Agent.name)' の sourcePaths がありません"
    }
    $sourcePaths = @($Agent.sourcePaths)
    if ($sourcePaths.Count -eq 0) {
        throw "担当 '$($Agent.name)' の sourcePaths が空です"
    }
    foreach ($sourcePath in $sourcePaths) {
        if ([string]::IsNullOrWhiteSpace([string]$sourcePath)) {
            throw "担当 '$($Agent.name)' の sourcePaths に空のパスがあります"
        }
    }
}

$RepositoryRoot = [IO.Path]::GetFullPath($RepositoryRoot)
if (-not (Test-Path -LiteralPath $RepositoryRoot -PathType Container)) {
    throw "RepositoryRoot が存在しません: $RepositoryRoot"
}
$DefinitionPath = Resolve-WatchPath -Path $DefinitionPath -BasePath $RepositoryRoot
if (-not (Test-Path -LiteralPath $DefinitionPath -PathType Leaf)) {
    throw "監視定義が存在しません: $DefinitionPath"
}

try {
    $definition = [IO.File]::ReadAllText($DefinitionPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
}
catch {
    throw "監視定義JSONを読めません: $DefinitionPath ($($_.Exception.Message))"
}
if ($null -eq $definition -or @($definition.PSObject.Properties.Name) -notcontains "agents") {
    throw "監視定義に agents 配列がありません: $DefinitionPath"
}
$agents = @($definition.agents)
if ($agents.Count -eq 0) {
    throw "監視定義の agents 配列が空です: $DefinitionPath"
}

$knownNames = @{}
foreach ($agent in $agents) {
    Assert-AgentDefinition -Agent $agent
    $agentName = [string]$agent.name
    if ($knownNames.ContainsKey($agentName)) {
        throw "担当名が重複しています: $agentName"
    }
    $knownNames[$agentName] = $true
}

$mode = if ($Once) { "1回判定" } else { "継続監視" }
Write-Output ("[停滞監視] モード={0} / 間隔={1}分 / 停滞閾値={2}分 / 動作=表示のみ（プロセス停止=0 / ファイル変更=0）" -f
    $mode, $IntervalMinutes, $StaleAfterMinutes)
Write-Output ("[停滞監視] 定義={0}" -f $DefinitionPath)

do {
    $nowUtc = [DateTime]::UtcNow
    $staleBoundary = $nowUtc.AddMinutes(-$StaleAfterMinutes)
    Write-Output ("[判定時刻] {0:yyyy-MM-dd HH:mm:ss}Z" -f $nowUtc)

    foreach ($agent in $agents) {
        $agentName = [string]$agent.name
        $reportState = Get-WatchedPathState -ConfiguredPath ([string]$agent.reportPath) -BasePath $RepositoryRoot -MustBeFile $true
        $sourceStates = @(
            foreach ($sourcePath in @($agent.sourcePaths)) {
                Get-WatchedPathState -ConfiguredPath ([string]$sourcePath) -BasePath $RepositoryRoot -MustBeFile $false
            }
        )
        $allStates = @($reportState) + $sourceStates
        $problems = @($allStates | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_.Problem) })
        $validStates = @($allStates | Where-Object { $null -ne $_.LastWriteTimeUtc })

        if ($problems.Count -gt 0 -or $validStates.Count -eq 0) {
            $status = "監視不能"
        }
        else {
            $latestState = $validStates | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
            $status = if ($latestState.LastWriteTimeUtc -gt $staleBoundary) { "稼働" } else { "停滞" }
        }

        if ($validStates.Count -gt 0) {
            $latestState = $validStates | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
            $latestSummary = Format-WatchTime -TimeUtc $latestState.LastWriteTimeUtc -NowUtc $nowUtc
            Write-Output ("[{0}] {1}: 最新={2} / {3}" -f $status, $agentName, $latestSummary, $latestState.LatestPath)
        }
        else {
            Write-Output ("[{0}] {1}: 最新=<取得不可>" -f $status, $agentName)
        }

        $reportTime = Format-WatchTime -TimeUtc $reportState.LastWriteTimeUtc -NowUtc $nowUtc
        Write-Output ("  報告書: {0} / bytes={1} / {2}" -f
            $reportState.FullPath, $(if ($null -eq $reportState.Bytes) { "-" } else { $reportState.Bytes }), $reportTime)
        foreach ($sourceState in $sourceStates) {
            $sourceTime = Format-WatchTime -TimeUtc $sourceState.LastWriteTimeUtc -NowUtc $nowUtc
            $sourceDetail = if ($null -eq $sourceState.LatestPath) { $sourceState.FullPath } else { $sourceState.LatestPath }
            Write-Output ("  ソース: {0} / bytes={1} / {2}" -f
                $sourceDetail, $(if ($null -eq $sourceState.Bytes) { "-" } else { $sourceState.Bytes }), $sourceTime)
        }
        foreach ($problem in $problems) {
            Write-Output ("  [要確認] {0}: {1}" -f $problem.FullPath, $problem.Problem)
        }
    }

    $processState = Get-SupplementalProcesses
    if (-not [string]::IsNullOrWhiteSpace([string]$processState.Warning)) {
        Write-Output ("[補助process注意] {0}" -f $processState.Warning)
    }
    $processes = @($processState.Processes)
    $cargoCount = @($processes | Where-Object Kind -eq "cargo").Count
    $rustcCount = @($processes | Where-Object Kind -eq "rustc").Count
    $testCount = @($processes | Where-Object Kind -eq "テスト").Count
    Write-Output ("[補助process] 合計={0} / cargo={1} / rustc={2} / テスト={3}" -f
        $processes.Count, $cargoCount, $rustcCount, $testCount)
    foreach ($process in $processes) {
        $cpuText = if ($null -eq $process.CpuSeconds) { "<取得不可>" } else { [string]$process.CpuSeconds }
        Write-Output ("  種別={0} / PID={1} / CPU秒={2} / CommandLine={3}" -f
            $process.Kind, $process.ProcessId, $cpuText, $process.CommandLine)
    }

    if (-not $Once) {
        Start-Sleep -Seconds ($IntervalMinutes * 60)
    }
} while (-not $Once)
