[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$sourceScript = Join-Path $PSScriptRoot "no-prohibited-doc.ps1"
$tempBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-no-prohibited-doc-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [System.IO.Path]::GetFullPath((Join-Path $tempBase $sandboxName))
$sandboxScript = Join-Path $sandboxRoot "no-prohibited-doc.ps1"
$script:AssertionCount = 0
$script:InvocationCount = 0

function Assert-ExitCode {
    param(
        [Parameter(Mandatory = $true)][int]$Actual,
        [Parameter(Mandatory = $true)][int]$Expected,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$Output = ""
    )
    $script:AssertionCount += 1
    if ($Actual -ne $Expected) {
        throw "ASSERTION FAILED: $Message (expected=$Expected, actual=$Actual)`n$Output"
    }
}

function Invoke-IsolatedCheck {
    param(
        [Parameter(Mandatory = $true)][string]$PowerShellPath,
        [string[]]$Paths = @()
    )
    $script:InvocationCount += 1
    $stdoutPath = Join-Path $sandboxRoot ("stdout-{0}.txt" -f $script:InvocationCount)
    $stderrPath = Join-Path $sandboxRoot ("stderr-{0}.txt" -f $script:InvocationCount)
    $arguments = @("-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $sandboxScript) + @($Paths)
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        & $PowerShellPath @arguments 1>$stdoutPath 2>$stderrPath
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    $output = @()
    if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) { $output += [System.IO.File]::ReadAllText($stdoutPath) }
    if (Test-Path -LiteralPath $stderrPath -PathType Leaf) { $output += [System.IO.File]::ReadAllText($stderrPath) }
    [pscustomobject]@{ ExitCode = $exitCode; Output = ($output -join "`n") }
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $sandboxRoot)) { return }
    $resolved = [System.IO.Path]::GetFullPath($sandboxRoot).TrimEnd([char[]]"\/")
    $parent = [System.IO.Path]::GetDirectoryName($resolved)
    $leaf = [System.IO.Path]::GetFileName($resolved)
    if (($parent -ne $tempBase) -or
        (-not [regex]::IsMatch($leaf, '^ori3-no-prohibited-doc-test-[0-9a-f]{32}$', [System.Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
        throw "Refusing unsafe self-test cleanup: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

if (-not (Test-Path -LiteralPath $sourceScript -PathType Leaf)) {
    throw "Required implementation is missing: $sourceScript"
}
$powerShellCommand = Get-Command powershell.exe, pwsh.exe, pwsh -ErrorAction SilentlyContinue | Select-Object -First 1
if ($null -eq $powerShellCommand) {
    throw "PowerShell executable is required for the isolated self-test"
}

[void][System.IO.Directory]::CreateDirectory($sandboxRoot)
try {
    [System.IO.File]::Copy($sourceScript, $sandboxScript, $true)

    Write-Output "[1/4] an unrelated document passes"
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -Paths @("docs/normal.md")
    Assert-ExitCode $result.ExitCode 0 "unrelated document must pass" $result.Output

    Write-Output "[2/4] the same leaf name below another directory passes"
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -Paths @("archive/docs/competitive-review-2026-08-20.md")
    Assert-ExitCode $result.ExitCode 0 "only the exact repository path is prohibited" $result.Output

    Write-Output "[3/4] the exact prohibited path is rejected"
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -Paths @("docs/competitive-review-2026-08-20.md")
    Assert-ExitCode $result.ExitCode 1 "exact prohibited path must be rejected" $result.Output

    Write-Output "[4/4] a Windows-style prohibited path is rejected"
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -Paths @(".\docs\competitive-review-2026-08-20.md")
    Assert-ExitCode $result.ExitCode 1 "normalized Windows path must be rejected" $result.Output

    Write-Output "no-prohibited-doc self-test passed: 4 cases, $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
