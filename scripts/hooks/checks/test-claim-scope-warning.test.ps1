[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$ScriptPath = Join-Path $PSScriptRoot "test-claim-scope-warning.ps1"
$PowerShellPath = (Get-Process -Id $PID).Path
$TempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\\/")
$SandboxName = "ori3-test-claim-scope-warning-test-{0}" -f [Guid]::NewGuid().ToString("N")
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
    $normalized = [regex]::Replace($Text, '\s+', ' ')
    if (-not $normalized.Contains($Expected)) {
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

function Invoke-Process {
    param(
        [Parameter(Mandatory = $true)][string]$FileName,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory
    )

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $FileName
    $startInfo.Arguments = ConvertTo-ProcessArgumentString -Values $Arguments
    $startInfo.WorkingDirectory = $WorkingDirectory
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

function Invoke-Git {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $result = Invoke-Process -FileName "git" -Arguments (@("-C", $Repository) + $Arguments) -WorkingDirectory $Repository
    if ($result.ExitCode -ne 0) {
        throw "git $($Arguments -join ' ') failed (exit=$($result.ExitCode))`n$($result.Output)"
    }
}

function Write-TestFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Content
    )

    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $Path))
    [IO.File]::WriteAllText($Path, $Content, [Text.UTF8Encoding]::new($false))
}

function New-TestRepository {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$InitialContent,
        [Parameter(Mandatory = $true)][string]$ChangedContent
    )

    $repository = Join-Path $SandboxRoot $Name
    [void][IO.Directory]::CreateDirectory($repository)
    Invoke-Git $repository @("init", "--quiet")
    $emptyIgnore = Join-Path $repository "empty-global-ignore"
    [IO.File]::WriteAllText($emptyIgnore, "", [Text.UTF8Encoding]::new($false))
    Invoke-Git $repository @("config", "core.excludesFile", $emptyIgnore)
    Invoke-Git $repository @("config", "user.email", "scope-test@example.invalid")
    Invoke-Git $repository @("config", "user.name", "Scope Test")
    $testPath = Join-Path $repository "crates\demo\tests\scope_test.rs"
    Write-TestFile $testPath $InitialContent
    Invoke-Git $repository @("add", "--", "crates\demo\tests\scope_test.rs")
    Invoke-Git $repository @("commit", "--quiet", "-m", "baseline")
    Write-TestFile $testPath $ChangedContent
    Invoke-Git $repository @("add", "--", "crates\demo\tests\scope_test.rs")
    return $repository
}

function Invoke-ScopeWarning {
    param([Parameter(Mandatory = $true)][string]$Repository)

    return Invoke-Process -FileName $PowerShellPath -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $ScriptPath, "-RepositoryRoot", $Repository
    ) -WorkingDirectory $Repository
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $SandboxRoot)) { return }
    $fullSandbox = [IO.Path]::GetFullPath($SandboxRoot).TrimEnd([char[]]"\\/")
    if ([IO.Path]::GetDirectoryName($fullSandbox) -ne $TempRoot -or [IO.Path]::GetFileName($fullSandbox) -notmatch '^ori3-test-claim-scope-warning-test-[0-9a-f]{32}$') {
        throw "Refusing unsafe self-test cleanup: $fullSandbox"
    }
    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

[void][IO.Directory]::CreateDirectory($SandboxRoot)

try {
    Write-Host "[1/3] dynamic iteration changed to a literal emits a warning"
    $dynamicRepository = New-TestRepository -Name "dynamic" -InitialContent @'
#[test]
fn covers_all_values() {
    for value in values.iter() {
        assert!(value.is_valid());
    }
}
'@ -ChangedContent @'
#[test]
fn covers_all_values() {
    for value in [first, second] {
        assert!(value.is_valid());
    }
}
'@
    $dynamicResult = Invoke-ScopeWarning $dynamicRepository
    Assert-Equal $dynamicResult.ExitCode 0 "warning scan must remain nonblocking" $dynamicResult.Output
    Assert-Contains $dynamicResult.Output "dynamic iteration may have become a fixed literal" "dynamic-to-literal signal must be reported"

    Write-Host "[2/3] removed assertion calls emit a warning"
    $assertionRepository = New-TestRepository -Name "assertion" -InitialContent @'
#[test]
fn verifies_result() {
    assert!(result.is_ok());
    assert_eq!(result.count(), 5);
}
'@ -ChangedContent @'
#[test]
fn verifies_result() {
    assert!(result.is_ok());
}
'@
    $assertionResult = Invoke-ScopeWarning $assertionRepository
    Assert-Equal $assertionResult.ExitCode 0 "assertion warning scan must remain nonblocking" $assertionResult.Output
    Assert-Contains $assertionResult.Output "assertion calls decreased" "removed assertion signal must be reported"

    Write-Host "[3/3] unchanged assertion count has no warning"
    $cleanRepository = New-TestRepository -Name "clean" -InitialContent @'
#[test]
fn verifies_result() {
    assert!(result.is_ok());
}
'@ -ChangedContent @'
#[test]
fn verifies_result() {
    assert!(result.is_ready());
}
'@
    $cleanResult = Invoke-ScopeWarning $cleanRepository
    Assert-Equal $cleanResult.ExitCode 0 "clean warning scan must exit 0" $cleanResult.Output
    Assert-Contains $cleanResult.Output "No staged test-claim narrowing signal" "no signal must be stated"

    Write-Host "[EVIDENCE] dynamic exit=$($dynamicResult.ExitCode); assertion exit=$($assertionResult.ExitCode); clean exit=$($cleanResult.ExitCode)"
    Write-Host "test-claim-scope-warning self-test passed: $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
