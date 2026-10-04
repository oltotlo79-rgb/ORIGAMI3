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
    # Read both pipes at the same time. A child that fills the stderr pipe while this process is
    # still waiting for stdout to end would otherwise block forever, so a script that regressed
    # into spamming errors would hang the self-test instead of failing it.
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    $stdout = $stdoutTask.Result
    $stderr = $stderrTask.Result
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

function Test-GitPathAtRef {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string]$ObjectName
    )

    $result = Invoke-Process -FileName "git" -Arguments @("-C", $Repository, "cat-file", "-e", $ObjectName) -WorkingDirectory $Repository
    return $result.ExitCode -eq 0
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
    New-TestFile (Join-Path $repository ".gitignore") "scratchpad/"
    [void][IO.Directory]::CreateDirectory((Join-Path $repository "scripts"))
    Copy-Item -LiteralPath $SourceScriptPath -Destination (Join-Path $repository "scripts\snapshot-worktrees.ps1") -Force
    Invoke-TestGit $repository @("add", "--", "crates", "docs", "scripts", ".gitignore") | Out-Null
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
        [string[]]$ExtraArguments = @(),
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
    foreach ($extraArgument in $ExtraArguments) { $arguments.Add($extraArgument) }
    return Invoke-Process -FileName $PowerShellPath -Arguments $arguments.ToArray() -WorkingDirectory $Fixture.Repository -EnvironmentVariables $EnvironmentVariables
}

# Creates scratchpad files from a relative-path -> content map and returns their total bytes.
function New-ScratchpadFiles {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][System.Collections.IDictionary]$Files
    )

    [int64]$total = 0
    foreach ($relative in $Files.Keys) {
        New-TestFile (Join-Path $Root ("scratchpad\" + $relative.Replace("/", "\"))) ([string]$Files[$relative])
        $total += [Text.Encoding]::UTF8.GetByteCount([string]$Files[$relative])
    }
    return $total
}

# Sorted list of the files a refs/wip snapshot holds below a folder (default: scratchpad/).
function Get-SnapshotTreeFiles {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string]$Ref,
        [string]$Prefix = "scratchpad"
    )

    $listing = Invoke-TestGit $Repository @("ls-tree", "-r", "--name-only", $Ref, "--", $Prefix)
    return @($listing -split "`r?`n" | Where-Object { $_ -ne "" } | Sort-Object)
}

function Get-ExpectedScratchpadFiles {
    param([Parameter(Mandatory = $true)][System.Collections.IDictionary]$Files)

    return @($Files.Keys | ForEach-Object { "scratchpad/" + $_ } | Sort-Object)
}

# Parent commit ids of a ref, separated by one space, first parent first.
function Get-CommitParents {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string]$Ref
    )

    $ids = @((Invoke-TestGit $Repository @("rev-list", "--parents", "-n", "1", $Ref)).Trim() -split "\s+")
    if ($ids.Count -le 1) { return "" }
    return ($ids[1..($ids.Count - 1)] -join " ")
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
    Write-Host "[1/14] the main worktree and derived worktree names snapshot and pass freshness check"
    $freshFixture = New-DisposableWorktreeFixture "fresh"
    New-TestFile (Join-Path $freshFixture.Worktree "scratchpad\note.md") "included markdown"
    New-TestFile (Join-Path $freshFixture.Worktree "scratchpad\nested\note.md") "included nested markdown"
    New-TestFile (Join-Path $freshFixture.Worktree "scratchpad\resume.patch") "included direct patch"
    New-TestFile (Join-Path $freshFixture.Worktree "scratchpad\resume.txt") "included direct text"
    # An ori3-wt-* worktree snapshots every depth, so its nested patch is included. The nested
    # patch that the documented legacy rule excludes is checked on the repository root instead
    # (refs/wip/main), which keeps that rule unchanged.
    New-TestFile (Join-Path $freshFixture.Worktree "scratchpad\nested\nested.patch") "included nested patch of a worktree"
    New-TestFile (Join-Path $freshFixture.Repository "scratchpad\root-note.md") "included markdown of the repository root"
    New-TestFile (Join-Path $freshFixture.Repository "scratchpad\nested\not-included.patch") "excluded nested patch"
    $headBeforeSnapshot = (Invoke-TestGit $freshFixture.Repository @("rev-parse", "HEAD")).Trim()
    $branchBeforeSnapshot = (Invoke-TestGit $freshFixture.Repository @("symbolic-ref", "-q", "HEAD")).Trim()
    $snapshotResult = Invoke-SnapshotProcess -Fixture $freshFixture
    Assert-Equal $snapshotResult.ExitCode 0 "snapshot process must exit 0" $snapshotResult.Output
    $headAfterSnapshot = (Invoke-TestGit $freshFixture.Repository @("rev-parse", "HEAD")).Trim()
    $branchAfterSnapshot = (Invoke-TestGit $freshFixture.Repository @("symbolic-ref", "-q", "HEAD")).Trim()
    Assert-Equal $headAfterSnapshot $headBeforeSnapshot "snapshot must not move the checked-out branch HEAD" $snapshotResult.Output
    Assert-Equal $branchAfterSnapshot $branchBeforeSnapshot "snapshot must not change the checked-out branch name" $snapshotResult.Output
    Assert-NotContains $snapshotResult.Output "fatal: pathspec" "an absent optional scratchpad class must not emit a git fatal"
    $freshCheck = Invoke-SnapshotProcess -Fixture $freshFixture -Check
    Assert-Equal $freshCheck.ExitCode 0 "fresh snapshot check must exit 0" $freshCheck.Output
    Assert-Contains $freshCheck.Output "snapshot targets=2, excluded=1, mode=check" "check output must disclose target and exclusion counts"
    Assert-Contains $freshCheck.Output "snapshot check completed: targets=2, findings=0" "check output must disclose a zero-finding success"
    Assert-Contains $freshCheck.Output "[EXCLUDE]" "check output must disclose excluded worktrees"
    $derivedRef = Invoke-TestGit $freshFixture.Repository @("rev-parse", "--verify", "refs/wip/merge")
    Assert-True ($derivedRef.Trim() -match '^[0-9a-f]{40}$') "ori3-wt-merge must derive refs/wip/merge" $derivedRef
    $rootRef = Invoke-TestGit $freshFixture.Repository @("rev-parse", "--verify", "refs/wip/main")
    Assert-True ($rootRef.Trim() -match '^[0-9a-f]{40}$') "the repository root must be snapshotted as refs/wip/main" $rootRef
    Assert-True (Test-GitPathAtRef $freshFixture.Repository "refs/wip/merge:scratchpad/note.md") "direct scratchpad markdown must be snapshotted"
    Assert-True (Test-GitPathAtRef $freshFixture.Repository "refs/wip/merge:scratchpad/nested/note.md") "nested scratchpad markdown must be snapshotted"
    Assert-True (Test-GitPathAtRef $freshFixture.Repository "refs/wip/merge:scratchpad/resume.patch") "direct scratchpad patch must be snapshotted"
    Assert-True (Test-GitPathAtRef $freshFixture.Repository "refs/wip/merge:scratchpad/resume.txt") "direct scratchpad text must be snapshotted"
    Assert-True (Test-GitPathAtRef $freshFixture.Repository "refs/wip/merge:scratchpad/nested/nested.patch") "a worktree must snapshot a nested scratchpad patch"
    Assert-True (Test-GitPathAtRef $freshFixture.Repository "refs/wip/main:scratchpad/root-note.md") "the repository root must snapshot its scratchpad markdown (control for the exclusion below)"
    Assert-True (-not (Test-GitPathAtRef $freshFixture.Repository "refs/wip/main:scratchpad/nested/not-included.patch")) "nested scratchpad patch of the repository root must keep the documented exclusion rule"
    $checkCopyRef = Invoke-Process -FileName "git" -Arguments @("-C", $freshFixture.Repository, "show-ref", "--verify", "--quiet", "refs/wip/ori3-push-check") -WorkingDirectory $freshFixture.Repository
    Assert-True ($checkCopyRef.ExitCode -ne 0) "a registered check copy outside the ori3-wt-* convention must be excluded" $checkCopyRef.Output

    Write-Host "[2/14] two thousand scratchpad markdown paths exceeding sixty thousand characters snapshot successfully"
    $largeFixture = New-DisposableWorktreeFixture "large-scratchpad"
    $largeScratchpadPaths = New-Object System.Collections.Generic.List[string]
    foreach ($number in 1..2000) {
        $relativePath = "scratchpad\bulk\pathspec-command-limit-entry-{0:D4}.md" -f $number
        $largeScratchpadPaths.Add($relativePath)
        New-TestFile (Join-Path $largeFixture.Worktree $relativePath) "large scratchpad entry $number"
    }
    $legacyPathLength = (($largeScratchpadPaths | ForEach-Object { '"' + $_ + '"' }) -join ' ').Length
    Assert-True ($legacyPathLength -gt 60000) "two thousand scratchpad paths must exceed the legacy command-line limit input size"
    $largeSnapshot = Invoke-SnapshotProcess -Fixture $largeFixture -Name "merge"
    Assert-Equal $largeSnapshot.ExitCode 0 "large scratchpad snapshot must exit 0" $largeSnapshot.Output
    $largeSnapshotTree = Invoke-TestGit $largeFixture.Repository @("ls-tree", "-r", "--name-only", "refs/wip/merge", "--", "scratchpad/bulk")
    $largeSnapshotPaths = @($largeSnapshotTree -split "`r?`n" | Where-Object { $_ -like "scratchpad/bulk/*.md" })
    Assert-Equal $largeSnapshotPaths.Count 2000 "every large scratchpad markdown path must be in the snapshot tree"

    Write-Host "[3/14] a worktree without a snapshot fails in a new process"
    $missingFixture = New-DisposableWorktreeFixture "missing"
    $missingCheck = Invoke-SnapshotProcess -Fixture $missingFixture -Check
    Assert-True ($missingCheck.ExitCode -ne 0) "missing snapshot check must have a nonzero process exit code" $missingCheck.Output
    Assert-Contains $missingCheck.Output "refs/wip/merge" "missing snapshot output must name the derived reference"

    Write-Host "[4/14] a snapshot older than source fails in a new process"
    $staleFixture = New-DisposableWorktreeFixture "stale"
    $staleSnapshot = Invoke-SnapshotProcess -Fixture $staleFixture -Name "merge"
    Assert-Equal $staleSnapshot.ExitCode 0 "stale fixture must first create a snapshot" $staleSnapshot.Output
    $newerSourcePath = Join-Path $staleFixture.Worktree "crates\demo\src\lib.rs"
    [IO.File]::AppendAllText($newerSourcePath, "`n// newer than snapshot", [Text.UTF8Encoding]::new($false))
    [IO.File]::SetLastWriteTimeUtc($newerSourcePath, [DateTime]::UtcNow.AddMinutes(1))
    $staleCheck = Invoke-SnapshotProcess -Fixture $staleFixture -Check
    Assert-True ($staleCheck.ExitCode -ne 0) "stale snapshot check must have a nonzero process exit code" $staleCheck.Output
    Assert-Contains $staleCheck.Output "refs/wip/merge" "stale snapshot output must name the derived reference"

    Write-Host "[5/14] a discovery result with no includable target fails visibly"
    $zeroFixture = New-ZeroTargetFixture
    $zeroCheck = Invoke-SnapshotProcess -Fixture $zeroFixture -Check -EnvironmentVariables @{ PATH = ($zeroFixture.FakeGitDirectory + ";" + $env:PATH) }
    Assert-True ($zeroCheck.ExitCode -ne 0) "zero targets must have a nonzero process exit code" $zeroCheck.Output
    Assert-Contains $zeroCheck.Output "snapshot targets=0" "zero-target check must report zero targets"
    Assert-Contains $zeroCheck.Output "snapshot check completed: targets=0, findings=1" "zero-target check must report an abnormal finding"
    Assert-Contains $zeroCheck.Output "[EXCLUDE]" "zero-target check must disclose why its only worktree was excluded"

    Write-Host "[6/14] an ori3-wt-* worktree snapshots scratchpad files at every depth and of every extension"
    $allFixture = New-DisposableWorktreeFixture "worktree-all-files"
    $allFiles = [ordered]@{
        "a/b/c.py" = "print(1)"
        "d.json" = "{}"
        "e/f.rs" = "fn main() {}"
        "g.md" = "# note"
        "h/i.txt" = "text"
        "j.patch" = "diff"
    }
    $allBytes = New-ScratchpadFiles -Root $allFixture.Worktree -Files $allFiles
    $allSnapshot = Invoke-SnapshotProcess -Fixture $allFixture
    Assert-Equal $allSnapshot.ExitCode 0 "all-files snapshot process must exit 0" $allSnapshot.Output
    foreach ($relative in $allFiles.Keys) {
        Assert-True (Test-GitPathAtRef $allFixture.Repository "refs/wip/merge:scratchpad/$relative") "worktree scratchpad file $relative must be in the refs/wip/merge tree" $allSnapshot.Output
    }
    Assert-Equal ((Get-SnapshotTreeFiles $allFixture.Repository "refs/wip/merge") -join "|") ((Get-ExpectedScratchpadFiles $allFiles) -join "|") "the refs/wip/merge tree must hold exactly the six scratchpad files" $allSnapshot.Output
    Assert-Contains $allSnapshot.Output "[SNAPSHOT-SCRATCHPAD] merge included=6 bytes=$allBytes skipped_dir=0 skipped_ext=0 skipped_size=0" "the worktree must report its scratchpad selection on one line"
    Assert-Contains $allSnapshot.Output "[SNAPSHOT-SCRATCHPAD] main included=0 bytes=0 skipped_dir=0 skipped_ext=0 skipped_size=0" "a snapshot without a scratchpad must still report its line"

    Write-Host "[7/14] a worktree snapshot leaves out build artifacts, archives, target/node_modules/.git folders and oversize files, and says so"
    $skipFixture = New-DisposableWorktreeFixture "worktree-exclusions"
    $skipLimit = 1024
    $keptFiles = [ordered]@{
        "keep/result.json" = "{}"
        "at-limit.json" = ("x" * 1024)
        "no-extension" = "plain"
        "targets/list.json" = "[]"
        "target-notes.txt" = "notes"
        "node_modules_backup/x.json" = "{}"
        "Upper/Case.PY" = "pass"
        "deep/er/est/z.py" = "print(2)"
    }
    $folderSkipped = [ordered]@{
        "target/l.rlib" = "x"
        "node_modules/m.js" = "x"
        "sub/target/n.json" = "x"
        "up/NODE_MODULES/upper.json" = "x"
        "proj/.git/config" = "x"
        "gitlink/.git" = "gitdir: ../elsewhere"
    }
    $extensionSkipped = [ordered]@{
        "k.exe" = "x"
        "ext/UPPER.EXE" = "x"
        "ext/pack.tar.gz" = "x"
        "ext/huge.zip" = ("x" * 2048)
    }
    foreach ($extension in @("exe", "pdb", "dll", "lib", "exp", "ilk", "rlib", "rmeta", "o", "obj", "d", "tar", "zip", "7z", "gz")) {
        $extensionSkipped["ext/f.$extension"] = "x"
    }
    $sizeSkipped = [ordered]@{
        "big.json" = ("x" * 2048)
        "deep/over-by-one.json" = ("x" * 1025)
    }
    $keptBytes = New-ScratchpadFiles -Root $skipFixture.Worktree -Files $keptFiles
    [void](New-ScratchpadFiles -Root $skipFixture.Worktree -Files $folderSkipped)
    [void](New-ScratchpadFiles -Root $skipFixture.Worktree -Files $extensionSkipped)
    [void](New-ScratchpadFiles -Root $skipFixture.Worktree -Files $sizeSkipped)
    $skipSnapshot = Invoke-SnapshotProcess -Fixture $skipFixture -ExtraArguments @("-ScratchpadMaxFileBytes", [string]$skipLimit)
    Assert-Equal $skipSnapshot.ExitCode 0 "exclusion snapshot process must exit 0" $skipSnapshot.Output
    Assert-True (-not (Test-GitPathAtRef $skipFixture.Repository "refs/wip/merge:scratchpad/k.exe")) "a .exe file must stay out of the snapshot" $skipSnapshot.Output
    Assert-True (-not (Test-GitPathAtRef $skipFixture.Repository "refs/wip/merge:scratchpad/target/l.rlib")) "a file below target/ must stay out of the snapshot" $skipSnapshot.Output
    Assert-True (-not (Test-GitPathAtRef $skipFixture.Repository "refs/wip/merge:scratchpad/node_modules/m.js")) "a file below node_modules/ must stay out of the snapshot" $skipSnapshot.Output
    Assert-True (-not (Test-GitPathAtRef $skipFixture.Repository "refs/wip/merge:scratchpad/big.json")) "a file above the size limit must stay out of the snapshot" $skipSnapshot.Output
    Assert-Equal ((Get-SnapshotTreeFiles $skipFixture.Repository "refs/wip/merge") -join "|") ((Get-ExpectedScratchpadFiles $keptFiles) -join "|") "the snapshot must hold exactly the kept files, including the file at the limit and the look-alike names" $skipSnapshot.Output
    Assert-Contains $skipSnapshot.Output "[SNAPSHOT-SCRATCHPAD] merge included=8 bytes=$keptBytes skipped_dir=6 skipped_ext=19 skipped_size=2" "the report must give the included count and bytes and the skipped counts per reason"
    Assert-Contains $skipSnapshot.Output "[SNAPSHOT-SCRATCHPAD-SKIPPED-SIZE] merge scratchpad/big.json bytes=2048 limit=1024" "a file skipped for its size must be named with its bytes"
    Assert-Contains $skipSnapshot.Output "[SNAPSHOT-SCRATCHPAD-SKIPPED-SIZE] merge scratchpad/deep/over-by-one.json bytes=1025 limit=1024" "a file one byte over the limit must be named with its bytes"
    Assert-NotContains $skipSnapshot.Output "scratchpad/ext/huge.zip" "an oversize archive is skipped for its extension and must not be reported as a size skip"
    $defaultLimitSnapshot = Invoke-SnapshotProcess -Fixture $skipFixture
    Assert-Equal $defaultLimitSnapshot.ExitCode 0 "default-limit snapshot process must exit 0" $defaultLimitSnapshot.Output
    Assert-Contains $defaultLimitSnapshot.Output "[SNAPSHOT-SCRATCHPAD] merge included=10 bytes=$($keptBytes + 2048 + 1025) skipped_dir=6 skipped_ext=19 skipped_size=0" "the default 20 MiB limit must keep the files that the small test limit skipped"
    Assert-True (Test-GitPathAtRef $skipFixture.Repository "refs/wip/merge:scratchpad/big.json") "a 2 KiB file must be snapshotted under the default limit" $defaultLimitSnapshot.Output

    Write-Host "[8/14] the repository root keeps the legacy scratchpad rule and has no size limit"
    $legacyFixture = New-DisposableWorktreeFixture "root-legacy"
    $legacyKept = [ordered]@{
        "q.md" = ("q" * 2048)
        "r.txt" = "r"
        "o/s.md" = "s"
        "t.patch" = "t"
    }
    $legacySkipped = [ordered]@{
        "n.py" = "n"
        "o/p.txt" = "p"
        "u/v.patch" = "v"
    }
    $legacyKeptBytes = New-ScratchpadFiles -Root $legacyFixture.Repository -Files $legacyKept
    [void](New-ScratchpadFiles -Root $legacyFixture.Repository -Files $legacySkipped)
    $legacySnapshot = Invoke-SnapshotProcess -Fixture $legacyFixture -ExtraArguments @("-ScratchpadMaxFileBytes", "1024")
    Assert-Equal $legacySnapshot.ExitCode 0 "legacy-rule snapshot process must exit 0" $legacySnapshot.Output
    Assert-True (-not (Test-GitPathAtRef $legacyFixture.Repository "refs/wip/main:scratchpad/n.py")) "a .py file of the repository root must stay out of refs/wip/main" $legacySnapshot.Output
    Assert-True (-not (Test-GitPathAtRef $legacyFixture.Repository "refs/wip/main:scratchpad/o/p.txt")) "a nested .txt file of the repository root must stay out of refs/wip/main" $legacySnapshot.Output
    Assert-True (Test-GitPathAtRef $legacyFixture.Repository "refs/wip/main:scratchpad/q.md") "a .md file of the repository root must be in refs/wip/main even above the worktree size limit" $legacySnapshot.Output
    Assert-True (Test-GitPathAtRef $legacyFixture.Repository "refs/wip/main:scratchpad/r.txt") "a direct .txt file of the repository root must be in refs/wip/main" $legacySnapshot.Output
    Assert-Equal ((Get-SnapshotTreeFiles $legacyFixture.Repository "refs/wip/main") -join "|") ((Get-ExpectedScratchpadFiles $legacyKept) -join "|") "refs/wip/main must hold exactly the files that the legacy rule selects" $legacySnapshot.Output
    Assert-Contains $legacySnapshot.Output "[SNAPSHOT-SCRATCHPAD] main included=4 bytes=$legacyKeptBytes skipped_dir=0 skipped_ext=3 skipped_size=0" "the repository root report must count its unselected files as skipped_ext"

    Write-Host "[9/14] -Check treats the selected scratchpad files of a worktree as sources, and nothing else"
    $scratchFreshFixture = New-DisposableWorktreeFixture "scratchpad-freshness"
    $worktreeScratchpad = Join-Path $scratchFreshFixture.Worktree "scratchpad"
    $rootScratchNote = Join-Path $scratchFreshFixture.Repository "scratchpad\q.md"
    New-TestFile (Join-Path $worktreeScratchpad "work\notes.json") "{}"
    New-TestFile (Join-Path $worktreeScratchpad "k.exe") "binary"
    New-TestFile (Join-Path $worktreeScratchpad "target\out.json") "{}"
    New-TestFile $rootScratchNote "root note"
    $scratchSave = Invoke-SnapshotProcess -Fixture $scratchFreshFixture
    Assert-Equal $scratchSave.ExitCode 0 "scratchpad freshness fixture must first create its snapshots" $scratchSave.Output
    $scratchBaseline = Invoke-SnapshotProcess -Fixture $scratchFreshFixture -Check
    Assert-Equal $scratchBaseline.ExitCode 0 "a snapshot taken after the last scratchpad change must pass -Check" $scratchBaseline.Output
    [IO.File]::SetLastWriteTimeUtc($rootScratchNote, [DateTime]::UtcNow.AddMinutes(1))
    $rootTouched = Invoke-SnapshotProcess -Fixture $scratchFreshFixture -Check
    Assert-Equal $rootTouched.ExitCode 0 "a newer scratchpad file of the repository root must not change the verdict" $rootTouched.Output
    [IO.File]::SetLastWriteTimeUtc((Join-Path $worktreeScratchpad "k.exe"), [DateTime]::UtcNow.AddMinutes(1))
    [IO.File]::SetLastWriteTimeUtc((Join-Path $worktreeScratchpad "target\out.json"), [DateTime]::UtcNow.AddMinutes(1))
    $excludedTouched = Invoke-SnapshotProcess -Fixture $scratchFreshFixture -Check
    Assert-Equal $excludedTouched.ExitCode 0 "newer worktree scratchpad files that the selection skips must not change the verdict" $excludedTouched.Output
    [IO.File]::SetLastWriteTimeUtc((Join-Path $worktreeScratchpad "work\notes.json"), [DateTime]::UtcNow.AddMinutes(1))
    $selectedTouched = Invoke-SnapshotProcess -Fixture $scratchFreshFixture -Check
    Assert-True ($selectedTouched.ExitCode -ne 0) "a newer selected worktree scratchpad file must make -Check fail" $selectedTouched.Output
    Assert-Contains $selectedTouched.Output "refs/wip/merge is stale" "the stale worktree snapshot must be named"
    Assert-Contains $selectedTouched.Output "notes.json" "the report must name the newer scratchpad file"
    Assert-Contains $selectedTouched.Output "[OK] main:" "the repository root must keep passing in the same run"
    $scratchResave = Invoke-SnapshotProcess -Fixture $scratchFreshFixture
    Assert-Equal $scratchResave.ExitCode 0 "saving again after the scratchpad change must exit 0" $scratchResave.Output
    $scratchRecheck = Invoke-SnapshotProcess -Fixture $scratchFreshFixture -Check
    Assert-Equal $scratchRecheck.ExitCode 0 "a fresh save must pass -Check again, so save and check agree on the selection" $scratchRecheck.Output

    Write-Host "[10/14] -Check reports a worktree whose tracked files vanished, and a save leaves its snapshot alone"
    $vanishFixture = New-DisposableWorktreeFixture "vanished"
    $vanishSave = Invoke-SnapshotProcess -Fixture $vanishFixture
    Assert-Equal $vanishSave.ExitCode 0 "vanish fixture must first create its snapshots" $vanishSave.Output
    $savedCommit = (Invoke-TestGit $vanishFixture.Repository @("rev-parse", "--verify", "refs/wip/merge")).Trim()
    $worktreeHead = (Invoke-TestGit $vanishFixture.Repository @("rev-parse", "HEAD")).Trim()
    $vanishHealthy = Invoke-SnapshotProcess -Fixture $vanishFixture -Check
    Assert-Equal $vanishHealthy.ExitCode 0 "an intact worktree must pass -Check" $vanishHealthy.Output
    Assert-NotContains $vanishHealthy.Output "contents vanished" "an intact worktree must not be reported as vanished"
    # The fixture tracks four files. The rule is "strictly more than half are missing".
    $trackedInWorktree = @("docs\guide.md", "crates\demo\src\lib.rs", ".gitignore", "scripts\snapshot-worktrees.ps1")
    Remove-Item -LiteralPath (Join-Path $vanishFixture.Worktree $trackedInWorktree[0]) -Force
    $oneMissing = Invoke-SnapshotProcess -Fixture $vanishFixture -Check
    Assert-Equal $oneMissing.ExitCode 0 "one of four tracked files missing (25 percent) must pass -Check" $oneMissing.Output
    Assert-NotContains $oneMissing.Output "contents vanished" "one missing tracked file is not a vanished worktree"
    Remove-Item -LiteralPath (Join-Path $vanishFixture.Worktree $trackedInWorktree[1]) -Force
    $halfMissing = Invoke-SnapshotProcess -Fixture $vanishFixture -Check
    Assert-Equal $halfMissing.ExitCode 0 "exactly half of the tracked files missing must pass -Check" $halfMissing.Output
    Assert-NotContains $halfMissing.Output "contents vanished" "exactly half missing is not a vanished worktree"
    Remove-Item -LiteralPath (Join-Path $vanishFixture.Worktree $trackedInWorktree[2]) -Force
    $mostMissing = Invoke-SnapshotProcess -Fixture $vanishFixture -Check
    Assert-True ($mostMissing.ExitCode -ne 0) "three of four tracked files missing must fail -Check" $mostMissing.Output
    Assert-Contains $mostMissing.Output "worktree contents vanished: 3/4 tracked files are missing" "the report must give the missing and tracked counts"
    Assert-Contains $mostMissing.Output "refs/wip/merge=$savedCommit" "the report must give the snapshot commit"
    Assert-Contains $mostMissing.Output "git archive $worktreeHead" "the report must give the worktree HEAD archive step"
    Assert-Contains $mostMissing.Output "git archive refs/wip/merge" "the report must give the snapshot archive step"
    Assert-Contains $mostMissing.Output "git hash-object" "the report must give the verification step"
    Assert-Contains $mostMissing.Output "[OK] main:" "the repository root must keep passing in the same run"
    Remove-Item -LiteralPath (Join-Path $vanishFixture.Worktree $trackedInWorktree[3]) -Force
    $allMissing = Invoke-SnapshotProcess -Fixture $vanishFixture -Check
    Assert-True ($allMissing.ExitCode -ne 0) "all tracked files missing must fail -Check" $allMissing.Output
    Assert-Contains $allMissing.Output "worktree contents vanished: 4/4 tracked files are missing" "the report must give 4/4 when everything is gone"
    Assert-NotContains $allMissing.Output "[SKIP] merge" "a vanished worktree must not be skipped silently as having no source"
    Remove-Item -LiteralPath (Join-Path $vanishFixture.Worktree ".git") -Force
    $noLink = Invoke-SnapshotProcess -Fixture $vanishFixture -Check
    Assert-True ($noLink.ExitCode -ne 0) "a worktree folder without its .git link file must fail -Check" $noLink.Output
    Assert-Contains $noLink.Output "the .git link file is missing" "the report must say why the folder could not be examined"
    Remove-Item -LiteralPath $vanishFixture.Worktree -Recurse -Force
    $noFolder = Invoke-SnapshotProcess -Fixture $vanishFixture -Check
    Assert-True ($noFolder.ExitCode -ne 0) "a registered worktree whose folder is gone must fail -Check" $noFolder.Output
    Assert-Contains $noFolder.Output "4/4 tracked files are missing" "the report must give 4/4 when the folder is gone"
    Assert-Contains $noFolder.Output "the worktree folder is missing" "the report must say that the folder is gone"
    $guardedSave = Invoke-SnapshotProcess -Fixture $vanishFixture
    Assert-True ($guardedSave.ExitCode -ne 0) "a save that meets a vanished worktree must exit nonzero" $guardedSave.Output
    Assert-Contains $guardedSave.Output "worktree contents vanished" "the save must report the vanished worktree"
    Assert-Equal (Invoke-TestGit $vanishFixture.Repository @("rev-parse", "--verify", "refs/wip/merge")).Trim() $savedCommit "a save must not overwrite the snapshot of a vanished worktree" $guardedSave.Output
    Assert-Contains $guardedSave.Output "[OK] main ->" "a vanished worktree must not stop the other snapshots"
    Invoke-TestGit $vanishFixture.Repository @("update-ref", "-d", "refs/wip/merge") | Out-Null
    $noSnapshotCheck = Invoke-SnapshotProcess -Fixture $vanishFixture -Check
    Assert-Contains $noSnapshotCheck.Output "refs/wip/merge is MISSING" "the report must say when no snapshot exists to restore from"

    Write-Host "[11/14] a selected scratchpad file that git silently refuses to index is reported, not counted as saved"
    $nestedFixture = New-DisposableWorktreeFixture "nested-repository"
    New-TestFile (Join-Path $nestedFixture.Worktree "scratchpad\ok.json") "{}"
    New-TestFile (Join-Path $nestedFixture.Worktree "scratchpad\clone\work.json") "{}"
    Invoke-TestGit (Join-Path $nestedFixture.Worktree "scratchpad\clone") @("init", "--quiet") | Out-Null
    $nestedSnapshot = Invoke-SnapshotProcess -Fixture $nestedFixture
    Assert-True ($nestedSnapshot.ExitCode -ne 0) "a snapshot that loses a selected scratchpad file must exit nonzero" $nestedSnapshot.Output
    Assert-Contains $nestedSnapshot.Output "2 scratchpad files were selected but only 1 reached the snapshot index" "the loss must be reported with both counts"
    Assert-True (Test-GitPathAtRef $nestedFixture.Repository "refs/wip/merge:scratchpad/ok.json") "the files git can index must still be snapshotted" $nestedSnapshot.Output
    Assert-True (-not (Test-GitPathAtRef $nestedFixture.Repository "refs/wip/merge:scratchpad/clone/work.json")) "a file inside a nested git repository is not in the snapshot" $nestedSnapshot.Output
    Assert-Contains $nestedSnapshot.Output "[OK] main ->" "the other snapshots must still be written"

    Write-Host "[12/14] a worktree that git cannot read is reported as a finding, and a save skips it"
    $unreadableFixture = New-DisposableWorktreeFixture "unreadable-worktree"
    $unreadableSave = Invoke-SnapshotProcess -Fixture $unreadableFixture
    Assert-Equal $unreadableSave.ExitCode 0 "unreadable-worktree fixture must first create its snapshots" $unreadableSave.Output
    $unreadableCommit = (Invoke-TestGit $unreadableFixture.Repository @("rev-parse", "--verify", "refs/wip/merge")).Trim()
    # Point the link file at a git directory that does not exist: git cannot answer inside it.
    # git marks the link file hidden, which makes an in-place overwrite fail, so replace it.
    $unreadableLink = Join-Path $unreadableFixture.Worktree ".git"
    Remove-Item -LiteralPath $unreadableLink -Force
    [IO.File]::WriteAllText($unreadableLink, "gitdir: C:/ori3-snapshot-test-no-such-gitdir`n", [Text.UTF8Encoding]::new($false))
    $unreadableCheck = Invoke-SnapshotProcess -Fixture $unreadableFixture -Check
    Assert-True ($unreadableCheck.ExitCode -ne 0) "-Check must fail for a worktree that git cannot read" $unreadableCheck.Output
    Assert-Contains $unreadableCheck.Output "cannot verify that the worktree contents are present" "the report must say that the worktree could not be verified"
    Assert-Contains $unreadableCheck.Output "[OK] main:" "the repository root must keep passing in the same run"
    $unreadableResave = Invoke-SnapshotProcess -Fixture $unreadableFixture
    Assert-True ($unreadableResave.ExitCode -ne 0) "a save that meets an unreadable worktree must exit nonzero" $unreadableResave.Output
    Assert-Equal (Invoke-TestGit $unreadableFixture.Repository @("rev-parse", "--verify", "refs/wip/merge")).Trim() $unreadableCommit "a save must leave the snapshot of an unreadable worktree alone" $unreadableResave.Output
    Assert-Contains $unreadableResave.Output "[OK] main ->" "an unreadable worktree must not stop the other snapshots"

    Write-Host "[13/14] a save leaves the snapshot of a worktree emptied by the temp cleanup alone, and fails loudly"
    $emptiedFixture = New-DisposableWorktreeFixture "emptied-worktree"
    $emptiedHead = (Invoke-TestGit $emptiedFixture.Repository @("rev-parse", "HEAD")).Trim()
    $emptiedFirst = Invoke-SnapshotProcess -Fixture $emptiedFixture
    Assert-Equal $emptiedFirst.ExitCode 0 "the emptied-worktree fixture must first create its snapshots" $emptiedFirst.Output
    # One deliberate deletion (1 of 4 tracked files) is an ordinary change and is still saved.
    Remove-Item -LiteralPath (Join-Path $emptiedFixture.Worktree "docs\guide.md") -Force
    $emptiedSmall = Invoke-SnapshotProcess -Fixture $emptiedFixture
    Assert-Equal $emptiedSmall.ExitCode 0 "a save after one deliberate deletion must exit 0" $emptiedSmall.Output
    $goodSnapshot = (Invoke-TestGit $emptiedFixture.Repository @("rev-parse", "--verify", "refs/wip/merge")).Trim()
    Assert-True (-not (Test-GitPathAtRef $emptiedFixture.Repository "refs/wip/merge:docs/guide.md")) "the deliberate deletion must be recorded in the new snapshot" $emptiedSmall.Output
    Assert-True (Test-GitPathAtRef $emptiedFixture.Repository "refs/wip/merge:crates/demo/src/lib.rs") "the files that remain must be in the new snapshot" $emptiedSmall.Output
    # The 2026-10-04 shape: every tracked file is gone and only the .git link file remains.
    foreach ($relative in @("crates\demo\src\lib.rs", ".gitignore", "scripts\snapshot-worktrees.ps1")) {
        Remove-Item -LiteralPath (Join-Path $emptiedFixture.Worktree $relative) -Force
    }
    $emptiedSave = Invoke-SnapshotProcess -Fixture $emptiedFixture
    Assert-True ($emptiedSave.ExitCode -ne 0) "a save that meets an emptied worktree must exit nonzero" $emptiedSave.Output
    Assert-Equal (Invoke-TestGit $emptiedFixture.Repository @("rev-parse", "--verify", "refs/wip/merge")).Trim() $goodSnapshot "a save must not move refs/wip/merge for an emptied worktree" $emptiedSave.Output
    Assert-True (Test-GitPathAtRef $emptiedFixture.Repository "refs/wip/merge:crates/demo/src/lib.rs") "the good snapshot must still hold the tracked files" $emptiedSave.Output
    Assert-Contains $emptiedSave.Output "[NG] merge: worktree contents vanished: 4/4 tracked files are missing" "the save must name the emptied worktree and the counts"
    Assert-Contains $emptiedSave.Output "[RESTORE] merge:" "the save must print the restore procedure"
    Assert-Contains $emptiedSave.Output "[OK] main ->" "an emptied worktree must not stop the other snapshots"
    # A snapshot that an older save wrote after the files vanished has an empty tree. Recognise it,
    # so that nobody restores from it by mistake.
    $emptyTreeId = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
    $emptiedByOldSave = (Invoke-TestGit $emptiedFixture.Repository @("commit-tree", $emptyTreeId, "-p", $emptiedHead, "-m", "emptied by an old save")).Trim()
    Invoke-TestGit $emptiedFixture.Repository @("update-ref", "refs/wip/merge", $emptiedByOldSave) | Out-Null
    $emptiedCheck = Invoke-SnapshotProcess -Fixture $emptiedFixture -Check
    Assert-True ($emptiedCheck.ExitCode -ne 0) "-Check must fail for an emptied worktree whose snapshot is empty too" $emptiedCheck.Output
    Assert-Contains $emptiedCheck.Output "this snapshot holds only 0 files (HEAD tracks 4)" "the report must say that the snapshot itself is nearly empty"
    Assert-Contains $emptiedCheck.Output "refs/wip/merge^2" "the report must point at the second parent as the place to look for the previous snapshot"

    Write-Host "[14/14] every snapshot keeps the previous snapshot reachable as its second parent"
    $parentFixture = New-DisposableWorktreeFixture "snapshot-parents"
    $parentHead = (Invoke-TestGit $parentFixture.Repository @("rev-parse", "HEAD")).Trim()
    $parentFirst = Invoke-SnapshotProcess -Fixture $parentFixture
    Assert-Equal $parentFirst.ExitCode 0 "the first snapshot must exit 0" $parentFirst.Output
    $firstSnapshot = (Invoke-TestGit $parentFixture.Repository @("rev-parse", "--verify", "refs/wip/merge")).Trim()
    Assert-Equal (Get-CommitParents $parentFixture.Repository "refs/wip/merge") $parentHead "a first snapshot has only the worktree HEAD as its parent" $parentFirst.Output
    [IO.File]::AppendAllText((Join-Path $parentFixture.Worktree "docs\guide.md"), "`nsecond", [Text.UTF8Encoding]::new($false))
    $parentSecond = Invoke-SnapshotProcess -Fixture $parentFixture
    Assert-Equal $parentSecond.ExitCode 0 "the second snapshot must exit 0" $parentSecond.Output
    $secondSnapshot = (Invoke-TestGit $parentFixture.Repository @("rev-parse", "--verify", "refs/wip/merge")).Trim()
    Assert-Equal (Get-CommitParents $parentFixture.Repository "refs/wip/merge") "$parentHead $firstSnapshot" "the second snapshot must keep the first as its second parent" $parentSecond.Output
    [IO.File]::AppendAllText((Join-Path $parentFixture.Worktree "docs\guide.md"), "`nthird", [Text.UTF8Encoding]::new($false))
    $parentThird = Invoke-SnapshotProcess -Fixture $parentFixture
    Assert-Equal $parentThird.ExitCode 0 "the third snapshot must exit 0" $parentThird.Output
    Assert-Equal (Get-CommitParents $parentFixture.Repository "refs/wip/merge") "$parentHead $secondSnapshot" "the third snapshot must keep the second as its second parent" $parentThird.Output
    $reachable = Invoke-Process -FileName "git" -Arguments @("-C", $parentFixture.Repository, "merge-base", "--is-ancestor", $firstSnapshot, "refs/wip/merge") -WorkingDirectory $parentFixture.Repository
    Assert-Equal $reachable.ExitCode 0 "the first snapshot must still be reachable from the latest one" $reachable.Output
    # When refs/wip/<name> points at the worktree HEAD itself, the HEAD must not be listed twice.
    Invoke-TestGit $parentFixture.Repository @("update-ref", "refs/wip/merge", $parentHead) | Out-Null
    $parentFourth = Invoke-SnapshotProcess -Fixture $parentFixture
    Assert-Equal $parentFourth.ExitCode 0 "a snapshot after a ref that equals the worktree HEAD must exit 0" $parentFourth.Output
    Assert-Equal (Get-CommitParents $parentFixture.Repository "refs/wip/merge") $parentHead "a previous snapshot that equals the worktree HEAD must not become a duplicate parent" $parentFourth.Output
    Assert-NotContains $parentFourth.Output "duplicate parent" "the save must not ask git to record the same parent twice"

    Write-Host "[EVIDENCE] child snapshot exit=$($snapshotResult.ExitCode); fresh check exit=$($freshCheck.ExitCode); missing check exit=$($missingCheck.ExitCode); stale check exit=$($staleCheck.ExitCode); zero-target check exit=$($zeroCheck.ExitCode); all-files snapshot exit=$($allSnapshot.ExitCode); exclusion snapshot exit=$($skipSnapshot.ExitCode); legacy-root snapshot exit=$($legacySnapshot.ExitCode); selected-scratchpad stale check exit=$($selectedTouched.ExitCode); vanished check exit=$($mostMissing.ExitCode); guarded save exit=$($guardedSave.ExitCode); nested-repository snapshot exit=$($nestedSnapshot.ExitCode); unreadable-worktree check exit=$($unreadableCheck.ExitCode); emptied-worktree save exit=$($emptiedSave.ExitCode); snapshot-parents save exit=$($parentFourth.ExitCode)"
    Write-Host "snapshot-worktrees self-test passed: $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
