[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$ScriptPath = Join-Path $PSScriptRoot "hook-health.ps1"
$PowerShellPath = (Get-Process -Id $PID).Path
$TempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\\/")
$Sandbox = Join-Path $TempBase ("ori3-hook-health-test-{0}" -f [Guid]::NewGuid().ToString("N"))
$Repository = Join-Path $Sandbox "repo"
$MissingRepository = Join-Path $Sandbox "missing"
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

function Invoke-Health {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $PowerShellPath
    $startInfo.Arguments = ConvertTo-ProcessArgumentString -Values (@(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $ScriptPath
    ) + $Arguments)
    $startInfo.WorkingDirectory = $Sandbox
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [Text.Encoding]::UTF8
    $startInfo.StandardErrorEncoding = [Text.Encoding]::UTF8
    $process = [Diagnostics.Process]::Start($startInfo)
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    return [PSCustomObject]@{ ExitCode = $process.ExitCode; Output = $stdout + $stderr }
}

function Get-HealthPaths {
    $normalized = [IO.Path]::GetFullPath($Repository).Replace("\", "/").TrimEnd("/").ToLowerInvariant()
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $key = -join ($sha256.ComputeHash([Text.Encoding]::UTF8.GetBytes($normalized)) | ForEach-Object { $_.ToString("x2") })
    }
    finally {
        $sha256.Dispose()
    }
    $healthRoot = Join-Path ([IO.Path]::GetTempPath()) "ori3-hook-health"
    $statePath = Join-Path $healthRoot ("{0}-test-claim-scope-warning.json" -f $key)
    return [PSCustomObject]@{ State = $statePath; Block = ($statePath + ".block") }
}

function Remove-TestSandbox {
    $healthPaths = Get-HealthPaths
    foreach ($path in @($healthPaths.State, $healthPaths.Block)) {
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            Remove-Item -LiteralPath $path -Force
        }
    }
    if (-not (Test-Path -LiteralPath $Sandbox)) { return }
    $fullSandbox = [IO.Path]::GetFullPath($Sandbox).TrimEnd([char[]]"\\/")
    if ([IO.Path]::GetDirectoryName($fullSandbox) -ne $TempBase -or [IO.Path]::GetFileName($fullSandbox) -notmatch '^ori3-hook-health-test-[0-9a-f]{32}$') {
        throw "Refusing unsafe self-test cleanup: $fullSandbox"
    }
    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

[void][IO.Directory]::CreateDirectory($Repository)
try {
    Write-Host "[1/5] missing state is a clean bootstrap condition"
    $missing = Invoke-Health @("-Action", "Check", "-RepositoryRoot", $MissingRepository)
    Assert-Equal $missing.ExitCode 0 "missing state must not deny a first instruction" $missing.Output
    Assert-Contains $missing.Output "state=missing" "missing state must be visible"

    Write-Host "[2/5] the first failed scan records one consecutive failure without blocking"
    $firstFailure = Invoke-Health @("-Action", "RecordFailure", "-RepositoryRoot", $Repository, "-FailureExitCode", "9", "-FailureKind", "scanner-exit")
    Assert-Equal $firstFailure.ExitCode 0 "recording a first failure must remain nonblocking" $firstFailure.Output
    Assert-Contains $firstFailure.Output "HOOK_HEALTH_DEGRADED" "failure output needs the stable identifier"
    Assert-Contains $firstFailure.Output "failures=1" "first failure count must be one"
    $firstCheck = Invoke-Health @("-Action", "Check", "-RepositoryRoot", $Repository, "-Threshold", "2")
    Assert-Equal $firstCheck.ExitCode 0 "one failure must remain below threshold" $firstCheck.Output
    Assert-Contains $firstCheck.Output "failures=1" "health check must expose the first failure count"

    Write-Host "[3/5] the second consecutive failed scan degrades health and gives an explicit escape path"
    $secondFailure = Invoke-Health @("-Action", "RecordFailure", "-RepositoryRoot", $Repository, "-FailureExitCode", "127", "-FailureKind", "powershell-unavailable")
    Assert-Equal $secondFailure.ExitCode 0 "recording a second failure must not block the commit" $secondFailure.Output
    Assert-Contains $secondFailure.Output "failures=2" "second failure count must be two"
    $degraded = Invoke-Health @("-Action", "Check", "-RepositoryRoot", $Repository, "-Threshold", "2")
    Assert-Equal $degraded.ExitCode 2 "two failures must be a degraded health result" $degraded.Output
    Assert-Contains $degraded.Output "check=test-claim-scope-warning" "degraded output must name the check"
    Assert-Contains $degraded.Output "lastSuccess=never" "degraded output must expose the last successful scan"
    $healthPaths = Get-HealthPaths
    Assert-Contains $degraded.Output ("acknowledgement=" + $healthPaths.Block) "degraded output must give the exact acknowledgement file to delete"
    Assert-Contains $degraded.Output ("instruction=delete-file-at-" + $healthPaths.Block) "degraded output must state the deletion instruction and exact path"
    Assert-Equal (Test-Path -LiteralPath $healthPaths.Block -PathType Leaf) $true "second failure must issue an acknowledgement file"

    Write-Host "[4/5] deleting the acknowledgement records a visible release until the next successful scan"
    Remove-Item -LiteralPath $healthPaths.Block -Force
    $released = Invoke-Health @("-Action", "Check", "-RepositoryRoot", $Repository, "-Threshold", "2")
    Assert-Equal $released.ExitCode 0 "an intentional acknowledgement deletion must allow the instruction path" $released.Output
    Assert-Contains $released.Output "HOOK_HEALTH_RELEASED" "release must stay visible"
    Assert-Contains $released.Output "releasedFailures=2" "release must record its failure count"
    $releasedState = [IO.File]::ReadAllText($healthPaths.State, [Text.UTF8Encoding]::new($false)) | ConvertFrom-Json
    Assert-Equal ([bool]$releasedState.releasePendingSuccess) $true "state must retain that release awaits a success"
    Assert-Equal ([int]$releasedState.releaseFailureCount) 2 "state must retain the failure count at release"
    Assert-Equal ([string]::IsNullOrWhiteSpace([string]$releasedState.releaseAcknowledgedAtUtc)) $false "state must retain the release time"
    $releasedAgain = Invoke-Health @("-Action", "Check", "-RepositoryRoot", $Repository, "-Threshold", "2")
    Assert-Contains $releasedAgain.Output "HOOK_HEALTH_RELEASED" "release must not become silent before a success"

    $thirdFailure = Invoke-Health @("-Action", "RecordFailure", "-RepositoryRoot", $Repository, "-FailureExitCode", "9", "-FailureKind", "scanner-exit")
    Assert-Equal $thirdFailure.ExitCode 0 "a later failed scan still records without blocking the commit" $thirdFailure.Output
    $blockedAgain = Invoke-Health @("-Action", "Check", "-RepositoryRoot", $Repository, "-Threshold", "2")
    Assert-Equal $blockedAgain.ExitCode 2 "a new failure after release must block again" $blockedAgain.Output
    Assert-Contains $blockedAgain.Output "failures=3" "reblocked health must retain all consecutive failures"

    Write-Host "[5/5] one successful scan clears the consecutive failure and release state"
    $success = Invoke-Health @("-Action", "RecordSuccess", "-RepositoryRoot", $Repository)
    Assert-Equal $success.ExitCode 0 "success recording must complete" $success.Output
    Assert-Contains $success.Output "failures=0" "success must reset the count"
    $healthy = Invoke-Health @("-Action", "Check", "-RepositoryRoot", $Repository, "-Threshold", "2")
    Assert-Equal $healthy.ExitCode 0 "one successful scan must restore health" $healthy.Output
    Assert-Contains $healthy.Output "failures=0" "restored health must expose zero failures"
    Assert-Equal ($healthy.Output.Contains("HOOK_HEALTH_RELEASED")) $false "success must clear the visible release state"

    Write-Host "[EVIDENCE] missing=$($missing.ExitCode); first=$($firstCheck.ExitCode); degraded=$($degraded.ExitCode); released=$($released.ExitCode); reblocked=$($blockedAgain.ExitCode); restored=$($healthy.ExitCode)"
    Write-Host "hook-health self-test passed: $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
