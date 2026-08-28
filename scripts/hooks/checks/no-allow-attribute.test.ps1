[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$sourceScript = Join-Path $PSScriptRoot "no-allow-attribute.ps1"
$tempBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-no-allow-attribute-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [System.IO.Path]::GetFullPath((Join-Path $tempBase $sandboxName))
$sandboxScript = Join-Path $sandboxRoot "no-allow-attribute.ps1"
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

function Invoke-TestGit {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        & git -c core.excludesFile=NUL -C $Repository @Arguments 1>$null 2>$null
        $gitExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($gitExitCode -ne 0) {
        throw "git failed in isolated fixture: git $($Arguments -join ' ') (exit=$gitExitCode)"
    }
}

function Invoke-IsolatedCheck {
    param(
        [Parameter(Mandatory = $true)][string]$PowerShellPath,
        [Parameter(Mandatory = $true)][string]$Repository,
        [string[]]$Paths = @()
    )

    $script:InvocationCount += 1
    $stdoutPath = Join-Path $sandboxRoot ("stdout-{0}.txt" -f $script:InvocationCount)
    $stderrPath = Join-Path $sandboxRoot ("stderr-{0}.txt" -f $script:InvocationCount)
    $arguments = @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $sandboxScript, "-RepositoryRoot", $Repository
    ) + @($Paths)
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
    if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) {
        $output += [System.IO.File]::ReadAllText($stdoutPath)
    }
    if (Test-Path -LiteralPath $stderrPath -PathType Leaf) {
        $output += [System.IO.File]::ReadAllText($stderrPath)
    }
    return [pscustomobject]@{ ExitCode = $exitCode; Output = ($output -join "`n") }
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $sandboxRoot)) {
        return
    }
    $resolved = [System.IO.Path]::GetFullPath($sandboxRoot).TrimEnd([char[]]"\/")
    $parent = [System.IO.Path]::GetDirectoryName($resolved)
    $leaf = [System.IO.Path]::GetFileName($resolved)
    if (($parent -ne $tempBase) -or
        (-not [regex]::IsMatch($leaf, '^ori3-no-allow-attribute-test-[0-9a-f]{32}$', [System.Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
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
    $repository = Join-Path $sandboxRoot "repo"
    $sourcePath = Join-Path $repository "src/lib.rs"
    [void][System.IO.Directory]::CreateDirectory((Split-Path -Parent $sourcePath))
    Invoke-TestGit -Repository $repository -Arguments @("init", "--quiet")
    Invoke-TestGit -Repository $repository -Arguments @("config", "user.name", "ORIGAMI3 Test")
    Invoke-TestGit -Repository $repository -Arguments @("config", "user.email", "test@example.invalid")
    [System.IO.File]::WriteAllText($sourcePath, "#[allow(dead_code)]`npub fn legacy() {}`n", [System.Text.UTF8Encoding]::new($false))
    Invoke-TestGit -Repository $repository -Arguments @("add", "--", "src/lib.rs")
    Invoke-TestGit -Repository $repository -Arguments @("commit", "--quiet", "--no-verify", "-m", "baseline")

    Write-Output "[1/3] an empty target list passes"
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -Repository $repository
    Assert-ExitCode $result.ExitCode 0 "empty target list must pass" $result.Output

    Write-Output "[2/3] deleting an existing allow attribute does not count as an addition"
    [System.IO.File]::WriteAllText($sourcePath, "pub fn legacy() {}`npub fn safe_addition() {}`n", [System.Text.UTF8Encoding]::new($false))
    Invoke-TestGit -Repository $repository -Arguments @("add", "--", "src/lib.rs")
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -Repository $repository -Paths @("src/lib.rs")
    Assert-ExitCode $result.ExitCode 0 "deleted or pre-existing attributes must not be rejected" $result.Output

    Write-Output "[3/3] a newly added allow attribute is rejected"
    [System.IO.File]::AppendAllText($sourcePath, "#[allow(unused_variables)]`npub fn violation() {}`n", [System.Text.UTF8Encoding]::new($false))
    Invoke-TestGit -Repository $repository -Arguments @("add", "--", "src/lib.rs")
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -Repository $repository -Paths @("src/lib.rs")
    Assert-ExitCode $result.ExitCode 1 "new #[allow( text must be rejected" $result.Output

    Write-Output "no-allow-attribute self-test passed: 3 cases, $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
