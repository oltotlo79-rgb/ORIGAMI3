[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$RequireScriptPath = Join-Path $PSScriptRoot "require-report-log.ps1"
$HealthScriptPath = Join-Path $PSScriptRoot "checks\hook-health.ps1"
$PowerShellPath = (Get-Process -Id $PID).Path
$TempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\\/")
$Sandbox = Join-Path $TempBase ("ori3-require-report-log-test-{0}" -f [Guid]::NewGuid().ToString("N"))
$Repository = Join-Path $Sandbox "repo"
$script:AssertionCount = 0

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

function ConvertTo-ProcessArgumentString {
    param([Parameter(Mandatory = $true)][string[]]$Values)

    $parts = foreach ($value in $Values) {
        $escaped = [regex]::Replace($value, '(\\*)"', '$1$1\\"')
        $trailingBackslashes = [regex]::Match($escaped, '\\*$').Value
        $escaped = $escaped + $trailingBackslashes
        '"' + $escaped + '"'
    }
    return ($parts -join " ")
}

function Invoke-ChildPowerShell {
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [hashtable]$EnvironmentVariables = @{},
        [AllowNull()][string]$StandardInput = $null
    )

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $PowerShellPath
    $startInfo.Arguments = ConvertTo-ProcessArgumentString -Values $Arguments
    $startInfo.WorkingDirectory = $Repository
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.RedirectStandardInput = $null -ne $StandardInput
    $startInfo.StandardOutputEncoding = [Text.Encoding]::UTF8
    $startInfo.StandardErrorEncoding = [Text.Encoding]::UTF8
    foreach ($key in $EnvironmentVariables.Keys) {
        $startInfo.EnvironmentVariables[[string]$key] = [string]$EnvironmentVariables[$key]
    }
    $process = [Diagnostics.Process]::Start($startInfo)
    if ($null -ne $StandardInput) {
        $process.StandardInput.Write($StandardInput)
        $process.StandardInput.Close()
    }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    return [PSCustomObject]@{ ExitCode = $process.ExitCode; Output = $stdout + $stderr }
}

function Invoke-Health {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)

    return Invoke-ChildPowerShell -Arguments (@(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", (Join-Path $Repository "scripts\hooks\checks\hook-health.ps1")
    ) + $Arguments)
}

function Invoke-RequireReport {
    return Invoke-ChildPowerShell -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", (Join-Path $Repository "scripts\hooks\require-report-log.ps1")
    ) -EnvironmentVariables @{ CLAUDE_PROJECT_DIR = $Repository } -StandardInput '{"tool_name":"Agent"}'
}

function Get-HealthStatePaths {
    $normalized = [IO.Path]::GetFullPath($Repository).Replace("\", "/").TrimEnd("/").ToLowerInvariant()
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $key = -join ($sha256.ComputeHash([Text.Encoding]::UTF8.GetBytes($normalized)) | ForEach-Object { $_.ToString("x2") })
    }
    finally {
        $sha256.Dispose()
    }
    $statePath = Join-Path (Join-Path ([IO.Path]::GetTempPath()) "ori3-hook-health") ("{0}-test-claim-scope-warning.json" -f $key)
    return [PSCustomObject]@{ State = $statePath; Block = ($statePath + ".block") }
}

function Remove-TestArtifacts {
    $healthPaths = Get-HealthStatePaths
    foreach ($path in @($healthPaths.State, $healthPaths.Block)) {
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            Remove-Item -LiteralPath $path -Force
        }
    }
    if (-not (Test-Path -LiteralPath $Sandbox)) { return }
    $fullSandbox = [IO.Path]::GetFullPath($Sandbox).TrimEnd([char[]]"\\/")
    if ([IO.Path]::GetDirectoryName($fullSandbox) -ne $TempBase -or [IO.Path]::GetFileName($fullSandbox) -notmatch '^ori3-require-report-log-test-[0-9a-f]{32}$') {
        throw "Refusing unsafe self-test cleanup: $fullSandbox"
    }
    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

[void][IO.Directory]::CreateDirectory((Join-Path $Repository "scripts\hooks\checks"))
[void][IO.Directory]::CreateDirectory((Join-Path $Repository "docs"))
try {
    [IO.File]::Copy($RequireScriptPath, (Join-Path $Repository "scripts\hooks\require-report-log.ps1"), $true)
    [IO.File]::Copy($HealthScriptPath, (Join-Path $Repository "scripts\hooks\checks\hook-health.ps1"), $true)
    $reportName = ([string][char]0x5831) + ([string][char]0x544A) + ([string][char]0x8A18) + ([string][char]0x9332) + ".md"
    [IO.File]::WriteAllText((Join-Path $Repository (Join-Path "docs" $reportName)), "# current`n", [Text.UTF8Encoding]::new($false))

    Write-Host "[1/4] missing health state allows a first instruction"
    $missing = Invoke-RequireReport
    Assert-Equal $missing.ExitCode 0 "PreToolUse hook must complete" $missing.Output
    Assert-Equal $missing.Output.Trim() "" "missing health state must not deny the first instruction" $missing.Output

    Write-Host "[2/4] two failed scans deny the next instruction with actionable state"
    $first = Invoke-Health @("-Action", "RecordFailure", "-RepositoryRoot", $Repository, "-FailureExitCode", "9", "-FailureKind", "scanner-exit")
    Assert-Equal $first.ExitCode 0 "first health failure recording must complete" $first.Output
    $second = Invoke-Health @("-Action", "RecordFailure", "-RepositoryRoot", $Repository, "-FailureExitCode", "127", "-FailureKind", "powershell-unavailable")
    Assert-Equal $second.ExitCode 0 "second health failure recording must complete" $second.Output
    $denied = Invoke-RequireReport
    Assert-Equal $denied.ExitCode 0 "PreToolUse denial must be a successful hook response" $denied.Output
    $denial = $denied.Output | ConvertFrom-Json
    Assert-Equal $denial.hookSpecificOutput.permissionDecision "deny" "two failures must deny the next instruction" $denied.Output
    Assert-Contains $denial.hookSpecificOutput.permissionDecisionReason "check=test-claim-scope-warning" "denial must name the failed check"
    Assert-Contains $denial.hookSpecificOutput.permissionDecisionReason "failures=2" "denial must show the consecutive failure count"
    Assert-Contains $denial.hookSpecificOutput.permissionDecisionReason "lastSuccess=never" "denial must show the last successful scan"
    $healthPaths = Get-HealthStatePaths
    Assert-Contains $denial.hookSpecificOutput.permissionDecisionReason $healthPaths.Block "denial must give the exact acknowledgement file to delete"
    Assert-Contains $denial.hookSpecificOutput.permissionDecisionReason ("instruction=delete-file-at-" + $healthPaths.Block) "denial must state the exact deletion instruction"

    Write-Host "[3/4] an explicit acknowledgement release permits delegation but remains visible"
    Remove-Item -LiteralPath $healthPaths.Block -Force
    $released = Invoke-RequireReport
    Assert-Equal $released.ExitCode 0 "released PreToolUse hook must complete" $released.Output
    Assert-Contains $released.Output "HOOK_HEALTH_RELEASED" "released instruction path must display the release state"
    Assert-Equal ($released.Output.Contains('"permissionDecision":"deny"')) $false "explicit release must allow delegation" $released.Output

    $third = Invoke-Health @("-Action", "RecordFailure", "-RepositoryRoot", $Repository, "-FailureExitCode", "9", "-FailureKind", "scanner-exit")
    Assert-Equal $third.ExitCode 0 "a post-release failure must be recordable" $third.Output
    $deniedAgain = Invoke-RequireReport
    Assert-Equal $deniedAgain.ExitCode 0 "reblocked PreToolUse hook must complete" $deniedAgain.Output
    $denialAgain = $deniedAgain.Output | ConvertFrom-Json
    Assert-Equal $denialAgain.hookSpecificOutput.permissionDecision "deny" "a new failed scan after release must deny delegation again" $deniedAgain.Output

    Write-Host "[4/4] one successful scan restores the instruction path and clears release disclosure"
    $success = Invoke-Health @("-Action", "RecordSuccess", "-RepositoryRoot", $Repository)
    Assert-Equal $success.ExitCode 0 "health success recording must complete" $success.Output
    $restored = Invoke-RequireReport
    Assert-Equal $restored.ExitCode 0 "restored PreToolUse hook must complete" $restored.Output
    Assert-Equal $restored.Output.Trim() "" "one successful scan must restore the instruction path" $restored.Output

    Write-Host "[EVIDENCE] missing=$($missing.ExitCode); denied=$($denied.ExitCode); released=$($released.ExitCode); reblocked=$($deniedAgain.ExitCode); restored=$($restored.ExitCode)"
    Write-Host "require-report-log self-test passed: $script:AssertionCount assertions"
}
finally {
    Remove-TestArtifacts
}
