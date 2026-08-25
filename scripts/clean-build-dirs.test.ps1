[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$ScriptPath = Join-Path $PSScriptRoot "clean-build-dirs.ps1"
$SandboxName = "ori3-clean-build-dirs-test-{0}" -f [Guid]::NewGuid().ToString("N")
$SandboxRoot = Join-Path ([IO.Path]::GetTempPath()) $SandboxName
$script:AssertionCount = 0

function Assert-True {
    param(
        [Parameter(Mandatory = $true)]
        [bool]$Condition,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    $script:AssertionCount += 1
    if (-not $Condition) {
        throw "ASSERTION FAILED: $Message"
    }
}

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        $Actual,

        [Parameter(Mandatory = $true)]
        [AllowNull()]
        $Expected,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    $script:AssertionCount += 1
    if ($Actual -ne $Expected) {
        throw "ASSERTION FAILED: $Message (expected=$Expected, actual=$Actual)"
    }
}

function New-TestFixture {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $root = Join-Path $SandboxRoot $Name
    $tempRoot = Join-Path $root "temp"
    $repositoryRoot = Join-Path $root "repo"
    [void][IO.Directory]::CreateDirectory($tempRoot)
    [void][IO.Directory]::CreateDirectory((Join-Path $repositoryRoot "verification"))

    [pscustomobject]@{
        Root = $root
        TempRoot = $tempRoot
        RepositoryRoot = $repositoryRoot
    }
}

function New-TestFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [DateTime]$LastWriteTimeUtc,

        [string]$Content = "fixture"
    )

    $parent = Split-Path -Parent $Path
    [void][IO.Directory]::CreateDirectory($parent)
    [IO.File]::WriteAllText($Path, $Content, [Text.UTF8Encoding]::new($false))
    [IO.File]::SetLastWriteTimeUtc($Path, $LastWriteTimeUtc)
}

function Set-OldDirectoryTimestamp {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    [IO.Directory]::SetLastWriteTimeUtc($Path, [DateTime]::UtcNow.AddDays(-2))
}

function Invoke-Cleaner {
    param(
        [Parameter(Mandatory = $true)]
        $Fixture,

        [string]$DeletePath
    )

    if (-not (Test-Path -LiteralPath $ScriptPath -PathType Leaf)) {
        throw "Required implementation is missing: $ScriptPath"
    }

    $parameters = @{
        TempRoot = $Fixture.TempRoot
        RepositoryRoot = $Fixture.RepositoryRoot
        ProtectHours = 6
        TestSandboxRoot = $SandboxRoot
    }
    if (-not [string]::IsNullOrWhiteSpace($DeletePath)) {
        $parameters.DeletePath = $DeletePath
        $parameters.Confirm = $false
    }

    @(& $ScriptPath @parameters)
}

function Get-SingleResult {
    param(
        [Parameter(Mandatory = $true)]
        [object[]]$Results,

        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $fullPath = [IO.Path]::GetFullPath($Path).TrimEnd([char[]]"\/")
    $matches = @($Results | Where-Object {
        [IO.Path]::GetFullPath([string]$_.Path).TrimEnd([char[]]"\/") -eq $fullPath
    })
    Assert-Equal $matches.Count 1 "candidate should produce exactly one result: $Path"
    $matches[0]
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $SandboxRoot)) {
        return
    }

    $fullSandbox = [IO.Path]::GetFullPath($SandboxRoot).TrimEnd([char[]]"\/")
    $fullTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
    $expectedParent = [IO.Path]::GetDirectoryName($fullSandbox)
    $leaf = [IO.Path]::GetFileName($fullSandbox)
    if (($expectedParent -ne $fullTemp) -or
        (-not [regex]::IsMatch($leaf, "^ori3-clean-build-dirs-test-[0-9a-f]{32}$", [Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
        throw "Refusing unsafe self-test cleanup: $fullSandbox"
    }

    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

[void][IO.Directory]::CreateDirectory($SandboxRoot)
$heldStream = $null
$helperProcess = $null

try {
    $old = [DateTime]::UtcNow.AddHours(-12)
    $recent = [DateTime]::UtcNow.AddMinutes(-5)

    Write-Host "[1/10] recent release/desktop.exe protects an old directory"
    $fixture = New-TestFixture "recent-desktop"
    $candidate = Join-Path $fixture.TempRoot "ori3-target-recent-desktop"
    New-TestFile (Join-Path $candidate "release\desktop.exe") $recent
    Set-OldDirectoryTimestamp $candidate
    $result = Get-SingleResult (Invoke-Cleaner $fixture -DeletePath $candidate) $candidate
    Assert-Equal $result.Decision "Keep" "recent desktop.exe must be kept"
    Assert-True (Test-Path -LiteralPath $candidate -PathType Container) "recent desktop.exe directory must still exist"
    Assert-True (@($result.ReasonCodes) -contains "RecentDesktopExecutable") "reason must identify the recent desktop executable"

    Write-Host "[2/10] a recent inner file protects an old directory"
    $fixture = New-TestFixture "recent-inner-file"
    $candidate = Join-Path $fixture.TempRoot "ori3-target-recent-inner"
    New-TestFile (Join-Path $candidate "debug\build\fresh-output.bin") $recent
    Set-OldDirectoryTimestamp $candidate
    $result = Get-SingleResult (Invoke-Cleaner $fixture -DeletePath $candidate) $candidate
    Assert-Equal $result.Decision "Keep" "a recent inner file must be kept even when the directory timestamp is old"
    Assert-True (Test-Path -LiteralPath $candidate -PathType Container) "recent inner-file directory must still exist"
    Assert-True (@($result.ReasonCodes) -contains "RecentInnerFile") "reason must identify the recent inner file"

    Write-Host "[3/10] default invocation is preview-only"
    $fixture = New-TestFixture "preview-only"
    $candidate = Join-Path $fixture.TempRoot "ori3-target-preview"
    New-TestFile (Join-Path $candidate "old.bin") $old
    Set-OldDirectoryTimestamp $candidate
    $result = Get-SingleResult (Invoke-Cleaner $fixture) $candidate
    Assert-Equal $result.Decision "WouldDelete" "default invocation must only report an eligible directory"
    Assert-True (Test-Path -LiteralPath $candidate -PathType Container) "default invocation must not delete anything"

    Write-Host "[4/10] explicit confirmation deletes only an eligible isolated fixture"
    $fixture = New-TestFixture "confirmed-delete"
    $candidate = Join-Path $fixture.TempRoot "ori3-target-delete"
    $nonCandidate = Join-Path $fixture.TempRoot "do-not-delete"
    $nestedCandidate = Join-Path $nonCandidate "ori3-target-nested"
    $outsideCanary = Join-Path $fixture.Root "outside-canary.bin"
    New-TestFile (Join-Path $candidate "old.bin") $old
    New-TestFile (Join-Path $nonCandidate "keep.bin") $old "keep-sibling"
    New-TestFile (Join-Path $nestedCandidate "keep.bin") $old "keep-nested"
    New-TestFile $outsideCanary $old "keep-outside"
    Set-OldDirectoryTimestamp $candidate
    $result = Get-SingleResult (Invoke-Cleaner $fixture -DeletePath $candidate) $candidate
    Assert-Equal $result.Decision "Deleted" "explicit confirmation should delete an eligible fixture"
    Assert-True (-not (Test-Path -LiteralPath $candidate)) "confirmed eligible fixture should be deleted"
    Assert-Equal ([IO.File]::ReadAllText((Join-Path $nonCandidate "keep.bin"))) "keep-sibling" "a non-candidate sibling must not be touched"
    Assert-Equal ([IO.File]::ReadAllText((Join-Path $nestedCandidate "keep.bin"))) "keep-nested" "a nested matching name must not be touched"
    Assert-Equal ([IO.File]::ReadAllText($outsideCanary)) "keep-outside" "a file outside candidate roots must not be touched"

    Write-Host "[5/10] .git metadata is an unconditional protection"
    $fixture = New-TestFixture "git-protection"
    $candidate = Join-Path $fixture.TempRoot "ori3-target-git"
    New-TestFile (Join-Path $candidate "old.bin") $old
    New-TestFile (Join-Path $candidate ".git\HEAD") $old "ref: refs/heads/main"
    Set-OldDirectoryTimestamp $candidate
    $result = Get-SingleResult (Invoke-Cleaner $fixture -DeletePath $candidate) $candidate
    Assert-Equal $result.Decision "Keep" ".git metadata must prevent deletion"
    Assert-True (@($result.ReasonCodes) -contains "ContainsGitMetadata") "reason must identify .git metadata"
    Assert-True (Test-Path -LiteralPath $candidate -PathType Container) ".git fixture must still exist"

    Write-Host "[6/10] an exclusively opened file is treated as active writing/use without partial deletion"
    $fixture = New-TestFixture "exclusive-lock"
    $candidate = Join-Path $fixture.TempRoot "ori3-target-locked"
    $lockedPath = Join-Path $candidate "locked.bin"
    $unlockedCanary = Join-Path $candidate "a-unlocked-canary.bin"
    New-TestFile $lockedPath $old
    New-TestFile $unlockedCanary $old "must-remain"
    Set-OldDirectoryTimestamp $candidate
    $heldStream = [IO.File]::Open($lockedPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    $result = Get-SingleResult (Invoke-Cleaner $fixture -DeletePath $candidate) $candidate
    Assert-Equal $result.Decision "Keep" "an exclusively opened file must prevent deletion"
    Assert-True (@($result.ReasonCodes) -contains "ActiveWriteOrLock") "reason must identify active writing or use"
    Assert-True (Test-Path -LiteralPath $candidate -PathType Container) "locked fixture must still exist"
    Assert-Equal ([IO.File]::ReadAllText($unlockedCanary)) "must-remain" "no unlocked file may be partially deleted after a lock is found"
    $heldStream.Dispose()
    $heldStream = $null

    Write-Host "[7/10] verification target, ci, and push-tree candidates are all counted"
    $fixture = New-TestFixture "verification-candidates"
    $verification = Join-Path $fixture.RepositoryRoot "verification"
    $targetCandidate = Join-Path $verification "target-old"
    $ciCandidate = Join-Path $verification "ci-old"
    $pushTreeCandidate = Join-Path $verification "push-tree"
    New-TestFile (Join-Path $targetCandidate "old.bin") $old
    New-TestFile (Join-Path $ciCandidate "old.bin") $old
    New-TestFile (Join-Path $pushTreeCandidate ".git") $old "gitdir: nowhere"
    Set-OldDirectoryTimestamp $targetCandidate
    Set-OldDirectoryTimestamp $ciCandidate
    Set-OldDirectoryTimestamp $pushTreeCandidate
    $results = Invoke-Cleaner $fixture
    Assert-Equal @($results | Where-Object Source -eq "VerificationTarget").Count 1 "verification/target-* must be counted"
    Assert-Equal @($results | Where-Object Source -eq "VerificationCi").Count 1 "verification/ci-* must be counted"
    Assert-Equal @($results | Where-Object Source -eq "PushTree").Count 1 "verification/push-tree must be counted"
    $pushResult = Get-SingleResult $results $pushTreeCandidate
    Assert-Equal $pushResult.Decision "Keep" "push-tree with .git metadata must be kept"

    Write-Host "[8/10] a running executable under a candidate protects the directory"
    $fixture = New-TestFixture "running-process"
    $candidate = Join-Path $fixture.TempRoot "ori3-target-running"
    $helperPath = Join-Path $candidate "release\fixture-worker.exe"
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $helperPath))
    [IO.File]::Copy((Join-Path $env:SystemRoot "System32\cmd.exe"), $helperPath)
    [IO.File]::SetLastWriteTimeUtc($helperPath, $old)
    Set-OldDirectoryTimestamp $candidate
    $helperProcess = Start-Process -FilePath $helperPath -ArgumentList '/d /c "ping.exe -n 6 127.0.0.1 >nul"' -WindowStyle Hidden -PassThru
    Start-Sleep -Milliseconds 150
    Assert-True (-not $helperProcess.HasExited) "fixture worker must still be running during the safety check"
    $result = Get-SingleResult (Invoke-Cleaner $fixture -DeletePath $candidate) $candidate
    Assert-Equal $result.Decision "Keep" "a running executable under the candidate must prevent deletion"
    Assert-True (@($result.ReasonCodes) -contains "RunningProcess") "reason must identify the running process"
    Assert-True (Test-Path -LiteralPath $candidate -PathType Container) "running-process fixture must still exist"
    [void]$helperProcess.WaitForExit(10000)
    Assert-True $helperProcess.HasExited "fixture worker must exit naturally"
    $helperProcess.Dispose()
    $helperProcess = $null

    Write-Host "[9/10] verification evidence files are never deleted"
    $fixture = New-TestFixture "verification-evidence"
    $candidate = Join-Path $fixture.RepositoryRoot "verification\target-evidence"
    $evidencePath = Join-Path $candidate "witness.png"
    New-TestFile (Join-Path $candidate "old.bin") $old
    New-TestFile $evidencePath $old "evidence-bytes"
    Set-OldDirectoryTimestamp $candidate
    $result = Get-SingleResult (Invoke-Cleaner $fixture -DeletePath $candidate) $candidate
    Assert-Equal $result.Decision "Keep" "verification evidence must prevent deletion"
    Assert-True (@($result.ReasonCodes) -contains "ContainsEvidenceFiles") "reason must identify protected evidence"
    Assert-Equal ([IO.File]::ReadAllText($evidencePath)) "evidence-bytes" "verification evidence bytes must remain unchanged"

    Write-Host "[10/10] a path outside the exact direct-child allowlist is rejected before deletion"
    $fixture = New-TestFixture "invalid-delete-path"
    $candidate = Join-Path $fixture.TempRoot "ori3-target-valid"
    $invalidPath = Join-Path $fixture.TempRoot "not-a-build-target"
    New-TestFile (Join-Path $candidate "old.bin") $old "valid-must-remain"
    New-TestFile (Join-Path $invalidPath "old.bin") $old "invalid-must-remain"
    Set-OldDirectoryTimestamp $candidate
    Set-OldDirectoryTimestamp $invalidPath
    $rejected = $false
    try {
        Invoke-Cleaner $fixture -DeletePath $invalidPath | Out-Null
    }
    catch {
        $rejected = $true
    }
    Assert-True $rejected "a non-candidate -DeletePath must be rejected"
    Assert-Equal ([IO.File]::ReadAllText((Join-Path $candidate "old.bin"))) "valid-must-remain" "validation failure must happen before another candidate can be deleted"
    Assert-Equal ([IO.File]::ReadAllText((Join-Path $invalidPath "old.bin"))) "invalid-must-remain" "an invalid requested path must never be deleted"

    Write-Host "clean-build-dirs self-test passed: $script:AssertionCount assertions"
}
finally {
    if ($null -ne $helperProcess) {
        [void]$helperProcess.WaitForExit(10000)
        $helperProcess.Dispose()
    }
    if ($null -ne $heldStream) {
        $heldStream.Dispose()
    }
    Remove-TestSandbox
}
