[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$scriptPath = Join-Path $PSScriptRoot "watch-forbidden-processes.ps1"
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-watch-forbidden-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [IO.Path]::GetFullPath((Join-Path $tempBase $sandboxName))
$script:AssertionCount = 0
$script:HelperProcesses = New-Object System.Collections.Generic.List[System.Diagnostics.Process]

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$Output = ""
    )
    $script:AssertionCount += 1
    if (-not $Condition) {
        throw "ASSERTION FAILED: $Message`n$Output"
    }
}

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)][AllowNull()]$Actual,
        [Parameter(Mandatory = $true)][AllowNull()]$Expected,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$Output = ""
    )
    $script:AssertionCount += 1
    if ($Actual -ne $Expected) {
        throw "ASSERTION FAILED: $Message (expected=$Expected, actual=$Actual)`n$Output"
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

function Assert-NotContains {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Unexpected,
        [Parameter(Mandatory = $true)][string]$Message
    )
    $script:AssertionCount += 1
    if ($Text.Contains($Unexpected)) {
        throw "ASSERTION FAILED: $Message (unexpected present='$Unexpected')`n$Text"
    }
}

# cmd.exeを指定名でコピーし、無害な待機コマンドで起動する。実際のブラウザ／desktop.exeは
# 一切起動しない（規約で禁止されているため）。watch-agents.test.ps1と同じ手法。
function Start-NamedHelperProcess {
    param(
        [Parameter(Mandatory = $true)][string]$BareName,
        [int]$WaitSeconds = 30
    )
    $helperPath = Join-Path $sandboxRoot ("{0}.exe" -f $BareName)
    if (-not (Test-Path -LiteralPath $helperPath)) {
        [IO.File]::Copy((Join-Path $env:SystemRoot "System32\cmd.exe"), $helperPath)
    }
    $proc = Start-Process -FilePath $helperPath -ArgumentList ("/d /c ""ping.exe -n {0} 127.0.0.1 >nul""" -f ($WaitSeconds + 2)) -WindowStyle Hidden -PassThru
    Start-Sleep -Milliseconds 300
    $script:HelperProcesses.Add($proc)
    return $proc
}

# powershell.exe(このプロセス自身の実行ファイル)を指定名でコピーし、指定portで
# TcpListenerを開かせる。実際に選ばれたportをファイルへ書き出させ、それを読み取る。
function Start-ListeningHelperProcess {
    param(
        [Parameter(Mandatory = $true)][string]$BareName,
        [int]$WaitSeconds = 30
    )
    $helperPath = Join-Path $sandboxRoot ("{0}.exe" -f $BareName)
    if (-not (Test-Path -LiteralPath $helperPath)) {
        [IO.File]::Copy((Get-Process -Id $PID).Path, $helperPath)
    }
    $portFile = Join-Path $sandboxRoot ("{0}-port.txt" -f $BareName)
    if (Test-Path -LiteralPath $portFile) { Remove-Item -LiteralPath $portFile -Force }

    $childCommand = '$listener = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Loopback, 0); ' +
        '$listener.Start(); ' +
        '[IO.File]::WriteAllText(' + "'" + $portFile.Replace("'", "''") + "'" + ', [string]$listener.LocalEndpoint.Port); ' +
        ('Start-Sleep -Seconds {0}; ' -f $WaitSeconds) +
        '$listener.Stop()'
    $encodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($childCommand))
    $proc = Start-Process -FilePath $helperPath -ArgumentList @("-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", $encodedCommand) -WindowStyle Hidden -PassThru
    $script:HelperProcesses.Add($proc)

    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    $port = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path -LiteralPath $portFile) {
            $portText = [IO.File]::ReadAllText($portFile).Trim()
            if (-not [string]::IsNullOrWhiteSpace($portText)) {
                $port = [int]$portText
                break
            }
        }
        Start-Sleep -Milliseconds 200
    }
    if ($null -eq $port) {
        throw "待ち受けhelperがportを報告しませんでした: $BareName"
    }
    return [pscustomobject]@{ Process = $proc; Port = $port }
}

function ConvertTo-PowerShellLiteral {
    param([Parameter(Mandatory = $true)][string]$Value)

    return "'" + $Value.Replace("'", "''") + "'"
}

# NamedArgs はキーが -パラメーター名、値が文字列1個または文字列配列のハッシュテーブル。
# `-File` へ生のargvを渡す方式は、複数値を[string[]]パラメーターへ束縛できない
# （子processの引数解析では、呼び出し側でのカンマ配列リテラルの構文解析が効かない
# ため）。実測で確認したこの制約を避けるため、実際のPowerShellソースを組み立てて
# -EncodedCommand で渡す。ignore-reason-has-number.test.ps1と同じ方式。
function Build-WatcherInvocationCommand {
    param([Parameter(Mandatory = $true)][hashtable]$NamedArgs)

    # $global:ProgressPreferenceを本体スクリプトの実行前にも設定する。本体スクリプト
    # 自身も同じ設定をしているが、そのさらに外側（この呼び出しの最初）でも止めておく
    # 二重の防御（watch-forbidden-processes.ps1側のコメント参照）。
    $command = "`$global:ProgressPreference = 'SilentlyContinue'; " +
        '[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); & ' + (ConvertTo-PowerShellLiteral $scriptPath)
    foreach ($key in $NamedArgs.Keys) {
        $value = $NamedArgs[$key]
        if ($value -is [switch] -or $value -is [bool]) {
            if ([bool]$value) {
                $command += (" -{0}" -f $key)
            }
            continue
        }
        $command += (" -{0}" -f $key)
        $valueArray = @($value)
        if ($valueArray.Count -eq 1) {
            $command += (" " + (ConvertTo-PowerShellLiteral ([string]$valueArray[0])))
        }
        else {
            $literals = @($valueArray | ForEach-Object { ConvertTo-PowerShellLiteral ([string]$_) })
            $command += (" " + ([string]::Join(",", $literals)))
        }
    }
    return $command
}

# $ProgressPreference/$global:ProgressPreferenceを両方止めても、CIM問い合わせの
# 進捗レコード(CLIXML)が稀にファイルへ混入することを実測した(2026-08-30、15回中1回)。
# 本体側の対策だけに頼らず、読み取った文字列側でも「#< CLIXML」で始まるブロックを
# 除去する二重の防御。ブロックが閉じていない(途中で切れた)場合は元の文字列を返す。
function Remove-CliXmlNoise {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    # 2つ目以降の混入ブロックでは「#< CLIXML」の次行の先頭に空白が入ることを実測した
    # （別の書き手からの行と競合して字下げされたとみられる）。[ \t]*で許容する。
    return [regex]::Replace($Text, '(?s)#<\s*CLIXML\r?\n[ \t]*<Objs[^>]*>.*?</Objs>\r?\n?', '')
}

# 背景監視の1回目判定(基準)が完了したことを、固定sleepではなく実際の出力の目印で
# 確かめる。Start-Job→cmd.exe→子powershell.exeという3段の起動待ち時間は機械の
# 混雑状況で伸び縮みするため、固定sleepだけでは「まだ基準が記録されていないのに
# 遅れて来たはずのprocessまで基準に含まれてしまう」誤判定が起き得る
# (2026-08-30、この誤判定を新規process 5回中1回の頻度で実測・再現した)。
function Wait-ForOutputMarker {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Marker,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            try {
                # cmd.exeの`1>`リダイレクトは書込み中のファイルを他プロセスの読取りと
                # 共有しない排他寄りのモードで開くため、書込みの最中に読もうとすると
                # IOException(共有違反)になることを実測した。ポーリング中の一時的な
                # 共有違反は「まだ読めない」として次の周期へ回す。
                # 子processは[Console]::OutputEncoding = UTF8Encoding($false)で起動している
                # ため、BOM無しUTF-8として読む(File.ReadAllTextの既定と同じにする。
                # [Text.Encoding]::Defaultはこの機ではShift_JIS系のシステム既定コードページに
                # なり、日本語の目印文字列が一致しなくなるため使わない)。
                $fileStream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
                try {
                    $reader = New-Object IO.StreamReader($fileStream, [Text.Encoding]::UTF8)
                    $text = $reader.ReadToEnd()
                }
                finally {
                    $fileStream.Dispose()
                }
                if ($text.Contains($Marker)) { return $true }
            }
            catch [IO.IOException] {
                # 共有違反等。次の周期で読み直す。
            }
        }
        Start-Sleep -Milliseconds 200
    }
    return $false
}

function Invoke-Watcher {
    param([Parameter(Mandatory = $true)][hashtable]$NamedArgs)

    # 註: 当初 `& $powerShellPath ... 1> file 2> file` を直接（cmd.exeを介さず）実行していた。
    # 対話セッションでの手元検証では再現しなかったが、統括の指摘を受けて cmd.exe /c 経由の
    # 完全に独立した新規プロセスで実測したところ、「子processが再びpowershell.exeである」場合に
    # Get-NetTCPConnectionの初回モジュール読込み進捗（CLIXML化される）をこのプロセス自身が
    # デシリアライズしようとして失敗し、exit 1になる不具合を実測・再現した（新規プロセスでの
    # 初回実行時に起きやすく、モジュールが暖まっている対話セッションでは再現しにくかった＝
    # 見かけ上「合格」に見えた原因）。Start-WatcherBackgroundと同じ cmd.exe /c 経由へ統一して解消。
    $stamp = [Guid]::NewGuid().ToString("N")
    $stdoutPath = Join-Path $sandboxRoot ("stdout-{0}.txt" -f $stamp)
    $stderrPath = Join-Path $sandboxRoot ("stderr-{0}.txt" -f $stamp)
    $powerShellPath = (Get-Process -Id $PID).Path
    $childCommand = Build-WatcherInvocationCommand -NamedArgs $NamedArgs
    $encodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($childCommand))
    $cmdLine = '"' + $powerShellPath + '" -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand ' +
        $encodedCommand + ' 1> "' + $stdoutPath + '" 2> "' + $stderrPath + '"'
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        & cmd.exe /c $cmdLine
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    $parts = New-Object System.Collections.Generic.List[string]
    if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) { $parts.Add((Remove-CliXmlNoise ([IO.File]::ReadAllText($stdoutPath)))) }
    if (Test-Path -LiteralPath $stderrPath -PathType Leaf) { $parts.Add((Remove-CliXmlNoise ([IO.File]::ReadAllText($stderrPath)))) }
    return [pscustomobject]@{ ExitCode = $exitCode; Output = ($parts -join "`n") }
}

# 継続監視モードをバックグラウンドで開始する（非ブロッキング）。反復1と反復2の間に
# 別のhelperを起動できるよう、待ち合わせずに戻る。
# 註1: 当初 Start-Process -PassThru + -RedirectStandardOutput で組んだところ、
# HasExited/WaitForExitは正しくtrueになるのに .ExitCode が常に$null になる
# (このPowerShell 5.1環境で実測・再現済み)。Start-Job はジョブ内で実行した
# 子processの $LASTEXITCODE を確実に受け取れるため、こちらに置き換えている。
# 註2: ジョブ内で `& powershell.exe ... 2> file` を使っても、「子processが
# powershell.exeである」場合はPowerShell自身が標準エラーをCLIXMLとして
# デシリアライズしようとし、モジュール初回読込みの進捗レコード等で
# 「XMLを処理できません」の例外になることを実測した（Invoke-Watcherの
# フォアグラウンド呼び出しでは起きないが、ジョブ内の入れ子呼び出しでは起きる）。
# cmd.exe /c 経由でリダイレクトすると、cmd側はPowerShellの構造化ストリームを
# 一切解釈せず生バイトのまま file へ落とすため、この問題を回避できる。
function Start-WatcherBackground {
    param([Parameter(Mandatory = $true)][hashtable]$NamedArgs)

    $powerShellPath = (Get-Process -Id $PID).Path
    $childCommand = Build-WatcherInvocationCommand -NamedArgs $NamedArgs
    $encodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($childCommand))
    $stamp = [Guid]::NewGuid().ToString("N")
    $stdoutPath = Join-Path $sandboxRoot ("bgstdout-{0}.txt" -f $stamp)
    $stderrPath = Join-Path $sandboxRoot ("bgstderr-{0}.txt" -f $stamp)
    $exitCodePath = Join-Path $sandboxRoot ("bgexitcode-{0}.txt" -f $stamp)

    # 註3: ジョブ自身の戻り値([pscustomobject]をReceive-Jobで受け取る経路)は、
    # ジョブをホストする子powershell.exe自身のCLIXML通信チャネル(標準出力)を使う。
    # $ProgressPreference/$global:ProgressPreferenceを止めてもGet-NetTCPConnection
    # の進捗レコードが稀に(2026-08-30の実測で新規process 15回中1回)このチャネルへ
    # 混入し、Receive-Jobが「ProcessStreamReader_CliXmlError」で失敗することを
    # 実測した。終了コードはこの脆いチャネルに頼らず、stdout/stderrと同じ
    # 「cmd.exe /c 経由でファイルへ書く」頑丈な経路で受け渡す。
    $job = Start-Job -ScriptBlock {
        param($ExePath, $Encoded, $OutPath, $ErrPath, $ExitPath)
        $cmdLine = '"' + $ExePath + '" -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand ' +
            $Encoded + ' 1> "' + $OutPath + '" 2> "' + $ErrPath + '"'
        & cmd.exe /c $cmdLine
        [IO.File]::WriteAllText($ExitPath, [string]$LASTEXITCODE)
    } -ArgumentList $powerShellPath, $encodedCommand, $stdoutPath, $stderrPath, $exitCodePath

    return [pscustomobject]@{ Job = $job; StdOutPath = $stdoutPath; StdErrPath = $stderrPath; ExitCodePath = $exitCodePath }
}

function Wait-WatcherBackground {
    param(
        [Parameter(Mandatory = $true)]$Handle,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds
    )

    $completed = Wait-Job -Job $Handle.Job -Timeout $TimeoutSeconds
    if ($null -eq $completed) {
        return [pscustomobject]@{ Waited = $false; ExitCode = $null; Output = "" }
    }
    # Receive-Jobの戻り値(ジョブ自身のCLIXML通信チャネル)は註3のとおり稀に壊れるため、
    # 終了コードの取得には使わない。ジョブに残る警告・エラー等を読み捨てるためだけに
    # 呼び、失敗しても(-ErrorActionとtry/catchの二重で)テストを止めない。
    try {
        Receive-Job -Job $Handle.Job -ErrorAction SilentlyContinue | Out-Null
    }
    catch {
        # 註3のCLIXML破損はここで無視してよい。終了コードはファイルから読む。
    }
    $exitCodeText = $null
    if (Test-Path -LiteralPath $Handle.ExitCodePath -PathType Leaf) {
        $exitCodeText = [IO.File]::ReadAllText($Handle.ExitCodePath).Trim()
    }
    $exitCode = $null
    if (-not [string]::IsNullOrWhiteSpace($exitCodeText)) { $exitCode = [int]$exitCodeText }
    $parts = New-Object System.Collections.Generic.List[string]
    if (Test-Path -LiteralPath $Handle.StdOutPath -PathType Leaf) { $parts.Add((Remove-CliXmlNoise ([IO.File]::ReadAllText($Handle.StdOutPath)))) }
    if (Test-Path -LiteralPath $Handle.StdErrPath -PathType Leaf) { $parts.Add((Remove-CliXmlNoise ([IO.File]::ReadAllText($Handle.StdErrPath)))) }
    return [pscustomobject]@{ Waited = $true; ExitCode = $exitCode; Output = ($parts -join "`n") }
}

function Remove-TestSandbox {
    Get-Job -ErrorAction SilentlyContinue | Stop-Job -ErrorAction SilentlyContinue
    Get-Job -ErrorAction SilentlyContinue | Remove-Job -Force -ErrorAction SilentlyContinue
    foreach ($proc in $script:HelperProcesses) {
        try {
            if (-not $proc.HasExited) {
                Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
            }
        }
        catch {
            # 既に終了している場合は無視する
        }
    }
    if (-not (Test-Path -LiteralPath $sandboxRoot)) { return }
    $resolved = [IO.Path]::GetFullPath($sandboxRoot).TrimEnd([char[]]"\/")
    $parent = [IO.Path]::GetDirectoryName($resolved)
    $leaf = [IO.Path]::GetFileName($resolved)
    if (($parent -ne $tempBase) -or
        (-not [regex]::IsMatch($leaf, '^ori3-watch-forbidden-test-[0-9a-f]{32}$', [Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
        throw "安全でない一時領域の削除を拒否しました: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction SilentlyContinue
}

if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
    throw "監視本体が見つかりません: $scriptPath"
}

[void][IO.Directory]::CreateDirectory($sandboxRoot)
try {
    Write-Output "[1/7] 単発判定でブラウザ・desktop.exe・配信サーバー(名前一致)を同時検出し、全て開始時点扱いにする"
    $fakeBrowserName = "fakechrome-{0}" -f [Guid]::NewGuid().ToString("N").Substring(0, 8)
    $fakeDesktopName = "fakedesktop-{0}" -f [Guid]::NewGuid().ToString("N").Substring(0, 8)
    $fakeViteName = "fakevite-{0}" -f [Guid]::NewGuid().ToString("N").Substring(0, 8)
    $browserProc = Start-NamedHelperProcess -BareName $fakeBrowserName
    $desktopProc = Start-NamedHelperProcess -BareName $fakeDesktopName
    $viteProc = Start-NamedHelperProcess -BareName $fakeViteName

    $result = Invoke-Watcher -NamedArgs @{
        Once = $true
        BrowserProcessNames = $fakeBrowserName
        DesktopProcessName = $fakeDesktopName
        DevServerProcessNames = $fakeViteName
    }
    Assert-Equal $result.ExitCode 0 "単発判定は常にexit 0であること(監視スクリプトはゲートではない)" $result.Output
    Assert-Contains $result.Output ("[検出/開始時点扱い] ブラウザ PID={0}" -f $browserProc.Id) "偽ブラウザをブラウザとして検出すること"
    Assert-Contains $result.Output ("[検出/開始時点扱い] desktop.exe PID={0}" -f $desktopProc.Id) "偽desktop.exeをdesktop.exeとして検出すること"
    Assert-Contains $result.Output ("[検出/開始時点扱い] 配信サーバー(名前一致) PID={0}" -f $viteProc.Id) "偽viteを配信サーバー(名前一致)として検出すること"
    Assert-Contains $result.Output "検出 3件" "検出件数3件を集計に表示すること"
    Assert-NotContains $result.Output "注意/開始後に出現" "単発判定は経過時間が無いため注意(新規出現)を出さないこと"

    Write-Output "[2/7] 対象外のプロセス名は検出しない(誤検出の回避)"
    $unrelatedName = "notepad-{0}" -f [Guid]::NewGuid().ToString("N").Substring(0, 8)
    $unrelatedProc = Start-NamedHelperProcess -BareName $unrelatedName
    $result = Invoke-Watcher -NamedArgs @{
        Once = $true
        BrowserProcessNames = $fakeBrowserName
        DesktopProcessName = $fakeDesktopName
        DevServerProcessNames = $fakeViteName
    }
    Assert-NotContains $result.Output ("PID={0}" -f $unrelatedProc.Id) "対象外プロセス名は一覧に出さないこと"

    Write-Output "[3/7] node待ち受けを名前とport待ち受けの両方から検出する"
    $fakeNodeName = "node-{0}" -f [Guid]::NewGuid().ToString("N").Substring(0, 8)
    $nodeListener = Start-ListeningHelperProcess -BareName $fakeNodeName
    $result = Invoke-Watcher -NamedArgs @{
        Once = $true
        BrowserProcessNames = $fakeBrowserName
        DesktopProcessName = $fakeDesktopName
        DevServerProcessNames = $fakeViteName
        NodeLikeProcessNames = $fakeNodeName
        DevServerPorts = "1"
    }
    Assert-Contains $result.Output ("PID={0}" -f $nodeListener.Process.Id) "node名の待ち受けプロセスを検出すること"
    Assert-Contains $result.Output "nodeの待ち受け" "検出理由にnodeの待ち受けを明示すること"
    Assert-Contains $result.Output ("待ち受けport={0}" -f $nodeListener.Port) "実際の待ち受けport番号を表示すること"

    Write-Output "[4/7] プロセス名を問わず既知の配信port待ち受けを検出する"
    $anonName = "toolhelper-{0}" -f [Guid]::NewGuid().ToString("N").Substring(0, 8)
    $anonListener = Start-ListeningHelperProcess -BareName $anonName
    $result = Invoke-Watcher -NamedArgs @{
        Once = $true
        BrowserProcessNames = $fakeBrowserName
        DesktopProcessName = $fakeDesktopName
        DevServerProcessNames = $fakeViteName
        NodeLikeProcessNames = $fakeNodeName
        DevServerPorts = [string]$anonListener.Port
    }
    Assert-Contains $result.Output ("PID={0}" -f $anonListener.Process.Id) "無関係な名前でも既知portの待ち受けは検出すること"
    # 註: [3/7]のnode名helperがまだ生きており出力全体には「nodeの待ち受け」が残るため、
    # 全体一致ではなく、この無関係な名前のhelper自身の理由文だけを厳密に照合する。
    Assert-Contains $result.Output ("配信サーバー(既知port待ち受け({0})) PID={1}" -f $anonListener.Port, $anonListener.Process.Id) "無関係な名前のhelperはnode理由を付けず既知port理由だけにすること"

    Write-Output "[5/7] 継続監視: 開始時点から居るものと、開始後に現れたものを区別する"
    $earlyBrowserName = "earlybrowser-{0}" -f [Guid]::NewGuid().ToString("N").Substring(0, 8)
    $lateBrowserName = "latebrowser-{0}" -f [Guid]::NewGuid().ToString("N").Substring(0, 8)
    $earlyProc = Start-NamedHelperProcess -BareName $earlyBrowserName -WaitSeconds 20
    $handle = Start-WatcherBackground -NamedArgs @{
        MaxIterations = "2"
        TestIntervalSeconds = "4"
        BrowserProcessNames = @($earlyBrowserName, $lateBrowserName)
    }
    $baselineReady = Wait-ForOutputMarker -Path $handle.StdOutPath -Marker "[集計] 監視開始時点の検出" -TimeoutSeconds 20
    Assert-True $baselineReady "背景監視の1回目判定(基準)が20秒以内に完了すること(固定sleepでなく実際の出力で確認)"
    $lateProc = Start-NamedHelperProcess -BareName $lateBrowserName -WaitSeconds 20
    $bg = Wait-WatcherBackground -Handle $handle -TimeoutSeconds 30
    Assert-True $bg.Waited "継続監視は2回の判定後に自ら終了すること(MaxIterations=2)"
    $bgOutput = $bg.Output
    Assert-Equal $bg.ExitCode 0 "継続監視の終了コードも0であること" $bgOutput
    Assert-Contains $bgOutput ("[開始時点] ブラウザ PID={0}" -f $earlyProc.Id) "先に居たプロセスは[開始時点]で記録されること"
    Assert-Contains $bgOutput ("[開始時点から継続] ブラウザ PID={0}" -f $earlyProc.Id) "先に居たプロセスは2回目も継続扱いになること"
    Assert-Contains $bgOutput ("[注意/開始後に出現] ブラウザ PID={0}" -f $lateProc.Id) "監視開始後に現れたプロセスは注意として出すこと"
    Assert-NotContains $bgOutput ("[開始時点] ブラウザ PID={0}" -f $lateProc.Id) "後から現れたプロセスを開始時点扱いにしないこと"

    Write-Output "[6/7] 監視はプロセスもファイルも変更しない(非破壊)"
    Assert-True (-not $earlyProc.HasExited) "先に居たテストprocessを監視スクリプトが止めていないこと"
    Assert-True (-not $lateProc.HasExited) "後から現れたテストprocessも監視スクリプトが止めていないこと"
    Assert-True (-not $browserProc.HasExited) "[1/7]の偽ブラウザも生きたままであること"
    Assert-True (-not $desktopProc.HasExited) "[1/7]の偽desktop.exeも生きたままであること"

    Write-Output "[7/7] 表示テキストで非破壊の既定動作を明示している"
    $result = Invoke-Watcher -NamedArgs @{ Once = $true }
    Assert-Contains $result.Output "動作=表示のみ（プロセス停止=0 / ファイル変更=0）" "既定動作の表明を毎回表示すること"
    Assert-Contains $result.Output "プロセス名だけで犯人と断定しません" "犯人断定をしない旨を明示すること"
    Assert-Equal $result.ExitCode 0 "実機のブラウザ等が検出されても単発判定はexit 0であること" $result.Output

    Write-Output ("watch-forbidden-processes self-test passed: 7 cases, {0} assertions" -f $script:AssertionCount)
}
finally {
    Remove-TestSandbox
}
