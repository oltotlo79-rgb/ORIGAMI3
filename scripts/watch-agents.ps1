<#
.SYNOPSIS
担当ごとの成果物更新と、長時間処理の補助情報を表示します。

.DESCRIPTION
JSON定義にある報告書と許可ソースの最新更新時刻を監視します。既定は10分間隔の
継続監視で、40分以上どの成果物にも更新が無ければ停滞と表示します。このスクリプトは
processの停止や監視対象の変更を行いません。継続監視だけは生存確認のため、RepositoryRoot
のscratchpadへ固定のUTF-8 latest output・runtime JSON・singleton lockを書きます。

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
$script:Utf8NoBom = New-Object Text.UTF8Encoding($false)
$script:CapturedWatchLines = $null
$script:FutureWriteToleranceMinutes = 2

function Write-WatchLine {
    param([Parameter(Mandatory = $true)][string]$Text)

    Write-Output $Text
    if ($null -ne $script:CapturedWatchLines) {
        $script:CapturedWatchLines.Add($Text)
    }
}

function Get-Sha256HexFromBytes {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][byte[]]$Bytes)

    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace("-", "").ToLowerInvariant()
    }
    finally {
        $sha.Dispose()
    }
}

function Get-Sha256HexFromText {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    return Get-Sha256HexFromBytes -Bytes $script:Utf8NoBom.GetBytes($Text)
}

function ConvertTo-CanonicalWatchPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $fullPath = [IO.Path]::GetFullPath($Path)
    return $fullPath.Replace([IO.Path]::AltDirectorySeparatorChar, [IO.Path]::DirectorySeparatorChar)
}

function ConvertTo-Base64Utf8 {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    return [Convert]::ToBase64String($script:Utf8NoBom.GetBytes($Text))
}

# schema 2のhash契約はscripts/hooks/check-agent-watch.ps1も独立再計算する。
# canonical textはUTF-8（BOMなし）・LF区切り・末尾LFなしで、可変文字列はBase64にする。
function Get-AgentKey {
    param(
        [Parameter(Mandatory = $true)]$Agent,
        [Parameter(Mandatory = $true)][string]$BasePath
    )

    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("version=1")
    $reportPath = Resolve-WatchPath -Path ([string]$Agent.reportPath) -BasePath $BasePath
    $lines.Add("reportPath={0}" -f (ConvertTo-Base64Utf8 -Text (ConvertTo-CanonicalWatchPath -Path $reportPath)))
    foreach ($sourcePath in @($Agent.sourcePaths)) {
        $resolvedSourcePath = Resolve-WatchPath -Path ([string]$sourcePath) -BasePath $BasePath
        $lines.Add("sourcePath={0}" -f (ConvertTo-Base64Utf8 -Text (ConvertTo-CanonicalWatchPath -Path $resolvedSourcePath)))
    }
    return Get-Sha256HexFromText -Text ($lines.ToArray() -join "`n")
}

function Get-ProblemDigest {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Problems)

    $lines = New-Object System.Collections.Generic.List[string]
    foreach ($problem in $Problems) {
        $problemPath = ConvertTo-CanonicalWatchPath -Path ([string]$problem.FullPath)
        $lines.Add(("path={0}`tproblem={1}" -f
                (ConvertTo-Base64Utf8 -Text $problemPath),
                (ConvertTo-Base64Utf8 -Text ([string]$problem.Problem))))
    }
    return Get-Sha256HexFromText -Text ($lines.ToArray() -join "`n")
}

function Get-IncidentId {
    param(
        [Parameter(Mandatory = $true)][string]$AgentKey,
        [Parameter(Mandatory = $true)][ValidateSet("stalled", "unmonitorable")][string]$Status,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$LatestPath,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$LatestWriteUtc,
        [Parameter(Mandatory = $true)][string]$ProblemDigest
    )

    $canonicalText = @(
        "version=1"
        "agentKey=$AgentKey"
        "status=$Status"
        "latestPath=$(ConvertTo-Base64Utf8 -Text $LatestPath)"
        "latestWriteUtc=$LatestWriteUtc"
        "problemDigest=$ProblemDigest"
    ) -join "`n"
    return Get-Sha256HexFromText -Text $canonicalText
}

function Get-AgentStatesCanonicalText {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$AgentStates)

    $lines = New-Object System.Collections.Generic.List[string]
    foreach ($agentState in $AgentStates) {
        $lines.Add((@(
                    "agentKey=$([string]$agentState.agentKey)"
                    "name=$(ConvertTo-Base64Utf8 -Text ([string]$agentState.name))"
                    "status=$([string]$agentState.status)"
                    "latestPath=$(ConvertTo-Base64Utf8 -Text ([string]$agentState.latestPath))"
                    "latestWriteUtc=$([string]$agentState.latestWriteUtc)"
                    "problemDigest=$([string]$agentState.problemDigest)"
                    "incidentId=$([string]$agentState.incidentId)"
                ) -join "`t"))
    }
    return $lines.ToArray() -join "`n"
}

function Get-AgentStatesSha256 {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$AgentStates)

    return Get-Sha256HexFromText -Text (Get-AgentStatesCanonicalText -AgentStates $AgentStates)
}

function Get-FileSha256Hex {
    param([Parameter(Mandatory = $true)][string]$Path)

    $stream = New-Object IO.FileStream(
        $Path,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
    )
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace("-", "").ToLowerInvariant()
    }
    finally {
        $sha.Dispose()
        $stream.Dispose()
    }
}

function Write-AtomicUtf8File {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Content
    )

    $operationId = [Guid]::NewGuid().ToString("N")
    $temporaryPath = "{0}.{1}.tmp" -f $Path, $operationId
    $backupPath = "{0}.{1}.bak" -f $Path, $operationId
    try {
        [IO.File]::WriteAllText($temporaryPath, $Content, $script:Utf8NoBom)
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            [IO.File]::Replace($temporaryPath, $Path, $backupPath)
        }
        else {
            [IO.File]::Move($temporaryPath, $Path)
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporaryPath -PathType Leaf) {
            Remove-Item -LiteralPath $temporaryPath -Force
        }
        if (Test-Path -LiteralPath $backupPath -PathType Leaf) {
            Remove-Item -LiteralPath $backupPath -Force
        }
    }
}

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
            Sort-Object -Property @(
                @{ Expression = { $_.LastWriteTimeUtc }; Descending = $true },
                @{ Expression = { ConvertTo-CanonicalWatchPath -Path ([string]$_.FullName) }; Descending = $false }
            ) |
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

    # CommandLineを含めると、target配下のtest exeを検索しているPowerShell自身まで
    # 「テスト」に数える。分類はOSが返した実行ファイル実パスだけを根拠にする。
    $locationText = [string]$Process.ExecutablePath
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

function Format-JapanTime {
    param([AllowNull()]$TimeUtc)

    if ($null -eq $TimeUtc) {
        return "<取得不可>"
    }
    $utc = ([DateTime]$TimeUtc).ToUniversalTime()
    $japan = [DateTimeOffset]::new($utc).ToOffset([TimeSpan]::FromHours(9))
    return $japan.ToString("yyyy-MM-dd HH:mm:ss zzz", [Globalization.CultureInfo]::InvariantCulture)
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

function Read-WatchDefinition {
    param([Parameter(Mandatory = $true)][string]$Path)

    try {
        $stream = New-Object IO.FileStream(
            $Path,
            [IO.FileMode]::Open,
            [IO.FileAccess]::Read,
            ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
        )
        try {
            if ($stream.Length -gt [int]::MaxValue) {
                throw "監視定義が大きすぎます: $Path"
            }
            $bytes = New-Object byte[] ([int]$stream.Length)
            $offset = 0
            while ($offset -lt $bytes.Length) {
                $read = $stream.Read($bytes, $offset, $bytes.Length - $offset)
                if ($read -le 0) {
                    throw "監視定義を最後まで読めませんでした: $Path"
                }
                $offset += $read
            }
        }
        finally {
            $stream.Dispose()
        }
        $definition = $script:Utf8NoBom.GetString($bytes) | ConvertFrom-Json
    }
    catch {
        throw "監視定義JSONを読めません: $Path ($($_.Exception.Message))"
    }

    if ($null -eq $definition -or @($definition.PSObject.Properties.Name) -notcontains "agents") {
        throw "監視定義に agents 配列がありません: $Path"
    }
    $agents = @($definition.agents)
    if ($agents.Count -eq 0) {
        throw "監視定義の agents 配列が空です: $Path"
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

    return [pscustomobject]@{
        Agents = $agents
        Sha256 = Get-Sha256HexFromBytes -Bytes $bytes
    }
}

function Initialize-WatchRuntime {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$ScriptPath
    )

    $scratchpadPath = Join-Path $Root "scratchpad"
    [void][IO.Directory]::CreateDirectory($scratchpadPath)
    $runtimePath = [IO.Path]::GetFullPath((Join-Path $scratchpadPath "watch-agents.runtime.json"))
    $outputPath = [IO.Path]::GetFullPath((Join-Path $scratchpadPath "watch-agents.latest.log"))
    $lockPath = [IO.Path]::GetFullPath((Join-Path $scratchpadPath "watch-agents.lock"))

    try {
        $lockStream = New-Object IO.FileStream(
            $lockPath,
            [IO.FileMode]::OpenOrCreate,
            [IO.FileAccess]::ReadWrite,
            [IO.FileShare]::None
        )
    }
    catch [IO.IOException] {
        throw "継続監視は既に稼働しています。singleton lockを取得できません: $lockPath"
    }

    try {
        [IO.File]::WriteAllText($outputPath, "", $script:Utf8NoBom)
        $process = Get-Process -Id $PID -ErrorAction Stop
        $processPath = [string]$process.Path
        if ([string]::IsNullOrWhiteSpace($processPath)) {
            throw "監視processの実行ファイル実パスを取得できません: PID=$PID"
        }
        return [pscustomobject]@{
            RuntimePath = $runtimePath
            OutputPath = $outputPath
            LockPath = $lockPath
            LockStream = $lockStream
            InstanceId = [Guid]::NewGuid().ToString("D")
            ProcessStartUtc = $process.StartTime.ToUniversalTime()
            ProcessExecutablePath = [IO.Path]::GetFullPath($processPath)
            ScriptPath = [IO.Path]::GetFullPath($ScriptPath)
            ScriptSha256 = Get-FileSha256Hex -Path $ScriptPath
            ScanSequence = 0
        }
    }
    catch {
        $lockStream.Dispose()
        throw
    }
}

function Publish-WatchRuntime {
    param(
        [Parameter(Mandatory = $true)]$Runtime,
        [Parameter(Mandatory = $true)][string[]]$HeaderLines,
        [Parameter(Mandatory = $true)][string[]]$CycleLines,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$DefinitionPath,
        [Parameter(Mandatory = $true)][string]$DefinitionSha256,
        [Parameter(Mandatory = $true)][int]$AgentCount,
        [Parameter(Mandatory = $true)][object[]]$AgentStates,
        [Parameter(Mandatory = $true)][string]$AgentStatesSha256,
        [Parameter(Mandatory = $true)][int]$ActiveCount,
        [Parameter(Mandatory = $true)][int]$StalledCount,
        [Parameter(Mandatory = $true)][int]$UnmonitorableCount,
        [Parameter(Mandatory = $true)][DateTime]$ScanCompletedUtc,
        [Parameter(Mandatory = $true)][int]$Interval,
        [Parameter(Mandatory = $true)][int]$StaleAfter
    )

    if ($AgentStates.Count -ne $AgentCount) {
        throw "agentStates件数が定義件数と一致しません: states=$($AgentStates.Count) agents=$AgentCount"
    }
    if (($ActiveCount + $StalledCount + $UnmonitorableCount) -ne $AgentCount) {
        throw "status件数の合計が定義件数と一致しません"
    }
    $computedStatesSha256 = Get-AgentStatesSha256 -AgentStates $AgentStates
    if ($computedStatesSha256 -cne $AgentStatesSha256) {
        throw "agentStates hashが発行直前の状態と一致しません"
    }

    $allLines = @($HeaderLines) + @($CycleLines)
    $text = ($allLines -join [Environment]::NewLine) + [Environment]::NewLine
    $bytes = $script:Utf8NoBom.GetBytes($text)
    Write-AtomicUtf8File -Path $Runtime.OutputPath -Content $text
    $outputItem = Get-Item -LiteralPath $Runtime.OutputPath -Force

    $Runtime.ScanSequence += 1
    $state = [ordered]@{
        schemaVersion = 2
        instanceId = $Runtime.InstanceId
        pid = $PID
        processStartUtc = $Runtime.ProcessStartUtc.ToString("o")
        processExecutablePath = $Runtime.ProcessExecutablePath
        scriptPath = $Runtime.ScriptPath
        scriptSha256 = $Runtime.ScriptSha256
        repositoryRoot = $Root
        definitionPath = $DefinitionPath
        definitionSha256 = $DefinitionSha256
        runtimePath = $Runtime.RuntimePath
        outputPath = $Runtime.OutputPath
        outputSha256 = Get-Sha256HexFromBytes -Bytes $bytes
        outputLength = [Int64]$bytes.Length
        outputLastWriteUtc = $outputItem.LastWriteTimeUtc.ToString("o")
        lockPath = $Runtime.LockPath
        mode = "continuous"
        intervalMinutes = $Interval
        staleAfterMinutes = $StaleAfter
        agentCount = $AgentCount
        activeCount = $ActiveCount
        stalledCount = $StalledCount
        unmonitorableCount = $UnmonitorableCount
        agentStatesSha256 = $AgentStatesSha256
        agentStates = @($AgentStates)
        scanSequence = $Runtime.ScanSequence
        scanCompletedUtc = $ScanCompletedUtc.ToString("o")
        stateWrittenUtc = [DateTime]::UtcNow.ToString("o")
    }
    Write-AtomicUtf8File -Path $Runtime.RuntimePath -Content ($state | ConvertTo-Json -Depth 8)
}

$RepositoryRoot = [IO.Path]::GetFullPath($RepositoryRoot)
if (-not (Test-Path -LiteralPath $RepositoryRoot -PathType Container)) {
    throw "RepositoryRoot が存在しません: $RepositoryRoot"
}
$DefinitionPath = Resolve-WatchPath -Path $DefinitionPath -BasePath $RepositoryRoot
if (-not (Test-Path -LiteralPath $DefinitionPath -PathType Leaf)) {
    throw "監視定義が存在しません: $DefinitionPath"
}
$definitionSnapshot = Read-WatchDefinition -Path $DefinitionPath
$scriptInvocationPath = [string]$MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($scriptInvocationPath)) {
    throw "監視scriptの実パスを取得できません"
}
$scriptInvocationPath = [IO.Path]::GetFullPath($scriptInvocationPath)

$mode = if ($Once) { "1回判定" } else { "継続監視" }
$behavior = if ($Once) {
    "表示のみ（プロセス停止=0 / ファイル変更=0）"
}
else {
    "表示のみ（プロセス停止=0 / 監視対象変更=0 / runtimeファイル更新=3）"
}
$headerLines = @(
    ("[停滞監視] モード={0} / 間隔={1}分 / 停滞閾値={2}分 / 動作={3}" -f
        $mode, $IntervalMinutes, $StaleAfterMinutes, $behavior),
    ("[停滞監視] 定義={0}" -f $DefinitionPath)
)
$headerLines | Write-Output

$runtime = $null
if (-not $Once) {
    $runtime = Initialize-WatchRuntime -Root $RepositoryRoot -ScriptPath $scriptInvocationPath
}

try {
    $previousScanStartedUtc = $null
    do {
        $definitionSnapshot = Read-WatchDefinition -Path $DefinitionPath
        $agents = @($definitionSnapshot.Agents)
        $cycleLines = New-Object System.Collections.Generic.List[string]
        $script:CapturedWatchLines = $cycleLines
        try {
            $nowUtc = [DateTime]::UtcNow
            $staleBoundary = $nowUtc.AddMinutes(-$StaleAfterMinutes)
            $futureBoundary = $nowUtc.AddMinutes($script:FutureWriteToleranceMinutes)
            Write-WatchLine ("[判定時刻] {0:yyyy-MM-dd HH:mm:ss}Z" -f $nowUtc)
            $previousScanElapsed = if ($null -eq $previousScanStartedUtc) {
                "<初回>"
            }
            else {
                [string]([Math]::Max(0.0, [Math]::Round(($nowUtc - $previousScanStartedUtc).TotalMinutes, 1)))
            }
            $nextScanJapan = if ($Once) {
                "<なし（1回判定）>"
            }
            else {
                Format-JapanTime -TimeUtc $nowUtc.AddMinutes($IntervalMinutes)
            }
            Write-WatchLine ("[判定時刻-日本] {0} / 次回走査予定時刻（日本時）={1} / 前回走査からの経過分数={2}" -f
                (Format-JapanTime -TimeUtc $nowUtc), $nextScanJapan, $previousScanElapsed)
            $previousScanStartedUtc = $nowUtc

            $agentStates = New-Object System.Collections.Generic.List[object]
            foreach ($agent in $agents) {
                $agentName = [string]$agent.name
                $agentKey = Get-AgentKey -Agent $agent -BasePath $RepositoryRoot
                $reportState = Get-WatchedPathState -ConfiguredPath ([string]$agent.reportPath) -BasePath $RepositoryRoot -MustBeFile $true
                $sourceStates = @(
                    foreach ($sourcePath in @($agent.sourcePaths)) {
                        Get-WatchedPathState -ConfiguredPath ([string]$sourcePath) -BasePath $RepositoryRoot -MustBeFile $false
                    }
                )
                $allStates = @($reportState) + $sourceStates
                $existingProblems = @($allStates | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_.Problem) })
                $futureProblems = @(
                    foreach ($watchedState in $allStates) {
                        if ($null -eq $watchedState.LastWriteTimeUtc -or
                            ([DateTime]$watchedState.LastWriteTimeUtc) -le $futureBoundary) {
                            continue
                        }
                        $futurePath = if ([string]::IsNullOrWhiteSpace([string]$watchedState.LatestPath)) {
                            [string]$watchedState.FullPath
                        }
                        else {
                            [string]$watchedState.LatestPath
                        }
                        $futureWriteUtc = ([DateTime]$watchedState.LastWriteTimeUtc).ToUniversalTime().ToString(
                            "o",
                            [Globalization.CultureInfo]::InvariantCulture
                        )
                        [pscustomobject]@{
                            FullPath = $futurePath
                            Problem = ("更新時刻が許容範囲の{0}分を超えて未来です: lastWriteUtc={1}" -f
                                $script:FutureWriteToleranceMinutes, $futureWriteUtc)
                        }
                    }
                )
                # 順序もproblemDigestの契約。通常問題をreport→source順、その後に未来mtimeを同じ順で並べる。
                $problems = @($existingProblems) + @($futureProblems)
                $validStates = @($allStates | Where-Object { $null -ne $_.LastWriteTimeUtc })
                $latestState = $null
                if ($validStates.Count -gt 0) {
                    $latestState = $validStates |
                        Sort-Object -Property @(
                            @{ Expression = { $_.LastWriteTimeUtc }; Descending = $true },
                            @{ Expression = { ConvertTo-CanonicalWatchPath -Path ([string]$_.LatestPath) }; Descending = $false }
                        ) |
                        Select-Object -First 1
                }

                if ($problems.Count -gt 0 -or $validStates.Count -eq 0) {
                    $machineStatus = "unmonitorable"
                }
                else {
                    $machineStatus = if ($latestState.LastWriteTimeUtc -gt $staleBoundary) { "active" } else { "stalled" }
                }

                $displayStatus = switch ($machineStatus) {
                    "active" { "稼働" }
                    "stalled" { "停滞" }
                    "unmonitorable" { "監視不能" }
                    default { throw "未知の監視statusです: $machineStatus" }
                }
                $latestPath = ""
                $latestWriteUtc = ""
                if ($null -ne $latestState) {
                    $latestPath = ConvertTo-CanonicalWatchPath -Path ([string]$latestState.LatestPath)
                    $latestWriteUtc = ([DateTime]$latestState.LastWriteTimeUtc).ToUniversalTime().ToString(
                        "o",
                        [Globalization.CultureInfo]::InvariantCulture
                    )
                }
                $problemDigest = Get-ProblemDigest -Problems $problems
                $incidentId = ""
                if ($machineStatus -ne "active") {
                    $incidentId = Get-IncidentId `
                        -AgentKey $agentKey `
                        -Status $machineStatus `
                        -LatestPath $latestPath `
                        -LatestWriteUtc $latestWriteUtc `
                        -ProblemDigest $problemDigest
                }
                $agentStates.Add([pscustomobject][ordered]@{
                    agentKey = $agentKey
                    name = $agentName
                    status = $machineStatus
                    latestPath = $latestPath
                    latestWriteUtc = $latestWriteUtc
                    problemDigest = $problemDigest
                    incidentId = $incidentId
                })

                if ($null -ne $latestState) {
                    $latestSummary = Format-WatchTime -TimeUtc $latestState.LastWriteTimeUtc -NowUtc $nowUtc
                    $latestJapan = Format-JapanTime -TimeUtc $latestState.LastWriteTimeUtc
                    Write-WatchLine ("[{0}] {1}: 最新={2} / 日本時={3} / {4}" -f
                        $displayStatus, $agentName, $latestSummary, $latestJapan, $latestState.LatestPath)
                }
                else {
                    Write-WatchLine ("[{0}] {1}: 最新=<取得不可>" -f $displayStatus, $agentName)
                }

                $reportTime = Format-WatchTime -TimeUtc $reportState.LastWriteTimeUtc -NowUtc $nowUtc
                Write-WatchLine ("  報告書: {0} / bytes={1} / {2}" -f
                    $reportState.FullPath, $(if ($null -eq $reportState.Bytes) { "-" } else { $reportState.Bytes }), $reportTime)
                foreach ($sourceState in $sourceStates) {
                    $sourceTime = Format-WatchTime -TimeUtc $sourceState.LastWriteTimeUtc -NowUtc $nowUtc
                    $sourceDetail = if ($null -eq $sourceState.LatestPath) { $sourceState.FullPath } else { $sourceState.LatestPath }
                    Write-WatchLine ("  ソース: {0} / bytes={1} / {2}" -f
                        $sourceDetail, $(if ($null -eq $sourceState.Bytes) { "-" } else { $sourceState.Bytes }), $sourceTime)
                }
                foreach ($problem in $problems) {
                    Write-WatchLine ("  [要確認] {0}: {1}" -f $problem.FullPath, $problem.Problem)
                }
            }

            $agentStateArray = @($agentStates.ToArray())
            $activeCount = @($agentStateArray | Where-Object status -eq "active").Count
            $stalledCount = @($agentStateArray | Where-Object status -eq "stalled").Count
            $unmonitorableCount = @($agentStateArray | Where-Object status -eq "unmonitorable").Count
            $agentStatesSha256 = Get-AgentStatesSha256 -AgentStates $agentStateArray
            Write-WatchLine ("AGENT_WATCH_STATUS schema=2 total={0} active={1} stalled={2} unmonitorable={3} states_sha256={4}" -f
                $agentStateArray.Count, $activeCount, $stalledCount, $unmonitorableCount, $agentStatesSha256)
            foreach ($agentState in $agentStateArray) {
                if ([string]$agentState.status -eq "active") {
                    continue
                }
                Write-WatchLine ("AGENT_WATCH_INCIDENT schema=2 status={0} incident={1} agent_key={2} name_b64={3}" -f
                    $agentState.status,
                    $agentState.incidentId,
                    $agentState.agentKey,
                    (ConvertTo-Base64Utf8 -Text ([string]$agentState.name)))
            }

            $processState = Get-SupplementalProcesses
            if (-not [string]::IsNullOrWhiteSpace([string]$processState.Warning)) {
                Write-WatchLine ("[補助process注意] {0}" -f $processState.Warning)
            }
            $processes = @($processState.Processes)
            $cargoCount = @($processes | Where-Object Kind -eq "cargo").Count
            $rustcCount = @($processes | Where-Object Kind -eq "rustc").Count
            $testCount = @($processes | Where-Object Kind -eq "テスト").Count
            Write-WatchLine ("[補助process] 合計={0} / cargo={1} / rustc={2} / テスト={3}" -f
                $processes.Count, $cargoCount, $rustcCount, $testCount)
            foreach ($process in $processes) {
                $cpuText = if ($null -eq $process.CpuSeconds) { "<取得不可>" } else { [string]$process.CpuSeconds }
                Write-WatchLine ("  種別={0} / PID={1} / CPU秒={2} / CommandLine={3}" -f
                    $process.Kind, $process.ProcessId, $cpuText, $process.CommandLine)
            }

            $scanCompletedUtc = [DateTime]::UtcNow
            Write-WatchLine ("[走査完了] {0:yyyy-MM-dd HH:mm:ss.fff}Z" -f $scanCompletedUtc)
            if (-not $Once) {
                Publish-WatchRuntime `
                    -Runtime $runtime `
                    -HeaderLines $headerLines `
                    -CycleLines $cycleLines.ToArray() `
                    -Root $RepositoryRoot `
                    -DefinitionPath $DefinitionPath `
                    -DefinitionSha256 $definitionSnapshot.Sha256 `
                    -AgentCount $agents.Count `
                    -AgentStates $agentStateArray `
                    -AgentStatesSha256 $agentStatesSha256 `
                    -ActiveCount $activeCount `
                    -StalledCount $stalledCount `
                    -UnmonitorableCount $unmonitorableCount `
                    -ScanCompletedUtc $scanCompletedUtc `
                    -Interval $IntervalMinutes `
                    -StaleAfter $StaleAfterMinutes
            }
        }
        finally {
            $script:CapturedWatchLines = $null
        }

        if (-not $Once) {
            Start-Sleep -Seconds ($IntervalMinutes * 60)
        }
    } while (-not $Once)
}
finally {
    $script:CapturedWatchLines = $null
    if ($null -ne $runtime) {
        if ($null -ne $runtime.LockStream) {
            $runtime.LockStream.Dispose()
        }
    }
}
