param(
    [string]$SourceHookPath,
    [string]$SourceSettingsPath,
    [string]$ChildPowerShellPath
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)

$script:Assertions = 0
$script:Failures = 0
$script:Cases = 0
$script:DeniedCases = 0
$script:AllowedCases = 0
$script:ExpectedWarningCases = 0
$script:ObservedWarningCases = 0

function Assert-True {
    param(
        [bool]$Condition,
        [string]$Message
    )

    $script:Assertions++
    if (-not $Condition) {
        $script:Failures++
        Write-Host ("  FAIL: {0}" -f $Message) -ForegroundColor Red
    }
}

function Invoke-ProcessWithInput {
    param(
        [string]$FileName,
        [string]$Arguments,
        [AllowEmptyString()][string]$InputText,
        [switch]$UseInheritedStandardInputWriter
    )

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $FileName
    $startInfo.Arguments = $Arguments
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = New-Object System.Text.UTF8Encoding($false)
    $startInfo.StandardErrorEncoding = New-Object System.Text.UTF8Encoding($false)

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    $savedInputEncoding = $null
    if (-not $UseInheritedStandardInputWriter) {
        # Windows PowerShell 5.1's .NET Framework has no
        # ProcessStartInfo.StandardInputEncoding property. Process.StandardInput inherits
        # Console.InputEncoding, so pin it from Start through Write/Close and then restore it.
        $savedInputEncoding = [Console]::InputEncoding
        [Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
    }
    try {
        if (-not $process.Start()) {
            throw 'policy hook process could not be started'
        }
        $process.StandardInput.Write($InputText)
        $process.StandardInput.Close()
    }
    finally {
        if ($null -ne $savedInputEncoding) {
            [Console]::InputEncoding = $savedInputEncoding
        }
    }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()

    if ($stderr.Contains('ORIGAMI3_HOOK_FAIL_OPEN:')) {
        $script:ObservedWarningCases++
    }

    return [pscustomobject]@{
        ExitCode = $process.ExitCode
        Stdout = $stdout
        Stderr = $stderr
    }
}

function Invoke-Hook {
    param(
        [string]$HookPath,
        [string]$Command,
        [string]$ToolName = 'PowerShell',
        [string]$EventName = 'PreToolUse',
        [string]$RawInput,
        [switch]$UseInheritedStandardInputWriter
    )

    if ($PSBoundParameters.ContainsKey('RawInput')) {
        $inputText = $RawInput
    }
    else {
        $inputText = [ordered]@{
            hook_event_name = $EventName
            tool_name = $ToolName
            tool_input = [ordered]@{
                command = $Command
            }
        } | ConvertTo-Json -Compress -Depth 4
    }

    $escapedHookPath = $HookPath.Replace('"', '""')
    $arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $escapedHookPath + '"'
    return Invoke-ProcessWithInput `
        -FileName $script:ChildPowerShellPath `
        -Arguments $arguments `
        -InputText $inputText `
        -UseInheritedStandardInputWriter:$UseInheritedStandardInputWriter
}

function Invoke-HookFromSettings {
    param(
        [string]$ProjectRoot,
        [string]$SettingsPath,
        [string]$RawInput
    )

    $settings = Get-Content -LiteralPath $SettingsPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $commandHook = @($settings.hooks.PreToolUse)[0].hooks[0]
    $expandedArgs = @(
        @($commandHook.args) | ForEach-Object {
            ([string]$_).Replace('${CLAUDE_PROJECT_DIR}', $ProjectRoot)
        }
    )
    $arguments = @(
        $expandedArgs | ForEach-Object {
            '"' + ([string]$_).Replace('"', '\"') + '"'
        }
    ) -join ' '
    return Invoke-ProcessWithInput `
        -FileName ([string]$commandHook.command) `
        -Arguments $arguments `
        -InputText $RawInput
}

function Get-ChildPowerShellRuntime {
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $script:ChildPowerShellPath
    $startInfo.Arguments = '-NoLogo -NoProfile -NonInteractive -Command ' +
        '"$PSVersionTable.PSVersion.ToString(); $PSVersionTable.PSEdition; ' +
        '$ExecutionContext.SessionState.LanguageMode; $PSHOME"'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = New-Object System.Text.UTF8Encoding($false)
    $startInfo.StandardErrorEncoding = New-Object System.Text.UTF8Encoding($false)

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw 'PowerShell runtime probe could not be started'
    }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()

    return [pscustomobject]@{
        ExitCode = $process.ExitCode
        Lines = @($stdout -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        Stderr = $stderr
    }
}

function Test-DeniedCase {
    param(
        [string]$HookPath,
        [string]$Name,
        [string]$Command,
        [string]$ToolName = 'PowerShell'
    )

    $script:Cases++
    $script:DeniedCases++
    $result = Invoke-Hook -HookPath $HookPath -Command $Command -ToolName $ToolName
    $decision = $null
    $reason = ''
    try {
        $parsed = $result.Stdout | ConvertFrom-Json
        $decision = [string]$parsed.hookSpecificOutput.permissionDecision
        $reason = [string]$parsed.hookSpecificOutput.permissionDecisionReason
    }
    catch {
        # Assertions below report malformed or absent output.
    }

    Assert-True ($result.ExitCode -eq 0) "$Name denial hook exit code was $($result.ExitCode), expected 0"
    Assert-True ($decision -eq 'deny') "$Name was not denied"
    Assert-True ($reason.Contains('§10.7.13')) "$Name reason did not name §10.7.13"
    Assert-True ($reason.Contains('担当へ委譲')) "$Name reason did not tell Claude to delegate"
    Assert-True ([string]::IsNullOrWhiteSpace($result.Stderr)) "$Name unexpectedly failed open: $($result.Stderr.Trim())"
    $warning = if ([string]::IsNullOrWhiteSpace($result.Stderr)) { 'none' } else { $result.Stderr.Trim() }
    Write-Output ("DENY {0}: exit={1}, decision={2}, reason={3}, warning={4}" -f $Name, $result.ExitCode, $decision, $reason, $warning)
}

function Test-AllowedCase {
    param(
        [string]$HookPath,
        [string]$Name,
        [string]$Command,
        [string]$ToolName = 'PowerShell'
    )

    $script:Cases++
    $script:AllowedCases++
    $result = Invoke-Hook -HookPath $HookPath -Command $Command -ToolName $ToolName
    Assert-True ($result.ExitCode -eq 0) "$Name allow hook exit code was $($result.ExitCode), expected 0"
    Assert-True ([string]::IsNullOrWhiteSpace($result.Stdout)) "$Name unexpectedly returned a denial"
    Assert-True ([string]::IsNullOrWhiteSpace($result.Stderr)) "$Name unexpectedly failed open: $($result.Stderr.Trim())"
    $warning = if ([string]::IsNullOrWhiteSpace($result.Stderr)) { 'none' } else { $result.Stderr.Trim() }
    Write-Output ("ALLOW {0}: exit={1}, denial=none, warning={2}" -f $Name, $result.ExitCode, $warning)
}

function Test-WarningAllowedCase {
    param(
        [string]$HookPath,
        [string]$Name,
        [string]$Command,
        [string]$RawInput
    )

    $script:Cases++
    $script:AllowedCases++
    $script:ExpectedWarningCases++
    if ($PSBoundParameters.ContainsKey('RawInput')) {
        $result = Invoke-Hook -HookPath $HookPath -RawInput $RawInput
    }
    else {
        $result = Invoke-Hook -HookPath $HookPath -Command $Command
    }

    Assert-True ($result.ExitCode -eq 0) "$Name warning hook exit code was $($result.ExitCode), expected 0"
    Assert-True ([string]::IsNullOrWhiteSpace($result.Stdout)) "$Name unexpectedly returned a denial"
    Assert-True ($result.Stderr.Contains('ORIGAMI3_HOOK_FAIL_OPEN:')) "$Name warning did not contain the fail-open marker"
    Assert-True ($result.Stderr.Contains('判定できなかった')) "$Name warning did not say that the hook could not decide"
    Assert-True ($result.Stderr.Contains('止めずに通しました')) "$Name warning did not say that execution was allowed"
    Assert-True ($result.Stderr.Contains('手動で確認')) "$Name warning did not request a manual check"
    Write-Output ("ALLOW-WITH-WARNING {0}: exit={1}, denial=none, warning={2}" -f $Name, $result.ExitCode, $result.Stderr.Trim())
}

function Test-RawQuietAllowedCase {
    param(
        [string]$HookPath,
        [string]$Name,
        [AllowEmptyString()][string]$RawInput
    )

    $script:Cases++
    $script:AllowedCases++
    $result = Invoke-Hook -HookPath $HookPath -RawInput $RawInput
    Assert-True ($result.ExitCode -eq 0) "$Name hook exit code was $($result.ExitCode), expected 0"
    Assert-True ([string]::IsNullOrWhiteSpace($result.Stdout)) "$Name unexpectedly returned a denial"
    Assert-True ([string]::IsNullOrWhiteSpace($result.Stderr)) "$Name must be allowed without a warning: $($result.Stderr.Trim())"
    Write-Output ("ALLOW-QUIET {0}: exit={1}, denial=none, warning=none" -f $Name, $result.ExitCode)
}

function Test-RawDeniedCase {
    param(
        [string]$HookPath,
        [string]$Name,
        [string]$RawInput,
        [switch]$UseInheritedUnicodeWriter
    )

    $script:Cases++
    $script:DeniedCases++
    if ($UseInheritedUnicodeWriter) {
        $savedInputEncoding = [Console]::InputEncoding
        try {
            [Console]::InputEncoding = [Text.Encoding]::Unicode
            $result = Invoke-Hook `
                -HookPath $HookPath `
                -RawInput $RawInput `
                -UseInheritedStandardInputWriter
        }
        finally {
            [Console]::InputEncoding = $savedInputEncoding
        }
    }
    else {
        $result = Invoke-Hook -HookPath $HookPath -RawInput $RawInput
    }
    $decision = $null
    try {
        $decision = [string](($result.Stdout | ConvertFrom-Json).hookSpecificOutput.permissionDecision)
    }
    catch {
        # Assertions below report malformed or absent output.
    }
    Assert-True ($result.ExitCode -eq 0) "$Name denial hook exit code was $($result.ExitCode), expected 0"
    Assert-True ($decision -eq 'deny') "$Name was not denied"
    Assert-True ([string]::IsNullOrWhiteSpace($result.Stderr)) "$Name unexpectedly failed open: $($result.Stderr.Trim())"
    Write-Output ("DENY-RAW {0}: exit={1}, decision={2}, warning=none" -f $Name, $result.ExitCode, $decision)
}

if ([string]::IsNullOrWhiteSpace($SourceHookPath)) {
    $SourceHookPath = Join-Path $PSScriptRoot 'block-direct-cargo-npm.ps1'
}
if ([string]::IsNullOrWhiteSpace($SourceSettingsPath)) {
    $SourceSettingsPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'settings.json'
}
if ([string]::IsNullOrWhiteSpace($ChildPowerShellPath)) {
    $ChildPowerShellPath = Join-Path $PSHOME 'powershell.exe'
}
$sourceHook = [System.IO.Path]::GetFullPath($SourceHookPath)
$sourceSettings = [System.IO.Path]::GetFullPath($SourceSettingsPath)
$script:ChildPowerShellPath = [System.IO.Path]::GetFullPath($ChildPowerShellPath)
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("ori3-cargo-npm-hook-{0}" -f [Guid]::NewGuid().ToString('N'))
$tempRoot = [System.IO.Path]::GetFullPath($tempRoot)
$safeParent = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$originalTestInputEncoding = [Console]::InputEncoding
# Reproduce the main-worktree failure on every run. Normal hook writes must remain UTF-8
# even when the PowerShell process running this test inherited UTF-16LE console input.
[Console]::InputEncoding = [Text.Encoding]::Unicode

try {
    $runtime = Get-ChildPowerShellRuntime
    Assert-True ($runtime.ExitCode -eq 0) "PowerShell runtime probe exit code was $($runtime.ExitCode), expected 0"
    Assert-True ([string]::IsNullOrWhiteSpace($runtime.Stderr)) "PowerShell runtime probe wrote an error: $($runtime.Stderr.Trim())"
    Assert-True ($runtime.Lines.Count -eq 4) "PowerShell runtime probe returned $($runtime.Lines.Count) lines, expected 4"
    $runtimeText = @($runtime.Lines) -join ' / '
    Write-Output (
        "test environment: child={0}, runtime={1}, parentInputEncoding={2}, cwd={3}, CLAUDE_PROJECT_DIR={4}, hook={5}" -f
            $script:ChildPowerShellPath,
            $runtimeText,
            [Console]::InputEncoding.WebName,
            (Get-Location).Path,
            [string]$env:CLAUDE_PROJECT_DIR,
            $sourceHook
    )

    New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null
    $isolatedClaude = Join-Path $tempRoot '.claude'
    $isolatedHooks = Join-Path $isolatedClaude 'hooks'
    New-Item -ItemType Directory -Path $isolatedHooks -Force | Out-Null
    $isolatedHook = Join-Path $isolatedHooks 'block-direct-cargo-npm.ps1'
    $isolatedSettings = Join-Path $isolatedClaude 'settings.json'
    Copy-Item -LiteralPath $sourceHook -Destination $isolatedHook
    Copy-Item -LiteralPath $sourceSettings -Destination $isolatedSettings

    $settings = Get-Content -LiteralPath $isolatedSettings -Raw -Encoding UTF8 | ConvertFrom-Json
    $preToolUse = @($settings.hooks.PreToolUse)
    Assert-True ($preToolUse.Count -eq 1) 'settings must contain exactly one PreToolUse entry'
    Assert-True ($preToolUse[0].matcher -eq 'Bash|PowerShell') 'settings matcher must be Bash|PowerShell'
    Assert-True (@($preToolUse[0].hooks).Count -eq 1) 'settings must contain exactly one command hook'
    Assert-True ($preToolUse[0].hooks[0].command -eq 'powershell.exe') 'settings must invoke powershell.exe'
    Assert-True ((@($preToolUse[0].hooks[0].args) -join ' ').Contains('block-direct-cargo-npm.ps1')) 'settings must point at the policy hook'

    $encodedNpm = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('npm run build'))
    $denyCases = @(
        @{ Name = 'direct cargo'; Command = 'cargo test --workspace'; Tool = 'PowerShell' },
        @{ Name = 'direct npm'; Command = 'npm run build'; Tool = 'Bash' },
        @{ Name = 'cargo exe path'; Command = '& "C:\Users\example\.cargo\bin\cargo.exe" check'; Tool = 'PowerShell' },
        @{ Name = 'npm cmd path'; Command = '& "C:\Program Files\nodejs\npm.cmd" run test'; Tool = 'PowerShell' },
        @{ Name = 'cmd wrapper'; Command = 'cmd.exe /d /c "cargo clippy"'; Tool = 'PowerShell' },
        @{ Name = 'PowerShell command wrapper'; Command = 'powershell.exe -NoProfile -Command "npm run lint"'; Tool = 'PowerShell' },
        @{ Name = 'PowerShell encoded wrapper'; Command = "powershell.exe -EncodedCommand $encodedNpm"; Tool = 'PowerShell' },
        @{ Name = 'bash wrapper'; Command = 'bash -lc "cargo test"'; Tool = 'Bash' },
        @{ Name = 'Start-Process wrapper'; Command = 'Start-Process -FilePath cargo.exe -ArgumentList test'; Tool = 'PowerShell' },
        @{ Name = 'rustup wrapper'; Command = 'rustup run stable cargo test'; Tool = 'PowerShell' },
        @{ Name = 'mixed command'; Command = 'git status; cargo test'; Tool = 'PowerShell' }
    )

    foreach ($case in $denyCases) {
        Test-DeniedCase -HookPath $isolatedHook -Name $case.Name -Command $case.Command -ToolName $case.Tool
    }

    $allowCases = @(
        @{ Name = 'check script'; Command = 'powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1'; Tool = 'PowerShell' },
        @{ Name = 'CI script'; Command = 'powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check-ci.ps1'; Tool = 'PowerShell' },
        @{ Name = 'release script'; Command = 'powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check-release-ready.ps1'; Tool = 'PowerShell' },
        @{ Name = 'git status'; Command = 'git status --short'; Tool = 'PowerShell' },
        @{ Name = 'git diff'; Command = 'git diff -- Cargo.toml'; Tool = 'Bash' },
        @{ Name = 'search word'; Command = 'rg cargo scripts'; Tool = 'PowerShell' },
        @{ Name = 'read manifest'; Command = 'Get-Content Cargo.toml'; Tool = 'PowerShell' },
        @{ Name = 'display word'; Command = "Write-Output 'cargo test; npm test'"; Tool = 'PowerShell' },
        @{ Name = 'command wrapper around allowed script'; Command = 'powershell -Command "& scripts/check.ps1"'; Tool = 'PowerShell' }
    )

    foreach ($case in $allowCases) {
        Test-AllowedCase -HookPath $isolatedHook -Name $case.Name -Command $case.Command -ToolName $case.Tool
    }

    $script:Cases++
    $script:AllowedCases++
    $wrongTool = Invoke-Hook -HookPath $isolatedHook -Command 'cargo test' -ToolName 'Read'
    Assert-True ($wrongTool.ExitCode -eq 0) 'non-shell tool must exit 0'
    Assert-True ([string]::IsNullOrWhiteSpace($wrongTool.Stdout)) 'non-shell tool must not be denied'
    Assert-True ([string]::IsNullOrWhiteSpace($wrongTool.Stderr)) "non-shell tool unexpectedly failed open: $($wrongTool.Stderr.Trim())"
    Write-Output ("ALLOW non-shell tool: exit={0}, denial=none, warning=none" -f $wrongTool.ExitCode)

    Test-RawQuietAllowedCase -HookPath $isolatedHook -Name 'empty stdin' -RawInput ''
    Test-RawQuietAllowedCase -HookPath $isolatedHook -Name 'newline-only stdin' -RawInput "`r`n"
    Test-RawQuietAllowedCase -HookPath $isolatedHook -Name 'whitespace-only stdin' -RawInput "`t `n"
    Test-RawQuietAllowedCase -HookPath $isolatedHook -Name 'BOM-only stdin' -RawInput ([string][char]0xFEFF)

    $cargoInputObject = [ordered]@{
        hook_event_name = 'PreToolUse'
        tool_name = 'PowerShell'
        tool_input = [ordered]@{ command = 'cargo test --workspace' }
    }
    $compactCargoInput = $cargoInputObject | ConvertTo-Json -Compress -Depth 4
    $prettyCargoInput = $cargoInputObject | ConvertTo-Json -Depth 4
    Test-RawDeniedCase -HookPath $isolatedHook -Name 'multiline JSON stdin' -RawInput $prettyCargoInput
    Test-RawDeniedCase -HookPath $isolatedHook -Name 'UTF-8 BOM plus compact JSON stdin' -RawInput (([string][char]0xFEFF) + $compactCargoInput)
    Test-RawDeniedCase -HookPath $isolatedHook -Name 'UTF-8 BOM plus multiline JSON stdin' -RawInput (([string][char]0xFEFF) + $prettyCargoInput)
    Test-RawDeniedCase `
        -HookPath $isolatedHook `
        -Name 'inherited UTF-16LE writer JSON stdin' `
        -RawInput $compactCargoInput `
        -UseInheritedUnicodeWriter

    $script:Cases++
    $script:DeniedCases++
    $settingsResult = Invoke-HookFromSettings -ProjectRoot $tempRoot -SettingsPath $isolatedSettings -RawInput $prettyCargoInput
    $settingsDecision = $null
    try {
        $settingsDecision = [string](($settingsResult.Stdout | ConvertFrom-Json).hookSpecificOutput.permissionDecision)
    }
    catch {
        # Assertions below report malformed or absent output.
    }
    Assert-True ($settingsResult.ExitCode -eq 0) "settings invocation exit code was $($settingsResult.ExitCode), expected 0"
    Assert-True ($settingsDecision -eq 'deny') 'settings invocation did not deny direct cargo'
    Assert-True ([string]::IsNullOrWhiteSpace($settingsResult.Stderr)) "settings invocation unexpectedly failed open: $($settingsResult.Stderr.Trim())"
    Write-Output ("DENY-SETTINGS direct cargo: exit={0}, decision={1}, warning=none" -f $settingsResult.ExitCode, $settingsDecision)

    $script:Cases++
    $script:AllowedCases++
    $settingsEmpty = Invoke-HookFromSettings -ProjectRoot $tempRoot -SettingsPath $isolatedSettings -RawInput ''
    Assert-True ($settingsEmpty.ExitCode -eq 0) "empty settings invocation exit code was $($settingsEmpty.ExitCode), expected 0"
    Assert-True ([string]::IsNullOrWhiteSpace($settingsEmpty.Stdout)) 'empty settings invocation unexpectedly returned a denial'
    Assert-True ([string]::IsNullOrWhiteSpace($settingsEmpty.Stderr)) "empty settings invocation must be quiet: $($settingsEmpty.Stderr.Trim())"
    Write-Output ("ALLOW-SETTINGS empty stdin: exit={0}, denial=none, warning=none" -f $settingsEmpty.ExitCode)

    Test-WarningAllowedCase -HookPath $isolatedHook -Name 'invalid JSON' -RawInput '{not-json'

    $deepCommand = 'command command command command command command command command command command Write-Output safe'
    Test-WarningAllowedCase -HookPath $isolatedHook -Name 'nested shell depth limit' -Command $deepCommand
}
finally {
    try {
        if (Test-Path -LiteralPath $tempRoot) {
            $resolvedTemp = [System.IO.Path]::GetFullPath($tempRoot)
            if (-not $resolvedTemp.StartsWith($safeParent, [StringComparison]::OrdinalIgnoreCase)) {
                throw "refusing to remove unexpected test path: $resolvedTemp"
            }
            Remove-Item -LiteralPath $resolvedTemp -Recurse -Force
        }
    }
    finally {
        [Console]::InputEncoding = $originalTestInputEncoding
    }
}

Assert-True ($script:ObservedWarningCases -eq $script:ExpectedWarningCases) (
    "unexpected fail-open count: observed {0}, expected {1}" -f
        $script:ObservedWarningCases,
        $script:ExpectedWarningCases
)
$unexpectedWarningCases = $script:ObservedWarningCases - $script:ExpectedWarningCases
Write-Output ("test result: {0} cases, {1} assertions, {2} failures" -f $script:Cases, $script:Assertions, $script:Failures)
Write-Output ("decision result: DENY {0}, ALLOW {1}, warnings {2}" -f $script:DeniedCases, $script:AllowedCases, $script:ObservedWarningCases)
Write-Output ("fail-open result: unexpected {0}, expected fault warnings {1}" -f $unexpectedWarningCases, $script:ExpectedWarningCases)
if ($script:Failures -ne 0) {
    exit 1
}
exit 0
