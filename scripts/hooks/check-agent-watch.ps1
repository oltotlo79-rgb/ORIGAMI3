<#
.SYNOPSIS
委譲前に、10分間隔の停滞監視が実際に継続していることを検査します。

.DESCRIPTION
Check は終了コード 0=正常、1=監視policy不適合、2=検査不能を返します。
Hook は Claude Code の PreToolUse payloadを標準入力から受け、mainが呼ぶ委譲系toolだけを
同じ条件でfail-closedにします。非空文字列agent_idのsubagentは対象外です。
Hook接続時は `-Action Hook` を明示してください。
#>
[CmdletBinding()]
param(
    [ValidateSet("Check", "Hook")]
    [string]$Action = "Check",

    [string]$RepositoryRoot = ""
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
$script:Utf8NoBom = New-Object Text.UTF8Encoding($false, $true)
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
$script:RequiredIntervalMinutes = 10
$script:RequiredStaleAfterMinutes = 40
# 実機で観測した継続scan間隔の最大は600.253秒だった。12分はそこへ
# 119.747秒（約2分）のscheduler/走査猶予を加え、1周期を逃した監視は通さない境界である。
$script:FreshnessMinutes = 12.0
$script:FutureToleranceMinutes = 2.0
$script:RetryCount = 4
$script:RetryMilliseconds = 50

function New-AgentWatchResult {
    param(
        [Parameter(Mandatory = $true)][int]$ExitCode,
        [Parameter(Mandatory = $true)][string]$Code,
        [Parameter(Mandatory = $true)][string]$Message,
        [object[]]$AgentStates = @(),
        [bool]$RuntimeMismatch = $false,
        [string]$RuntimeMismatchMessage = ""
    )

    return [pscustomobject]@{
        ExitCode = $ExitCode
        Code = $Code
        Message = $Message
        AgentStates = @($AgentStates)
        RuntimeMismatch = $RuntimeMismatch
        RuntimeMismatchMessage = $RuntimeMismatchMessage
    }
}

function Read-Utf8StandardInput {
    $stream = [Console]::OpenStandardInput()
    $reader = New-Object IO.StreamReader($stream, $script:Utf8NoBom, $false)
    try {
        $text = $reader.ReadToEnd()
    }
    finally {
        $reader.Dispose()
    }
    while ($text.Length -gt 0 -and $text[0] -eq [char]0xFEFF) {
        $text = $text.Substring(1)
    }
    return $text
}

function New-PolicyResult {
    param(
        [Parameter(Mandatory = $true)][string]$Code,
        [Parameter(Mandatory = $true)][string]$Message
    )
    return New-AgentWatchResult -ExitCode 1 -Code $Code -Message $Message
}

function New-CheckErrorResult {
    param(
        [Parameter(Mandatory = $true)][string]$Code,
        [Parameter(Mandatory = $true)][string]$Message
    )
    return New-AgentWatchResult -ExitCode 2 -Code $Code -Message $Message
}

function Get-ScriptFullPath {
    $path = [string]$MyInvocation.ScriptName
    if ([string]::IsNullOrWhiteSpace($path)) {
        $path = [string]$PSCommandPath
    }
    if ([string]::IsNullOrWhiteSpace($path)) {
        throw "検査scriptの実パスを取得できません"
    }
    return [IO.Path]::GetFullPath($path)
}

function Resolve-RepositoryRoot {
    param([string]$SuppliedRoot)

    $root = $SuppliedRoot
    if ([string]::IsNullOrWhiteSpace($root)) {
        $root = [string]$env:CLAUDE_PROJECT_DIR
    }
    if ([string]::IsNullOrWhiteSpace($root)) {
        $hookPath = Get-ScriptFullPath
        $hookDirectory = Split-Path -Parent $hookPath
        $scriptsDirectory = Split-Path -Parent $hookDirectory
        $root = Split-Path -Parent $scriptsDirectory
    }
    if ([string]::IsNullOrWhiteSpace($root)) {
        throw "RepositoryRootを解決できません"
    }
    $fullRoot = [IO.Path]::GetFullPath($root).TrimEnd([char[]]"\/")
    if (-not (Test-Path -LiteralPath $fullRoot -PathType Container)) {
        throw "RepositoryRootが存在しません: $fullRoot"
    }
    return $fullRoot
}

function Read-SharedFileBytes {
    param([Parameter(Mandatory = $true)][string]$Path)

    $stream = New-Object IO.FileStream(
        $Path,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
    )
    try {
        if ($stream.Length -gt [int]::MaxValue) {
            throw "検査対象が大きすぎます: $Path"
        }
        $bytes = New-Object byte[] ([int]$stream.Length)
        $offset = 0
        while ($offset -lt $bytes.Length) {
            $read = $stream.Read($bytes, $offset, $bytes.Length - $offset)
            if ($read -le 0) {
                throw "検査対象を最後まで読めません: $Path"
            }
            $offset += $read
        }
        return $bytes
    }
    finally {
        $stream.Dispose()
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

function Get-SharedFileSha256Hex {
    param([Parameter(Mandatory = $true)][string]$Path)
    return Get-Sha256HexFromBytes -Bytes (Read-SharedFileBytes -Path $Path)
}

function Get-Sha256HexFromText {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)
    return Get-Sha256HexFromBytes -Bytes $script:Utf8NoBom.GetBytes($Text)
}

function ConvertTo-WatchBase64 {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)
    return [Convert]::ToBase64String($script:Utf8NoBom.GetBytes($Text))
}

function ConvertTo-CanonicalWatchPath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$BasePath
    )

    $fullPath = if ([IO.Path]::IsPathRooted($Path)) {
        [IO.Path]::GetFullPath($Path)
    }
    else {
        [IO.Path]::GetFullPath((Join-Path $BasePath $Path))
    }
    return $fullPath.Replace([IO.Path]::AltDirectorySeparatorChar, [IO.Path]::DirectorySeparatorChar)
}

function Get-AgentKeyFromDefinition {
    param(
        [Parameter(Mandatory = $true)]$Agent,
        [Parameter(Mandatory = $true)][string]$Root
    )

    $names = @($Agent.PSObject.Properties.Name)
    if ($names -notcontains "reportPath" -or $names -notcontains "sourcePaths") {
        throw "監視定義の担当にreportPath/sourcePathsがありません"
    }
    $reportPath = [string]$Agent.reportPath
    $sourcePaths = @($Agent.sourcePaths)
    if ([string]::IsNullOrWhiteSpace($reportPath) -or $sourcePaths.Count -eq 0) {
        throw "監視定義のreportPath/sourcePathsが空です"
    }
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("version=1")
    $lines.Add("reportPath=$(ConvertTo-WatchBase64 -Text (ConvertTo-CanonicalWatchPath -Path $reportPath -BasePath $Root))")
    foreach ($sourcePath in $sourcePaths) {
        $sourceText = [string]$sourcePath
        if ([string]::IsNullOrWhiteSpace($sourceText)) {
            throw "監視定義のsourcePathsに空のpathがあります"
        }
        $lines.Add("sourcePath=$(ConvertTo-WatchBase64 -Text (ConvertTo-CanonicalWatchPath -Path $sourceText -BasePath $Root))")
    }
    return Get-Sha256HexFromText -Text ($lines -join "`n")
}

function Get-IncidentIdFromState {
    param([Parameter(Mandatory = $true)]$AgentState)

    $text = @(
        "version=1",
        "agentKey=$([string]$AgentState.agentKey)",
        "status=$([string]$AgentState.status)",
        "latestPath=$(ConvertTo-WatchBase64 -Text ([string]$AgentState.latestPath))",
        "latestWriteUtc=$([string]$AgentState.latestWriteUtc)",
        "problemDigest=$([string]$AgentState.problemDigest)"
    ) -join "`n"
    return Get-Sha256HexFromText -Text $text
}

function Get-AgentStatesSha256 {
    param([Parameter(Mandatory = $true)][object[]]$AgentStates)

    $lines = foreach ($agentState in $AgentStates) {
        @(
            "agentKey=$([string]$agentState.agentKey)",
            "name=$(ConvertTo-WatchBase64 -Text ([string]$agentState.name))",
            "status=$([string]$agentState.status)",
            "latestPath=$(ConvertTo-WatchBase64 -Text ([string]$agentState.latestPath))",
            "latestWriteUtc=$([string]$agentState.latestWriteUtc)",
            "problemDigest=$([string]$agentState.problemDigest)",
            "incidentId=$([string]$agentState.incidentId)"
        ) -join "`t"
    }
    return Get-Sha256HexFromText -Text (@($lines) -join "`n")
}

function Get-NativeProcessArguments {
    param([Parameter(Mandatory = $true)][int]$ProcessId)

    if (-not ("Ori3.AgentWatchNativeCommandLine" -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

namespace Ori3 {
    public static class AgentWatchNativeCommandLine {
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern IntPtr OpenProcess(uint access, bool inheritHandle, int processId);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool CloseHandle(IntPtr handle);

        [DllImport("ntdll.dll")]
        private static extern int NtQueryInformationProcess(
            IntPtr processHandle,
            int processInformationClass,
            IntPtr processInformation,
            int processInformationLength,
            out int returnLength);

        [DllImport("shell32.dll", SetLastError = true)]
        private static extern IntPtr CommandLineToArgvW(
            [MarshalAs(UnmanagedType.LPWStr)] string commandLine,
            out int argumentCount);

        [DllImport("kernel32.dll")]
        private static extern IntPtr LocalFree(IntPtr memory);

        public static string Query(int processId) {
            const uint PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;
            const int ProcessCommandLineInformation = 60;
            IntPtr process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, processId);
            if (process == IntPtr.Zero) {
                throw new Win32Exception(Marshal.GetLastWin32Error(), "OpenProcess failed");
            }
            try {
                int required;
                NtQueryInformationProcess(process, ProcessCommandLineInformation, IntPtr.Zero, 0, out required);
                if (required <= 0) {
                    throw new InvalidOperationException("NtQueryInformationProcess did not return a buffer length");
                }
                IntPtr buffer = Marshal.AllocHGlobal(required);
                try {
                    int returned;
                    int status = NtQueryInformationProcess(
                        process,
                        ProcessCommandLineInformation,
                        buffer,
                        required,
                        out returned);
                    if (status != 0) {
                        throw new InvalidOperationException(
                            "NtQueryInformationProcess failed: NTSTATUS=0x" + status.ToString("x8"));
                    }
                    int length = (ushort)Marshal.ReadInt16(buffer, 0);
                    int pointerOffset = IntPtr.Size == 8 ? 8 : 4;
                    IntPtr textPointer = Marshal.ReadIntPtr(buffer, pointerOffset);
                    if (length < 0 || (length % 2) != 0 || (length > 0 && textPointer == IntPtr.Zero)) {
                        throw new InvalidOperationException("Process command line UNICODE_STRING is invalid");
                    }
                    return length == 0 ? String.Empty : Marshal.PtrToStringUni(textPointer, length / 2);
                }
                finally {
                    Marshal.FreeHGlobal(buffer);
                }
            }
            finally {
                CloseHandle(process);
            }
        }

        public static string[] Parse(string commandLine) {
            int count;
            IntPtr values = CommandLineToArgvW(commandLine, out count);
            if (values == IntPtr.Zero) {
                throw new Win32Exception(Marshal.GetLastWin32Error(), "CommandLineToArgvW failed");
            }
            try {
                string[] result = new string[count];
                for (int index = 0; index < count; index++) {
                    IntPtr value = Marshal.ReadIntPtr(values, index * IntPtr.Size);
                    result[index] = Marshal.PtrToStringUni(value);
                }
                return result;
            }
            finally {
                LocalFree(values);
            }
        }
    }
}
'@
    }
    $commandLine = [Ori3.AgentWatchNativeCommandLine]::Query($ProcessId)
    if ([string]::IsNullOrWhiteSpace($commandLine)) {
        throw "監視processのcommand lineが空です: PID=$ProcessId"
    }
    return [Ori3.AgentWatchNativeCommandLine]::Parse($commandLine)
}

function Test-WatcherProcessArguments {
    param(
        [Parameter(Mandatory = $true)][int]$ProcessId,
        [Parameter(Mandatory = $true)][string]$ProcessExecutablePath,
        [Parameter(Mandatory = $true)][string]$WatcherPath,
        [Parameter(Mandatory = $true)][string]$DefinitionPath,
        [Parameter(Mandatory = $true)][string]$Root
    )

    try {
        $arguments = @(Get-NativeProcessArguments -ProcessId $ProcessId)
    }
    catch {
        return New-CheckErrorResult -Code "PROCESS_COMMAND_READ_ERROR" -Message "監視processのnative command lineを読めません: PID=$ProcessId ($($_.Exception.Message))"
    }
    if ($arguments.Count -ne 16) {
        return New-PolicyResult -Code "PROCESS_COMMAND_MISMATCH" -Message "監視processのargv件数が固定起動形と一致しません: PID=$ProcessId argc=$($arguments.Count) expected=16"
    }
    foreach ($pathPair in @(
        @($arguments[0], $ProcessExecutablePath, "argv[0]"),
        @($arguments[7], $WatcherPath, "-File"),
        @($arguments[9], $DefinitionPath, "-DefinitionPath"),
        @($arguments[11], $Root, "-RepositoryRoot")
    )) {
        if (-not (Test-SamePath -Actual ([string]$pathPair[0]) -Expected ([string]$pathPair[1]))) {
            return New-PolicyResult -Code "PROCESS_COMMAND_MISMATCH" -Message "監視processの$($pathPair[2]) pathが固定起動形と一致しません。"
        }
    }
    $expectedLiteralArguments = [ordered]@{
        1 = "-NoLogo"
        2 = "-NoProfile"
        3 = "-NonInteractive"
        4 = "-ExecutionPolicy"
        5 = "Bypass"
        6 = "-File"
        8 = "-DefinitionPath"
        10 = "-RepositoryRoot"
        12 = "-IntervalMinutes"
        13 = "10"
        14 = "-StaleAfterMinutes"
        15 = "40"
    }
    foreach ($entry in $expectedLiteralArguments.GetEnumerator()) {
        $index = [int]$entry.Key
        if (-not [string]::Equals([string]$arguments[$index], [string]$entry.Value, [StringComparison]::Ordinal)) {
            return New-PolicyResult -Code "PROCESS_COMMAND_MISMATCH" -Message "監視processのargv[$index]が固定起動形と一致しません。"
        }
    }
    return New-AgentWatchResult -ExitCode 0 -Code "PROCESS_COMMAND_OK" -Message "監視processのnative argvは固定起動形と一致しています。"
}

function Test-ReparsePoint {
    param([Parameter(Mandatory = $true)][string]$Path)

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    return (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)
}

function Test-SamePath {
    param(
        [Parameter(Mandatory = $true)][string]$Actual,
        [Parameter(Mandatory = $true)][string]$Expected
    )
    try {
        $actualFull = [IO.Path]::GetFullPath($Actual).TrimEnd([char[]]"\/")
        $expectedFull = [IO.Path]::GetFullPath($Expected).TrimEnd([char[]]"\/")
    }
    catch {
        return $false
    }
    return [string]::Equals($actualFull, $expectedFull, [StringComparison]::OrdinalIgnoreCase)
}

function Get-DelegationText {
    param(
        [Parameter(Mandatory = $true)]$HookPayload,
        [Parameter(Mandatory = $true)][string]$ToolName
    )

    $fieldByTool = @{
        "Agent" = "prompt"
        "SendMessage" = "message"
        "mcp__codex__codex" = "prompt"
        "mcp__codex__codex-reply" = "prompt"
    }
    if (-not $fieldByTool.ContainsKey($ToolName)) {
        throw "停滞応答を検査する委譲toolではありません: $ToolName"
    }
    $toolInputProperty = $HookPayload.PSObject.Properties["tool_input"]
    if ($null -eq $toolInputProperty -or $null -eq $toolInputProperty.Value) {
        throw "PreToolUse payloadにtool_inputがありません: tool=$ToolName"
    }
    $fieldName = [string]$fieldByTool[$ToolName]
    $textProperty = $toolInputProperty.Value.PSObject.Properties[$fieldName]
    if ($null -eq $textProperty -or -not ($textProperty.Value -is [string])) {
        throw "PreToolUse payloadの実text fieldが文字列ではありません: tool=$ToolName field=tool_input.$fieldName"
    }
    return [string]$textProperty.Value
}

function Test-AgentWatchResponses {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$IncidentIds
    )

    $openingMarker = "[AGENT_WATCH_RESPONSE schema=1]"
    $closingMarker = "[/AGENT_WATCH_RESPONSE]"
    $current = @{}
    foreach ($incidentId in $IncidentIds) {
        if (-not [regex]::IsMatch($incidentId, '^[0-9a-f]{64}$')) {
            return New-CheckErrorResult -Code "INCIDENT_ID_INVALID" -Message "runtime stateのincidentIdが不正です: $incidentId"
        }
        if ($current.ContainsKey($incidentId)) {
            return New-CheckErrorResult -Code "INCIDENT_ID_DUPLICATE" -Message "runtime stateのincidentIdが重複しています: $incidentId"
        }
        $current[$incidentId] = $true
    }

    if ($current.Count -eq 0) {
        if ($Text.Contains($openingMarker) -or $Text.Contains($closingMarker)) {
            return New-PolicyResult -Code "STALL_RESPONSE_UNEXPECTED" -Message "現在の停滞に対応しないAGENT_WATCH_RESPONSEがあります。古い・未知の宣言は使えません。"
        }
        return New-AgentWatchResult -ExitCode 0 -Code "STALL_RESPONSE_NOT_REQUIRED" -Message "停滞応答は不要です。"
    }

    if (-not $Text.StartsWith($openingMarker, [StringComparison]::Ordinal)) {
        $reason = if ($Text.Contains($openingMarker) -or $Text.Contains($closingMarker)) {
            "AGENT_WATCH_RESPONSEは委譲textの先頭に、引用・code fence・字下げなしで置いてください。"
        }
        else {
            "現在の停滞すべてに対するAGENT_WATCH_RESPONSEがありません: incidents=$(@($current.Keys | Sort-Object) -join ',')"
        }
        return New-PolicyResult -Code "STALL_RESPONSE_MISSING" -Message $reason
    }

    $blockPattern = ('\A' +
        '\[AGENT_WATCH_RESPONSE schema=1\]\r?\n' +
        'incident=(?<incident>[0-9a-f]{64})\r?\n' +
        'action=(?<action>investigate|continue|reassign|stop-request|complete-check)\r?\n' +
        'evidence=(?<evidence>[^\r\n]+)\r?\n' +
        'next=(?<next>[^\r\n]+)\r?\n' +
        '\[/AGENT_WATCH_RESPONSE\]')
    $seen = @{}
    $remaining = $Text
    while ($remaining.StartsWith($openingMarker, [StringComparison]::Ordinal)) {
        $match = [regex]::Match($remaining, $blockPattern, [Text.RegularExpressions.RegexOptions]::CultureInvariant)
        if (-not $match.Success) {
            return New-PolicyResult -Code "STALL_RESPONSE_FORMAT" -Message "AGENT_WATCH_RESPONSEのfield、順序、改行、actionが承認済み形式と一致しません。"
        }
        $incident = [string]$match.Groups["incident"].Value
        $action = [string]$match.Groups["action"].Value
        $evidence = [string]$match.Groups["evidence"].Value
        $next = [string]$match.Groups["next"].Value
        if (-not [string]::Equals($evidence, $evidence.Trim(), [StringComparison]::Ordinal) -or [string]::IsNullOrWhiteSpace($evidence)) {
            return New-PolicyResult -Code "STALL_RESPONSE_EVIDENCE" -Message "evidenceには空白だけでない実測または判断根拠を1行で書いてください。"
        }
        if ($action -eq "reassign" -and -not [string]::Equals(
            $evidence,
            "agent-inquiry-timeout-v1 attempt1=timeout:7200s attempt2=timeout:7200s",
            [StringComparison]::Ordinal
        )) {
            return New-PolicyResult -Code "STALL_REASSIGN_EVIDENCE" -Message (
                "action=reassignには、同じincidentへの問い合わせ2件が各7200秒で時間切れになった実測が必要です。" +
                "evidenceは 'agent-inquiry-timeout-v1 attempt1=timeout:7200s attempt2=timeout:7200s' の完全一致にしてください。" +
                "更新時刻・process数・CPU・空応答だけではreassignできません。証拠がなければaction=investigateを使ってください。"
            )
        }
        if (-not [string]::Equals($next, $next.Trim(), [StringComparison]::Ordinal) -or [string]::IsNullOrWhiteSpace($next)) {
            return New-PolicyResult -Code "STALL_RESPONSE_NEXT" -Message "nextには空白だけでない次の行動または再確認条件を1行で書いてください。"
        }
        if ($action -eq "continue" -and
            -not [regex]::IsMatch($next, '^progress-when:\S(?:.*\S)?$', [Text.RegularExpressions.RegexOptions]::CultureInvariant)) {
            return New-PolicyResult -Code "STALL_CONTINUE_CONDITION" -Message "action=continueではnext=progress-when:<空白でない観測条件>を必須とします。"
        }
        if (-not $current.ContainsKey($incident)) {
            return New-PolicyResult -Code "STALL_RESPONSE_UNKNOWN" -Message "現在の停滞に無い、古いまたは未知のincidentです: $incident"
        }
        if ($seen.ContainsKey($incident)) {
            return New-PolicyResult -Code "STALL_RESPONSE_DUPLICATE" -Message "同じincidentへの宣言が重複しています: $incident"
        }
        $seen[$incident] = $true
        $remaining = $remaining.Substring($match.Length)
        if ($remaining.StartsWith("`r`n", [StringComparison]::Ordinal)) {
            $remaining = $remaining.Substring(2)
        }
        elseif ($remaining.StartsWith("`n", [StringComparison]::Ordinal)) {
            $remaining = $remaining.Substring(1)
        }
        elseif ($remaining.Length -gt 0) {
            return New-PolicyResult -Code "STALL_RESPONSE_FORMAT" -Message "AGENT_WATCH_RESPONSEの終端後には改行が必要です。"
        }
    }

    if ($remaining.Contains($openingMarker) -or $remaining.Contains($closingMarker)) {
        return New-PolicyResult -Code "STALL_RESPONSE_QUOTED" -Message "先頭の宣言領域外、引用、code fence内のAGENT_WATCH_RESPONSEは宣言として扱いません。"
    }
    $missing = @($current.Keys | Where-Object { -not $seen.ContainsKey($_) } | Sort-Object)
    if ($missing.Count -gt 0) {
        return New-PolicyResult -Code "STALL_RESPONSE_INCOMPLETE" -Message (
            "現在の停滞への宣言が不足しています: missing={0} incident={1}" -f $missing.Count, ($missing -join ",")
        )
    }
    return New-AgentWatchResult -ExitCode 0 -Code "STALL_RESPONSE_OK" -Message "現在の停滞すべてに1対1の対応宣言があります: incidents=$($seen.Count)"
}

function Get-RequiredStateValue {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Name
    )

    if (@($State.PSObject.Properties.Name) -notcontains $Name) {
        throw "runtime stateの必須fieldがありません: $Name"
    }
    return $State.$Name
}

function Get-RequiredIntegerStateValue {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Name
    )

    $value = Get-RequiredStateValue -State $State -Name $Name
    if ($null -eq $value) {
        throw "runtime stateの整数fieldがnullです: $Name"
    }
    $typeName = $value.GetType().FullName
    if (@(
            "System.SByte", "System.Byte", "System.Int16", "System.UInt16",
            "System.Int32", "System.UInt32", "System.Int64"
        ) -notcontains $typeName) {
        throw "runtime stateの整数fieldがJSON整数ではありません: $Name type=$typeName"
    }
    return [Convert]::ToInt64($value, [Globalization.CultureInfo]::InvariantCulture)
}

function Parse-RoundtripUtc {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$FieldName
    )

    try {
        $parsed = [DateTime]::ParseExact(
            $Text,
            "o",
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::RoundtripKind
        )
    }
    catch {
        throw "runtime stateの日時fieldを読めません: $FieldName=$Text"
    }
    return $parsed.ToUniversalTime()
}

function Test-FreshTimestamp {
    param(
        [Parameter(Mandatory = $true)][DateTime]$TimestampUtc,
        [Parameter(Mandatory = $true)][DateTime]$NowUtc,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $ageMinutes = ($NowUtc - $TimestampUtc).TotalMinutes
    if ($ageMinutes -lt (-1.0 * $script:FutureToleranceMinutes)) {
        return New-CheckErrorResult -Code "FUTURE_TIMESTAMP" -Message (
            "{0} が現在より {1:N1} 分未来です。時計またはstateを確認してください。" -f $Label, (-1.0 * $ageMinutes)
        )
    }
    if ($ageMinutes -gt $script:FreshnessMinutes) {
        return New-PolicyResult -Code "STALE" -Message (
            "{0} が {1:N1} 分更新されていません（上限 {2:N0} 分）。" -f $Label, $ageMinutes, $script:FreshnessMinutes
        )
    }
    return $null
}

function Test-LockHeld {
    param([Parameter(Mandatory = $true)][string]$LockPath)

    try {
        $probe = New-Object IO.FileStream(
            $LockPath,
            [IO.FileMode]::Open,
            [IO.FileAccess]::ReadWrite,
            [IO.FileShare]::None
        )
        $probe.Dispose()
        return New-PolicyResult -Code "LOCK_NOT_HELD" -Message "singleton lockが保持されていません: $LockPath"
    }
    catch [IO.IOException] {
        $nativeCode = ($_.Exception.GetBaseException().HResult -band 0xFFFF)
        if ($nativeCode -eq 32 -or $nativeCode -eq 33) {
            return $null
        }
        return New-CheckErrorResult -Code "LOCK_CHECK_ERROR" -Message (
            "singleton lockを検査できません（Win32=$nativeCode）: $LockPath ($($_.Exception.Message))"
        )
    }
    catch {
        return New-CheckErrorResult -Code "LOCK_CHECK_ERROR" -Message (
            "singleton lockを検査できません: $LockPath ($($_.Exception.Message))"
        )
    }
}

function Get-CurrentWatchedPathState {
    param(
        [Parameter(Mandatory = $true)][string]$ConfiguredPath,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][bool]$MustBeFile
    )

    $fullPath = ConvertTo-CanonicalWatchPath -Path $ConfiguredPath -BasePath $Root
    if (-not (Test-Path -LiteralPath $fullPath)) {
        return [pscustomobject]@{
            FullPath = $fullPath
            LatestPath = ""
            LastWriteTimeUtc = $null
            Problem = "存在しません"
        }
    }
    $item = Get-Item -LiteralPath $fullPath -Force -ErrorAction Stop
    if ($MustBeFile -and -not ($item -is [IO.FileInfo])) {
        return [pscustomobject]@{
            FullPath = $fullPath
            LatestPath = ""
            LastWriteTimeUtc = $null
            Problem = "報告書にファイルでないパスが指定されています"
        }
    }
    if ($item -is [IO.FileInfo]) {
        return [pscustomobject]@{
            FullPath = $fullPath
            LatestPath = ConvertTo-CanonicalWatchPath -Path $item.FullName -BasePath $Root
            LastWriteTimeUtc = $item.LastWriteTimeUtc
            Problem = ""
        }
    }
    try {
        $files = @(Get-ChildItem -LiteralPath $fullPath -File -Recurse -Force -ErrorAction Stop)
    }
    catch {
        return [pscustomobject]@{
            FullPath = $fullPath
            LatestPath = ""
            LastWriteTimeUtc = $null
            Problem = "配下を走査できません: $($_.Exception.Message)"
        }
    }
    if ($files.Count -eq 0) {
        return [pscustomobject]@{
            FullPath = $fullPath
            LatestPath = ""
            LastWriteTimeUtc = $null
            Problem = "配下に監視できるファイルがありません"
        }
    }
    $latest = $files |
        Sort-Object `
            @{ Expression = { $_.LastWriteTimeUtc }; Descending = $true },
            @{ Expression = { ConvertTo-CanonicalWatchPath -Path $_.FullName -BasePath $Root }; Ascending = $true } |
        Select-Object -First 1
    return [pscustomobject]@{
        FullPath = $fullPath
        LatestPath = ConvertTo-CanonicalWatchPath -Path $latest.FullName -BasePath $Root
        LastWriteTimeUtc = $latest.LastWriteTimeUtc
        Problem = ""
    }
}

function Get-CurrentProblemDigest {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Problems)

    $lines = foreach ($problem in $Problems) {
        "path=$(ConvertTo-WatchBase64 -Text ([string]$problem.FullPath))`tproblem=$(ConvertTo-WatchBase64 -Text ([string]$problem.Problem))"
    }
    return Get-Sha256HexFromText -Text (@($lines) -join "`n")
}

function Get-CurrentAgentStates {
    param(
        [Parameter(Mandatory = $true)][object[]]$DefinitionAgents,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][DateTime]$NowUtc
    )

    $states = New-Object System.Collections.Generic.List[object]
    foreach ($agent in $DefinitionAgents) {
        $reportState = Get-CurrentWatchedPathState `
            -ConfiguredPath ([string]$agent.reportPath) `
            -Root $Root `
            -MustBeFile $true
        $sourceStates = @(
            foreach ($sourcePath in @($agent.sourcePaths)) {
                Get-CurrentWatchedPathState `
                    -ConfiguredPath ([string]$sourcePath) `
                    -Root $Root `
                    -MustBeFile $false
            }
        )
        $allStates = @($reportState) + @($sourceStates)
        $problems = @($allStates | Where-Object { -not [string]::IsNullOrEmpty([string]$_.Problem) })
        $futureBoundaryUtc = $NowUtc.AddMinutes($script:FutureToleranceMinutes)
        $futureProblems = @(
            foreach ($watchedState in $allStates) {
                if ($null -eq $watchedState.LastWriteTimeUtc -or
                    $watchedState.LastWriteTimeUtc -le $futureBoundaryUtc) {
                    continue
                }
                $futurePath = if ([string]::IsNullOrEmpty([string]$watchedState.LatestPath)) {
                    [string]$watchedState.FullPath
                }
                else {
                    [string]$watchedState.LatestPath
                }
                [pscustomobject]@{
                    FullPath = $futurePath
                    Problem = "更新時刻が許容範囲の2分を超えて未来です: lastWriteUtc=$(([DateTime]$watchedState.LastWriteTimeUtc).ToUniversalTime().ToString('o'))"
                }
            }
        )
        $problems = @($problems) + @($futureProblems)
        $validStates = @($allStates | Where-Object { $null -ne $_.LastWriteTimeUtc })
        $latestState = $null
        if ($validStates.Count -gt 0) {
            $latestState = $validStates |
                Sort-Object `
                    @{ Expression = { $_.LastWriteTimeUtc }; Descending = $true },
                    @{ Expression = { [string]$_.LatestPath }; Ascending = $true } |
                Select-Object -First 1
        }
        $status = if ($problems.Count -gt 0 -or $null -eq $latestState) {
            "unmonitorable"
        }
        elseif ($latestState.LastWriteTimeUtc -gt $NowUtc.AddMinutes(-1.0 * $script:RequiredStaleAfterMinutes)) {
            "active"
        }
        else {
            "stalled"
        }
        $latestPath = if ($null -eq $latestState) { "" } else { [string]$latestState.LatestPath }
        $latestWriteUtc = if ($null -eq $latestState) { "" } else { ([DateTime]$latestState.LastWriteTimeUtc).ToUniversalTime().ToString("o") }
        $agentState = [pscustomobject][ordered]@{
            agentKey = Get-AgentKeyFromDefinition -Agent $agent -Root $Root
            name = [string]$agent.name
            status = $status
            latestPath = $latestPath
            latestWriteUtc = $latestWriteUtc
            problemDigest = Get-CurrentProblemDigest -Problems $problems
            incidentId = ""
        }
        if ($status -ne "active") {
            $agentState.incidentId = Get-IncidentIdFromState -AgentState $agentState
        }
        $states.Add($agentState)
    }
    return $states.ToArray()
}

function Test-AgentStateContract {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$DefinitionPath,
        [Parameter(Mandatory = $true)][object[]]$AgentStates,
        [Parameter(Mandatory = $true)][int]$AgentCount,
        [Parameter(Mandatory = $true)][int]$ActiveCount,
        [Parameter(Mandatory = $true)][int]$StalledCount,
        [Parameter(Mandatory = $true)][int]$UnmonitorableCount,
        [Parameter(Mandatory = $true)][string]$AgentStatesSha256,
        [Parameter(Mandatory = $true)][byte[]]$OutputBytes
    )

    try {
        $definitionText = $script:Utf8NoBom.GetString((Read-SharedFileBytes -Path $DefinitionPath))
        $definition = $definitionText | ConvertFrom-Json
        if ($null -eq $definition -or @($definition.PSObject.Properties.Name) -notcontains "agents") {
            throw "監視定義にagents配列がありません"
        }
        $definitionAgents = @($definition.agents)
    }
    catch {
        return New-CheckErrorResult -Code "DEFINITION_SCHEMA_ERROR" -Message "監視定義からagent stateを照合できません: $($_.Exception.Message)"
    }
    if ($definitionAgents.Count -ne $AgentCount) {
        return New-PolicyResult -Code "DEFINITION_AGENT_COUNT_MISMATCH" -Message (
            "runtime stateと監視定義の担当件数が一致しません: runtime=$AgentCount definition=$($definitionAgents.Count)"
        )
    }

    $allowedProperties = @("agentKey", "name", "status", "latestPath", "latestWriteUtc", "problemDigest", "incidentId")
    $seenKeys = @{}
    $seenIncidents = @{}
    $actualActive = 0
    $actualStalled = 0
    $actualUnmonitorable = 0
    for ($index = 0; $index -lt $AgentStates.Count; $index++) {
        $agentState = $AgentStates[$index]
        if ($null -eq $agentState) {
            return New-CheckErrorResult -Code "AGENT_STATE_SCHEMA_ERROR" -Message "agentStates[$index]がnullです。"
        }
        $properties = @($agentState.PSObject.Properties.Name)
        foreach ($requiredProperty in $allowedProperties) {
            if ($properties -notcontains $requiredProperty) {
                return New-CheckErrorResult -Code "AGENT_STATE_SCHEMA_ERROR" -Message "agentStates[$index]の必須fieldがありません: $requiredProperty"
            }
        }
        $extraProperties = @($properties | Where-Object { $allowedProperties -notcontains $_ })
        if ($extraProperties.Count -gt 0) {
            return New-PolicyResult -Code "AGENT_STATE_SCHEMA_MISMATCH" -Message "agentStates[$index]にschema 2で未定義のfieldがあります: $($extraProperties -join ',')"
        }

        $agentKey = [string]$agentState.agentKey
        $name = [string]$agentState.name
        $status = [string]$agentState.status
        $latestPath = [string]$agentState.latestPath
        $latestWriteUtc = [string]$agentState.latestWriteUtc
        $problemDigest = [string]$agentState.problemDigest
        $incidentId = [string]$agentState.incidentId
        if (-not [regex]::IsMatch($agentKey, '^[0-9a-f]{64}$') -or
            -not [regex]::IsMatch($problemDigest, '^[0-9a-f]{64}$')) {
            return New-CheckErrorResult -Code "AGENT_STATE_DIGEST_INVALID" -Message "agentStates[$index]のagentKey/problemDigestがlowercase SHA-256ではありません。"
        }
        if ($seenKeys.ContainsKey($agentKey)) {
            return New-PolicyResult -Code "AGENT_KEY_DUPLICATE" -Message "agentStatesのagentKeyが重複しています: $agentKey"
        }
        $seenKeys[$agentKey] = $true

        $definitionAgent = $definitionAgents[$index]
        $definitionNames = @($definitionAgent.PSObject.Properties.Name)
        if ($definitionNames -notcontains "name" -or [string]::IsNullOrWhiteSpace([string]$definitionAgent.name)) {
            return New-CheckErrorResult -Code "DEFINITION_SCHEMA_ERROR" -Message "監視定義のagents[$index].nameが空です。"
        }
        if (-not [string]::Equals($name, [string]$definitionAgent.name, [StringComparison]::Ordinal)) {
            return New-PolicyResult -Code "AGENT_NAME_MISMATCH" -Message "agentStates[$index].nameが監視定義と一致しません。"
        }
        try {
            $expectedAgentKey = Get-AgentKeyFromDefinition -Agent $definitionAgent -Root $Root
        }
        catch {
            return New-CheckErrorResult -Code "DEFINITION_SCHEMA_ERROR" -Message $_.Exception.Message
        }
        if (-not [string]::Equals($agentKey, $expectedAgentKey, [StringComparison]::Ordinal)) {
            return New-PolicyResult -Code "AGENT_KEY_MISMATCH" -Message "agentStates[$index].agentKeyが監視定義のpathから再計算した値と一致しません。"
        }

        if ([string]::IsNullOrEmpty($latestPath) -ne [string]::IsNullOrEmpty($latestWriteUtc)) {
            return New-PolicyResult -Code "AGENT_LATEST_MISMATCH" -Message "agentStates[$index]のlatestPath/latestWriteUtcの有無が一致しません。"
        }
        if (-not [string]::IsNullOrEmpty($latestPath)) {
            try {
                $canonicalLatestPath = ConvertTo-CanonicalWatchPath -Path $latestPath -BasePath $Root
                $parsedLatestUtc = Parse-RoundtripUtc -Text $latestWriteUtc -FieldName "agentStates[$index].latestWriteUtc"
            }
            catch {
                return New-CheckErrorResult -Code "AGENT_LATEST_INVALID" -Message $_.Exception.Message
            }
            if (-not [string]::Equals($latestPath, $canonicalLatestPath, [StringComparison]::Ordinal) -or
                -not [string]::Equals($latestWriteUtc, $parsedLatestUtc.ToString("o"), [StringComparison]::Ordinal)) {
                return New-PolicyResult -Code "AGENT_LATEST_NOT_CANONICAL" -Message "agentStates[$index]のlatest path/timeがcanonical表現ではありません。"
            }
        }

        switch ($status) {
            "active" {
                $actualActive += 1
                if ([string]::IsNullOrEmpty($latestWriteUtc) -or -not [string]::IsNullOrEmpty($incidentId)) {
                    return New-PolicyResult -Code "AGENT_STATUS_MISMATCH" -Message "active stateはlatestを持ちincidentIdを持たない必要があります: index=$index"
                }
            }
            "stalled" {
                $actualStalled += 1
                if ([string]::IsNullOrEmpty($latestWriteUtc) -or -not [regex]::IsMatch($incidentId, '^[0-9a-f]{64}$')) {
                    return New-PolicyResult -Code "AGENT_STATUS_MISMATCH" -Message "stalled stateはlatestとlowercase incidentIdを持つ必要があります: index=$index"
                }
            }
            "unmonitorable" {
                $actualUnmonitorable += 1
                if (-not [regex]::IsMatch($incidentId, '^[0-9a-f]{64}$')) {
                    return New-PolicyResult -Code "AGENT_STATUS_MISMATCH" -Message "unmonitorable stateはlowercase incidentIdを持つ必要があります: index=$index"
                }
            }
            default {
                return New-PolicyResult -Code "AGENT_STATUS_INVALID" -Message "agentStates[$index].statusがschema 2の語彙ではありません: $status"
            }
        }
        if ($status -ne "active") {
            $expectedIncidentId = Get-IncidentIdFromState -AgentState $agentState
            if (-not [string]::Equals($incidentId, $expectedIncidentId, [StringComparison]::Ordinal)) {
                return New-PolicyResult -Code "INCIDENT_HASH_MISMATCH" -Message "agentStates[$index].incidentIdがstateから再計算した値と一致しません。"
            }
            if ($seenIncidents.ContainsKey($incidentId)) {
                return New-PolicyResult -Code "INCIDENT_ID_DUPLICATE" -Message "異なる担当のincidentIdが重複しています: $incidentId"
            }
            $seenIncidents[$incidentId] = $true
        }
    }
    if ($actualActive -ne $ActiveCount -or $actualStalled -ne $StalledCount -or $actualUnmonitorable -ne $UnmonitorableCount) {
        return New-PolicyResult -Code "AGENT_STATUS_COUNT_MISMATCH" -Message (
            "agentStatesのstatus件数がtop-level countと一致しません: active=$actualActive/$ActiveCount stalled=$actualStalled/$StalledCount unmonitorable=$actualUnmonitorable/$UnmonitorableCount"
        )
    }
    $actualStatesHash = Get-AgentStatesSha256 -AgentStates $AgentStates
    if (-not [string]::Equals($actualStatesHash, $AgentStatesSha256, [StringComparison]::Ordinal)) {
        return New-PolicyResult -Code "AGENT_STATES_HASH_MISMATCH" -Message "agentStatesSha256がagentStatesのcanonical hashと一致しません。"
    }

    try {
        $outputText = $script:Utf8NoBom.GetString($OutputBytes)
    }
    catch {
        return New-CheckErrorResult -Code "OUTPUT_UTF8_ERROR" -Message "latest outputを厳密UTF-8として読めません: $($_.Exception.Message)"
    }
    $outputLines = @($outputText -split "`r?`n")
    $expectedSummary = "AGENT_WATCH_STATUS schema=2 total=$AgentCount active=$ActiveCount stalled=$StalledCount unmonitorable=$UnmonitorableCount states_sha256=$AgentStatesSha256"
    $summaryLines = @($outputLines | Where-Object { $_.StartsWith("AGENT_WATCH_STATUS", [StringComparison]::Ordinal) })
    if ($summaryLines.Count -ne 1 -or -not [string]::Equals([string]$summaryLines[0], $expectedSummary, [StringComparison]::Ordinal)) {
        return New-PolicyResult -Code "AGENT_STATUS_SUMMARY_MISMATCH" -Message "latest outputのmachine-readable status summaryがruntime stateとexact一致しません。"
    }
    $expectedIncidentLines = @(
        foreach ($agentState in $AgentStates) {
            if ([string]$agentState.status -eq "active") { continue }
            "AGENT_WATCH_INCIDENT schema=2 status=$([string]$agentState.status) incident=$([string]$agentState.incidentId) agent_key=$([string]$agentState.agentKey) name_b64=$(ConvertTo-WatchBase64 -Text ([string]$agentState.name))"
        }
    )
    $actualIncidentLines = @($outputLines | Where-Object { $_.StartsWith("AGENT_WATCH_INCIDENT", [StringComparison]::Ordinal) })
    if ($actualIncidentLines.Count -ne $expectedIncidentLines.Count) {
        return New-PolicyResult -Code "AGENT_INCIDENT_SUMMARY_MISMATCH" -Message "latest outputのincident行数がruntime stateと一致しません。"
    }
    for ($index = 0; $index -lt $expectedIncidentLines.Count; $index++) {
        if (-not [string]::Equals([string]$actualIncidentLines[$index], [string]$expectedIncidentLines[$index], [StringComparison]::Ordinal)) {
            return New-PolicyResult -Code "AGENT_INCIDENT_SUMMARY_MISMATCH" -Message "latest outputのincident行がruntime stateとexact一致しません: index=$index"
        }
    }
    try {
        $liveStates = @(Get-CurrentAgentStates `
            -DefinitionAgents $definitionAgents `
            -Root $Root `
            -NowUtc ([DateTime]::UtcNow))
    }
    catch {
        return New-CheckErrorResult -Code "LIVE_AGENT_STATE_ERROR" -Message "監視対象をhook側で再走査できません: $($_.Exception.Message)"
    }
    $mismatches = New-Object System.Collections.Generic.List[string]
    $stateFields = @("agentKey", "name", "status", "latestPath", "latestWriteUtc", "problemDigest", "incidentId")
    for ($index = 0; $index -lt $AgentStates.Count; $index++) {
        foreach ($field in $stateFields) {
            $runtimeValue = [string]$AgentStates[$index].$field
            $liveValue = [string]$liveStates[$index].$field
            if (-not [string]::Equals($runtimeValue, $liveValue, [StringComparison]::Ordinal)) {
                $mismatches.Add("index=$index field=$field runtime=$runtimeValue live=$liveValue")
            }
        }
    }
    if ($mismatches.Count -gt 0) {
        $detail = @($mismatches | Select-Object -First 4) -join "; "
        return New-AgentWatchResult `
            -ExitCode 0 `
            -Code "AGENT_STATES_LIVE_CHANGED" `
            -Message "runtime/outputは自己整合していますが、hookの再走査結果と一致しません。live stateを委譲gateの正本にします: mismatches=$($mismatches.Count) $detail" `
            -AgentStates $liveStates `
            -RuntimeMismatch $true `
            -RuntimeMismatchMessage $detail
    }
    return New-AgentWatchResult -ExitCode 0 -Code "AGENT_STATES_OK" -Message "agent state契約はlive再走査まで一致しています。" -AgentStates $liveStates
}

function Test-AgentWatchSnapshot {
    param([Parameter(Mandatory = $true)][string]$Root)

    $expectedWatcherPath = [IO.Path]::GetFullPath((Join-Path $Root "scripts\watch-agents.ps1"))
    $scratchpadPath = [IO.Path]::GetFullPath((Join-Path $Root "scratchpad"))
    $expectedRuntimePath = [IO.Path]::GetFullPath((Join-Path $scratchpadPath "watch-agents.runtime.json"))
    $expectedOutputPath = [IO.Path]::GetFullPath((Join-Path $scratchpadPath "watch-agents.latest.log"))
    $expectedLockPath = [IO.Path]::GetFullPath((Join-Path $scratchpadPath "watch-agents.lock"))

    if (-not (Test-Path -LiteralPath $expectedWatcherPath -PathType Leaf)) {
        return New-CheckErrorResult -Code "WATCH_SCRIPT_MISSING" -Message "監視scriptが存在しません: $expectedWatcherPath"
    }
    if (-not (Test-Path -LiteralPath $expectedRuntimePath -PathType Leaf)) {
        return New-PolicyResult -Code "STATE_MISSING" -Message "継続監視のruntime stateがありません: $expectedRuntimePath"
    }

    try {
        if (Test-ReparsePoint -Path $expectedRuntimePath) {
            return New-PolicyResult -Code "STATE_REPARSE_POINT" -Message "runtime stateにreparse pointは使えません: $expectedRuntimePath"
        }
        $stateBytes = Read-SharedFileBytes -Path $expectedRuntimePath
        $stateText = $script:Utf8NoBom.GetString($stateBytes)
        $state = $stateText | ConvertFrom-Json
    }
    catch {
        return New-CheckErrorResult -Code "STATE_READ_ERROR" -Message "runtime stateを検査できません: $($_.Exception.Message)"
    }

    try {
        $schemaVersion = [int](Get-RequiredIntegerStateValue -State $state -Name "schemaVersion")
    }
    catch {
        return New-CheckErrorResult -Code "STATE_SCHEMA_ERROR" -Message $_.Exception.Message
    }
    if ($schemaVersion -ne 2) {
        return New-PolicyResult -Code "SCHEMA_MISMATCH" -Message "runtime state schemaVersionが2ではありません。watcherを現在版で再起動してください: $schemaVersion"
    }

    try {
        $schemaVersion = [int](Get-RequiredIntegerStateValue -State $state -Name "schemaVersion")
        $instanceId = [string](Get-RequiredStateValue -State $state -Name "instanceId")
        $watchPid = [int](Get-RequiredIntegerStateValue -State $state -Name "pid")
        $processStartUtc = Parse-RoundtripUtc -Text ([string](Get-RequiredStateValue -State $state -Name "processStartUtc")) -FieldName "processStartUtc"
        $processExecutablePath = [string](Get-RequiredStateValue -State $state -Name "processExecutablePath")
        $scriptPath = [string](Get-RequiredStateValue -State $state -Name "scriptPath")
        $scriptSha256 = [string](Get-RequiredStateValue -State $state -Name "scriptSha256")
        $stateRoot = [string](Get-RequiredStateValue -State $state -Name "repositoryRoot")
        $definitionPath = [string](Get-RequiredStateValue -State $state -Name "definitionPath")
        $definitionSha256 = [string](Get-RequiredStateValue -State $state -Name "definitionSha256")
        $runtimePath = [string](Get-RequiredStateValue -State $state -Name "runtimePath")
        $outputPath = [string](Get-RequiredStateValue -State $state -Name "outputPath")
        $outputSha256 = [string](Get-RequiredStateValue -State $state -Name "outputSha256")
        $outputLength = [Int64](Get-RequiredIntegerStateValue -State $state -Name "outputLength")
        $outputLastWriteUtc = Parse-RoundtripUtc -Text ([string](Get-RequiredStateValue -State $state -Name "outputLastWriteUtc")) -FieldName "outputLastWriteUtc"
        $lockPath = [string](Get-RequiredStateValue -State $state -Name "lockPath")
        $mode = [string](Get-RequiredStateValue -State $state -Name "mode")
        $intervalMinutes = [int](Get-RequiredIntegerStateValue -State $state -Name "intervalMinutes")
        $staleAfterMinutes = [int](Get-RequiredIntegerStateValue -State $state -Name "staleAfterMinutes")
        $agentCount = [int](Get-RequiredIntegerStateValue -State $state -Name "agentCount")
        $activeCount = [int](Get-RequiredIntegerStateValue -State $state -Name "activeCount")
        $stalledCount = [int](Get-RequiredIntegerStateValue -State $state -Name "stalledCount")
        $unmonitorableCount = [int](Get-RequiredIntegerStateValue -State $state -Name "unmonitorableCount")
        $agentStatesSha256 = [string](Get-RequiredStateValue -State $state -Name "agentStatesSha256")
        $agentStates = @((Get-RequiredStateValue -State $state -Name "agentStates"))
        $scanSequence = [Int64](Get-RequiredIntegerStateValue -State $state -Name "scanSequence")
        $scanCompletedUtc = Parse-RoundtripUtc -Text ([string](Get-RequiredStateValue -State $state -Name "scanCompletedUtc")) -FieldName "scanCompletedUtc"
        $stateWrittenUtc = Parse-RoundtripUtc -Text ([string](Get-RequiredStateValue -State $state -Name "stateWrittenUtc")) -FieldName "stateWrittenUtc"
    }
    catch {
        return New-CheckErrorResult -Code "STATE_SCHEMA_ERROR" -Message $_.Exception.Message
    }

    $parsedInstanceId = [Guid]::Empty
    if (-not [Guid]::TryParse($instanceId, [ref]$parsedInstanceId) -or $parsedInstanceId -eq [Guid]::Empty) {
        return New-CheckErrorResult -Code "INSTANCE_ID_INVALID" -Message "runtime stateのinstanceIdが不正です: $instanceId"
    }
    if ($mode -ne "continuous") {
        return New-PolicyResult -Code "MODE_NOT_CONTINUOUS" -Message "-Once/単発判定は有効な継続監視として扱いません: mode=$mode"
    }
    if ($intervalMinutes -ne $script:RequiredIntervalMinutes) {
        return New-PolicyResult -Code "INTERVAL_MISMATCH" -Message (
            "監視間隔が10分ではありません: intervalMinutes=$intervalMinutes"
        )
    }
    if ($staleAfterMinutes -ne $script:RequiredStaleAfterMinutes) {
        return New-PolicyResult -Code "STALE_THRESHOLD_MISMATCH" -Message (
            "担当停滞の閾値が40分ではありません: staleAfterMinutes=$staleAfterMinutes"
        )
    }
    if ($watchPid -le 0 -or $agentCount -le 0 -or $scanSequence -le 0 -or
        $activeCount -lt 0 -or $stalledCount -lt 0 -or $unmonitorableCount -lt 0) {
        return New-CheckErrorResult -Code "STATE_VALUE_INVALID" -Message (
            "runtime stateの数値が不正です: pid=$watchPid agentCount=$agentCount active=$activeCount stalled=$stalledCount unmonitorable=$unmonitorableCount scanSequence=$scanSequence"
        )
    }
    if ($agentStates.Count -ne $agentCount -or ($activeCount + $stalledCount + $unmonitorableCount) -ne $agentCount) {
        return New-PolicyResult -Code "AGENT_STATE_COUNT_MISMATCH" -Message (
            "runtime stateの担当件数が一致しません: total=$agentCount states=$($agentStates.Count) active=$activeCount stalled=$stalledCount unmonitorable=$unmonitorableCount"
        )
    }
    if (-not [regex]::IsMatch($agentStatesSha256, '^[0-9a-f]{64}$')) {
        return New-CheckErrorResult -Code "AGENT_STATES_HASH_INVALID" -Message "runtime stateのagentStatesSha256がlowercase SHA-256ではありません。"
    }

    $pathChecks = @(
        @("repositoryRoot", $stateRoot, $Root),
        @("scriptPath", $scriptPath, $expectedWatcherPath),
        @("runtimePath", $runtimePath, $expectedRuntimePath),
        @("outputPath", $outputPath, $expectedOutputPath),
        @("lockPath", $lockPath, $expectedLockPath)
    )
    foreach ($pathCheck in $pathChecks) {
        if (-not (Test-SamePath -Actual ([string]$pathCheck[1]) -Expected ([string]$pathCheck[2]))) {
            return New-PolicyResult -Code "PATH_MISMATCH" -Message (
                "runtime stateの{0}が固定実パスと一致しません: actual={1} expected={2}" -f
                    $pathCheck[0], $pathCheck[1], $pathCheck[2]
            )
        }
    }

    foreach ($requiredFile in @($expectedWatcherPath, $definitionPath, $expectedOutputPath, $expectedLockPath)) {
        if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
            return New-PolicyResult -Code "RUNTIME_FILE_MISSING" -Message "監視runtimeの必須fileがありません: $requiredFile"
        }
        try {
            if (Test-ReparsePoint -Path $requiredFile) {
                return New-PolicyResult -Code "RUNTIME_REPARSE_POINT" -Message "監視runtimeにreparse pointは使えません: $requiredFile"
            }
        }
        catch {
            return New-CheckErrorResult -Code "FILE_METADATA_ERROR" -Message "file metadataを検査できません: $requiredFile ($($_.Exception.Message))"
        }
    }

    try {
        $process = Get-Process -Id $watchPid -ErrorAction Stop
    }
    catch {
        if ($_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound) {
            return New-PolicyResult -Code "PROCESS_MISSING" -Message "runtime stateの監視processが存在しません: PID=$watchPid"
        }
        return New-CheckErrorResult -Code "PROCESS_CHECK_ERROR" -Message "監視processを検査できません: PID=$watchPid ($($_.Exception.Message))"
    }

    try {
        $actualStartUtc = $process.StartTime.ToUniversalTime()
        $actualProcessPath = [IO.Path]::GetFullPath([string]$process.Path)
    }
    catch {
        return New-CheckErrorResult -Code "PROCESS_METADATA_ERROR" -Message "監視processの開始時刻/実パスを取得できません: PID=$watchPid ($($_.Exception.Message))"
    }
    if ($actualStartUtc.Ticks -ne $processStartUtc.Ticks) {
        return New-PolicyResult -Code "PROCESS_START_MISMATCH" -Message (
            "PIDの開始時刻がruntime stateと一致しません: PID=$watchPid state=$($processStartUtc.ToString('o')) actual=$($actualStartUtc.ToString('o'))"
        )
    }
    if (-not (Test-SamePath -Actual $actualProcessPath -Expected $processExecutablePath)) {
        return New-PolicyResult -Code "PROCESS_PATH_MISMATCH" -Message (
            "監視processの実行ファイルがruntime stateと一致しません: PID=$watchPid"
        )
    }
    $lockResult = Test-LockHeld -LockPath $expectedLockPath
    if ($null -ne $lockResult) {
        return $lockResult
    }

    try {
        if ((Get-SharedFileSha256Hex -Path $expectedWatcherPath) -ne $scriptSha256.ToLowerInvariant()) {
            return New-PolicyResult -Code "SCRIPT_HASH_MISMATCH" -Message "稼働中watcherと現在のwatch-agents.ps1のhashが一致しません。再起動してください。"
        }
        if ((Get-SharedFileSha256Hex -Path $definitionPath) -ne $definitionSha256.ToLowerInvariant()) {
            return New-PolicyResult -Code "DEFINITION_HASH_MISMATCH" -Message "監視定義が最後のscan後に変わっています。次のscan完了まで委譲できません。"
        }
        $outputBytes = Read-SharedFileBytes -Path $expectedOutputPath
        if ([Int64]$outputBytes.Length -ne $outputLength) {
            return New-PolicyResult -Code "OUTPUT_METADATA_MISMATCH" -Message "latest outputの長さがruntime stateと一致しません。"
        }
        if ((Get-Sha256HexFromBytes -Bytes $outputBytes) -ne $outputSha256.ToLowerInvariant()) {
            return New-PolicyResult -Code "OUTPUT_HASH_MISMATCH" -Message "latest outputのhashがruntime stateと一致しません。"
        }
        $runtimeItem = Get-Item -LiteralPath $expectedRuntimePath -Force -ErrorAction Stop
        $outputItem = Get-Item -LiteralPath $expectedOutputPath -Force -ErrorAction Stop
    }
    catch {
        return New-CheckErrorResult -Code "HASH_CHECK_ERROR" -Message "監視fileのhash/metadataを検査できません: $($_.Exception.Message)"
    }
    if ($outputItem.LastWriteTimeUtc.Ticks -ne $outputLastWriteUtc.Ticks) {
        return New-PolicyResult -Code "OUTPUT_METADATA_MISMATCH" -Message "latest outputの更新時刻がruntime stateと一致しません。"
    }

    $agentStateResult = Test-AgentStateContract `
        -Root $Root `
        -DefinitionPath $definitionPath `
        -AgentStates $agentStates `
        -AgentCount $agentCount `
        -ActiveCount $activeCount `
        -StalledCount $stalledCount `
        -UnmonitorableCount $unmonitorableCount `
        -AgentStatesSha256 $agentStatesSha256 `
        -OutputBytes $outputBytes
    if ($agentStateResult.ExitCode -ne 0) {
        return $agentStateResult
    }

    $nowUtc = [DateTime]::UtcNow
    foreach ($freshnessCheck in @(
        @($scanCompletedUtc, "runtime stateのscanCompletedUtc"),
        @($stateWrittenUtc, "runtime stateのstateWrittenUtc"),
        @($runtimeItem.LastWriteTimeUtc, "runtime state file"),
        @($outputItem.LastWriteTimeUtc, "latest output file")
    )) {
        $freshnessResult = Test-FreshTimestamp -TimestampUtc ([DateTime]$freshnessCheck[0]) -NowUtc $nowUtc -Label ([string]$freshnessCheck[1])
        if ($null -ne $freshnessResult) {
            return $freshnessResult
        }
    }

    $processCommandResult = Test-WatcherProcessArguments `
        -ProcessId $watchPid `
        -ProcessExecutablePath $processExecutablePath `
        -WatcherPath $expectedWatcherPath `
        -DefinitionPath $definitionPath `
        -Root $Root
    if ($processCommandResult.ExitCode -ne 0) {
        return $processCommandResult
    }

    return New-AgentWatchResult -ExitCode 0 -Code "OK" -Message (
        "継続監視は正常です: PID={0} interval=10分 agents={1} active={2} stalled={3} unmonitorable={4} scanAge={5:N1}分 output={6} liveMismatch={7}" -f
            $watchPid, $agentCount, $activeCount, $stalledCount, $unmonitorableCount,
            (($nowUtc - $scanCompletedUtc).TotalMinutes), $expectedOutputPath, $agentStateResult.RuntimeMismatch
    ) `
        -AgentStates $agentStateResult.AgentStates `
        -RuntimeMismatch $agentStateResult.RuntimeMismatch `
        -RuntimeMismatchMessage $agentStateResult.RuntimeMismatchMessage
}

function Invoke-AgentWatchCheck {
    param([Parameter(Mandatory = $true)][string]$Root)

    $retryCodes = @("STATE_READ_ERROR", "OUTPUT_METADATA_MISMATCH", "OUTPUT_HASH_MISMATCH", "HASH_CHECK_ERROR")
    $result = $null
    for ($attempt = 1; $attempt -le $script:RetryCount; $attempt++) {
        $result = Test-AgentWatchSnapshot -Root $Root
        if ($result.ExitCode -eq 0 -or $retryCodes -notcontains $result.Code -or $attempt -eq $script:RetryCount) {
            return $result
        }
        Start-Sleep -Milliseconds $script:RetryMilliseconds
    }
    return $result
}

function Write-HookDeny {
    param(
        [Parameter(Mandatory = $true)]$Result,
        [Parameter(Mandatory = $true)][string]$Root
    )

    $kind = if ($Result.ExitCode -eq 1) { "AGENT_WATCH_POLICY_NG" } else { "AGENT_WATCH_CHECK_ERROR" }
    $guidance = if ([string]$Result.Code -like "STALL_*") {
        "latest outputの現在incidentを確認し、委譲text先頭へ承認済みAGENT_WATCH_RESPONSEを各incident 1件ずつ書いてください。action=continueではnext=progress-when:<観測条件>が必要です。"
    }
    else {
        "担当へ委譲する前に scripts/watch-agents.ps1 を -Once なし・10分間隔で継続稼働させてください。"
    }
    $reason = @(
        ("{0} [{1}]: {2}" -f $kind, $Result.Code, $Result.Message),
        $guidance,
        ("確認先: {0}" -f (Join-Path $Root "scratchpad\watch-agents.latest.log"))
    ) -join " "
    $payload = [ordered]@{
        hookSpecificOutput = [ordered]@{
            hookEventName = "PreToolUse"
            permissionDecision = "deny"
            permissionDecisionReason = $reason
        }
    }
    $payload | ConvertTo-Json -Depth 5 -Compress | Write-Output
}

if ($Action -eq "Hook") {
    try {
        $raw = Read-Utf8StandardInput
        if ([string]::IsNullOrWhiteSpace($raw)) {
            $missingPayload = New-CheckErrorResult -Code "HOOK_PAYLOAD_MISSING" -Message "PreToolUse payloadが空です。"
            $fallbackRoot = if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) { [string]$env:CLAUDE_PROJECT_DIR } else { $RepositoryRoot }
            if ([string]::IsNullOrWhiteSpace($fallbackRoot)) { $fallbackRoot = "<不明>" }
            Write-HookDeny -Result $missingPayload -Root $fallbackRoot
            exit 0
        }
        $hookPayload = $raw | ConvertFrom-Json
        $toolName = [string]$hookPayload.tool_name
        $gatedTools = @("mcp__codex__codex", "mcp__codex__codex-reply", "Agent", "SendMessage")
        if ($gatedTools -notcontains $toolName) {
            exit 0
        }
        # Claude Codeのidentity契約では、agent_idが非空の文字列の場合だけsubagentである。
        # 欠落、空文字列、null、数値、配列、objectはmainとしてfail-closed検査へ進める。
        $agentIdProperty = $hookPayload.PSObject.Properties["agent_id"]
        if ($null -ne $agentIdProperty -and
            $agentIdProperty.Value -is [string] -and
            ([string]$agentIdProperty.Value).Length -gt 0) {
            exit 0
        }
        $resolvedRoot = Resolve-RepositoryRoot -SuppliedRoot $RepositoryRoot
        $hookResult = Invoke-AgentWatchCheck -Root $resolvedRoot
        if ($hookResult.ExitCode -ne 0) {
            Write-HookDeny -Result $hookResult -Root $resolvedRoot
            exit 0
        }
        $delegationText = Get-DelegationText -HookPayload $hookPayload -ToolName $toolName
        $incidentIds = @(
            $hookResult.AgentStates |
                Where-Object { [string]$_.status -eq "stalled" -or [string]$_.status -eq "unmonitorable" } |
                ForEach-Object { [string]$_.incidentId }
        )
        $responseResult = Test-AgentWatchResponses -Text $delegationText -IncidentIds $incidentIds
        if ($responseResult.ExitCode -ne 0) {
            $responseResult.Message = "{0} current={1}" -f $responseResult.Message, (@($incidentIds | Sort-Object) -join ",")
            Write-HookDeny -Result $responseResult -Root $resolvedRoot
        }
        exit 0
    }
    catch {
        $hookError = New-CheckErrorResult -Code "HOOK_CHECK_ERROR" -Message $_.Exception.Message
        $fallbackRoot = if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) { [string]$env:CLAUDE_PROJECT_DIR } else { $RepositoryRoot }
        if ([string]::IsNullOrWhiteSpace($fallbackRoot)) { $fallbackRoot = "<不明>" }
        Write-HookDeny -Result $hookError -Root $fallbackRoot
        exit 0
    }
}

try {
    $resolvedRoot = Resolve-RepositoryRoot -SuppliedRoot $RepositoryRoot
    $checkResult = Invoke-AgentWatchCheck -Root $resolvedRoot
    if ($checkResult.ExitCode -eq 0) {
        $attentionStates = @(
            $checkResult.AgentStates |
                Where-Object { [string]$_.status -eq "stalled" -or [string]$_.status -eq "unmonitorable" }
        )
        if ($attentionStates.Count -gt 0) {
            $checkResult = New-PolicyResult -Code "STALL_RESPONSE_REQUIRED" -Message (
                "監視は正常ですが対応宣言が必要な担当があります: count={0} incidents={1}" -f
                    $attentionStates.Count, (@($attentionStates | ForEach-Object { [string]$_.incidentId }) -join ",")
            )
        }
        elseif ($checkResult.RuntimeMismatch) {
            $checkResult = New-PolicyResult -Code "WATCH_STATE_LAG" -Message (
                "runtime/outputとlive再走査が一致しません。liveはactiveなので委譲hookは締め出しませんが、次scanでstateが追随する必要があります: {0}" -f
                    $checkResult.RuntimeMismatchMessage
            )
        }
    }
}
catch {
    $checkResult = New-CheckErrorResult -Code "CHECK_ERROR" -Message $_.Exception.Message
}
Write-Output ("[{0}] {1}" -f $checkResult.Code, $checkResult.Message)
exit $checkResult.ExitCode
