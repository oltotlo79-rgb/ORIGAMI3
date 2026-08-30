[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$ScriptPath = Join-Path $PSScriptRoot "check-report-log.ps1"
$PowerShellPath = (Get-Process -Id $PID).Path
$TempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\\/")
$SandboxName = "ori3-check-report-log-test-{0}" -f [Guid]::NewGuid().ToString("N")
$SandboxRoot = [IO.Path]::GetFullPath((Join-Path $TempRoot $SandboxName))
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
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Values)

    $parts = foreach ($value in $Values) {
        $escaped = [regex]::Replace($value, '(\\*)"', '$1$1\\"')
        $trailingBackslashes = [regex]::Match($escaped, '\\*$').Value
        $escaped = $escaped + $trailingBackslashes
        '"' + $escaped + '"'
    }
    return ($parts -join " ")
}

function Invoke-ReportCheck {
    param([Parameter(Mandatory = $true)][string]$FixtureScriptPath)

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $PowerShellPath
    $startInfo.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}"' -f $FixtureScriptPath
    $startInfo.WorkingDirectory = $PSScriptRoot
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

function Write-TestReport {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][DateTime]$HeadingTime,
        [Parameter(Mandatory = $true)][DateTime]$FileTime
    )

    $heading = "## {0} {1} {2} time test" -f $HeadingTime.ToString("yyyy-MM-dd"), $HeadingTime.ToString("HH:mm"), [char]0x2014
    [IO.File]::WriteAllText($Path, $heading + "`n`nBody.`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::SetLastWriteTime($Path, $FileTime)
}

function New-ReportFixture {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][DateTime]$HeadingTime,
        [Parameter(Mandatory = $true)][DateTime]$FileTime
    )

    $root = Join-Path $SandboxRoot $Name
    [void][IO.Directory]::CreateDirectory((Join-Path $root "scripts"))
    [void][IO.Directory]::CreateDirectory((Join-Path $root "docs"))
    Copy-Item -LiteralPath $ScriptPath -Destination (Join-Path $root "scripts\check-report-log.ps1") -Force
    $reportName = ([string][char]0x5831) + ([string][char]0x544A) + ([string][char]0x8A18) + ([string][char]0x9332) + ".md"
    $reportPath = Join-Path (Join-Path $root "docs") $reportName
    Write-TestReport -Path $reportPath -HeadingTime $HeadingTime -FileTime $FileTime
    $unused = @(& git init --quiet $root)
    if ($LASTEXITCODE -ne 0) { throw "git init failed for report-log fixture" }
    $emptyIgnore = Join-Path $root "empty-global-ignore"
    [IO.File]::WriteAllText($emptyIgnore, "", [Text.UTF8Encoding]::new($false))
    $unused = @(& git -C $root config core.excludesFile $emptyIgnore)
    if ($LASTEXITCODE -ne 0) { throw "git config core.excludesFile failed for report-log fixture" }
    [void][IO.Directory]::CreateDirectory((Join-Path $root "apps"))
    [IO.File]::WriteAllText((Join-Path $root "apps\source.txt"), "fixture", [Text.UTF8Encoding]::new($false))
    $unused = @(& git -C $root config user.email "report-log-test@example.invalid")
    if ($LASTEXITCODE -ne 0) { throw "git config user.email failed for report-log fixture" }
    $unused = @(& git -C $root config user.name "Report Log Test")
    if ($LASTEXITCODE -ne 0) { throw "git config user.name failed for report-log fixture" }
    $unused = @(& git -C $root add -- apps)
    if ($LASTEXITCODE -ne 0) { throw "git add failed for report-log fixture" }
    $unused = @(& git -C $root commit --quiet -m "fixture baseline")
    if ($LASTEXITCODE -ne 0) { throw "git commit failed for report-log fixture" }
    return Join-Path $root "scripts\check-report-log.ps1"
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $SandboxRoot)) { return }
    $fullSandbox = [IO.Path]::GetFullPath($SandboxRoot).TrimEnd([char[]]"\\/")
    if ([IO.Path]::GetDirectoryName($fullSandbox) -ne $TempRoot -or [IO.Path]::GetFileName($fullSandbox) -notmatch '^ori3-check-report-log-test-[0-9a-f]{32}$') {
        throw "Refusing unsafe self-test cleanup: $fullSandbox"
    }
    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

[void][IO.Directory]::CreateDirectory($SandboxRoot)

try {
    $now = Get-Date

    Write-Host "[1/2] heading earlier than file update time passes"
    $validScript = New-ReportFixture -Name "valid" -HeadingTime $now.AddMinutes(-2) -FileTime $now
    $validResult = Invoke-ReportCheck -FixtureScriptPath $validScript
    Assert-Equal $validResult.ExitCode 0 "valid report check must exit 0 in a new process" $validResult.Output

    Write-Host "[2/2] heading later than file update time fails"
    $futureScript = New-ReportFixture -Name "future" -HeadingTime $now.AddDays(1) -FileTime $now
    $futureResult = Invoke-ReportCheck -FixtureScriptPath $futureScript
    Assert-Equal $futureResult.ExitCode 2 "future heading must be a format failure in a new process" $futureResult.Output
    Assert-Contains $futureResult.Output "later than the file update time" "future heading diagnostic must explain the violation"

    Write-Host "[EVIDENCE] valid check exit=$($validResult.ExitCode); future heading check exit=$($futureResult.ExitCode)"
    Write-Host "check-report-log self-test passed: $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
