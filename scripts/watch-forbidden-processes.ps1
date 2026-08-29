<#
.SYNOPSIS
作業担当（Claudeのサブエージェント）が起動してはいけないブラウザ・desktop.exe・
配信サーバーの有無を表示だけで監視します。

.DESCRIPTION
規約 docs/rules/02-禁止事項.md:3-4 と docs/rules/01-役割と委譲.md:15「担当はgitへ
書き込まず、desktop.exe、配信サーバー、ブラウザの窓を起動しない」を、表示だけで
見張ります。`scripts/watch-agents.ps1` と同じ作りで、プロセスの停止もファイルの
変更も一切行いません。

プロセス名だけで「誰が起動したか」を断定できません（利用者が自分で開いた
ブラウザと区別できないため）。そこで「監視開始時点に既にあったもの」と
「開始後に現れたもの」を分けて表示し、後者だけを注意として出します。
`-Once`（単発判定）は観測できる経過時間が無いため、検出は全て「開始時点」
の扱いとし、注意（新規出現）とは判定しません。継続監視の1回目の判定が
基準（baseline）になり、2回目以降でその基準に無いものだけを注意とします。

監視対象:
  - ブラウザ:既定 chrome / msedge / firefox / brave（プロセス名）
  - desktop.exe（プロセス名。既定 "desktop"）
  - 配信サーバー: 既定 vite（プロセス名）／ node系プロセスの待ち受け／
    既定port 1420・4173・5173 の待ち受け（プロセス名を問わない）

.PARAMETER IntervalMinutes
継続監視の間隔（分）。既定10分。

.PARAMETER Once
1回だけ判定して終了します。経過時間が無いため、検出は全て開始時点扱いです。

.PARAMETER MaxIterations
継続監視を終える判定回数の上限です。0（既定）は無制限（watch-agents.ps1と同じ
「止めるまで動き続ける」動作）。試験や定点確認で回数を区切りたいときに使います。

.PARAMETER TestIntervalSeconds
指定すると、この秒数を間隔として使い、IntervalMinutesより優先します。0（既定）
は使いません。試験で継続監視を短時間に圧縮するための引数です。

.PARAMETER BrowserProcessNames
ブラウザとして扱うプロセス名の一覧（.exeは付けない）。既定はchrome/msedge/
firefox/brave。試験で安全な代替名に差し替えられるよう引数化しています。

.PARAMETER DesktopProcessName
desktop.exeとして扱うプロセス名（.exeは付けない）。既定は"desktop"。

.PARAMETER DevServerProcessNames
配信サーバーの実行ファイル名として扱うプロセス名の一覧。既定はvite。

.PARAMETER NodeLikeProcessNames
「node待ち受け」として扱うプロセス名の一覧。既定はnode。

.PARAMETER DevServerPorts
配信サーバーの既知待ち受けportの一覧。既定は1420・4173・5173。
#>
[CmdletBinding()]
param(
    [ValidateRange(1, 1440)]
    [int]$IntervalMinutes = 10,

    [switch]$Once,

    [ValidateRange(0, 100000)]
    [int]$MaxIterations = 0,

    [ValidateRange(0, 3600)]
    [int]$TestIntervalSeconds = 0,

    [string[]]$BrowserProcessNames = @("chrome", "msedge", "firefox", "brave"),

    [string]$DesktopProcessName = "desktop",

    [string[]]$DevServerProcessNames = @("vite"),

    [string[]]$NodeLikeProcessNames = @("node"),

    [int[]]$DevServerPorts = @(1420, 4173, 5173)
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
# Get-NetTCPConnection等が出す進捗表示を止める。非対話実行（ジョブ・パイプ経由の
# 呼び出しなど）では進捗レコードがCLIXMLとして標準エラーへ直列化され、この出力を
# 別のPowerShellプロセスが読もうとすると「XMLを処理できません」で失敗することを
# 試験中に実測した。表示専用の監視scriptに進捗バーは不要なため、常に止める。
# script scopeだけでなくglobal scopeにも設定する。Get-NetTCPConnection(CIM/CDXML
# 実装)のCIM問い合わせが遅い場合、進捗レコードの生成がglobal scopeの
# $ProgressPreferenceだけを見ることを15回中1回の頻度で実測したため
# （2026-08-30、watch-forbidden-processes.test.ps1を新規processで連続実行し
# 再現・特定した）。両方を止めないと稀に取りこぼす。
$ProgressPreference = "SilentlyContinue"
$global:ProgressPreference = "SilentlyContinue"

function ConvertTo-BareProcessName {
    param([Parameter(Mandatory = $true)][string]$Name)

    return [regex]::Replace($Name, '\.exe$', '', [Text.RegularExpressions.RegexOptions]::IgnoreCase)
}

function New-ForbiddenProcessRecord {
    param(
        [Parameter(Mandatory = $true)]$Process,
        [Parameter(Mandatory = $true)][string]$Category,
        [int[]]$Ports = @()
    )

    # Get-CimInstance(Win32_Process)のCreationDateは既に[DateTime]へ変換済みで
    # 返る（生のWMI DMTF文字列ではない）。念のためDateTime以外（文字列でのDMTF
    # 表現や$null）も許容し、両方を試したうえで取得不可なら安全側に倒す。
    $startTicks = [int64]0
    $startText = "<取得不可>"
    try {
        $creationRaw = $Process.CreationDate
        if ($creationRaw -is [DateTime]) {
            $creationUtc = ([DateTime]$creationRaw).ToUniversalTime()
            $startTicks = $creationUtc.Ticks
            $startText = "{0:yyyy-MM-dd HH:mm:ss}Z" -f $creationUtc
        }
        elseif ($null -ne $creationRaw -and -not [string]::IsNullOrWhiteSpace([string]$creationRaw)) {
            $creation = [Management.ManagementDateTimeConverter]::ToDateTime([string]$creationRaw)
            $creationUtc = $creation.ToUniversalTime()
            $startTicks = $creationUtc.Ticks
            $startText = "{0:yyyy-MM-dd HH:mm:ss}Z" -f $creationUtc
        }
    }
    catch {
        $startTicks = [int64]0
        $startText = "<取得不可>"
    }

    $processIdValue = [int]$Process.ProcessId
    [pscustomobject]@{
        ProcessId = $processIdValue
        Name = [string]$Process.Name
        Category = $Category
        ExecutablePath = [string]$Process.ExecutablePath
        CommandLine = [string]$Process.CommandLine
        StartTimeText = $startText
        Identity = ("{0}|{1}" -f $processIdValue, $startTicks)
        Ports = @($Ports)
    }
}

function Get-ForbiddenProcessSnapshot {
    param(
        [Parameter(Mandatory = $true)][string[]]$BrowserProcessNames,
        [Parameter(Mandatory = $true)][string]$DesktopProcessName,
        [Parameter(Mandatory = $true)][string[]]$DevServerProcessNames,
        [Parameter(Mandatory = $true)][string[]]$NodeLikeProcessNames,
        [Parameter(Mandatory = $true)][int[]]$DevServerPorts
    )

    $browserSet = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($n in $BrowserProcessNames) { [void]$browserSet.Add((ConvertTo-BareProcessName $n)) }
    $nodeSet = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($n in $NodeLikeProcessNames) { [void]$nodeSet.Add((ConvertTo-BareProcessName $n)) }
    $devServerSet = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($n in $DevServerProcessNames) { [void]$devServerSet.Add((ConvertTo-BareProcessName $n)) }
    $desktopBareName = ConvertTo-BareProcessName $DesktopProcessName

    $findings = New-Object System.Collections.Generic.List[object]
    $foundIdentities = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $warnings = New-Object System.Collections.Generic.List[string]

    $processes = @()
    try {
        $processes = @(Get-CimInstance Win32_Process -ErrorAction Stop)
    }
    catch {
        $warnings.Add("Win32_Processを取得できません: $($_.Exception.Message)")
    }

    foreach ($p in $processes) {
        $bareName = ConvertTo-BareProcessName ([string]$p.Name)
        $category = $null
        if ($browserSet.Contains($bareName)) {
            $category = "ブラウザ"
        }
        elseif ([string]::Equals($bareName, $desktopBareName, [StringComparison]::OrdinalIgnoreCase)) {
            $category = "desktop.exe"
        }
        elseif ($devServerSet.Contains($bareName)) {
            $category = "配信サーバー(名前一致)"
        }

        if ($null -ne $category) {
            $record = New-ForbiddenProcessRecord -Process $p -Category $category
            $findings.Add($record)
            [void]$foundIdentities.Add($record.Identity)
        }
    }

    # 待ち受けportからの検出（node待ち受け・既知の配信port）。
    # プロセス名の一致とは別経路のため、既に見つかっている場合は種別を追記するだけにする。
    try {
        $connections = @(Get-NetTCPConnection -State Listen -ErrorAction Stop)
        $listenPortsByPid = @{}
        foreach ($c in $connections) {
            $ownerPid = [int]$c.OwningProcess
            if (-not $listenPortsByPid.ContainsKey($ownerPid)) {
                $listenPortsByPid[$ownerPid] = New-Object System.Collections.Generic.List[int]
            }
            $listenPortsByPid[$ownerPid].Add([int]$c.LocalPort)
        }

        foreach ($ownerPid in $listenPortsByPid.Keys) {
            $ownerProcess = $processes | Where-Object { [int]$_.ProcessId -eq $ownerPid } | Select-Object -First 1
            if ($null -eq $ownerProcess) {
                continue
            }
            $ports = @($listenPortsByPid[$ownerPid] | Sort-Object -Unique)
            $ownerBareName = ConvertTo-BareProcessName ([string]$ownerProcess.Name)
            $isNodeListening = $nodeSet.Contains($ownerBareName)
            $matchedPorts = @($ports | Where-Object { $DevServerPorts -contains $_ })
            $isKnownPort = $matchedPorts.Count -gt 0

            if (-not $isNodeListening -and -not $isKnownPort) {
                continue
            }

            $reasonParts = New-Object System.Collections.Generic.List[string]
            if ($isNodeListening) { $reasonParts.Add("nodeの待ち受け") }
            if ($isKnownPort) { $reasonParts.Add("既知port待ち受け(" + ($matchedPorts -join ",") + ")") }
            $reasonText = "配信サーバー(" + ($reasonParts -join ", ") + ")"

            $existing = $findings | Where-Object { $_.ProcessId -eq $ownerPid } | Select-Object -First 1
            if ($null -ne $existing) {
                $existing.Category = $existing.Category + " / " + $reasonText
                $existing.Ports = @($ports)
            }
            else {
                $record = New-ForbiddenProcessRecord -Process $ownerProcess -Category $reasonText -Ports $ports
                $findings.Add($record)
                [void]$foundIdentities.Add($record.Identity)
            }
        }
    }
    catch {
        $warnings.Add("待ち受けport一覧を取得できません: $($_.Exception.Message)")
    }

    # 註: List[T]をそのまま @() で包むとWindows PowerShell 5.1のバインダーで
    # 「Argument types do not match」が発生することが実測で分かったため、
    # .ToArray() で明示的に配列化する（check-agent-instruction.ps1と同じ回避）。
    [pscustomobject]@{
        Findings = $findings.ToArray()
        Warnings = $warnings.ToArray()
    }
}

function Write-Finding {
    param(
        [Parameter(Mandatory = $true)][string]$Tag,
        [Parameter(Mandatory = $true)]$Finding,
        [switch]$Detailed
    )

    Write-Output ("  [{0}] {1} PID={2} 開始={3} 実行ファイル={4}" -f
        $Tag, $Finding.Category, $Finding.ProcessId, $Finding.StartTimeText, $Finding.ExecutablePath)
    if ($Detailed) {
        Write-Output ("    CommandLine={0}" -f $Finding.CommandLine)
        if (@($Finding.Ports).Count -gt 0) {
            Write-Output ("    待ち受けport={0}" -f (($Finding.Ports) -join ","))
        }
    }
}

$effectiveMaxIterations = 0
if ($Once) {
    $effectiveMaxIterations = 1
}
elseif ($MaxIterations -gt 0) {
    $effectiveMaxIterations = $MaxIterations
}

$sleepSeconds = $IntervalMinutes * 60
if ($TestIntervalSeconds -gt 0) {
    $sleepSeconds = $TestIntervalSeconds
}

$mode = if ($Once) { "1回判定" } else { "継続監視" }
Write-Output ("[禁止process監視] モード={0} / 間隔={1}秒 / 動作=表示のみ（プロセス停止=0 / ファイル変更=0）" -f $mode, $sleepSeconds)
Write-Output ("[禁止process監視] 対象: ブラウザ({0}) / desktop.exe({1}) / 配信サーバー(名前:{2}・node待ち受け・port:{3})" -f
    ($BrowserProcessNames -join ","), $DesktopProcessName, ($DevServerProcessNames -join ","), ($DevServerPorts -join ","))
Write-Output "[禁止process監視] 註: プロセス名だけで犯人と断定しません。監視開始時点に既にあったものと、開始後に現れたものを分けて表示します。"

$script:BaselineIdentities = $null
$iteration = 0

do {
    $iteration += 1
    $nowUtc = [DateTime]::UtcNow
    Write-Output ("[判定{0}] {1:yyyy-MM-dd HH:mm:ss}Z" -f $iteration, $nowUtc)

    $snapshot = Get-ForbiddenProcessSnapshot `
        -BrowserProcessNames $BrowserProcessNames `
        -DesktopProcessName $DesktopProcessName `
        -DevServerProcessNames $DevServerProcessNames `
        -NodeLikeProcessNames $NodeLikeProcessNames `
        -DevServerPorts $DevServerPorts

    foreach ($warning in $snapshot.Warnings) {
        Write-Output ("[要確認] {0}" -f $warning)
    }

    if ($Once) {
        foreach ($finding in $snapshot.Findings) {
            Write-Finding -Tag "検出/開始時点扱い" -Finding $finding -Detailed
        }
        Write-Output ("[集計] 検出 {0}件（単発判定のため経過時間が無く、全て開始時点扱いです。犯人と断定はできません）" -f @($snapshot.Findings).Count)
    }
    else {
        if ($null -eq $script:BaselineIdentities) {
            $script:BaselineIdentities = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
            foreach ($finding in $snapshot.Findings) {
                [void]$script:BaselineIdentities.Add($finding.Identity)
                Write-Finding -Tag "開始時点" -Finding $finding -Detailed
            }
            Write-Output ("[集計] 監視開始時点の検出 {0}件（基準として記録。まだ注意対象はありません）" -f @($snapshot.Findings).Count)
        }
        else {
            $preExisting = New-Object System.Collections.Generic.List[object]
            $appeared = New-Object System.Collections.Generic.List[object]
            foreach ($finding in $snapshot.Findings) {
                if ($script:BaselineIdentities.Contains($finding.Identity)) {
                    $preExisting.Add($finding)
                }
                else {
                    $appeared.Add($finding)
                }
            }
            foreach ($finding in $preExisting) {
                Write-Finding -Tag "開始時点から継続" -Finding $finding
            }
            foreach ($finding in $appeared) {
                Write-Finding -Tag "注意/開始後に出現" -Finding $finding -Detailed
            }
            Write-Output ("[集計] 開始時点から継続 {0}件 / 開始後に出現(注意) {1}件（プロセス名だけで犯人と断定はできません）" -f $preExisting.Count, $appeared.Count)
        }
    }

    if ($effectiveMaxIterations -gt 0 -and $iteration -ge $effectiveMaxIterations) {
        break
    }
    Start-Sleep -Seconds $sleepSeconds
} while ($true)

exit 0
