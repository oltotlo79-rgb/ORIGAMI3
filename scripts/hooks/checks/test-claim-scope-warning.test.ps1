[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$ScriptPath = Join-Path $PSScriptRoot "test-claim-scope-warning.ps1"
$HealthScriptPath = Join-Path $PSScriptRoot "hook-health.ps1"
$HookPath = Join-Path $PSScriptRoot "..\pre-commit"
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

function Assert-NotContains {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Unexpected,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $script:AssertionCount += 1
    if ($Text.Contains($Unexpected)) {
        throw "ASSERTION FAILED: $Message (unexpected='$Unexpected')`n$Text"
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
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [hashtable]$EnvironmentVariables = @{}
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
    foreach ($key in $EnvironmentVariables.Keys) {
        $startInfo.EnvironmentVariables[[string]$key] = [string]$EnvironmentVariables[$key]
    }
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

function New-HookTestRepository {
    $repository = Join-Path $SandboxRoot "hook"
    [void][IO.Directory]::CreateDirectory($repository)
    Invoke-Git $repository @("init", "--quiet")
    $emptyIgnore = Join-Path $repository "empty-global-ignore"
    [IO.File]::WriteAllText($emptyIgnore, "", [Text.UTF8Encoding]::new($false))
    Invoke-Git $repository @("config", "core.excludesFile", $emptyIgnore)
    Invoke-Git $repository @("config", "user.email", "scope-hook-test@example.invalid")
    Invoke-Git $repository @("config", "user.name", "Scope Hook Test")

    $testPath = Join-Path $repository "scripts\demo.test.ps1"
    Write-TestFile $testPath @'
Describe "all values" {
    It "checks every value" {
        foreach ($value in $values) { Assert-True $value }
        Assert-True $first
        Assert-True $second
    }
}
'@
    Invoke-Git $repository @("add", "--", "scripts\demo.test.ps1")
    Invoke-Git $repository @("commit", "--quiet", "-m", "baseline")

    Write-TestFile $testPath @'
Describe "selected values" {
    It "checks one value" {
        Assert-True $first
    }
}
'@
    Invoke-Git $repository @("add", "--", "scripts\demo.test.ps1")

    $checkerDestination = Join-Path $repository "scripts\hooks\checks\test-claim-scope-warning.ps1"
    $healthDestination = Join-Path $repository "scripts\hooks\checks\hook-health.ps1"
    $hookDestination = Join-Path $repository "scripts\hooks\pre-commit"
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $checkerDestination))
    [IO.File]::Copy($ScriptPath, $checkerDestination, $true)
    [IO.File]::Copy($HealthScriptPath, $healthDestination, $true)
    [IO.File]::Copy($HookPath, $hookDestination, $true)
    [IO.File]::Copy($HookPath, (Join-Path $repository ".git\hooks\pre-commit"), $true)
    return $repository
}

function Invoke-ScopeWarning {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [switch]$FailOnVacuousRustTest
    )

    $arguments = @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $ScriptPath, "-RepositoryRoot", $Repository
    )
    if ($FailOnVacuousRustTest) { $arguments += "-FailOnVacuousRustTest" }
    return Invoke-Process -FileName $PowerShellPath -Arguments $arguments -WorkingDirectory $Repository
}

function Get-GitBashPath {
    $gitCommand = Get-Command git -ErrorAction Stop
    $gitDirectory = Split-Path -Parent $gitCommand.Source
    $gitRoot = Split-Path -Parent $gitDirectory
    $candidate = Join-Path $gitRoot "bin\sh.exe"
    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
        return $candidate
    }
    $shCommand = Get-Command sh -ErrorAction SilentlyContinue
    if ($null -ne $shCommand) {
        return $shCommand.Source
    }
    throw "Git Bash sh.exe was not found."
}

function Invoke-GitBash {
    param(
        [Parameter(Mandatory = $true)][string]$GitBashPath,
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string]$Command
    )

    $shellRepository = $Repository.Replace("\\", "/")
    if ($shellRepository.Contains("'")) {
        throw "The temporary repository path cannot contain a single quote."
    }
    return Invoke-Process -FileName $GitBashPath -Arguments @("-lc", "cd '$shellRepository' && $Command") -WorkingDirectory $Repository
}

function Invoke-HookHealthCheck {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][int]$ExpectedExitCode
    )

    $result = Invoke-Process -FileName $PowerShellPath -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $HealthScriptPath,
        "-Action", "Check",
        "-RepositoryRoot", $Repository,
        "-Threshold", "2"
    ) -WorkingDirectory $Repository
    Assert-Equal $result.ExitCode $ExpectedExitCode "hook health check exit code must match the expected state" $result.Output
    return $result
}

function Get-HookHealthStatePath {
    param([Parameter(Mandatory = $true)][string]$Repository)

    $normalized = [IO.Path]::GetFullPath($Repository).Replace("\", "/").TrimEnd("/").ToLowerInvariant()
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $key = -join ($sha256.ComputeHash([Text.Encoding]::UTF8.GetBytes($normalized)) | ForEach-Object { $_.ToString("x2") })
    }
    finally {
        $sha256.Dispose()
    }
    return Join-Path (Join-Path ([IO.Path]::GetTempPath()) "ori3-hook-health") ("{0}-test-claim-scope-warning.json" -f $key)
}

function Remove-TestSandbox {
    $hookRepository = Join-Path $SandboxRoot "hook"
    $healthStatePath = Get-HookHealthStatePath -Repository $hookRepository
    if (Test-Path -LiteralPath $healthStatePath -PathType Leaf) {
        Remove-Item -LiteralPath $healthStatePath -Force
    }
    if (-not (Test-Path -LiteralPath $SandboxRoot)) { return }
    $fullSandbox = [IO.Path]::GetFullPath($SandboxRoot).TrimEnd([char[]]"\\/")
    if ([IO.Path]::GetDirectoryName($fullSandbox) -ne $TempRoot -or [IO.Path]::GetFileName($fullSandbox) -notmatch '^ori3-test-claim-scope-warning-test-[0-9a-f]{32}$') {
        throw "Refusing unsafe self-test cleanup: $fullSandbox"
    }
    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

[void][IO.Directory]::CreateDirectory($SandboxRoot)

try {
    Write-Host "[1/9] dynamic iteration changed to a literal emits a warning"
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

    Write-Host "[2/9] removed assertion calls emit a warning"
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

    Write-Host "[3/9] unchanged assertion count has no warning"
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
    Assert-Contains $cleanResult.Output "test-claim-scope scan completed: targets=1, findings=0" "no-signal completion must state target and finding counts"

    Write-Host "[4/9] a newly added Rust test without a failure signal emits a warning, then removing it clears the warning"
    $vacuousRepository = New-TestRepository -Name "vacuous" -InitialContent @'
// Baseline intentionally has no tests.
'@ -ChangedContent @'
#[test]
fn does_not_verify_anything() {
    let unused = 42;
    let _ = unused;
}
'@
    $vacuousResult = Invoke-ScopeWarning $vacuousRepository
    Assert-Equal $vacuousResult.ExitCode 0 "a vacuous-test finding must remain nonblocking" $vacuousResult.Output
    Assert-Contains $vacuousResult.Output "new Rust test has no direct failure signal" "a newly added vacuous Rust test must be reported"
    Assert-Contains $vacuousResult.Output "function=does_not_verify_anything" "the finding must identify the vacuous test function"
    $blockingVacuousResult = Invoke-ScopeWarning $vacuousRepository -FailOnVacuousRustTest
    Assert-Equal $blockingVacuousResult.ExitCode 2 "a vacuous Rust test must make the blocking scanner fail" $blockingVacuousResult.Output
    $vacuousTestPath = Join-Path $vacuousRepository "crates\demo\tests\scope_test.rs"
    Write-TestFile $vacuousTestPath "// Baseline intentionally has no tests.`n"
    Invoke-Git $vacuousRepository @("add", "--", "crates\demo\tests\scope_test.rs")
    $removedVacuousResult = Invoke-ScopeWarning $vacuousRepository
    Assert-Equal $removedVacuousResult.ExitCode 0 "removing the vacuous test must keep the scanner healthy" $removedVacuousResult.Output
    Assert-Contains $removedVacuousResult.Output "findings=0" "removing the vacuous test must clear the finding"

    Write-Host "[5/9] a newly added Rust test with an assertion has no false warning"
    $verifiedRepository = New-TestRepository -Name "verified" -InitialContent @'
// Baseline intentionally has no tests.
'@ -ChangedContent @'
#[test]
fn verifies_a_value() {
    assert_eq!(2 + 2, 4);
}
'@
    $verifiedResult = Invoke-ScopeWarning $verifiedRepository
    Assert-Equal $verifiedResult.ExitCode 0 "a verified Rust test must keep the scanner healthy" $verifiedResult.Output
    Assert-Contains $verifiedResult.Output "test-claim-scope scan completed: targets=1, findings=0" "a newly added assertion-bearing Rust test must have no warning"
    $blockingVerifiedResult = Invoke-ScopeWarning $verifiedRepository -FailOnVacuousRustTest
    Assert-Equal $blockingVerifiedResult.ExitCode 0 "a verified Rust test must not make the blocking scanner fail" $blockingVerifiedResult.Output

    $shouldPanicRepository = New-TestRepository -Name "should-panic" -InitialContent @'
// Baseline intentionally has no tests.
'@ -ChangedContent @'
#[test]
#[should_panic]
fn reports_an_expected_panic() {
    let unused = 42;
    let _ = unused;
}
'@
    $shouldPanicResult = Invoke-ScopeWarning $shouldPanicRepository -FailOnVacuousRustTest
    Assert-Equal $shouldPanicResult.ExitCode 0 "a #[should_panic] test must not make the blocking scanner fail" $shouldPanicResult.Output
    Assert-Contains $shouldPanicResult.Output "findings=0" "a #[should_panic] test must not emit a vacuous-test finding"

    Write-Host "[6/9] adding a verified Rust test does not make shifted existing test names new"
    $lineShiftRepository = New-TestRepository -Name "line-shift" -InitialContent @'
fn helper() {
    prepare_roundtrip();
}

#[test]
fn existing_roundtrip_record() {
    record_roundtrip();
}
'@ -ChangedContent @'
#[test]
fn newly_verified_roundtrip() {
    assert_eq!(2 + 2, 4);
}

#[test]
fn existing_roundtrip_record() {
    record_roundtrip();
}

fn helper() {
    prepare_roundtrip();
}
'@
    $lineShiftResult = Invoke-ScopeWarning $lineShiftRepository -FailOnVacuousRustTest
    Assert-Equal $lineShiftResult.ExitCode 0 "adding a verified test must not make a shifted existing test block the scanner" $lineShiftResult.Output
    Assert-Contains $lineShiftResult.Output "test-claim-scope scan completed: targets=1, findings=0" "a shifted existing Rust test name must not be treated as new"

    Write-Host "[7/9] Git Bash -File fallback and pre-commit invocation both run the scanner"
    $hookRepository = New-HookTestRepository
    $gitBashPath = Get-GitBashPath
    $emptyRootCommand = @'
if (-not [string]::IsNullOrWhiteSpace($PSScriptRoot)) {
    throw "The PSScriptRoot-empty fixture did not start with an empty value."
}
$source = [IO.File]::ReadAllText($env:ORI3_SCOPE_WARNING_SCRIPT)
& ([ScriptBlock]::Create($source)) -RepositoryRoot $env:ORI3_SCOPE_WARNING_REPOSITORY
'@
    $emptyRootResult = Invoke-Process -FileName $PowerShellPath -Arguments @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", $emptyRootCommand
    ) -WorkingDirectory $hookRepository -EnvironmentVariables @{
        ORI3_SCOPE_WARNING_SCRIPT = (Join-Path $hookRepository "scripts\hooks\checks\test-claim-scope-warning.ps1")
        ORI3_SCOPE_WARNING_REPOSITORY = $hookRepository
    }
    Assert-Equal $emptyRootResult.ExitCode 0 "an empty PSScriptRoot process must run the scanner when RepositoryRoot is explicit" $emptyRootResult.Output
    Assert-Contains $emptyRootResult.Output "assertion calls decreased" "the empty-root fixture must scan staged test content"

    $directBashResult = Invoke-GitBash -GitBashPath $gitBashPath -Repository $hookRepository -Command 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File scripts/hooks/checks/test-claim-scope-warning.ps1 -Verbose'
    Assert-Equal $directBashResult.ExitCode 0 "Git Bash -File invocation without RepositoryRoot must scan successfully" $directBashResult.Output
    Assert-Contains $directBashResult.Output "Default RepositoryRoot resolved through `$PSScriptRoot" "default root calculation must occur after parameter binding"
    Assert-Contains $directBashResult.Output "assertion calls decreased" "fallback invocation must scan staged test content"

    $hookResult = Invoke-GitBash -GitBashPath $gitBashPath -Repository $hookRepository -Command 'sh scripts/hooks/pre-commit'
    Assert-Equal $hookResult.ExitCode 0 "pre-commit must remain nonblocking after a scope warning" $hookResult.Output
    Assert-Contains $hookResult.Output "assertion calls decreased" "pre-commit must actually emit the scanner result"
    Assert-NotContains $hookResult.Output "scope warning scan could not run" "pre-commit must not hide a scanner startup failure"

    Write-Host "[8/9] actual commits retain existing warnings, block a vacuous Rust test, then recover after its removal"
    $existingWarningCommit = Invoke-Process -FileName "git" -Arguments @("commit", "--quiet", "-m", "existing warning remains nonblocking") -WorkingDirectory $hookRepository
    Assert-Equal $existingWarningCommit.ExitCode 0 "an existing assertion-decrease warning must not block an actual commit" $existingWarningCommit.Output

    $vacuousCommitPath = Join-Path $hookRepository "scratchpad\vacuous_test.rs"
    Write-TestFile $vacuousCommitPath @'
#[test]
fn does_not_verify_anything() {
    let unused = 42;
    let _ = unused;
}
'@
    Invoke-Git $hookRepository @("add", "--", "scratchpad\vacuous_test.rs")
    $blockedCommit = Invoke-Process -FileName "git" -Arguments @("commit", "--quiet", "-m", "vacuous test must be rejected") -WorkingDirectory $hookRepository
    Assert-Equal ($blockedCommit.ExitCode -ne 0) $true "a vacuous Rust test must block an actual commit" $blockedCommit.Output
    Assert-Contains $blockedCommit.Output "function=does_not_verify_anything" "a blocked commit must identify the test function"
    Assert-Contains $blockedCommit.Output "Add a failure assertion" "a blocked commit must tell the author how to repair it"
    Invoke-Git $hookRepository @("rm", "--cached", "--quiet", "--", "scratchpad\vacuous_test.rs")
    Remove-Item -LiteralPath $vacuousCommitPath -Force

    Write-TestFile (Join-Path $hookRepository "scratchpad\recovered-after-vacuous-test.md") "The rejected vacuous Rust test was removed before this commit.`n"
    Invoke-Git $hookRepository @("add", "--", "scratchpad\recovered-after-vacuous-test.md")
    $recoveredCommit = Invoke-Process -FileName "git" -Arguments @("commit", "--quiet", "-m", "removing the vacuous test restores commits") -WorkingDirectory $hookRepository
    Assert-Equal $recoveredCommit.ExitCode 0 "removing the vacuous test must allow an actual commit" $recoveredCommit.Output

    Write-Host "[9/9] two unavailable PowerShell starts persist degraded health, then one success restores it"
    $gitCommandDirectory = [IO.Path]::GetDirectoryName((Get-Command git -ErrorAction Stop).Source)
    $gitBashCommandDirectory = [regex]::Replace($gitCommandDirectory, '^([A-Za-z]):\\', '/$1/').Replace("\\", "/")
    $withoutPowerShellCommand = "PATH='${gitBashCommandDirectory}:/usr/bin'; export PATH; sh scripts/hooks/pre-commit"
    $unavailableFirst = Invoke-GitBash -GitBashPath $gitBashPath -Repository $hookRepository -Command $withoutPowerShellCommand
    Assert-Equal $unavailableFirst.ExitCode 0 "PowerShell absence must not block the commit" $unavailableFirst.Output
    Assert-Contains $unavailableFirst.Output "PowerShell is unavailable" "the unavailable path must be visible"
    Assert-Contains $unavailableFirst.Output "HOOK_HEALTH_DEGRADED" "the unavailable path must record health degradation"
    $firstHealth = Invoke-HookHealthCheck -Repository $hookRepository -ExpectedExitCode 0
    Assert-Contains $firstHealth.Output "failures=1" "first unavailable start must record one failure"

    $unavailableSecond = Invoke-GitBash -GitBashPath $gitBashPath -Repository $hookRepository -Command $withoutPowerShellCommand
    Assert-Equal $unavailableSecond.ExitCode 0 "a repeated unavailable PowerShell state must keep the commit nonblocking" $unavailableSecond.Output
    $degradedHealth = Invoke-HookHealthCheck -Repository $hookRepository -ExpectedExitCode 2
    Assert-Contains $degradedHealth.Output "failures=2" "second unavailable start must reach the threshold"

    $recoveredHook = Invoke-GitBash -GitBashPath $gitBashPath -Repository $hookRepository -Command 'sh scripts/hooks/pre-commit'
    Assert-Equal $recoveredHook.ExitCode 0 "a healthy scanner must restore the nonblocking hook" $recoveredHook.Output
    $restoredHealth = Invoke-HookHealthCheck -Repository $hookRepository -ExpectedExitCode 0
    Assert-Contains $restoredHealth.Output "failures=0" "one successful scanner run must reset health"

    Write-Host "[EVIDENCE] dynamic exit=$($dynamicResult.ExitCode); assertion exit=$($assertionResult.ExitCode); clean exit=$($cleanResult.ExitCode); vacuous exit=$($vacuousResult.ExitCode); vacuous-blocking exit=$($blockingVacuousResult.ExitCode); vacuous-removed exit=$($removedVacuousResult.ExitCode); verified exit=$($verifiedResult.ExitCode); verified-blocking exit=$($blockingVerifiedResult.ExitCode); should-panic exit=$($shouldPanicResult.ExitCode); line-shift exit=$($lineShiftResult.ExitCode); empty-root fixture exit=$($emptyRootResult.ExitCode); bash fallback exit=$($directBashResult.ExitCode); pre-commit exit=$($hookResult.ExitCode); existing-warning-commit=$($existingWarningCommit.ExitCode); blocked-commit=$($blockedCommit.ExitCode); recovered-commit=$($recoveredCommit.ExitCode); unavailable-first=$($unavailableFirst.ExitCode); unavailable-second=$($unavailableSecond.ExitCode); restored=$($recoveredHook.ExitCode)"
    Write-Host "test-claim-scope-warning self-test passed: $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
