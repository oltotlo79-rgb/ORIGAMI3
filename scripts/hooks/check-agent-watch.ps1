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
$script:AgentReplyQuietMinutes = 15.0
$script:AgentWatchScopeSchemaVersion = 1
$script:AgentSendLedgerSchemaVersion = 1
$script:AgentSendLedgerFileName = "watch-agents.sends.json"
$script:AgentSendLedgerLockTimeoutMilliseconds = 500

function New-AgentWatchResult {
    param(
        [Parameter(Mandatory = $true)][int]$ExitCode,
        [Parameter(Mandatory = $true)][string]$Code,
        [Parameter(Mandatory = $true)][string]$Message,
        [object[]]$AgentStates = @(),
        [bool]$RuntimeMismatch = $false,
        [string]$RuntimeMismatchMessage = "",
        [string[]]$Warnings = @()
    )

    return [pscustomobject]@{
        ExitCode = $ExitCode
        Code = $Code
        Message = $Message
        AgentStates = @($AgentStates)
        RuntimeMismatch = $RuntimeMismatch
        RuntimeMismatchMessage = $RuntimeMismatchMessage
        Warnings = @($Warnings)
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

function Split-DelegationRunClauses {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    $clauses = New-Object System.Collections.Generic.List[string]
    $builder = New-Object Text.StringBuilder
    $depth = 0
    for ($index = 0; $index -lt $Text.Length; $index++) {
        $character = $Text[$index]
        if ($character -eq "`r") {
            continue
        }
        # 改行は括弧の閉じ忘れが後続command全体を飲み込まないよう、常に境界にする。
        if ($character -eq "`n") {
            if ($builder.Length -gt 0) {
                $clauses.Add($builder.ToString())
                [void]$builder.Clear()
            }
            $depth = 0
            continue
        }
        if ($character -eq '(' -or $character -eq '[' -or $character -eq '{') {
            $depth += 1
            [void]$builder.Append($character)
            continue
        }
        if ($character -eq ')' -or $character -eq ']' -or $character -eq '}') {
            if ($depth -gt 0) {
                $depth -= 1
            }
            [void]$builder.Append($character)
            continue
        }
        $isShellBoundary = $false
        if ($depth -eq 0 -and ($character -eq '&' -or $character -eq '|')) {
            $isShellBoundary = $true
            if ($index + 1 -lt $Text.Length -and $Text[$index + 1] -eq $character) {
                $index += 1
            }
        }
        # 読点・commaは `cargo testを、走らせて` のcommandと命令形を結ぶため
        # 境界にしない。shellの複数commandは&&/||/|/semicolonで分ける。
        $isSentenceBoundary = $depth -eq 0 -and
            ('。！？!?；;'.IndexOf($character) -ge 0)
        if ($isShellBoundary -or $isSentenceBoundary) {
            if ($builder.Length -gt 0) {
                $clauses.Add($builder.ToString())
                [void]$builder.Clear()
            }
            continue
        }
        [void]$builder.Append($character)
    }
    if ($builder.Length -gt 0) {
        $clauses.Add($builder.ToString())
    }
    return @($clauses)
}

function Test-BareDelegationRunCommand {
    param(
        [Parameter(Mandatory = $true)][string]$Clause,
        [Parameter(Mandatory = $true)]$CommandMatch,
        [Parameter(Mandatory = $true)][int]$ContextEnd
    )

    $prefix = $Clause.Substring(0, $CommandMatch.Index)
    if (-not [regex]::IsMatch(
            $prefix,
            '^\s*(?:>\s*)*(?:(?:[-*+]|\d+[.)])\s*)?(?:\[[ xX]\]\s*)?[*_~`]*\s*$',
            [Text.RegularExpressions.RegexOptions]::CultureInvariant
        )) {
        return $false
    }

    # bare commandの後ろにある括弧注記は除き、それ以外に非ASCIIの文字があれば
    # 状態説明・調査文として扱う。固定の日本語record語彙を増やし続けないための境界。
    $tail = $Clause.Substring(
        $CommandMatch.Index + $CommandMatch.Length,
        $ContextEnd - ($CommandMatch.Index + $CommandMatch.Length)
    )
    $outsideParentheses = New-Object Text.StringBuilder
    $depth = 0
    foreach ($character in $tail.ToCharArray()) {
        if ($character -eq '(' -or $character -eq '[' -or $character -eq '{') {
            $depth += 1
            continue
        }
        if ($character -eq ')' -or $character -eq ']' -or $character -eq '}') {
            if ($depth -gt 0) {
                $depth -= 1
                continue
            }
        }
        if ($depth -eq 0) {
            [void]$outsideParentheses.Append($character)
        }
    }
    $nonAscii = [regex]::Replace($outsideParentheses.ToString(), '[\x00-\x7f]', '')
    return -not [regex]::IsMatch(
        $nonAscii,
        '[\p{L}\p{N}]',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
}

function Test-RunDirectiveTargetsCommand {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$AfterCommand,
        [Parameter(Mandatory = $true)][string]$DirectivePattern
    )

    # 同じ節の後半に別作業の命令があっても、直前のcargo/npmへ係る命令とは限らない。
    # commandと動詞の間をshell引数・構成注記・助詞だけに限定し、
    # 「cargo testの結果を見て、報告書の更新を行う」を実行指示にしない。
    $directiveMatches = @([regex]::Matches(
        $AfterCommand,
        $DirectivePattern,
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ))
    foreach ($directiveMatch in $directiveMatches) {
        $prefix = $AfterCommand.Substring(0, $directiveMatch.Index)
        $resumptionMatches = @([regex]::Matches(
            $prefix,
            '(?ix)(?:今回は|今回だけは|ただし|しかし|それでも|例外として)\s*[、,]?',
            [Text.RegularExpressions.RegexOptions]::CultureInvariant
        ))
        if ($resumptionMatches.Count -gt 0) {
            $lastResumption = $resumptionMatches[$resumptionMatches.Count - 1]
            $prefix = $prefix.Substring($lastResumption.Index + $lastResumption.Length)
        }
        $prefix = [regex]::Replace(
            $prefix,
            '(?s)[\(（\[［\{][^\)）\]］\}]*[\)）\]］\}]',
            ''
        )
        $prefix = [regex]::Replace(
            $prefix,
            '(?ix)--release\s*(?:を)?(?:付け|指定|使わ|用い)(?:ない(?:で)?|ず|ません)',
            ''
        )
        $prefix = [regex]::Replace(
            $prefix,
            '(?ix)(?:debug|デバッグ)\s*構成\s*(?:ではない|ではなく|でなく|以外|を使わない|にしない)',
            ''
        )
        $prefix = [regex]::Replace(
            $prefix,
            '(?ix)(?:debug|デバッグ)\s*構成\s*(?:で|として)?',
            ''
        )
        $prefix = [regex]::Replace($prefix, '(?:として|を|は|で)', '')
        $nonAscii = [regex]::Replace($prefix, '[\x00-\x7f]', '')
        if (-not [regex]::IsMatch(
                $nonAscii,
                '[\p{L}\p{N}]',
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            )) {
            return $true
        }
    }
    return $false
}

function Test-DelegationRunConfiguration {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    # 構成を問うのは、build profileで実行時間や合否が変わる実行指示だけに限る。
    # 単なるcommand欄、過去の結果、実行中という説明、禁止事項は対象にしない。
    $commandPattern = @'
(?ix)
(?<![A-Z0-9_.-])
(?:
  cargo(?:\.exe)?(?:\s+\+[A-Z0-9_.-]+)?\s+(?:test|build)
  (?=$|[\s`'"）)\]\],;。、！？!?をはがのでと]|してください)
 |
  npm(?:\.cmd|\.exe)?
  (?:
    \s+--(?:prefix|workspace)(?:=\S+|\s+\S+)
   |\s+--[A-Z0-9][A-Z0-9-]*(?:=\S+)?
  )*
  \s+test(?=$|[\s`'"）)\]\],;。、！？!?をはがのでと]|してください)
)
'@
    $negativeDirectivePattern = @'
(?ix)
(?:
  走らせ(?:ない(?:で(?:ください)?|こと)?|ず|てはいけない|てはならない|るな)
 |(?:実行|再実行|起動|検査|実測|テスト|ビルド)し
   (?:ない(?:で(?:ください)?|こと)?|なくて(?:よい|構いません)|てはいけない|てはならない)
 |回さ(?:ない(?:で(?:ください)?|こと)?|ず)
 |(?:実行|起動|検査|テスト|ビルド)(?:は|を)?(?:禁止|不可|不要)
 |(?:do\s+not|don't|must\s+not|never)\s+(?:run|execute|build|test)
)
'@
    $affirmativeDirectivePattern = @'
(?ix)
(?:
  走らせ(?:て(?:ください|構いません)?|ること)
 |(?:実行|再実行|起動|ビルド|実施)し
   (?:て(?:ください|構いません)?|なさい)
 |(?:実行|再実行|起動|ビルド|実施)すること
 |回して(?:ください)?
 |行って(?:ください)?
 |(?:please\s+)?(?:run|execute)\b
)
'@
    $directPleasePattern = @'
(?ix)
(?:
  ^\s*(?:して)?ください(?=\s*(?:\(|$))
 |^(?:\s+(?:--?[A-Z0-9_.-]+|[A-Z0-9_./:\\-]+))*\s*をお願いします\s*$
)
'@
    $recordPattern = @'
(?ix)
(?:
  対象検査\s*command
 |baseline
 |終了(?:コード)?
 |passed|failed|skipped
 |完走|実行中|検査中|走り続け
 |既存結果|結果を(?:報告|確認)|終了コードを(?:報告|確認)
 |(?:でした|だった|済み|完了しました)
)
'@
    $releasePattern = '(?i)(?<![A-Z0-9_-])--release(?=$|[\s`''"）\]\),;])'
    $releaseNegatedPattern = '(?ix)--release.{0,32}(?:付け|指定|使わ|用い)(?:ない|ず|ません)|without\s+--release'
    $debugPattern = @'
(?ix)
(?<![A-Z0-9_-])(?:debug|デバッグ)\s*構成\s*
(?:
  (?=[。.!！）)\]\],、]|$)
 |で(?=\s*(?:走らせ|実行|再実行|起動|回し|ビルド|実施|行っ))
 |として(?=\s*(?:走らせ|実行|再実行|起動|回し|ビルド|実施|行っ))
 |です(?=[。.!！）)\]\],、]|$)
 |である(?:こと(?:を明記)?)?(?=[。.!！）)\]\],、]|$)
)
'@
    $debugNegatedPattern = '(?ix)(?:debug|デバッグ)\s*構成\s*(?:ではない|ではなく|でなく|以外|を使わない|にしない)'
    $debugBeforeCommandPattern = '(?ix)(?:debug|デバッグ)\s*構成\s*で\s*[、,]?\s*$'

    $normalized = $Text.Normalize([Text.NormalizationForm]::FormKC)
    $clauses = @(Split-DelegationRunClauses -Text $normalized)
    $missing = New-Object System.Collections.Generic.List[string]
    foreach ($clauseValue in $clauses) {
        $clause = [string]$clauseValue
        if ([string]::IsNullOrWhiteSpace($clause)) {
            continue
        }
        $commandMatches = @([regex]::Matches(
            $clause,
            $commandPattern,
            [Text.RegularExpressions.RegexOptions]::CultureInvariant
        ))
        if ($commandMatches.Count -eq 0) {
            continue
        }
        $clauseHasAffirmativeDirective = [regex]::IsMatch(
            $clause,
            $affirmativeDirectivePattern,
            [Text.RegularExpressions.RegexOptions]::CultureInvariant
        )
        for ($index = 0; $index -lt $commandMatches.Count; $index++) {
            $commandMatch = $commandMatches[$index]
            $contextEnd = if ($index + 1 -lt $commandMatches.Count) {
                $commandMatches[$index + 1].Index
            }
            else {
                $clause.Length
            }
            $directiveContext = $clause.Substring(
                $commandMatch.Index,
                $contextEnd - $commandMatch.Index
            )
            $configurationContext = $clause.Substring(
                $commandMatch.Index,
                $contextEnd - $commandMatch.Index
            )
            $afterCommand = $configurationContext.Substring($commandMatch.Length)
            $hasAffirmativeDirective = Test-RunDirectiveTargetsCommand `
                -AfterCommand $afterCommand `
                -DirectivePattern $affirmativeDirectivePattern
            $hasNegativeDirective = Test-RunDirectiveTargetsCommand `
                -AfterCommand $afterCommand `
                -DirectivePattern $negativeDirectivePattern
            $hasDirectPlease = [regex]::IsMatch(
                $afterCommand,
                $directPleasePattern,
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            )
            if ($hasNegativeDirective -and
                -not $hasAffirmativeDirective -and
                -not $hasDirectPlease) {
                continue
            }
            $isBareCommand = Test-BareDelegationRunCommand `
                -Clause $clause `
                -CommandMatch $commandMatch `
                -ContextEnd $contextEnd
            $isRecord = [regex]::IsMatch(
                $directiveContext,
                $recordPattern,
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            )
            $isEnumeratedWithNext = $false
            if ($index + 1 -lt $commandMatches.Count) {
                $betweenCommands = $clause.Substring(
                    $commandMatch.Index + $commandMatch.Length,
                    $commandMatches[$index + 1].Index - ($commandMatch.Index + $commandMatch.Length)
                )
                $isEnumeratedWithNext = [regex]::IsMatch(
                    $betweenCommands,
                    '(?ix)^\s*(?:と|および|及び|ならびに|and)\s*$',
                    [Text.RegularExpressions.RegexOptions]::CultureInvariant
                )
            }
            if (-not $hasAffirmativeDirective -and
                -not $hasDirectPlease -and
                -not ($isEnumeratedWithNext -and $clauseHasAffirmativeDirective -and -not $isRecord) -and
                -not ($isBareCommand -and -not $isRecord)) {
                continue
            }

            # 1つの構成語を同じ節の複数commandへ流用できないよう、現在commandから
            # 次のcommand直前までだけを構成の帰属範囲にする。
            $releaseIsExplicit = [regex]::IsMatch(
                $configurationContext,
                $releasePattern,
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            ) -and -not [regex]::IsMatch(
                $configurationContext,
                $releaseNegatedPattern,
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            )
            $debugIsExplicit = [regex]::IsMatch(
                $configurationContext,
                $debugPattern,
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            ) -and -not [regex]::IsMatch(
                $configurationContext,
                $debugNegatedPattern,
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            )
            $preCommandStart = if ($index -eq 0) { 0 } else {
                $commandMatches[$index - 1].Index + $commandMatches[$index - 1].Length
            }
            $preCommandContext = $clause.Substring(
                $preCommandStart,
                $commandMatch.Index - $preCommandStart
            )
            $debugBeforeCommandIsExplicit = [regex]::IsMatch(
                $preCommandContext,
                $debugBeforeCommandPattern,
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            ) -and -not [regex]::IsMatch(
                $preCommandContext,
                $debugNegatedPattern,
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            )
            if (-not $releaseIsExplicit -and
                -not $debugIsExplicit -and
                -not $debugBeforeCommandIsExplicit) {
                $missing.Add(([string]$commandMatch.Value).Trim())
            }
        }
    }

    if ($missing.Count -gt 0) {
        $commands = @($missing | Select-Object -Unique | Select-Object -First 3) -join ', '
        return New-PolicyResult `
            -Code "AGENT_RUN_CONFIGURATION_REQUIRED" `
            -Message "実行指示の構成が不明です: $commands。--release を付けるか、同じ命令文でdebug構成であることを明記してください。"
    }
    return New-AgentWatchResult `
        -ExitCode 0 `
        -Code "AGENT_RUN_CONFIGURATION_OK" `
        -Message "実行指示のrelease/debug構成は明示されています。"
}

function Get-DelegationTargetThreadId {
    param(
        [Parameter(Mandatory = $true)]$HookPayload,
        [Parameter(Mandatory = $true)][string]$ToolName
    )

    $toolInput = $HookPayload.tool_input
    $fieldName = switch ($ToolName) {
        "Agent" { "resume" }
        "SendMessage" { "recipient" }
        "mcp__codex__codex" { "threadId" }
        "mcp__codex__codex-reply" { "threadId" }
        default { throw "送信先を検査する委譲toolではありません: $ToolName" }
    }
    $property = $toolInput.PSObject.Properties[$fieldName]
    if ($null -eq $property) {
        if ($ToolName -eq "SendMessage" -or $ToolName -eq "mcp__codex__codex-reply") {
            throw "PreToolUse payloadに送信先fieldがありません: tool=$ToolName field=tool_input.$fieldName"
        }
        return ""
    }
    if (-not ($property.Value -is [string]) -or [string]::IsNullOrWhiteSpace([string]$property.Value)) {
        throw "PreToolUse payloadの送信先fieldが空または文字列ではありません: tool=$ToolName field=tool_input.$fieldName"
    }
    $threadId = ([string]$property.Value).Trim()
    if (-not [regex]::IsMatch($threadId, '^[A-Za-z0-9][A-Za-z0-9._:-]{5,127}$', [Text.RegularExpressions.RegexOptions]::CultureInvariant)) {
        throw "PreToolUse payloadの送信先IDが承認済み形式ではありません: tool=$ToolName field=tool_input.$fieldName"
    }
    return $threadId
}

function Get-AgentThreadIdFromName {
    param([Parameter(Mandatory = $true)][string]$Name)

    $match = [regex]::Match(
        $Name,
        '\((?<threadId>[A-Za-z0-9][A-Za-z0-9._:-]{5,127}),\s*[^()]+\)\s*$',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if (-not $match.Success) {
        return ""
    }
    return [string]$match.Groups["threadId"].Value
}

function Get-WatchPathComparisonKey {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$BasePath
    )

    $canonical = ConvertTo-CanonicalWatchPath -Path $Path -BasePath $BasePath
    $root = [IO.Path]::GetPathRoot($canonical)
    if ($canonical.Length -gt $root.Length) {
        $canonical = $canonical.TrimEnd([char[]]"\/")
    }
    return $canonical.ToLowerInvariant()
}

function Test-WatchPathOverlap {
    param(
        [Parameter(Mandatory = $true)][string]$FirstPath,
        [Parameter(Mandatory = $true)][string]$SecondPath
    )

    if ([string]::Equals($FirstPath, $SecondPath, [StringComparison]::OrdinalIgnoreCase)) {
        return $true
    }
    $separator = [IO.Path]::DirectorySeparatorChar
    $firstPrefix = $FirstPath.TrimEnd([char[]]"\/") + $separator
    $secondPrefix = $SecondPath.TrimEnd([char[]]"\/") + $separator
    return $SecondPath.StartsWith($firstPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        $FirstPath.StartsWith($secondPrefix, [StringComparison]::OrdinalIgnoreCase)
}

function Get-StrongPublicTypeMutationSourceHints {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    # Cargo graphを読む前に、同じ文の中へsourceと強い公開型変更命令がそろっているかを
    # 判定する。完了報告・調査・禁止を型変更の許可へ読み替えない。
    $typeTargetPattern = @'
(?ix)
(?:
  (?:公開\s*(?:型|API)|pub\s+(?:struct|enum|trait|type))
 |
  `?(?:(?:[a-z][a-z0-9_-]*::)?[A-Z][A-Za-z0-9_]*)`?
  [^。！？!?；;\r\n]{0,80}
  (?:field|フィールド|項目|variant|列挙子|型引数|引数|戻り値(?:型)?)
)
'@
    $mutationDirectivePattern = @'
(?ix)
(?:
  (?:追加|変更|削除|除去|改名|置換|拡張)\s*
  (?:
    して(?:ください|よい|構いません)
   |すること
   |を許可(?:してください|します|すること)?
   |させ(?:てください|る)
  )
 |足\s*(?:してください|してよい|すこと|させてください|させる)
 |変え\s*(?:てください|ること)
)
'@
    $negativeMutationPattern = @'
(?ix)
(?:追加|変更|削除|除去|改名|置換|拡張)
\s*(?:
  し(?:ない|ません|てはいけない)
 |するな
 |を\s*許可\s*しない
 |(?:は|を)\s*(?:禁止|不可)
)
'@
    $pastMutationDirectivePattern = @'
(?ix)
(?:追加|変更|削除|除去|改名|置換|拡張)
\s*(?:してください|すること)
[^。！？!?；;\r\n]{0,32}
(?:と\s*)?(?:以前|過去|既に|すでに)?\s*(?:指示|依頼)(?:済み|しました|した|だった)
'@
    $memberPathPattern = '(?i)(?<member>crates[\\/][A-Za-z0-9_-]+|apps[\\/]desktop[\\/]src-tauri)(?=$|[\\/\s`''"：:])'
    $qualifiedTypePattern = '(?<crate>[a-z][a-z0-9_-]*)::[A-Z][A-Za-z0-9_]*'
    $typeSymbolPattern = '(?<![A-Za-z0-9_])(?<symbol>[A-Z][A-Za-z0-9_]*)(?![A-Za-z0-9_])'

    $hintsByKey = New-Object 'Collections.Generic.Dictionary[string,object]' ([StringComparer]::OrdinalIgnoreCase)
    $normalized = $Text.Normalize([Text.NormalizationForm]::FormKC)
    foreach ($clauseValue in @(Split-DelegationRunClauses -Text $normalized)) {
        $clause = [string]$clauseValue
        $mutationMatches = @([regex]::Matches(
                $clause,
                $mutationDirectivePattern,
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            ))
        if (-not [regex]::IsMatch(
                $clause,
                $typeTargetPattern,
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            ) -or $mutationMatches.Count -eq 0) {
            continue
        }

        # 否定・過去の表現が同じ文に1つあっても、別spanの現在の肯定命令を
        # 捨てない。各mutation matchと除外spanが重なるものだけを落とす。
        $excludedMutationSpans = @([regex]::Matches(
                $clause,
                $negativeMutationPattern,
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            )) + @([regex]::Matches(
                $clause,
                $pastMutationDirectivePattern,
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            ))
        $currentPositiveMutations = New-Object 'Collections.Generic.List[object]'
        foreach ($mutationMatch in $mutationMatches) {
            $mutationStart = [int]$mutationMatch.Index
            $mutationEnd = $mutationStart + [int]$mutationMatch.Length
            $isExcluded = $false
            foreach ($excludedSpan in $excludedMutationSpans) {
                $excludedStart = [int]$excludedSpan.Index
                $excludedEnd = $excludedStart + [int]$excludedSpan.Length
                if ($mutationStart -lt $excludedEnd -and $excludedStart -lt $mutationEnd) {
                    $isExcluded = $true
                    break
                }
            }
            if (-not $isExcluded) {
                $currentPositiveMutations.Add($mutationMatch)
            }
        }
        if ($currentPositiveMutations.Count -eq 0) {
            continue
        }

        $collectSegmentHints = {
            param(
                [Parameter(Mandatory = $true)][string]$SegmentText,
                [Parameter(Mandatory = $true)][int]$SegmentOffset
            )
            $segmentHints = New-Object 'Collections.Generic.Dictionary[string,object]' ([StringComparer]::OrdinalIgnoreCase)
            foreach ($hintSpec in @(
                    [pscustomobject]@{ Kind = "path"; Pattern = $memberPathPattern; Group = "member" },
                    [pscustomobject]@{ Kind = "package"; Pattern = $qualifiedTypePattern; Group = "crate" },
                    [pscustomobject]@{ Kind = "symbol"; Pattern = $typeSymbolPattern; Group = "symbol" }
                )) {
                foreach ($hintMatch in @([regex]::Matches(
                            $SegmentText,
                            [string]$hintSpec.Pattern,
                            [Text.RegularExpressions.RegexOptions]::CultureInvariant
                        ))) {
                    $absoluteStart = $SegmentOffset + [int]$hintMatch.Index
                    $absoluteEnd = $absoluteStart + [int]$hintMatch.Length
                    $hintIsExcluded = $false
                    foreach ($excludedSpan in $excludedMutationSpans) {
                        $excludedStart = [int]$excludedSpan.Index
                        $excludedEnd = $excludedStart + [int]$excludedSpan.Length
                        if ($absoluteStart -lt $excludedEnd -and $excludedStart -lt $absoluteEnd) {
                            $hintIsExcluded = $true
                            break
                        }
                    }
                    if ($hintIsExcluded) {
                        continue
                    }
                    $value = [string]$hintMatch.Groups[[string]$hintSpec.Group].Value
                    if ([string]$hintSpec.Kind -eq "package") {
                        $value = $value.Replace("_", "-")
                    }
                    $key = "$([string]$hintSpec.Kind)|$value"
                    if (-not $segmentHints.ContainsKey($key)) {
                        $segmentHints.Add($key, [pscustomobject]@{
                                Kind = [string]$hintSpec.Kind
                                Value = $value
                            })
                    }
                }
            }
            return @($segmentHints.Values)
        }

        foreach ($positiveMutation in $currentPositiveMutations) {
            $mutationStart = [int]$positiveMutation.Index
            $mutationEnd = $mutationStart + [int]$positiveMutation.Length
            $segmentStart = 0
            if ($mutationStart -gt 0) {
                $prefix = $clause.Substring(0, $mutationStart)
                $segmentBoundaries = @([regex]::Matches(
                        $prefix,
                        '(?:、|,|(?:ました|ます|です|だ|しない|ません)が|けれど(?:も)?|一方で?)',
                        [Text.RegularExpressions.RegexOptions]::CultureInvariant
                    ))
                if ($segmentBoundaries.Count -gt 0) {
                    $lastBoundary = $segmentBoundaries[$segmentBoundaries.Count - 1]
                    $segmentStart = [int]$lastBoundary.Index + [int]$lastBoundary.Length
                }
            }
            $segmentText = $clause.Substring($segmentStart, $mutationEnd - $segmentStart)
            $selectedHints = @(& $collectSegmentHints -SegmentText $segmentText -SegmentOffset $segmentStart)
            if ($selectedHints.Count -eq 0) {
                $clauseHints = @(& $collectSegmentHints -SegmentText $clause -SegmentOffset 0)
                $uniqueSymbols = @(
                    $clauseHints |
                        Where-Object { [string]$_.Kind -eq "symbol" } |
                        ForEach-Object { [string]$_.Value } |
                        Sort-Object -Unique
                )
                if ($uniqueSymbols.Count -eq 1) {
                    $selectedHints = $clauseHints
                }
                elseif ($uniqueSymbols.Count -eq 0) {
                    $uniqueExplicitHints = @(
                        $clauseHints |
                            Where-Object { [string]$_.Kind -in @("path", "package") } |
                            ForEach-Object { "$([string]$_.Kind)|$([string]$_.Value)" } |
                            Sort-Object -Unique
                    )
                    if ($uniqueExplicitHints.Count -eq 1) {
                        $selectedHints = $clauseHints
                    }
                }
            }
            foreach ($selectedHint in $selectedHints) {
                $key = "$([string]$selectedHint.Kind)|$([string]$selectedHint.Value)"
                if (-not $hintsByKey.ContainsKey($key)) {
                    $hintsByKey.Add($key, $selectedHint)
                }
            }
        }
    }
    return @($hintsByKey.Values)
}

function Read-WorkspaceCargoDependencyGraph {
    param([Parameter(Mandatory = $true)][string]$Root)

    $workspaceManifestPath = Join-Path $Root "Cargo.toml"
    if (-not (Test-Path -LiteralPath $workspaceManifestPath -PathType Leaf)) {
        throw "workspace Cargo.tomlがありません: $workspaceManifestPath"
    }
    $workspaceText = $script:Utf8NoBom.GetString((Read-SharedFileBytes -Path $workspaceManifestPath))
    $workspaceSection = [regex]::Match(
        $workspaceText,
        '(?ms)^\[workspace\]\s*(?<body>.*?)(?=^\[|\z)',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if (-not $workspaceSection.Success) {
        throw "Cargo.tomlに[workspace]がありません"
    }
    $membersMatch = [regex]::Match(
        [string]$workspaceSection.Groups["body"].Value,
        '(?ms)^\s*members\s*=\s*\[(?<body>.*?)\]',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if (-not $membersMatch.Success) {
        throw "Cargo.tomlの[workspace].membersを読めません"
    }
    $memberPaths = @(
        [regex]::Matches(
            [string]$membersMatch.Groups["body"].Value,
            '"(?<path>[^"\r\n]+)"',
            [Text.RegularExpressions.RegexOptions]::CultureInvariant
        ) | ForEach-Object { [string]$_.Groups["path"].Value }
    )
    if ($memberPaths.Count -eq 0) {
        throw "Cargo.tomlの[workspace].membersが空です"
    }

    $workspaceAliases = New-Object 'Collections.Generic.Dictionary[string,string]' ([StringComparer]::OrdinalIgnoreCase)
    $workspaceDependencies = [regex]::Match(
        $workspaceText,
        '(?ms)^\[workspace\.dependencies\]\s*(?<body>.*?)(?=^\[|\z)',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if ($workspaceDependencies.Success) {
        foreach ($entry in @([regex]::Matches(
                    [string]$workspaceDependencies.Groups["body"].Value,
                    '(?m)^\s*(?<key>[A-Za-z0-9_-]+)\s*=\s*(?<rhs>.+)$',
                    [Text.RegularExpressions.RegexOptions]::CultureInvariant
                ))) {
            $alias = [string]$entry.Groups["key"].Value
            $packageName = $alias
            $packageMatch = [regex]::Match(
                [string]$entry.Groups["rhs"].Value,
                '\bpackage\s*=\s*"(?<name>[^"\r\n]+)"',
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            )
            if ($packageMatch.Success) {
                $packageName = [string]$packageMatch.Groups["name"].Value
            }
            $workspaceAliases[$alias] = $packageName
        }
    }

    $packagesByName = New-Object 'Collections.Generic.Dictionary[string,object]' ([StringComparer]::OrdinalIgnoreCase)
    foreach ($memberPath in $memberPaths) {
        $memberRoot = ConvertTo-CanonicalWatchPath -Path $memberPath -BasePath $Root
        $manifestPath = Join-Path $memberRoot "Cargo.toml"
        if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
            throw "workspace memberのCargo.tomlがありません: $manifestPath"
        }
        $manifestText = $script:Utf8NoBom.GetString((Read-SharedFileBytes -Path $manifestPath))
        $packageSection = [regex]::Match(
            $manifestText,
            '(?ms)^\[package\]\s*(?<body>.*?)(?=^\[|\z)',
            [Text.RegularExpressions.RegexOptions]::CultureInvariant
        )
        $packageNameMatch = [regex]::Match(
            [string]$packageSection.Groups["body"].Value,
            '(?m)^\s*name\s*=\s*"(?<name>[^"\r\n]+)"',
            [Text.RegularExpressions.RegexOptions]::CultureInvariant
        )
        if (-not $packageSection.Success -or -not $packageNameMatch.Success) {
            throw "workspace memberの[package].nameを読めません: $manifestPath"
        }
        $packageName = [string]$packageNameMatch.Groups["name"].Value
        if ($packagesByName.ContainsKey($packageName)) {
            throw "workspace package nameが重複しています: $packageName"
        }
        $packagesByName.Add($packageName, [pscustomobject]@{
                Name = $packageName
                MemberRoot = $memberRoot
                ManifestPath = $manifestPath
                ManifestText = $manifestText
                DirectDependencies = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
            })
    }

    foreach ($package in @($packagesByName.Values)) {
        $dependencySections = @([regex]::Matches(
                [string]$package.ManifestText,
                '(?ms)^\[(?:dependencies|target\.[^\]\r\n]+\.dependencies)\]\s*(?<body>.*?)(?=^\[|\z)',
                [Text.RegularExpressions.RegexOptions]::CultureInvariant
            ))
        foreach ($dependencySection in $dependencySections) {
            foreach ($entry in @([regex]::Matches(
                        [string]$dependencySection.Groups["body"].Value,
                        '(?m)^\s*(?<key>[A-Za-z0-9_-]+)(?<workspace>\.workspace)?\s*=\s*(?<rhs>.+)$',
                        [Text.RegularExpressions.RegexOptions]::CultureInvariant
                    ))) {
                $dependencyName = [string]$entry.Groups["key"].Value
                $rhs = [string]$entry.Groups["rhs"].Value
                $packageMatch = [regex]::Match(
                    $rhs,
                    '\bpackage\s*=\s*"(?<name>[^"\r\n]+)"',
                    [Text.RegularExpressions.RegexOptions]::CultureInvariant
                )
                if ($packageMatch.Success) {
                    $dependencyName = [string]$packageMatch.Groups["name"].Value
                }
                elseif ($entry.Groups["workspace"].Success -and $workspaceAliases.ContainsKey($dependencyName)) {
                    $dependencyName = [string]$workspaceAliases[$dependencyName]
                }
                if ($packagesByName.ContainsKey($dependencyName)) {
                    [void]$package.DirectDependencies.Add($dependencyName)
                }
            }
        }
    }
    return [pscustomobject]@{
        WorkspaceRoot = $Root
        WorkspaceManifestPath = $workspaceManifestPath
        PackagesByName = $packagesByName
    }
}

function Find-CargoWorkspaceRootForScopePath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $canonicalPath = [IO.Path]::GetFullPath($Path)
    $cursor = if (Test-Path -LiteralPath $canonicalPath -PathType Leaf) {
        [IO.Path]::GetDirectoryName($canonicalPath)
    }
    elseif (Test-Path -LiteralPath $canonicalPath -PathType Container) {
        $canonicalPath
    }
    elseif ([IO.Path]::HasExtension([IO.Path]::GetFileName($canonicalPath))) {
        [IO.Path]::GetDirectoryName($canonicalPath)
    }
    else {
        $canonicalPath
    }
    while (-not [string]::IsNullOrEmpty($cursor)) {
        $manifestPath = Join-Path $cursor "Cargo.toml"
        if (Test-Path -LiteralPath $manifestPath -PathType Leaf) {
            $manifestText = $script:Utf8NoBom.GetString((Read-SharedFileBytes -Path $manifestPath))
            if ([regex]::IsMatch(
                    $manifestText,
                    '(?m)^\[workspace\]\s*$',
                    [Text.RegularExpressions.RegexOptions]::CultureInvariant
                )) {
                return [IO.Path]::GetFullPath($cursor)
            }
        }
        $parent = [IO.Directory]::GetParent($cursor)
        if ($null -eq $parent -or
            [string]::Equals([string]$parent.FullName, $cursor, [StringComparison]::OrdinalIgnoreCase)) {
            break
        }
        $cursor = [string]$parent.FullName
    }
    return ""
}

function Get-CargoWorkspaceGraphsForScopePaths {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$SourcePaths)

    $rootsByKey = New-Object 'Collections.Generic.Dictionary[string,string]' ([StringComparer]::OrdinalIgnoreCase)
    $discoveryErrors = New-Object 'Collections.Generic.List[string]'
    foreach ($sourcePath in $SourcePaths) {
        try {
            $workspaceRoot = Find-CargoWorkspaceRootForScopePath -Path ([string]$sourcePath)
            if (-not [string]::IsNullOrEmpty($workspaceRoot)) {
                $key = $workspaceRoot.TrimEnd([char[]]"\/")
                $rootsByKey[$key] = $workspaceRoot
            }
        }
        catch {
            $detail = [regex]::Replace([string]$_.Exception.Message, '\s+', ' ').Trim()
            $discoveryErrors.Add("scopePath=$sourcePath detail=$detail")
        }
    }

    $graphs = New-Object 'Collections.Generic.List[object]'
    foreach ($workspaceRoot in @($rootsByKey.Values | Sort-Object)) {
        try {
            $graph = Read-WorkspaceCargoDependencyGraph -Root ([string]$workspaceRoot)
            $belongsToWorkspace = $false
            foreach ($sourcePath in $SourcePaths) {
                foreach ($package in @($graph.PackagesByName.Values)) {
                    if (Test-WatchPathOverlap `
                            -FirstPath ([string]$sourcePath) `
                            -SecondPath ([string]$package.MemberRoot)) {
                        $belongsToWorkspace = $true
                        break
                    }
                }
                if ($belongsToWorkspace) { break }
            }
            if ($belongsToWorkspace) {
                $graphs.Add($graph)
            }
        }
        catch {
            $detail = [regex]::Replace([string]$_.Exception.Message, '\s+', ' ').Trim()
            $discoveryErrors.Add("workspaceRoot=$workspaceRoot detail=$detail")
        }
    }
    return [pscustomobject]@{
        Graphs = @($graphs.ToArray())
        Errors = @($discoveryErrors.ToArray())
    }
}

function Test-ScopeAllowsCargoPackage {
    param(
        [Parameter(Mandatory = $true)]$Package,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$SourcePaths
    )

    foreach ($sourcePath in $SourcePaths) {
        if (Test-WatchPathOverlap -FirstPath ([string]$sourcePath) -SecondPath ([string]$Package.MemberRoot)) {
            return $true
        }
    }
    return $false
}

function Test-WatchScopePathContainsTarget {
    param(
        [Parameter(Mandatory = $true)][string]$ScopePath,
        [Parameter(Mandatory = $true)][string]$TargetPath
    )

    $scopeKey = Get-WatchPathComparisonKey -Path $ScopePath -BasePath ([IO.Path]::GetPathRoot($ScopePath))
    $targetKey = Get-WatchPathComparisonKey -Path $TargetPath -BasePath ([IO.Path]::GetPathRoot($TargetPath))
    if ([string]::Equals($scopeKey, $targetKey, [StringComparison]::OrdinalIgnoreCase)) {
        return $true
    }
    $scopePrefix = $scopeKey.TrimEnd([char[]]"\/") + [IO.Path]::DirectorySeparatorChar
    return $targetKey.StartsWith($scopePrefix, [StringComparison]::OrdinalIgnoreCase)
}

function Test-ScopeCoversCargoDependentPackage {
    param(
        [Parameter(Mandatory = $true)]$Package,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$SourcePaths
    )

    $packageRoot = [string]$Package.MemberRoot
    $packageSourceRoot = Join-Path $packageRoot "src"
    foreach ($sourcePath in $SourcePaths) {
        # 公開型変更への追随を許可済みと見なすのは、crate全体または全srcを
        # scopeが包含する場合だけ。単一の無関係fileを持つだけでは抑止しない。
        if ((Test-WatchScopePathContainsTarget -ScopePath ([string]$sourcePath) -TargetPath $packageRoot) -or
            (Test-WatchScopePathContainsTarget -ScopePath ([string]$sourcePath) -TargetPath $packageSourceRoot)) {
            return $true
        }
    }
    return $false
}

function Test-ScopeCoversWireContractPeer {
    param(
        [Parameter(Mandatory = $true)][string]$PeerName,
        [Parameter(Mandatory = $true)]$Graph,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$SourcePaths
    )

    $peer = $Graph.PackagesByName[$PeerName]
    if (Test-ScopeCoversCargoDependentPackage -Package $peer -SourcePaths $SourcePaths) {
        return $true
    }
    $mirrorPaths = New-Object 'Collections.Generic.List[string]'
    if ($PeerName -eq "desktop") {
        $mirrorPaths.Add((Join-Path ([string]$peer.MemberRoot) "src/commands.rs"))
        $mirrorPaths.Add((Join-Path ([string]$peer.MemberRoot) "src/store.rs"))
    }
    elseif ($PeerName -eq "ori3-app-core") {
        $mirrorPaths.Add((Join-Path ([string]$peer.MemberRoot) "src/lib.rs"))
    }
    if ($mirrorPaths.Count -eq 0) {
        return $false
    }
    foreach ($mirrorPath in $mirrorPaths) {
        $covered = $false
        foreach ($sourcePath in $SourcePaths) {
            if (Test-WatchScopePathContainsTarget -ScopePath ([string]$sourcePath) -TargetPath $mirrorPath) {
                $covered = $true
                break
            }
        }
        if (-not $covered) {
            return $false
        }
    }
    return $true
}

function Test-PackageDeclaresPublicTypeSymbol {
    param(
        [Parameter(Mandatory = $true)]$Package,
        [Parameter(Mandatory = $true)][string]$Symbol,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$SourcePaths
    )

    $packageSourceRoot = Join-Path ([string]$Package.MemberRoot) "src"
    if (-not (Test-Path -LiteralPath $packageSourceRoot)) {
        return $false
    }
    $scanRootsByKey = New-Object 'Collections.Generic.Dictionary[string,string]' ([StringComparer]::OrdinalIgnoreCase)
    $packageSourceKey = Get-WatchPathComparisonKey -Path $packageSourceRoot -BasePath ([string]$Package.MemberRoot)
    foreach ($sourcePath in $SourcePaths) {
        $sourceKey = Get-WatchPathComparisonKey -Path ([string]$sourcePath) -BasePath ([string]$Package.MemberRoot)
        if (-not (Test-WatchPathOverlap -FirstPath $sourceKey -SecondPath $packageSourceKey)) {
            continue
        }
        $scanRoot = if ($sourceKey.StartsWith(
                $packageSourceKey.TrimEnd([char[]]"\/") + [IO.Path]::DirectorySeparatorChar,
                [StringComparison]::OrdinalIgnoreCase
            ) -or [string]::Equals($sourceKey, $packageSourceKey, [StringComparison]::OrdinalIgnoreCase)) {
            [string]$sourcePath
        }
        else {
            $packageSourceRoot
        }
        $scanRootKey = Get-WatchPathComparisonKey -Path $scanRoot -BasePath ([string]$Package.MemberRoot)
        $scanRootsByKey[$scanRootKey] = $scanRoot
    }
    if ($scanRootsByKey.Count -eq 0) {
        return $false
    }

    $escapedSymbol = [regex]::Escape($Symbol)
    $declarationPattern = "(?m)^\s*pub\s+(?:struct|enum|type|trait)\s+$escapedSymbol\b"
    foreach ($scanRoot in @($scanRootsByKey.Values)) {
        $rustFiles = if (Test-Path -LiteralPath $scanRoot -PathType Leaf) {
            if ([string]::Equals([IO.Path]::GetExtension($scanRoot), ".rs", [StringComparison]::OrdinalIgnoreCase)) {
                @(Get-Item -LiteralPath $scanRoot)
            }
            else {
                @()
            }
        }
        elseif (Test-Path -LiteralPath $scanRoot -PathType Container) {
            @(Get-ChildItem -LiteralPath $scanRoot -Recurse -File -Filter "*.rs")
        }
        else {
            @()
        }
        foreach ($rustFile in $rustFiles) {
            $rustText = $script:Utf8NoBom.GetString((Read-SharedFileBytes -Path ([string]$rustFile.FullName)))
            if ([regex]::IsMatch(
                    $rustText,
                    $declarationPattern,
                    [Text.RegularExpressions.RegexOptions]::CultureInvariant
                )) {
                return $true
            }
        }
    }
    return $false
}

function Resolve-PublicTypeMutationSourcePackages {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Hints,
        [Parameter(Mandatory = $true)]$Graph,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$SourcePaths
    )

    $resolved = New-Object 'Collections.Generic.Dictionary[string,object]' ([StringComparer]::OrdinalIgnoreCase)
    $hasExplicitSourceHint = @($Hints | Where-Object { [string]$_.Kind -in @("path", "package") }).Count -gt 0
    foreach ($hint in $Hints) {
        $candidate = $null
        if ([string]$hint.Kind -eq "package") {
            $packageName = ([string]$hint.Value).Replace("_", "-")
            if ($Graph.PackagesByName.ContainsKey($packageName)) {
                $candidate = $Graph.PackagesByName[$packageName]
            }
        }
        elseif ([string]$hint.Kind -eq "path") {
            $hintPath = ConvertTo-CanonicalWatchPath -Path ([string]$hint.Value) -BasePath $Root
            $candidate = @(
                $Graph.PackagesByName.Values |
                    Where-Object {
                        Test-WatchPathOverlap -FirstPath $hintPath -SecondPath ([string]$_.MemberRoot)
                    } |
                    Sort-Object @{ Expression = { ([string]$_.MemberRoot).Length }; Descending = $true }
            ) | Select-Object -First 1
        }
        elseif ([string]$hint.Kind -eq "symbol" -and -not $hasExplicitSourceHint) {
            $symbolOwners = @(
                $Graph.PackagesByName.Values |
                    Where-Object {
                        (Test-ScopeAllowsCargoPackage -Package $_ -SourcePaths $SourcePaths) -and
                        (Test-PackageDeclaresPublicTypeSymbol `
                            -Package $_ `
                            -Symbol ([string]$hint.Value) `
                            -SourcePaths $SourcePaths)
                    }
            )
            if ($symbolOwners.Count -eq 1) {
                $candidate = $symbolOwners[0]
            }
        }
        if ($null -ne $candidate -and
            (Test-ScopeAllowsCargoPackage -Package $candidate -SourcePaths $SourcePaths) -and
            -not $resolved.ContainsKey([string]$candidate.Name)) {
            $resolved.Add([string]$candidate.Name, $candidate)
        }
    }
    return @($resolved.Values)
}

function Get-DesktopAppCoreWireContractPath {
    param(
        [Parameter(Mandatory = $true)]$Graph
    )

    if (-not $Graph.PackagesByName.ContainsKey("desktop") -or
        -not $Graph.PackagesByName.ContainsKey("ori3-app-core")) {
        return ""
    }
    $appCore = $Graph.PackagesByName["ori3-app-core"]
    $buildPath = Join-Path ([string]$appCore.MemberRoot) "build.rs"
    if (-not (Test-Path -LiteralPath $buildPath -PathType Leaf)) {
        return ""
    }
    try {
        $buildText = $script:Utf8NoBom.GetString((Read-SharedFileBytes -Path $buildPath))
    }
    catch {
        return ""
    }
    foreach ($requiredFragment in @(
            "const WIRE_TYPES",
            '../../apps/desktop/src-tauri/src/commands.rs',
            '../../apps/desktop/src-tauri/src/store.rs',
            'item_fingerprint(desktop_source',
            'item_fingerprint(&app_core'
        )) {
        if (-not $buildText.Contains($requiredFragment)) {
            return ""
        }
    }
    return $buildPath
}

function Get-PublicTypeDependencyWarnings {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)]$Scope,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Hints
    )

    if ($Hints.Count -eq 0 -or [bool]$Scope.ReadOnly) {
        return @()
    }
    [string[]]$scopeSources = @($Scope.SourcePaths | ForEach-Object { [string]$_ })
    $warningsByKey = New-Object 'Collections.Generic.Dictionary[string,string]' ([StringComparer]::OrdinalIgnoreCase)
    $workspaceDiscovery = Get-CargoWorkspaceGraphsForScopePaths -SourcePaths $scopeSources
    $errorIndex = 0
    foreach ($discoveryError in @($workspaceDiscovery.Errors)) {
        $warningsByKey["unavailable|$errorIndex"] = (
            "AGENT_PUBLIC_TYPE_DEPENDENCY_CHECK_UNAVAILABLE: 強い公開型変更命令を検出しましたがscope所属Cargo workspaceを読めません。依存側の追随可否を確認してください。detail={0}" -f
                ([string]$discoveryError)
        )
        $errorIndex += 1
    }

    foreach ($graph in @($workspaceDiscovery.Graphs)) {
        try {
            $sourcePackages = @(
                Resolve-PublicTypeMutationSourcePackages `
                    -Hints $Hints `
                    -Graph $graph `
                    -Root ([string]$graph.WorkspaceRoot) `
                    -SourcePaths $scopeSources
            )
        }
        catch {
            $detail = [regex]::Replace([string]$_.Exception.Message, '\s+', ' ').Trim()
            $key = "unavailable|$errorIndex"
            $warningsByKey[$key] = (
                "AGENT_PUBLIC_TYPE_DEPENDENCY_CHECK_UNAVAILABLE: 強い公開型変更命令を検出しましたが公開型ownerを読めません。依存側の追随可否を確認してください。workspace={0} detail={1}" -f
                    ([string]$graph.WorkspaceRoot), $detail
            )
            $errorIndex += 1
            continue
        }
        foreach ($sourcePackage in $sourcePackages) {
            foreach ($dependentPackage in @($graph.PackagesByName.Values | Sort-Object Name)) {
                if (-not $dependentPackage.DirectDependencies.Contains([string]$sourcePackage.Name) -or
                    (Test-ScopeCoversCargoDependentPackage -Package $dependentPackage -SourcePaths $scopeSources)) {
                    continue
                }
                $key = "cargo|$([string]$graph.WorkspaceRoot)|$([string]$sourcePackage.Name)|$([string]$dependentPackage.Name)"
                $warningsByKey[$key] = (
                    "AGENT_PUBLIC_TYPE_DEPENDENCY_WARNING: source={0} dependent={1} relation=cargo-direct manifest={2}。この禁止は、依存している側の追随を妨げる可能性があります。" -f
                        ([string]$sourcePackage.Name), ([string]$dependentPackage.Name), ([string]$dependentPackage.ManifestPath)
                )
            }

            $wireContractPath = Get-DesktopAppCoreWireContractPath -Graph $graph
            if (-not [string]::IsNullOrEmpty($wireContractPath)) {
                $peerName = if ([string]$sourcePackage.Name -eq "desktop") {
                    "ori3-app-core"
                }
                elseif ([string]$sourcePackage.Name -eq "ori3-app-core") {
                    "desktop"
                }
                else {
                    ""
                }
                if (-not [string]::IsNullOrEmpty($peerName) -and
                    -not (Test-ScopeCoversWireContractPeer -PeerName $peerName -Graph $graph -SourcePaths $scopeSources)) {
                    $key = "wire|$([string]$graph.WorkspaceRoot)|$([string]$sourcePackage.Name)|$peerName"
                    $warningsByKey[$key] = (
                        "AGENT_PUBLIC_TYPE_DEPENDENCY_WARNING: source={0} dependent={1} relation=wire-contract-peer contract={2}。この禁止は、依存している側の追随を妨げる可能性があります。" -f
                            ([string]$sourcePackage.Name), $peerName, $wireContractPath
                    )
                }
            }
        }
    }
    return @($warningsByKey.GetEnumerator() | Sort-Object Key | ForEach-Object { [string]$_.Value })
}

function Get-AgentWatchScopeSha256 {
    param(
        [Parameter(Mandatory = $true)][string]$ThreadId,
        [Parameter(Mandatory = $true)][bool]$ReadOnly,
        [Parameter(Mandatory = $true)][string]$ReportPath,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$SourcePaths,
        [Parameter(Mandatory = $true)][string]$Root
    )

    $reportKey = Get-WatchPathComparisonKey -Path $ReportPath -BasePath $Root
    [string[]]$sourceKeys = @(
        $SourcePaths | ForEach-Object { Get-WatchPathComparisonKey -Path $_ -BasePath $Root }
    )
    [Array]::Sort($sourceKeys, [StringComparer]::Ordinal)
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("version=$($script:AgentWatchScopeSchemaVersion)")
    $lines.Add("threadId=$ThreadId")
    $lines.Add("readOnly=$($ReadOnly.ToString().ToLowerInvariant())")
    $lines.Add("reportPath=$(ConvertTo-WatchBase64 -Text $reportKey)")
    foreach ($sourceKey in $sourceKeys) {
        $lines.Add("sourcePath=$(ConvertTo-WatchBase64 -Text $sourceKey)")
    }
    return Get-Sha256HexFromText -Text ($lines -join "`n")
}

function Read-AgentWatchRecoveryContext {
    param([Parameter(Mandatory = $true)][string]$Root)

    $runtimePath = Join-Path (Join-Path $Root "scratchpad") "watch-agents.runtime.json"
    $runtimeBytes = Read-SharedFileBytes -Path $runtimePath
    try {
        $runtime = $script:Utf8NoBom.GetString($runtimeBytes) | ConvertFrom-Json
    }
    catch {
        throw "scope検査用runtime stateをJSONとして読めません: $($_.Exception.Message)"
    }
    $definitionPath = [string](Get-RequiredStateValue -State $runtime -Name "definitionPath")
    $runtimeDefinitionSha256 = [string](Get-RequiredStateValue -State $runtime -Name "definitionSha256")
    return [pscustomobject]@{
        DefinitionPath = $definitionPath
        RuntimeDefinitionSha256 = $runtimeDefinitionSha256.ToLowerInvariant()
        RuntimeAgentStates = @((Get-RequiredStateValue -State $runtime -Name "agentStates"))
        Runtime = $runtime
    }
}

function Read-AgentWatchDefinitionContext {
    param([Parameter(Mandatory = $true)][string]$Root)

    $recoveryContext = Read-AgentWatchRecoveryContext -Root $Root
    $definitionPath = [string]$recoveryContext.DefinitionPath
    $definitionBytes = Read-SharedFileBytes -Path $definitionPath
    try {
        $definition = $script:Utf8NoBom.GetString($definitionBytes) | ConvertFrom-Json
    }
    catch {
        throw "監視定義をscope JSONとして読めません: $($_.Exception.Message)"
    }
    if ($null -eq $definition -or @($definition.PSObject.Properties.Name) -notcontains "agents") {
        throw "監視定義にagents配列がありません"
    }
    $scopeSchemaProperty = $definition.PSObject.Properties["scopeSchemaVersion"]
    return [pscustomobject]@{
        Strict = ($null -ne $scopeSchemaProperty)
        Definition = $definition
        DefinitionPath = $definitionPath
        DefinitionSha256 = Get-Sha256HexFromBytes -Bytes $definitionBytes
        RuntimeDefinitionSha256 = [string]$recoveryContext.RuntimeDefinitionSha256
        RuntimeAgentStates = @($recoveryContext.RuntimeAgentStates)
        Runtime = $recoveryContext.Runtime
    }
}

function Test-DefinitionRefreshRecoverySnapshot {
    param(
        [Parameter(Mandatory = $true)]$DefinitionContext,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][DateTime]$NowUtc
    )

    try {
        $runtime = $DefinitionContext.Runtime
        $runtimePath = [string](Get-RequiredStateValue -State $runtime -Name "runtimePath")
        $outputPath = [string](Get-RequiredStateValue -State $runtime -Name "outputPath")
        $outputSha256 = [string](Get-RequiredStateValue -State $runtime -Name "outputSha256")
        $outputLength = [Int64](Get-RequiredIntegerStateValue -State $runtime -Name "outputLength")
        $outputLastWriteUtc = Parse-RoundtripUtc -Text ([string](Get-RequiredStateValue -State $runtime -Name "outputLastWriteUtc")) -FieldName "outputLastWriteUtc"
        $scanCompletedUtc = Parse-RoundtripUtc -Text ([string](Get-RequiredStateValue -State $runtime -Name "scanCompletedUtc")) -FieldName "scanCompletedUtc"
        $stateWrittenUtc = Parse-RoundtripUtc -Text ([string](Get-RequiredStateValue -State $runtime -Name "stateWrittenUtc")) -FieldName "stateWrittenUtc"
        $watchPid = [int](Get-RequiredIntegerStateValue -State $runtime -Name "pid")
        $processExecutablePath = [string](Get-RequiredStateValue -State $runtime -Name "processExecutablePath")
        $watcherPath = [string](Get-RequiredStateValue -State $runtime -Name "scriptPath")
        $storedAgentStatesSha256 = [string](Get-RequiredStateValue -State $runtime -Name "agentStatesSha256")
        $outputBytes = Read-SharedFileBytes -Path $outputPath
        $runtimeItem = Get-Item -LiteralPath $runtimePath -Force -ErrorAction Stop
        $outputItem = Get-Item -LiteralPath $outputPath -Force -ErrorAction Stop
    }
    catch {
        return New-CheckErrorResult -Code "DEFINITION_REFRESH_RECOVERY_INVALID" -Message "定義更新中の返信回復に必要なruntimeを検査できません: $($_.Exception.Message)"
    }
    if ($outputBytes.Length -ne $outputLength -or
        -not [string]::Equals((Get-Sha256HexFromBytes -Bytes $outputBytes), $outputSha256.ToLowerInvariant(), [StringComparison]::Ordinal) -or
        $outputItem.LastWriteTimeUtc.Ticks -ne $outputLastWriteUtc.Ticks) {
        return New-PolicyResult -Code "DEFINITION_REFRESH_RECOVERY_STALE" -Message "定義更新中の返信回復でlatest outputの長さ/hash/時刻が一致しません。"
    }
    if (-not [string]::Equals(
        (Get-AgentStatesSha256 -AgentStates @($DefinitionContext.RuntimeAgentStates)),
        $storedAgentStatesSha256,
        [StringComparison]::Ordinal
    )) {
        return New-PolicyResult -Code "DEFINITION_REFRESH_RECOVERY_STALE" -Message "定義更新中の返信回復でagentStates hashが一致しません。"
    }
    foreach ($freshnessCheck in @(
        @($scanCompletedUtc, "runtime stateのscanCompletedUtc"),
        @($stateWrittenUtc, "runtime stateのstateWrittenUtc"),
        @($runtimeItem.LastWriteTimeUtc, "runtime state file"),
        @($outputItem.LastWriteTimeUtc, "latest output file")
    )) {
        $freshnessResult = Test-FreshTimestamp -TimestampUtc ([DateTime]$freshnessCheck[0]) -NowUtc $NowUtc -Label ([string]$freshnessCheck[1])
        if ($null -ne $freshnessResult) {
            return $freshnessResult
        }
    }
    $processCommandResult = Test-WatcherProcessArguments `
        -ProcessId $watchPid `
        -ProcessExecutablePath $processExecutablePath `
        -WatcherPath $watcherPath `
        -DefinitionPath ([string]$DefinitionContext.DefinitionPath) `
        -Root $Root
    if ($processCommandResult.ExitCode -ne 0) {
        return $processCommandResult
    }
    return New-AgentWatchResult -ExitCode 0 -Code "DEFINITION_REFRESH_RECOVERY_OK" -Message "定義更新中もruntime/output/processはfreshで自己整合しています。"
}

function Get-AgentIdentityMap {
    param(
        [Parameter(Mandatory = $true)]$DefinitionContext,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$AgentStates
    )

    $definitionAgents = @($DefinitionContext.Definition.agents)
    $agentsByThread = New-Object 'Collections.Generic.Dictionary[string,object]' ([StringComparer]::Ordinal)
    for ($index = 0; $index -lt $AgentStates.Count; $index++) {
        $agentState = $AgentStates[$index]
        $threadId = ""
        if ($index -lt $definitionAgents.Count) {
            $threadProperty = $definitionAgents[$index].PSObject.Properties["threadId"]
            if ($null -ne $threadProperty -and $threadProperty.Value -is [string]) {
                $candidate = ([string]$threadProperty.Value).Trim()
                if ([regex]::IsMatch($candidate, '^[A-Za-z0-9][A-Za-z0-9._:-]{5,127}$', [Text.RegularExpressions.RegexOptions]::CultureInvariant)) {
                    $threadId = $candidate
                }
            }
        }
        if ([string]::IsNullOrEmpty($threadId)) {
            $threadId = Get-AgentThreadIdFromName -Name ([string]$agentState.name)
        }
        if ([string]::IsNullOrEmpty($threadId)) {
            continue
        }
        if ($agentsByThread.ContainsKey($threadId)) {
            throw "監視定義のthread IDが重複しています: $threadId"
        }
        $agentsByThread.Add($threadId, $agentState)
    }
    return ,$agentsByThread
}

function Test-AgentWatchScopeDefinition {
    param(
        [Parameter(Mandatory = $true)]$DefinitionContext,
        [Parameter(Mandatory = $true)][string]$Root
    )

    if (-not [bool]$DefinitionContext.Strict) {
        return [pscustomobject]@{
            Result = New-AgentWatchResult -ExitCode 0 -Code "AGENT_WATCH_SCOPE_LEGACY" -Message "旧監視定義ではscope検査を追加しません。"
            AgentsByThread = (New-Object 'Collections.Generic.Dictionary[string,object]' ([StringComparer]::Ordinal))
        }
    }
    if (-not [string]::Equals(
        [string]$DefinitionContext.DefinitionSha256,
        [string]$DefinitionContext.RuntimeDefinitionSha256,
        [StringComparison]::Ordinal
    )) {
        return [pscustomobject]@{
            Result = New-PolicyResult -Code "DEFINITION_HASH_MISMATCH" -Message "監視定義が最後のscan後に変わっています。次のscan完了まで委譲できません。"
            AgentsByThread = $null
        }
    }

    $definition = $DefinitionContext.Definition
    $topProperties = @($definition.PSObject.Properties.Name)
    $expectedTopProperties = @("scopeSchemaVersion", "agents")
    if (@($expectedTopProperties | Where-Object { $topProperties -notcontains $_ }).Count -gt 0 -or
        @($topProperties | Where-Object { $expectedTopProperties -notcontains $_ }).Count -gt 0) {
        return [pscustomobject]@{
            Result = New-CheckErrorResult -Code "AGENT_WATCH_SCOPE_DEFINITION_INVALID" -Message "strict監視定義のtop-level fieldがscope schema 1と一致しません。"
            AgentsByThread = $null
        }
    }
    try {
        $scopeSchemaVersion = Get-RequiredIntegerStateValue -State $definition -Name "scopeSchemaVersion"
    }
    catch {
        return [pscustomobject]@{
            Result = New-CheckErrorResult -Code "AGENT_WATCH_SCOPE_DEFINITION_INVALID" -Message $_.Exception.Message
            AgentsByThread = $null
        }
    }
    if ($scopeSchemaVersion -ne $script:AgentWatchScopeSchemaVersion) {
        return [pscustomobject]@{
            Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_SCHEMA_MISMATCH" -Message "監視定義のscopeSchemaVersionが未対応です: $scopeSchemaVersion"
            AgentsByThread = $null
        }
    }

    $definitionAgents = @($definition.agents)
    $agentsByThread = New-Object 'Collections.Generic.Dictionary[string,object]' ([StringComparer]::Ordinal)
    $reports = New-Object 'Collections.Generic.Dictionary[string,string]' ([StringComparer]::OrdinalIgnoreCase)
    $writeOwners = New-Object System.Collections.Generic.List[object]
    $expectedAgentProperties = @("name", "threadId", "readOnly", "reportPath", "sourcePaths", "scopeSha256")
    for ($index = 0; $index -lt $definitionAgents.Count; $index++) {
        $agent = $definitionAgents[$index]
        if ($null -eq $agent) {
            return [pscustomobject]@{
                Result = New-CheckErrorResult -Code "AGENT_WATCH_SCOPE_DEFINITION_INVALID" -Message "監視定義agents[$index]がnullです。"
                AgentsByThread = $null
            }
        }
        $properties = @($agent.PSObject.Properties.Name)
        if (@($expectedAgentProperties | Where-Object { $properties -notcontains $_ }).Count -gt 0 -or
            @($properties | Where-Object { $expectedAgentProperties -notcontains $_ }).Count -gt 0) {
            return [pscustomobject]@{
                Result = New-CheckErrorResult -Code "AGENT_WATCH_SCOPE_DEFINITION_INVALID" -Message "監視定義agents[$index]のfieldがscope schema 1と一致しません。"
                AgentsByThread = $null
            }
        }
        foreach ($stringField in @("name", "threadId", "reportPath", "scopeSha256")) {
            if (-not ($agent.PSObject.Properties[$stringField].Value -is [string])) {
                return [pscustomobject]@{
                    Result = New-CheckErrorResult -Code "AGENT_WATCH_SCOPE_DEFINITION_INVALID" -Message "監視定義agents[$index].$stringField がJSON文字列ではありません。"
                    AgentsByThread = $null
                }
            }
        }
        $name = [string]$agent.name
        $threadId = [string]$agent.threadId
        $reportPath = [string]$agent.reportPath
        $scopeSha256 = [string]$agent.scopeSha256
        if ([string]::IsNullOrWhiteSpace($name) -or
            -not [regex]::IsMatch($threadId, '^[A-Za-z0-9][A-Za-z0-9._:-]{5,127}$', [Text.RegularExpressions.RegexOptions]::CultureInvariant) -or
            [string]::IsNullOrWhiteSpace($reportPath) -or
            -not [regex]::IsMatch($scopeSha256, '^[0-9a-f]{64}$')) {
            return [pscustomobject]@{
                Result = New-CheckErrorResult -Code "AGENT_WATCH_SCOPE_DEFINITION_INVALID" -Message "監視定義agents[$index]のname/threadId/reportPath/scopeSha256が不正です。"
                AgentsByThread = $null
            }
        }
        $readOnlyProperty = $agent.PSObject.Properties["readOnly"]
        if ($null -eq $readOnlyProperty -or -not ($readOnlyProperty.Value -is [bool])) {
            return [pscustomobject]@{
                Result = New-CheckErrorResult -Code "AGENT_WATCH_SCOPE_DEFINITION_INVALID" -Message "監視定義agents[$index].readOnlyがJSON booleanではありません。"
                AgentsByThread = $null
            }
        }
        $readOnly = [bool]$readOnlyProperty.Value
        if ($null -eq $agent.sourcePaths -or -not ($agent.sourcePaths -is [Array])) {
            return [pscustomobject]@{
                Result = New-CheckErrorResult -Code "AGENT_WATCH_SCOPE_DEFINITION_INVALID" -Message "監視定義agents[$index].sourcePathsがJSON配列ではありません。"
                AgentsByThread = $null
            }
        }
        foreach ($configuredSource in @($agent.sourcePaths)) {
            if (-not ($configuredSource -is [string])) {
                return [pscustomobject]@{
                    Result = New-CheckErrorResult -Code "AGENT_WATCH_SCOPE_DEFINITION_INVALID" -Message "監視定義agents[$index].sourcePathsにJSON文字列以外があります。"
                    AgentsByThread = $null
                }
            }
        }
        [string[]]$configuredSources = @($agent.sourcePaths | ForEach-Object { [string]$_ })
        $reportKey = Get-WatchPathComparisonKey -Path $reportPath -BasePath $Root
        [string[]]$logicalSources = @()
        if ($readOnly) {
            if ($configuredSources.Count -ne 1 -or
                -not [string]::Equals(
                    (Get-WatchPathComparisonKey -Path $configuredSources[0] -BasePath $Root),
                    $reportKey,
                    [StringComparison]::OrdinalIgnoreCase
                )) {
                return [pscustomobject]@{
                    Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_READ_ONLY_SENTINEL_INVALID" -Message "readOnly担当のsourcePathsはwatcher互換用のreportPath 1件だけにしてください: agent=$name"
                    AgentsByThread = $null
                }
            }
        }
        else {
            if ($configuredSources.Count -eq 0) {
                return [pscustomobject]@{
                    Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_DEFINITION_INVALID" -Message "write担当のsourcePathsが空です: agent=$name"
                    AgentsByThread = $null
                }
            }
            $logicalSources = $configuredSources
        }
        $sourceSet = New-Object 'Collections.Generic.Dictionary[string,string]' ([StringComparer]::OrdinalIgnoreCase)
        foreach ($sourcePath in $logicalSources) {
            if ([string]::IsNullOrWhiteSpace($sourcePath)) {
                return [pscustomobject]@{
                    Result = New-CheckErrorResult -Code "AGENT_WATCH_SCOPE_DEFINITION_INVALID" -Message "監視定義agents[$index].sourcePathsに空のpathがあります。"
                    AgentsByThread = $null
                }
            }
            $sourceKey = Get-WatchPathComparisonKey -Path $sourcePath -BasePath $Root
            if ($sourceSet.ContainsKey($sourceKey)) {
                return [pscustomobject]@{
                    Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_PATH_DUPLICATE" -Message "同じ担当のsourcePathsが重複しています: agent=$name path=$sourceKey"
                    AgentsByThread = $null
                }
            }
            $sourceSet.Add($sourceKey, $sourcePath)
        }
        $expectedScopeSha256 = Get-AgentWatchScopeSha256 `
            -ThreadId $threadId `
            -ReadOnly $readOnly `
            -ReportPath $reportPath `
            -SourcePaths $logicalSources `
            -Root $Root
        if (-not [string]::Equals($scopeSha256, $expectedScopeSha256, [StringComparison]::Ordinal)) {
            return [pscustomobject]@{
                Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_HASH_MISMATCH" -Message "監視定義agents[$index].scopeSha256がpath集合から再計算した値と一致しません: agent=$name"
                AgentsByThread = $null
            }
        }
        if ($agentsByThread.ContainsKey($threadId)) {
            return [pscustomobject]@{
                Result = New-PolicyResult -Code "AGENT_THREAD_ID_DUPLICATE" -Message "監視定義のthreadIdが重複しています: $threadId"
                AgentsByThread = $null
            }
        }
        if ($reports.ContainsKey($reportKey)) {
            return [pscustomobject]@{
                Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_REPORT_DUPLICATE" -Message "監視定義のreportPathが担当間で重複しています: $reportKey"
                AgentsByThread = $null
            }
        }
        $reports.Add($reportKey, $threadId)
        # 報告書も担当が実際に書くpathである。別担当のsource tree配下へ置くと
        # その更新を別担当の進捗として数えるため、所有writeとして重複検査する。
        $writeOwners.Add([pscustomobject]@{
            Name = $name
            ThreadId = $threadId
            Path = $reportKey
        })
        $entry = [pscustomobject]@{
            Name = $name
            ThreadId = $threadId
            ReadOnly = $readOnly
            ReportPath = $reportKey
            SourcePaths = @($sourceSet.Keys)
            ScopeSha256 = $scopeSha256
        }
        $agentsByThread.Add($threadId, $entry)
        if (-not $readOnly) {
            foreach ($sourceKey in @($sourceSet.Keys)) {
                $writeOwners.Add([pscustomobject]@{
                    Name = $name
                    ThreadId = $threadId
                    Path = [string]$sourceKey
                })
            }
        }
    }

    for ($firstIndex = 0; $firstIndex -lt $writeOwners.Count; $firstIndex++) {
        for ($secondIndex = $firstIndex + 1; $secondIndex -lt $writeOwners.Count; $secondIndex++) {
            $first = $writeOwners[$firstIndex]
            $second = $writeOwners[$secondIndex]
            if ([string]::Equals([string]$first.ThreadId, [string]$second.ThreadId, [StringComparison]::Ordinal)) {
                continue
            }
            if (Test-WatchPathOverlap -FirstPath ([string]$first.Path) -SecondPath ([string]$second.Path)) {
                return [pscustomobject]@{
                    Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_OVERLAP" -Message (
                        "同時write担当のpathが同一または親子で重複しています: {0} ({1}) <-> {2} ({3})" -f
                            $first.Name, $first.Path, $second.Name, $second.Path
                    )
                    AgentsByThread = $null
                }
            }
        }
    }
    return [pscustomobject]@{
        Result = New-AgentWatchResult -ExitCode 0 -Code "AGENT_WATCH_SCOPE_DEFINITION_OK" -Message "監視定義のthreadId/readOnly/path所有は一意です。"
        AgentsByThread = $agentsByThread
    }
}

function Read-AgentWatchScopeBlock {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text,
        [Parameter(Mandatory = $true)][string]$Root
    )

    $openingMarker = "[AGENT_WATCH_SCOPE schema=1]"
    $closingMarker = "[/AGENT_WATCH_SCOPE]"
    $openingMatches = [regex]::Matches($Text, '(?m)^\[AGENT_WATCH_SCOPE schema=1\]\r?$')
    $closingMatches = [regex]::Matches($Text, '(?m)^\[/AGENT_WATCH_SCOPE\]\r?$')
    if ($openingMatches.Count -eq 0 -or $closingMatches.Count -eq 0) {
        return [pscustomobject]@{
            Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_MISSING" -Message "strict監視定義では委譲本文にAGENT_WATCH_SCOPEを1件書いてください。"
            Scope = $null
        }
    }
    if ($openingMatches.Count -ne 1 -or $closingMatches.Count -ne 1 -or
        $closingMatches[0].Index -le $openingMatches[0].Index) {
        return [pscustomobject]@{
            Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_INVALID" -Message "AGENT_WATCH_SCOPE markerは引用やcode fenceでなく1組だけ書いてください。"
            Scope = $null
        }
    }
    $prefix = $Text.Substring(0, $openingMatches[0].Index)
    if (([regex]::Matches($prefix, '(?m)^```').Count % 2) -ne 0) {
        return [pscustomobject]@{
            Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_INVALID" -Message "code fence内のAGENT_WATCH_SCOPEは宣言として扱いません。"
            Scope = $null
        }
    }
    $bodyStart = $openingMatches[0].Index + $openingMatches[0].Length
    $body = $Text.Substring($bodyStart, $closingMatches[0].Index - $bodyStart).Trim([char[]]"`r`n")
    $fields = @{}
    $sources = New-Object System.Collections.Generic.List[string]
    foreach ($line in @($body -split "`r?`n")) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            return [pscustomobject]@{
                Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_INVALID" -Message "AGENT_WATCH_SCOPE内に空行は置けません。"
                Scope = $null
            }
        }
        $separatorIndex = $line.IndexOf('=')
        if ($separatorIndex -le 0) {
            return [pscustomobject]@{
                Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_INVALID" -Message "AGENT_WATCH_SCOPEの各行はkey=valueで書いてください。"
                Scope = $null
            }
        }
        $key = $line.Substring(0, $separatorIndex)
        $value = $line.Substring($separatorIndex + 1)
        if ([string]::IsNullOrWhiteSpace($value) -or -not [string]::Equals($value, $value.Trim(), [StringComparison]::Ordinal)) {
            return [pscustomobject]@{
                Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_INVALID" -Message "AGENT_WATCH_SCOPEの値は空や前後空白を含められません: key=$key"
                Scope = $null
            }
        }
        if ($key -eq "sourcePath") {
            $sources.Add($value)
            continue
        }
        if (@("threadId", "readOnly", "reportPath") -notcontains $key -or $fields.ContainsKey($key)) {
            return [pscustomobject]@{
                Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_INVALID" -Message "AGENT_WATCH_SCOPEに未知または重複fieldがあります: $key"
                Scope = $null
            }
        }
        $fields[$key] = $value
    }
    foreach ($requiredField in @("threadId", "readOnly", "reportPath")) {
        if (-not $fields.ContainsKey($requiredField)) {
            return [pscustomobject]@{
                Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_INVALID" -Message "AGENT_WATCH_SCOPEの必須fieldがありません: $requiredField"
                Scope = $null
            }
        }
    }
    $threadId = [string]$fields["threadId"]
    if (-not [regex]::IsMatch($threadId, '^[A-Za-z0-9][A-Za-z0-9._:-]{5,127}$', [Text.RegularExpressions.RegexOptions]::CultureInvariant)) {
        return [pscustomobject]@{
            Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_INVALID" -Message "AGENT_WATCH_SCOPEのthreadIdが不正です。"
            Scope = $null
        }
    }
    $readOnlyText = [string]$fields["readOnly"]
    if ($readOnlyText -ne "true" -and $readOnlyText -ne "false") {
        return [pscustomobject]@{
            Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_INVALID" -Message "AGENT_WATCH_SCOPEのreadOnlyはtrue/falseの小文字で書いてください。"
            Scope = $null
        }
    }
    $readOnly = ($readOnlyText -eq "true")
    if ($readOnly -and $sources.Count -ne 0) {
        return [pscustomobject]@{
            Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_INVALID" -Message "readOnly=trueの指示scopeにはsourcePathを置けません。"
            Scope = $null
        }
    }
    $reportKey = Get-WatchPathComparisonKey -Path ([string]$fields["reportPath"]) -BasePath $Root
    $sourceSet = New-Object 'Collections.Generic.Dictionary[string,string]' ([StringComparer]::OrdinalIgnoreCase)
    foreach ($sourcePath in $sources) {
        $sourceKey = Get-WatchPathComparisonKey -Path $sourcePath -BasePath $Root
        if ($sourceSet.ContainsKey($sourceKey)) {
            return [pscustomobject]@{
                Result = New-PolicyResult -Code "AGENT_WATCH_SCOPE_PATH_DUPLICATE" -Message "AGENT_WATCH_SCOPEのsourcePathが重複しています: $sourceKey"
                Scope = $null
            }
        }
        $sourceSet.Add($sourceKey, $sourcePath)
    }
    $scope = [pscustomobject]@{
        ThreadId = $threadId
        ReadOnly = $readOnly
        ReportPath = $reportKey
        SourcePaths = @($sourceSet.Keys)
        ScopeSha256 = Get-AgentWatchScopeSha256 `
            -ThreadId $threadId `
            -ReadOnly $readOnly `
            -ReportPath ([string]$fields["reportPath"]) `
            -SourcePaths @($sources) `
            -Root $Root
    }
    return [pscustomobject]@{
        Result = New-AgentWatchResult -ExitCode 0 -Code "AGENT_WATCH_SCOPE_BLOCK_OK" -Message "AGENT_WATCH_SCOPEを構文解析しました。"
        Scope = $scope
    }
}

function Test-AgentWatchDelegationScope {
    param(
        [Parameter(Mandatory = $true)]$DefinitionContext,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$DelegationText,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$TargetThreadId
    )

    if (-not [string]::Equals(
        [string]$DefinitionContext.DefinitionSha256,
        [string]$DefinitionContext.RuntimeDefinitionSha256,
        [StringComparison]::Ordinal
    )) {
        return New-PolicyResult -Code "DEFINITION_HASH_MISMATCH" -Message "監視定義が最後のscan後に変わっています。次のscan完了まで委譲できません。"
    }
    if (-not [bool]$DefinitionContext.Strict) {
        return New-AgentWatchResult -ExitCode 0 -Code "AGENT_WATCH_SCOPE_LEGACY" -Message "旧監視定義では委譲scope blockを要求しません。"
    }
    if ([string]::IsNullOrEmpty($TargetThreadId)) {
        return New-PolicyResult -Code "AGENT_WATCH_SCOPE_TARGET_REQUIRED" -Message "strict監視中の新規targetless委譲は実threadIdを事前に確定できません。既存5担当へのtarget付き送信だけを行ってください。"
    }
    $definitionResult = Test-AgentWatchScopeDefinition -DefinitionContext $DefinitionContext -Root $Root
    if ($definitionResult.Result.ExitCode -ne 0) {
        return $definitionResult.Result
    }
    $blockResult = Read-AgentWatchScopeBlock -Text $DelegationText -Root $Root
    if ($blockResult.Result.ExitCode -ne 0) {
        return $blockResult.Result
    }
    $scope = $blockResult.Scope
    if (-not [string]::IsNullOrEmpty($TargetThreadId) -and
        -not [string]::Equals($TargetThreadId, [string]$scope.ThreadId, [StringComparison]::Ordinal)) {
        return New-PolicyResult -Code "AGENT_WATCH_SCOPE_THREAD_MISMATCH" -Message "payloadの送信先とAGENT_WATCH_SCOPE.threadIdが一致しません。"
    }
    if (-not $definitionResult.AgentsByThread.ContainsKey([string]$scope.ThreadId)) {
        return New-PolicyResult -Code "AGENT_WATCH_SCOPE_THREAD_UNKNOWN" -Message "AGENT_WATCH_SCOPE.threadIdに対応する監視担当がありません: $($scope.ThreadId)"
    }
    $expected = $definitionResult.AgentsByThread[[string]$scope.ThreadId]
    if ([bool]$scope.ReadOnly -ne [bool]$expected.ReadOnly) {
        return New-PolicyResult -Code "AGENT_WATCH_SCOPE_MODE_MISMATCH" -Message "指示のreadOnlyと監視定義が一致しません: thread=$($scope.ThreadId)"
    }
    if (-not [string]::Equals([string]$scope.ReportPath, [string]$expected.ReportPath, [StringComparison]::OrdinalIgnoreCase)) {
        return New-PolicyResult -Code "AGENT_WATCH_SCOPE_REPORT_MISMATCH" -Message "指示のreportPathと監視定義が一致しません: expected=$($expected.ReportPath) actual=$($scope.ReportPath)"
    }
    $actualSources = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    foreach ($sourcePath in @($scope.SourcePaths)) { [void]$actualSources.Add([string]$sourcePath) }
    $expectedSources = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    foreach ($sourcePath in @($expected.SourcePaths)) { [void]$expectedSources.Add([string]$sourcePath) }
    $missing = @($expectedSources | Where-Object { -not $actualSources.Contains([string]$_) } | Sort-Object)
    if ($missing.Count -gt 0) {
        return New-PolicyResult -Code "AGENT_WATCH_SCOPE_MISSING_PATH" -Message "指示scopeに監視定義のwrite pathが不足しています: $($missing -join ', ')"
    }
    $extra = @($actualSources | Where-Object { -not $expectedSources.Contains([string]$_) } | Sort-Object)
    if ($extra.Count -gt 0) {
        return New-PolicyResult -Code "AGENT_WATCH_SCOPE_EXTRA_PATH" -Message "指示scopeに監視定義にないwrite pathがあります: $($extra -join ', ')"
    }
    if (-not [string]::Equals([string]$scope.ScopeSha256, [string]$expected.ScopeSha256, [StringComparison]::Ordinal)) {
        return New-PolicyResult -Code "AGENT_WATCH_SCOPE_HASH_MISMATCH" -Message "指示scopeのhashが監視定義と一致しません: thread=$($scope.ThreadId)"
    }
    return New-AgentWatchResult -ExitCode 0 -Code "AGENT_WATCH_SCOPE_OK" -Message "指示scopeと監視定義が不足・余分なく一致しています。"
}

function Read-AgentSendLedger {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return @()
    }
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "送信履歴が通常fileではありません: $Path"
    }
    if (Test-ReparsePoint -Path $Path) {
        throw "送信履歴にreparse pointは使えません: $Path"
    }
    $bytes = Read-SharedFileBytes -Path $Path
    if ($bytes.Length -eq 0) {
        throw "送信履歴が空です: $Path"
    }
    if ($bytes.Length -gt 1048576) {
        throw "送信履歴が上限1MiBを超えています: $Path"
    }
    try {
        $ledger = $script:Utf8NoBom.GetString($bytes) | ConvertFrom-Json
    }
    catch {
        throw "送信履歴を厳密UTF-8 JSONとして読めません: $($_.Exception.Message)"
    }
    if ($null -eq $ledger) {
        throw "送信履歴がnullです"
    }
    $topProperties = @($ledger.PSObject.Properties.Name)
    $expectedTopProperties = @("schemaVersion", "records")
    if (@($expectedTopProperties | Where-Object { $topProperties -notcontains $_ }).Count -gt 0 -or
        @($topProperties | Where-Object { $expectedTopProperties -notcontains $_ }).Count -gt 0) {
        throw "送信履歴のtop-level fieldがschema 1と一致しません"
    }
    $schemaVersion = Get-RequiredIntegerStateValue -State $ledger -Name "schemaVersion"
    if ($schemaVersion -ne $script:AgentSendLedgerSchemaVersion) {
        throw "送信履歴のschemaVersionが未対応です: $($ledger.schemaVersion)"
    }
    if ($null -eq $ledger.records -or $ledger.records.GetType().FullName -ne "System.Object[]") {
        throw "送信履歴のrecordsがJSON配列ではありません"
    }
    $records = @($ledger.records)
    $seenThreads = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal)
    foreach ($record in $records) {
        if ($null -eq $record) {
            throw "送信履歴recordsにnullがあります"
        }
        $properties = @($record.PSObject.Properties.Name)
        $expectedProperties = @("agentKey", "threadId", "lastCoordinatorSendUtc", "acknowledgedLatestWriteUtc")
        if (@($expectedProperties | Where-Object { $properties -notcontains $_ }).Count -gt 0 -or
            @($properties | Where-Object { $expectedProperties -notcontains $_ }).Count -gt 0) {
            throw "送信履歴recordのfieldがschema 1と一致しません"
        }
        $threadId = [string]$record.threadId
        if (-not [regex]::IsMatch($threadId, '^[A-Za-z0-9][A-Za-z0-9._:-]{5,127}$', [Text.RegularExpressions.RegexOptions]::CultureInvariant)) {
            throw "送信履歴のthreadIdが不正です"
        }
        if (-not $seenThreads.Add($threadId)) {
            throw "送信履歴のthreadIdが重複しています: $threadId"
        }
        if (-not [regex]::IsMatch([string]$record.agentKey, '^[0-9a-f]{64}$')) {
            throw "送信履歴のagentKeyがlowercase SHA-256ではありません: thread=$threadId"
        }
        $lastSent = Parse-RoundtripUtc -Text ([string]$record.lastCoordinatorSendUtc) -FieldName "sendLedger[$threadId].lastCoordinatorSendUtc"
        if (-not [string]::Equals([string]$record.lastCoordinatorSendUtc, $lastSent.ToString("o"), [StringComparison]::Ordinal)) {
            throw "送信履歴のlastCoordinatorSendUtcがcanonical表現ではありません: thread=$threadId"
        }
        $acknowledged = [string]$record.acknowledgedLatestWriteUtc
        if (-not [string]::IsNullOrEmpty($acknowledged)) {
            $acknowledgedUtc = Parse-RoundtripUtc -Text $acknowledged -FieldName "sendLedger[$threadId].acknowledgedLatestWriteUtc"
            if (-not [string]::Equals($acknowledged, $acknowledgedUtc.ToString("o"), [StringComparison]::Ordinal)) {
                throw "送信履歴のacknowledgedLatestWriteUtcがcanonical表現ではありません: thread=$threadId"
            }
        }
    }
    return $records
}

function Write-AgentSendLedger {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Records
    )

    $orderedRecords = @(
        $Records |
            Sort-Object @{ Expression = { [string]$_.threadId }; Ascending = $true } |
            ForEach-Object {
                [ordered]@{
                    agentKey = [string]$_.agentKey
                    threadId = [string]$_.threadId
                    lastCoordinatorSendUtc = [string]$_.lastCoordinatorSendUtc
                    acknowledgedLatestWriteUtc = [string]$_.acknowledgedLatestWriteUtc
                }
            }
    )
    $payload = [ordered]@{
        schemaVersion = $script:AgentSendLedgerSchemaVersion
        records = $orderedRecords
    }
    $bytes = $script:Utf8NoBom.GetBytes(($payload | ConvertTo-Json -Depth 6) + "`n")
    $temporaryPath = "{0}.{1}.tmp" -f $Path, [Guid]::NewGuid().ToString("N")
    $backupPath = "{0}.{1}.bak" -f $Path, [Guid]::NewGuid().ToString("N")
    $committed = $false
    try {
        $stream = New-Object IO.FileStream(
            $temporaryPath,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None
        )
        try {
            $stream.Write($bytes, 0, $bytes.Length)
            $stream.Flush($true)
        }
        finally {
            $stream.Dispose()
        }
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            [IO.File]::Replace($temporaryPath, $Path, $backupPath, $true)
        }
        else {
            [IO.File]::Move($temporaryPath, $Path)
        }
        $committed = $true
    }
    finally {
        try {
            if (Test-Path -LiteralPath $temporaryPath -PathType Leaf) {
                Remove-Item -LiteralPath $temporaryPath -Force
            }
            if (Test-Path -LiteralPath $backupPath -PathType Leaf) {
                Remove-Item -LiteralPath $backupPath -Force
            }
        }
        catch {
            if (-not $committed) {
                throw
            }
        }
    }
}

function Invoke-AgentReplyGate {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)]$HookPayload,
        [Parameter(Mandatory = $true)][string]$ToolName,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$DelegationText,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$AgentStates,
        [Parameter(Mandatory = $true)][DateTime]$NowUtc,
        [switch]$DefinitionRefreshRecoveryOnly
    )

    if ([string]::IsNullOrWhiteSpace($DelegationText)) {
        return New-PolicyResult -Code "AGENT_REPLY_TEXT_EMPTY" -Message "空の送信では返信待ちを解消できません。担当へ送る本文を書いてください。"
    }
    $targetThreadId = Get-DelegationTargetThreadId -HookPayload $HookPayload -ToolName $ToolName
    try {
        if ($DefinitionRefreshRecoveryOnly) {
            $definitionContext = $null
            $agentsByThread = New-Object 'Collections.Generic.Dictionary[string,object]' ([StringComparer]::Ordinal)
        }
        else {
            $definitionContext = Read-AgentWatchDefinitionContext -Root $Root
            $agentsByThread = Get-AgentIdentityMap -DefinitionContext $definitionContext -AgentStates $AgentStates
        }
    }
    catch {
        return New-CheckErrorResult -Code "AGENT_WATCH_SCOPE_DEFINITION_INVALID" -Message $_.Exception.Message
    }

    $ledgerPath = Join-Path (Join-Path $Root "scratchpad") $script:AgentSendLedgerFileName
    $mutexHash = Get-Sha256HexFromText -Text ([IO.Path]::GetFullPath($ledgerPath).ToLowerInvariant())
    $mutex = New-Object Threading.Mutex($false, ("Local\Ori3AgentWatchSendLedger_{0}" -f $mutexHash))
    $lockTaken = $false
    try {
        try {
            $lockTaken = $mutex.WaitOne($script:AgentSendLedgerLockTimeoutMilliseconds)
        }
        catch [Threading.AbandonedMutexException] {
            $lockTaken = $true
        }
        if (-not $lockTaken) {
            return New-CheckErrorResult -Code "AGENT_SEND_LEDGER_LOCK_TIMEOUT" -Message "送信履歴の排他取得が時間切れになりました"
        }

        try {
            $records = @(Read-AgentSendLedger -Path $ledgerPath)
        }
        catch {
            return New-CheckErrorResult -Code "AGENT_SEND_LEDGER_INVALID" -Message $_.Exception.Message
        }
        $recordsByThread = New-Object 'Collections.Generic.Dictionary[string,object]' ([StringComparer]::Ordinal)
        foreach ($record in $records) {
            $recordsByThread.Add([string]$record.threadId, $record)
        }

        if ($DefinitionRefreshRecoveryOnly) {
            # strict定義が編集中でも、既存ledgerのagentKeyと旧runtime stateが双方向に
            # 1対1ならthreadIdを復元できる。通常経路では旧threadを復活させない。
            foreach ($record in $records) {
                $recordThreadId = [string]$record.threadId
                $sameKeyRecords = @(
                    $records | Where-Object {
                        [string]::Equals([string]$_.agentKey, [string]$record.agentKey, [StringComparison]::Ordinal)
                    }
                )
                $matchingStates = @(
                    $AgentStates | Where-Object {
                        [string]::Equals([string]$_.agentKey, [string]$record.agentKey, [StringComparison]::Ordinal)
                    }
                )
                if ($sameKeyRecords.Count -eq 1 -and $matchingStates.Count -eq 1) {
                    $agentsByThread.Add($recordThreadId, $matchingStates[0])
                }
            }
        }

        $futureBoundaryUtc = $NowUtc.AddMinutes($script:FutureToleranceMinutes)
        $waitingAgents = New-Object System.Collections.Generic.List[object]
        foreach ($entry in $agentsByThread.GetEnumerator()) {
            $threadId = [string]$entry.Key
            $agentState = $entry.Value
            if (-not $recordsByThread.ContainsKey($threadId) -or
                ($DefinitionRefreshRecoveryOnly -and
                    -not [string]::Equals([string]$recordsByThread[$threadId].agentKey, [string]$agentState.agentKey, [StringComparison]::Ordinal)) -or
                [string]::IsNullOrEmpty([string]$agentState.latestWriteUtc)) {
                continue
            }
            $record = $recordsByThread[$threadId]
            $latestWriteUtc = Parse-RoundtripUtc -Text ([string]$agentState.latestWriteUtc) -FieldName "agent[$threadId].latestWriteUtc"
            $lastSentUtc = Parse-RoundtripUtc -Text ([string]$record.lastCoordinatorSendUtc) -FieldName "sendLedger[$threadId].lastCoordinatorSendUtc"
            if ($lastSentUtc -gt $futureBoundaryUtc) {
                return New-CheckErrorResult -Code "AGENT_SEND_LEDGER_FUTURE" -Message "送信履歴が許容範囲の2分を超えて未来です: thread=$threadId"
            }
            $acknowledgedUtc = [DateTime]::MinValue
            if (-not [string]::IsNullOrEmpty([string]$record.acknowledgedLatestWriteUtc)) {
                $acknowledgedUtc = Parse-RoundtripUtc -Text ([string]$record.acknowledgedLatestWriteUtc) -FieldName "sendLedger[$threadId].acknowledgedLatestWriteUtc"
                if ($acknowledgedUtc -gt $futureBoundaryUtc) {
                    return New-CheckErrorResult -Code "AGENT_SEND_LEDGER_FUTURE" -Message "送信時に確認した更新時刻が許容範囲の2分を超えて未来です: thread=$threadId"
                }
            }
            if ($latestWriteUtc -gt $lastSentUtc -and
                $latestWriteUtc -gt $acknowledgedUtc -and
                $latestWriteUtc -le $NowUtc.AddMinutes(-1.0 * $script:AgentReplyQuietMinutes)) {
                $waitingAgents.Add([pscustomobject]@{
                    ThreadId = $threadId
                    AgentState = $agentState
                    LatestWriteUtc = $latestWriteUtc
                })
            }
        }

        $targetIsWaiting = $false
        foreach ($waitingAgent in $waitingAgents) {
            if ([string]::Equals($targetThreadId, [string]$waitingAgent.ThreadId, [StringComparison]::Ordinal)) {
                $targetIsWaiting = $true
                break
            }
        }
        if ($waitingAgents.Count -gt 0 -and -not $targetIsWaiting) {
            $waitingDescription = @(
                $waitingAgents |
                    Sort-Object @{ Expression = { [string]$_.ThreadId }; Ascending = $true } |
                    ForEach-Object {
                        "{0} (thread={1}, latestWriteUtc={2})" -f
                            ([string]$_.AgentState.name), ([string]$_.ThreadId), $_.LatestWriteUtc.ToString("o")
                    }
            ) -join "; "
            return New-PolicyResult -Code "AGENT_REPLY_REQUIRED" -Message "先に次の担当へ返事をしてください: $waitingDescription"
        }

        if ($DefinitionRefreshRecoveryOnly -and -not $targetIsWaiting) {
            return New-PolicyResult -Code "DEFINITION_REFRESH_REPLY_NOT_WAITING" -Message "定義更新中の例外は、15分以上返信待ちの本人への送信だけに使えます。"
        }

        $dependencyWarnings = New-Object 'Collections.Generic.List[string]'

        # scope回復中の返信待ち本人も含め、実行指示の構成だけは常に検査する。
        # ここで拒否すればledgerは更新されず、構成を足した本文をそのまま再送できる。
        $runConfigurationResult = Test-DelegationRunConfiguration -Text $DelegationText
        if ($runConfigurationResult.ExitCode -ne 0) {
            return $runConfigurationResult
        }

        # 返信待ち本人には、誤ったscope定義から回復する道を必ず残す。別担当・新規委譲は
        # この例外へ入らず、下の不足/余分/重複を含むstrict検査を通る必要がある。
        if (-not $targetIsWaiting) {
            $scopeResult = Test-AgentWatchDelegationScope `
                -DefinitionContext $definitionContext `
                -Root $Root `
                -DelegationText $DelegationText `
                -TargetThreadId $targetThreadId
            if ($scopeResult.ExitCode -ne 0) {
                return $scopeResult
            }

            # strict scopeが確定してからだけCargo依存を読む。返信待ち本人への回復路は
            # Cargo.tomlやscope定義が壊れていても塞がない。警告は送信自体を拒否せず、
            # 後段でadditionalContextとして返す。
            $strongPublicTypeHints = @(Get-StrongPublicTypeMutationSourceHints -Text $DelegationText)
            if ([bool]$definitionContext.Strict -and $strongPublicTypeHints.Count -gt 0) {
                $scopeBlock = Read-AgentWatchScopeBlock -Text $DelegationText -Root $Root
                foreach ($dependencyWarning in @(Get-PublicTypeDependencyWarnings `
                            -Text $DelegationText `
                            -Root $Root `
                            -Scope $scopeBlock.Scope `
                            -Hints $strongPublicTypeHints)) {
                    $dependencyWarnings.Add([string]$dependencyWarning)
                }
            }
        }

        if (-not [string]::IsNullOrEmpty($targetThreadId) -and $agentsByThread.ContainsKey($targetThreadId)) {
            $targetAgent = $agentsByThread[$targetThreadId]
            $acknowledgedLatestWriteUtc = [string]$targetAgent.latestWriteUtc
            $updatedRecord = [pscustomobject][ordered]@{
                agentKey = [string]$targetAgent.agentKey
                threadId = $targetThreadId
                lastCoordinatorSendUtc = $NowUtc.ToUniversalTime().ToString("o")
                acknowledgedLatestWriteUtc = $acknowledgedLatestWriteUtc
            }
            if ($recordsByThread.ContainsKey($targetThreadId)) {
                $recordsByThread[$targetThreadId] = $updatedRecord
            }
            else {
                $recordsByThread.Add($targetThreadId, $updatedRecord)
            }
            try {
                Write-AgentSendLedger -Path $ledgerPath -Records @($recordsByThread.Values)
            }
            catch {
                return New-CheckErrorResult -Code "AGENT_SEND_LEDGER_WRITE_ERROR" -Message "送信履歴を保存できません: $($_.Exception.Message)"
            }
        }
        return New-AgentWatchResult `
            -ExitCode 0 `
            -Code "AGENT_REPLY_GATE_OK" `
            -Message "返信待ち担当との送信順序は正常です。" `
            -Warnings @($dependencyWarnings.ToArray())
    }
    finally {
        if ($lockTaken) {
            $mutex.ReleaseMutex()
        }
        $mutex.Dispose()
    }
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
    elseif ([string]$Result.Code -eq "AGENT_REPLY_REQUIRED") {
        "待っている担当本人への送信は許可されます。表示された担当へ先に返事を送ってください。"
    }
    elseif ([string]$Result.Code -eq "AGENT_REPLY_TEXT_EMPTY") {
        "空でない返事を待っている担当本人へ送ってください。"
    }
    elseif ([string]$Result.Code -eq "AGENT_RUN_CONFIGURATION_REQUIRED") {
        "cargo/npmの実行指示へ --release を付けるか、同じ命令文でdebug構成であることを明記してください。禁止・実行中・過去結果の記述には追記不要です。"
    }
    elseif ([string]$Result.Code -like "AGENT_WATCH_SCOPE_*") {
        "監視定義のthreadId/readOnly/reportPath/sourcePathsと委譲本文のAGENT_WATCH_SCOPEを不足・余分なく一致させてください。path重複時は所有を1担当へ絞ってください。"
    }
    elseif ([string]$Result.Code -eq "DEFINITION_HASH_MISMATCH") {
        "監視定義を変更した直後です。次のscan完了を待ってください。15分以上返信待ちの本人への非空返信だけは回復路で通ります。"
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

function Write-HookAdditionalContext {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Messages)

    if (@($Messages).Count -eq 0) {
        return
    }
    # warningは既存のdeny判定を上書きしない。明示allowも出さず、呼び出し側が
    # 判断材料として読めるadditionalContextだけを返す。
    $payload = [ordered]@{
        hookSpecificOutput = [ordered]@{
            hookEventName = "PreToolUse"
            additionalContext = (@($Messages) -join "`n")
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
            # 定義更新後のscan待ちであっても、旧runtimeと送信履歴から「15分以上
            # 返信待ちの本人」と実証できる相手への非空返信だけは回復路として通す。
            # 別担当・新規委譲は従来どおりDEFINITION_HASH_MISMATCHで停止する。
            if ([string]$hookResult.Code -eq "DEFINITION_HASH_MISMATCH") {
                $recoveryText = Get-DelegationText -HookPayload $hookPayload -ToolName $toolName
                try {
                    $recoveryContext = Read-AgentWatchRecoveryContext -Root $resolvedRoot
                    $recoverySnapshot = Test-DefinitionRefreshRecoverySnapshot `
                        -DefinitionContext $recoveryContext `
                        -Root $resolvedRoot `
                        -NowUtc ([DateTime]::UtcNow)
                    if ($recoverySnapshot.ExitCode -eq 0) {
                        $recoveryGate = Invoke-AgentReplyGate `
                            -Root $resolvedRoot `
                            -HookPayload $hookPayload `
                            -ToolName $toolName `
                            -DelegationText $recoveryText `
                            -AgentStates @($recoveryContext.RuntimeAgentStates) `
                            -NowUtc ([DateTime]::UtcNow) `
                            -DefinitionRefreshRecoveryOnly
                        if ($recoveryGate.ExitCode -eq 0) {
                            exit 0
                        }
                    }
                }
                catch {
                    # 回復路の検査不能を通常の許可へ倒さない。元のhash不一致を返す。
                }
            }
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
            exit 0
        }
        $replyGateResult = Invoke-AgentReplyGate `
            -Root $resolvedRoot `
            -HookPayload $hookPayload `
            -ToolName $toolName `
            -DelegationText $delegationText `
            -AgentStates @($hookResult.AgentStates) `
            -NowUtc ([DateTime]::UtcNow)
        if ($replyGateResult.ExitCode -ne 0) {
            Write-HookDeny -Result $replyGateResult -Root $resolvedRoot
        }
        elseif (@($replyGateResult.Warnings).Count -gt 0) {
            Write-HookAdditionalContext -Messages @($replyGateResult.Warnings)
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
