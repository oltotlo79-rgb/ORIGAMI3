[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$SourceScriptPath = Join-Path $PSScriptRoot "snapshot-worktrees.ps1"
$PowerShellPath = (Get-Process -Id $PID).Path
$TempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\\/")
$SandboxName = "ori3-snapshot-worktrees-test-{0}" -f [Guid]::NewGuid().ToString("N")
$SandboxRoot = [IO.Path]::GetFullPath((Join-Path $TempRoot $SandboxName))
$script:AssertionCount = 0

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$Output = ""
    )

    $script:AssertionCount += 1
    if (-not $Condition) {
        throw "ASSERTION FAILED: $Message`n$Output"
    }
}

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

function Invoke-Process {
    param(
        [Parameter(Mandatory = $true)][string]$FileName,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [hashtable]$EnvironmentVariables = @{}
    )

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $FileName
    $startInfo.Arguments = ConvertTo-ProcessArgumentString $Arguments
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
    return [PSCustomObject]@{
        ExitCode = $process.ExitCode
        Output = $stdout + $stderr
    }
}

function Invoke-TestGit {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $result = Invoke-Process -FileName "git" -Arguments (@("-C", $Repository) + $Arguments) -WorkingDirectory $Repository
    if ($result.ExitCode -ne 0) {
        throw "git $($Arguments -join ' ') failed (exit=$($result.ExitCode))`n$($result.Output)"
    }
    return $result.Output
}

function New-TestFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Content
    )

    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $Path))
    [IO.File]::WriteAllText($Path, $Content, [Text.UTF8Encoding]::new($false))
}

function New-DisposableWorktreeFixture {
    param([Parameter(Mandatory = $true)][string]$Name)

    $fixtureRoot = Join-Path $SandboxRoot $Name
    $repository = Join-Path $fixtureRoot "repo"
    $worktree = Join-Path $fixtureRoot "ori3-wt-merge"
    $checkCopy = Join-Path $fixtureRoot "ori3-push-check"
    [void][IO.Directory]::CreateDirectory($repository)
    Invoke-TestGit $repository @("init", "--quiet") | Out-Null
    Invoke-TestGit $repository @("config", "user.email", "snapshot-test@example.invalid") | Out-Null
    Invoke-TestGit $repository @("config", "user.name", "Snapshot Test") | Out-Null
    New-TestFile (Join-Path $repository "crates\demo\src\lib.rs") "pub fn snapshot_fixture() {}"
    New-TestFile (Join-Path $repository "docs\guide.md") "# fixture"
    [void][IO.Directory]::CreateDirectory((Join-Path $repository "scripts"))
    Copy-Item -LiteralPath $SourceScriptPath -Destination (Join-Path $repository "scripts\snapshot-worktrees.ps1") -Force
    Invoke-TestGit $repository @("add", "--", "crates", "docs", "scripts") | Out-Null
    Invoke-TestGit $repository @("commit", "--quiet", "-m", "fixture baseline") | Out-Null
    Invoke-TestGit $repository @("worktree", "add", "--detach", $worktree, "HEAD") | Out-Null
    Invoke-TestGit $repository @("worktree", "add", "--detach", $checkCopy, "HEAD") | Out-Null

    [PSCustomObject]@{
        Repository = $repository
        Worktree = $worktree
        CheckCopy = $checkCopy
        ScriptPath = Join-Path $repository "scripts\snapshot-worktrees.ps1"
    }
}

function New-ZeroTargetFixture {
    $fixtureRoot = Join-Path $SandboxRoot "zero-target"
    $repository = Join-Path $fixtureRoot "repo"
    $verificationCopy = Join-Path $repository "verification\push-tree"
    $fakeGitDirectory = Join-Path $fixtureRoot "fake-git"
    [void][IO.Directory]::CreateDirectory($verificationCopy)
    [void][IO.Directory]::CreateDirectory((Join-Path $repository "scripts"))
    [void][IO.Directory]::CreateDirectory($fakeGitDirectory)
    Copy-Item -LiteralPath $SourceScriptPath -Destination (Join-Path $repository "scripts\snapshot-worktrees.ps1") -Force
    $fakeGit = @(
        '@echo off',
        'if /I "%~1"=="worktree" (',
        ('  echo worktree {0}' -f $verificationCopy),
        '  echo HEAD 0000000000000000000000000000000000000000',
        '  echo detached',
        '  exit /b 0',
        ')',
        'echo unexpected git invocation: %* 1>&2',
        'exit /b 1'
    ) -join "`r`n"
    [IO.File]::WriteAllText((Join-Path $fakeGitDirectory "git.cmd"), $fakeGit, [Text.UTF8Encoding]::new($false))
    return [PSCustomObject]@{
        Repository = $repository
        ScriptPath = Join-Path $repository "scripts\snapshot-worktrees.ps1"
        FakeGitDirectory = $fakeGitDirectory
    }
}

function Invoke-SnapshotProcess {
    param(
        [Parameter(Mandatory = $true)]$Fixture,
        [switch]$Check,
        [string]$Name,
        [hashtable]$EnvironmentVariables = @{}
    )

    $arguments = New-Object System.Collections.Generic.List[string]
    $arguments.Add("-NoProfile")
    $arguments.Add("-NonInteractive")
    $arguments.Add("-ExecutionPolicy")
    $arguments.Add("Bypass")
    $arguments.Add("-File")
    $arguments.Add($Fixture.ScriptPath)
    $arguments.Add("-RepositoryRoot")
    $arguments.Add($Fixture.Repository)
    if ($Check) { $arguments.Add("-Check") }
    if (-not [string]::IsNullOrWhiteSpace($Name)) {
        $arguments.Add("-Name")
        $arguments.Add($Name)
    }
    return Invoke-Process -FileName $PowerShellPath -Arguments $arguments.ToArray() -WorkingDirectory $Fixture.Repository -EnvironmentVariables $EnvironmentVariables
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $SandboxRoot)) { return }

    $fullSandbox = [IO.Path]::GetFullPath($SandboxRoot).TrimEnd([char[]]"\\/")
    $expectedParent = [IO.Path]::GetDirectoryName($fullSandbox)
    $leaf = [IO.Path]::GetFileName($fullSandbox)
    if ($expectedParent -ne $TempRoot -or $leaf -notmatch '^ori3-snapshot-worktrees-test-[0-9a-f]{32}$') {
        throw "Refusing unsafe self-test cleanup: $fullSandbox"
    }
    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

[void][IO.Directory]::CreateDirectory($SandboxRoot)

try {
    Write-Host "[1/4] the main worktree and derived worktree names snapshot and pass freshness check"
    $freshFixture = New-DisposableWorktreeFixture "fresh"
    $snapshotResult = Invoke-SnapshotProcess -Fixture $freshFixture
    Assert-Equal $snapshotResult.ExitCode 0 "snapshot process must exit 0" $snapshotResult.Output
    $freshCheck = Invoke-SnapshotProcess -Fixture $freshFixture -Check
    Assert-Equal $freshCheck.ExitCode 0 "fresh snapshot check must exit 0" $freshCheck.Output
    Assert-Contains $freshCheck.Output "snapshot targets=2, excluded=1, mode=check" "check output must disclose target and exclusion counts"
    Assert-Contains $freshCheck.Output "snapshot check completed: targets=2, findings=0" "check output must disclose a zero-finding success"
    Assert-Contains $freshCheck.Output "[EXCLUDE]" "check output must disclose excluded worktrees"
    $derivedRef = Invoke-TestGit $freshFixture.Repository @("rev-parse", "--verify", "refs/wip/merge")
    Assert-True ($derivedRef.Trim() -match '^[0-9a-f]{40}$') "ori3-wt-merge must derive refs/wip/merge" $derivedRef
    $rootRef = Invoke-TestGit $freshFixture.Repository @("rev-parse", "--verify", "refs/wip/main")
    Assert-True ($rootRef.Trim() -match '^[0-9a-f]{40}$') "the repository root must be snapshotted as refs/wip/main" $rootRef
    $checkCopyRef = Invoke-Process -FileName "git" -Arguments @("-C", $freshFixture.Repository, "show-ref", "--verify", "--quiet", "refs/wip/ori3-push-check") -WorkingDirectory $freshFixture.Repository
    Assert-True ($checkCopyRef.ExitCode -ne 0) "a registered check copy outside the ori3-wt-* convention must be excluded" $checkCopyRef.Output

    Write-Host "[2/4] a worktree without a snapshot fails in a new process"
    $missingFixture = New-DisposableWorktreeFixture "missing"
    $missingCheck = Invoke-SnapshotProcess -Fixture $missingFixture -Check
    Assert-True ($missingCheck.ExitCode -ne 0) "missing snapshot check must have a nonzero process exit code" $missingCheck.Output
    Assert-Contains $missingCheck.Output "refs/wip/merge" "missing snapshot output must name the derived reference"

    Write-Host "[3/4] a snapshot older than source fails in a new process"
    $staleFixture = New-DisposableWorktreeFixture "stale"
    $staleSnapshot = Invoke-SnapshotProcess -Fixture $staleFixture -Name "merge"
    Assert-Equal $staleSnapshot.ExitCode 0 "stale fixture must first create a snapshot" $staleSnapshot.Output
    $newerSourcePath = Join-Path $staleFixture.Worktree "crates\demo\src\lib.rs"
    [IO.File]::AppendAllText($newerSourcePath, "`n// newer than snapshot", [Text.UTF8Encoding]::new($false))
    [IO.File]::SetLastWriteTimeUtc($newerSourcePath, [DateTime]::UtcNow.AddMinutes(1))
    $staleCheck = Invoke-SnapshotProcess -Fixture $staleFixture -Check
    Assert-True ($staleCheck.ExitCode -ne 0) "stale snapshot check must have a nonzero process exit code" $staleCheck.Output
    Assert-Contains $staleCheck.Output "refs/wip/merge" "stale snapshot output must name the derived reference"

    Write-Host "[4/4] a discovery result with no includable target fails visibly"
    $zeroFixture = New-ZeroTargetFixture
    $zeroCheck = Invoke-SnapshotProcess -Fixture $zeroFixture -Check -EnvironmentVariables @{ PATH = ($zeroFixture.FakeGitDirectory + ";" + $env:PATH) }
    Assert-True ($zeroCheck.ExitCode -ne 0) "zero targets must have a nonzero process exit code" $zeroCheck.Output
    Assert-Contains $zeroCheck.Output "snapshot targets=0" "zero-target check must report zero targets"
    Assert-Contains $zeroCheck.Output "snapshot check completed: targets=0, findings=1" "zero-target check must report an abnormal finding"
    Assert-Contains $zeroCheck.Output "[EXCLUDE]" "zero-target check must disclose why its only worktree was excluded"

    Write-Host "[EVIDENCE] child snapshot exit=$($snapshotResult.ExitCode); fresh check exit=$($freshCheck.ExitCode); missing check exit=$($missingCheck.ExitCode); stale check exit=$($staleCheck.ExitCode); zero-target check exit=$($zeroCheck.ExitCode)"
    Write-Host "snapshot-worktrees self-test passed: $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
