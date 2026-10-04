<#
.SYNOPSIS
Snapshots registered worktrees into refs/wip/<derived-name> and checks freshness.

.DESCRIPTION
The target list is derived exclusively from `git worktree list --porcelain`. The
repository root itself, worktrees located under the root's verification/ directory,
and registered check copies whose leaf does not begin ori3-wt- are excluded by
rules. The repository root is always included as refs/wip/main; all assigned
ori3-wt-* worktrees are included.
For example, the directory ori3-wt-merge becomes refs/wip/merge.

Normal execution records each worktree with git plumbing only. -Check requires a
snapshot newer than the latest source file in crates/, apps/, docs/, or scripts/.
The source walk excludes target/, .git/, and node_modules/ directories.

Scratchpad selection (scratchpad/ is ignored by git, so the selected files are added
with git add -f through --pathspec-from-file):
- ori3-wt-* worktrees: every file at every depth, except (a) files below a directory
  named target, node_modules, or .git (a file named .git is skipped too), (b) files
  whose extension is a build artifact or archive (.exe .pdb .dll .lib .exp .ilk .rlib
  .rmeta .o .obj .d .tar .zip .7z .gz, case-insensitive), and (c) files larger than
  -ScratchpadMaxFileBytes. Each skipped file is counted once, in the order a, b, c.
- the repository root (refs/wip/main): the legacy rule, unchanged. .md at every depth;
  .patch and .txt directly under scratchpad/ only. Every other file counts as skipped_ext.
Each snapshot prints one [SNAPSHOT-SCRATCHPAD] line (name, included files, their bytes,
skipped counts per reason) and one line per file skipped for its size (name and bytes).
git silently skips a selected file inside a nested git repository, so the save compares the
selected count with the count that reached the index and exits nonzero when they differ.

For ori3-wt-* worktrees, -Check also counts the newest selected scratchpad file as a
source, so a snapshot older than the latest diagnostic output is reported as stale. The
repository root keeps the source-only freshness rule.

A worktree whose tracked files are mostly absent (more than 50 percent of the files
tracked at its HEAD report " D" in git status) is reported as vanished: -Check exits
nonzero, and normal execution refuses to overwrite its refs/wip snapshot, because
git add -A would record the deletions and replace the good snapshot with an almost
empty tree. The report names the missing/tracked counts, the refs/wip commit, and the
restore procedure. A worktree that git cannot read at all is reported as a finding too.

Each new snapshot commit has the worktree HEAD as its first parent and, when a
refs/wip/<name> already exists and is not the HEAD itself, that previous snapshot as
its second parent. A snapshot that is replaced by mistake therefore stays reachable as
refs/wip/<name>^2 and git gc cannot drop it.

.PARAMETER Name
The derived snapshot name to operate on. Omit it to operate on every target.

.PARAMETER Check
Verify freshness only. Exit nonzero for a missing or stale refs/wip snapshot, or for a
vanished worktree.

.PARAMETER ScratchpadMaxFileBytes
Largest scratchpad file, in bytes, that an ori3-wt-* worktree snapshot includes. The
default is 20 MiB and is defined only here, as the default value of this parameter.
Only the self-test passes a smaller value.
#>
[CmdletBinding()]
param(
    [string]$Name,
    [switch]$Check,
    [string]$RepositoryRoot,
    [ValidateRange(1, [int64]::MaxValue)][int64]$ScratchpadMaxFileBytes = 20971520
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$script:LastGitExitCode = 0
$script:SnapshotFindingCount = 0

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $scriptDirectory = $PSScriptRoot
    if ([string]::IsNullOrWhiteSpace($scriptDirectory)) {
        $scriptPath = $MyInvocation.MyCommand.Path
        if ([string]::IsNullOrWhiteSpace($scriptPath)) {
            throw "Cannot determine the snapshot script location; pass -RepositoryRoot explicitly."
        }
        $scriptDirectory = Split-Path -Parent $scriptPath
    }
    $RepositoryRoot = Join-Path $scriptDirectory ".."
}
$RepositoryRoot = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd([char[]]"\\/")
$ExcludedPaths = @(
    "docs/competitive-review-2026-08-20.md",
    "traditional_crane_math_bundle",
    "traditional_crane_complete_cp.png"
)
# Scratchpad selection for ori3-wt-* worktrees (see .DESCRIPTION).
$ScratchpadExcludedDirectoryNames = @("target", "node_modules", ".git")
$ScratchpadExcludedExtensions = @(".exe", ".pdb", ".dll", ".lib", ".exp", ".ilk", ".rlib", ".rmeta", ".o", ".obj", ".d", ".tar", ".zip", ".7z", ".gz")
# A worktree is reported as vanished when more than this percentage of the files tracked
# at its HEAD are absent from the folder. Why 50: an intentional deletion in a live
# worktree removes a handful of files, while the temp-folder cleanup removes everything
# (measured 2026-10-04: 1000 of 1000 tracked files were missing). Strictly more than half
# is far from both cases, so a small deliberate deletion is not mistaken for a total loss.
$VanishedPercentThreshold = 50

function Invoke-Git {
    param(
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [string]$IndexFile,
        [hashtable]$Environment,
        [switch]$AllowFailure
    )

    $previousIndex = $env:GIT_INDEX_FILE
    $previousEnvironment = @{}
    if ($IndexFile) { $env:GIT_INDEX_FILE = $IndexFile }
    if ($null -ne $Environment) {
        foreach ($key in $Environment.Keys) {
            $previousEnvironment[$key] = [Environment]::GetEnvironmentVariable($key, "Process")
            [Environment]::SetEnvironmentVariable($key, [string]$Environment[$key], "Process")
        }
    }
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        # Stop when the folder cannot be entered. Under Continue the failure would only print an
        # error and git would then run in whatever folder the caller happens to be in, which could
        # snapshot the wrong tree under this worktree's name.
        Push-Location -LiteralPath $WorkingDirectory -ErrorAction Stop
        try {
            $output = & git @Arguments
            $exitCode = $LASTEXITCODE
        }
        finally {
            Pop-Location
        }
    }
    finally {
        $ErrorActionPreference = $previousPreference
        if ($IndexFile) {
            if ($null -eq $previousIndex) { Remove-Item Env:GIT_INDEX_FILE -ErrorAction SilentlyContinue }
            else { $env:GIT_INDEX_FILE = $previousIndex }
        }
        if ($null -ne $Environment) {
            foreach ($key in $Environment.Keys) {
                [Environment]::SetEnvironmentVariable($key, $previousEnvironment[$key], "Process")
            }
        }
    }
    $script:LastGitExitCode = $exitCode
    if ($exitCode -ne 0 -and -not $AllowFailure) {
        throw "git $($Arguments -join ' ') failed with exit code ${exitCode}: $output"
    }
    return ($output | Where-Object { $_ -ne $null } | ForEach-Object { $_.ToString() })
}

function Normalize-DirectoryPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    return [IO.Path]::GetFullPath($Path).TrimEnd([char[]]"\\/")
}

function Test-IsVerificationCopy {
    param([Parameter(Mandatory = $true)][string]$Worktree)

    if (-not $Worktree.StartsWith($RepositoryRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }
    $relative = $Worktree.Substring($RepositoryRoot.Length).TrimStart([char[]]"\\/")
    return $relative -match '^(?i:verification)(?:[\\/]|$)'
}

function ConvertTo-SnapshotName {
    param([Parameter(Mandatory = $true)][string]$Worktree)

    $leaf = Split-Path -Leaf $Worktree
    if ($leaf.StartsWith("ori3-wt-", [StringComparison]::OrdinalIgnoreCase)) {
        $leaf = $leaf.Substring("ori3-wt-".Length)
    }
    $name = [regex]::Replace($leaf, '[^A-Za-z0-9._-]', '-')
    $name = [regex]::Replace($name, '\.{2,}', '.')
    $name = $name.Trim([char[]]".-")
    if ($name.EndsWith(".lock", [StringComparison]::OrdinalIgnoreCase)) { $name = $name + "-worktree" }
    if ([string]::IsNullOrWhiteSpace($name)) {
        throw "Cannot derive a refs/wip name from worktree: $Worktree"
    }
    return $name
}

function Get-WorktreeInventory {
    $lines = Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("worktree", "list", "--porcelain")
    $targets = New-Object System.Collections.Generic.List[object]
    $excluded = New-Object System.Collections.Generic.List[object]
    $names = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    $current = $null
    foreach ($line in $lines) {
        if ($line.StartsWith("HEAD ", [StringComparison]::Ordinal)) {
            # The HEAD line follows its worktree line; keep it for targets only.
            if ($null -ne $current) { $current.Head = $line.Substring("HEAD ".Length).Trim() }
            continue
        }
        if (-not $line.StartsWith("worktree ", [StringComparison]::Ordinal)) { continue }
        $current = $null
        $worktree = Normalize-DirectoryPath $line.Substring("worktree ".Length)
        $isRepositoryRoot = $false
        if ([string]::Equals($worktree, $RepositoryRoot, [StringComparison]::OrdinalIgnoreCase)) {
            $snapshotName = "main"
            $isRepositoryRoot = $true
        }
        elseif (Test-IsVerificationCopy -Worktree $worktree) {
            $excluded.Add([PSCustomObject]@{ Worktree = $worktree; Reason = "under repository verification/ directory" })
            continue
        }
        elseif (-not (Split-Path -Leaf $worktree).StartsWith("ori3-wt-", [StringComparison]::OrdinalIgnoreCase)) {
            $excluded.Add([PSCustomObject]@{ Worktree = $worktree; Reason = "leaf does not follow the assigned ori3-wt-* convention" })
            continue
        }
        else {
            $snapshotName = ConvertTo-SnapshotName $worktree
        }
        if (-not $names.Add($snapshotName)) {
            throw "Multiple worktrees derive the same refs/wip name: $snapshotName"
        }
        # IsRepositoryRoot is decided here, once, by the same comparison that names the
        # snapshot main. Callers pass it on; nothing guesses it from a path string again.
        $current = [PSCustomObject]@{ Name = $snapshotName; Worktree = $worktree; IsRepositoryRoot = $isRepositoryRoot; Head = $null }
        $targets.Add($current)
    }
    return [PSCustomObject]@{ Targets = $targets.ToArray(); Excluded = $excluded.ToArray() }
}

function Get-LatestSourceFile {
    param(
        [Parameter(Mandatory = $true)][string]$Worktree,
        # Result of Get-SnapshotScratchpadPaths for an ori3-wt-* worktree. Its newest selected
        # file competes with the sources. Pass nothing for the repository root.
        [object]$ScratchpadSelection
    )

    $latest = $null
    foreach ($directoryName in @("crates", "apps", "docs", "scripts")) {
        $directory = Join-Path $Worktree $directoryName
        if (-not (Test-Path -LiteralPath $directory -PathType Container)) { continue }
        foreach ($file in @(Get-ChildItem -LiteralPath $directory -File -Recurse -Force -ErrorAction Stop)) {
            $relative = $file.FullName.Substring($directory.Length).TrimStart([char[]]"\\/")
            if ($relative -match '(^|[\\/])(?:target|\.git|node_modules)(?:[\\/]|$)') { continue }
            if ($null -eq $latest -or $file.LastWriteTimeUtc -gt $latest.LastWriteTimeUtc) { $latest = $file }
        }
    }
    if ($null -ne $ScratchpadSelection -and $null -ne $ScratchpadSelection.LatestFile) {
        if ($null -eq $latest -or $ScratchpadSelection.LatestFile.LastWriteTimeUtc -gt $latest.LastWriteTimeUtc) {
            $latest = $ScratchpadSelection.LatestFile
        }
    }
    return $latest
}

function Get-SnapshotCommitId {
    param([Parameter(Mandatory = $true)][string]$SnapshotName)

    $ref = "refs/wip/$SnapshotName"
    $commit = (Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("rev-parse", "--verify", "--quiet", "${ref}^{commit}") -AllowFailure) -join ""
    if ($commit -notmatch '^[0-9a-f]{40}$') { return $null }
    return $commit
}

function Get-SnapshotCommitTimeUtc {
    param([Parameter(Mandatory = $true)][string]$SnapshotName)

    $ref = "refs/wip/$SnapshotName"
    $commit = Get-SnapshotCommitId -SnapshotName $SnapshotName
    if ($null -eq $commit) { return $null }
    $secondsText = (Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("show", "-s", "--format=%ct", $commit)) -join ""
    [long]$seconds = 0
    if (-not [long]::TryParse($secondsText.Trim(), [ref]$seconds)) {
        throw "Cannot read commit timestamp for ${ref}: $secondsText"
    }
    return [DateTimeOffset]::FromUnixTimeSeconds($seconds).UtcDateTime
}

function Get-SnapshotScratchpadPaths {
    param(
        [Parameter(Mandatory = $true)][string]$Worktree,
        # True only for the repository root (refs/wip/main). The caller knows this from the
        # worktree inventory; it is never inferred from the path here.
        [Parameter(Mandatory = $true)][bool]$IsRepositoryRoot,
        [Parameter(Mandatory = $true)][int64]$MaxFileBytes
    )

    $paths = New-Object System.Collections.Generic.List[string]
    $sizeSkipped = New-Object System.Collections.Generic.List[object]
    [int64]$includedBytes = 0
    $skippedDirectory = 0
    $skippedExtension = 0
    $skippedSize = 0
    $latestFile = $null

    $scratchpad = Join-Path $Worktree "scratchpad"
    if (Test-Path -LiteralPath $scratchpad -PathType Container) {
        foreach ($file in @(Get-ChildItem -LiteralPath $scratchpad -File -Recurse -Force -ErrorAction Stop)) {
            $relative = $file.FullName.Substring($scratchpad.Length).TrimStart([char[]]"\\/")
            $displayPath = "scratchpad/" + $relative.Replace("\", "/")
            $extension = $file.Extension
            $reason = $null
            if ($IsRepositoryRoot) {
                # Legacy rule, unchanged: .md at every depth; .patch and .txt only directly
                # under scratchpad/. Whatever does not match counts as skipped_ext.
                $isDirectChild = $relative -notmatch '[\\/]'
                if (-not ($extension -ieq ".md" -or ($isDirectChild -and ($extension -ieq ".patch" -or $extension -ieq ".txt")))) {
                    $reason = "ext"
                }
            }
            else {
                $parts = @($relative -split '[\\/]')
                $inExcludedDirectory = $false
                for ($index = 0; $index -lt ($parts.Count - 1); $index++) {
                    if ($ScratchpadExcludedDirectoryNames -contains $parts[$index]) { $inExcludedDirectory = $true; break }
                }
                if ($inExcludedDirectory -or $parts[$parts.Count - 1] -ieq ".git") { $reason = "dir" }
                elseif ($ScratchpadExcludedExtensions -contains $extension) { $reason = "ext" }
                elseif ($file.Length -gt $MaxFileBytes) { $reason = "size" }
            }
            if ($null -ne $reason) {
                if ($reason -eq "dir") { $skippedDirectory += 1 }
                elseif ($reason -eq "ext") { $skippedExtension += 1 }
                else {
                    $skippedSize += 1
                    $sizeSkipped.Add([PSCustomObject]@{ Path = $displayPath; Bytes = $file.Length })
                }
                continue
            }
            $paths.Add($displayPath)
            $includedBytes += $file.Length
            if ($null -eq $latestFile -or $file.LastWriteTimeUtc -gt $latestFile.LastWriteTimeUtc) { $latestFile = $file }
        }
    }
    return [PSCustomObject]@{
        Paths = $paths.ToArray()
        IncludedCount = $paths.Count
        IncludedBytes = $includedBytes
        SkippedDirectory = $skippedDirectory
        SkippedExtension = $skippedExtension
        SkippedSize = $skippedSize
        SizeSkipped = $sizeSkipped.ToArray()
        LatestFile = $latestFile
    }
}

function Get-WorktreeVanishedState {
    param([Parameter(Mandatory = $true)][object]$Target)

    $state = [PSCustomObject]@{ Vanished = $false; Failed = $false; Tracked = 0; Deleted = 0; Reason = ""; Detail = "" }
    $head = [string]$Target.Head
    if ($head -notmatch '^[0-9a-f]{40}$' -or $head -match '^0{40}$') {
        return $state
    }
    # The tracked count is read from the repository root, so it still works when the
    # worktree folder itself is gone.
    $trackedLines = @(Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("ls-tree", "-r", "--name-only", $head) -AllowFailure)
    if ($script:LastGitExitCode -ne 0) {
        $state.Failed = $true
        $state.Detail = "git ls-tree for HEAD $head exited with $($script:LastGitExitCode)"
        return $state
    }
    $state.Tracked = $trackedLines.Count
    if (-not (Test-Path -LiteralPath $Target.Worktree -PathType Container)) {
        $state.Deleted = $state.Tracked
        $state.Reason = "the worktree folder is missing"
    }
    elseif (-not (Test-Path -LiteralPath (Join-Path $Target.Worktree ".git"))) {
        # Never run git here: without the link file git would search the parent folders and
        # could answer for an unrelated repository.
        $state.Deleted = $state.Tracked
        $state.Reason = "the .git link file is missing"
    }
    else {
        $statusLines = @(Invoke-Git -WorkingDirectory $Target.Worktree -Arguments @("--no-optional-locks", "status", "--porcelain", "--untracked-files=no") -AllowFailure)
        if ($script:LastGitExitCode -ne 0) {
            $state.Failed = $true
            $state.Detail = "git status exited with $($script:LastGitExitCode)"
            return $state
        }
        # " D" = tracked at HEAD but absent from the worktree folder.
        $state.Deleted = @($statusLines | Where-Object { $_.Length -ge 2 -and $_[0] -eq " " -and $_[1] -eq "D" }).Count
        $state.Reason = "git status reports the tracked files as deleted"
    }
    $state.Vanished = (($state.Deleted * 100) -gt ($state.Tracked * $VanishedPercentThreshold))
    return $state
}

function Get-VanishedFindingText {
    param(
        [Parameter(Mandatory = $true)][object]$Target,
        [Parameter(Mandatory = $true)][object]$State
    )

    $name = $Target.Name
    if ($State.Failed) {
        return "${name}: cannot verify that the worktree contents are present: $($State.Detail)"
    }
    $commit = Get-SnapshotCommitId -SnapshotName $name
    if ($null -eq $commit) {
        $snapshotText = "refs/wip/$name is MISSING (only the committed HEAD content can be restored)"
    }
    else {
        $snapshotTime = Get-SnapshotCommitTimeUtc -SnapshotName $name
        $snapshotText = "refs/wip/$name=$commit (snapshot $($snapshotTime.ToString('o')) UTC)"
        # A save that ran after the files vanished (before this guard existed) leaves a snapshot
        # that is itself nearly empty; say so, so that nobody restores from it by mistake.
        $snapshotFileCount = @(Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("ls-tree", "-r", "--name-only", $commit) -AllowFailure).Count
        if (($snapshotFileCount * 100) -le ($State.Tracked * $VanishedPercentThreshold)) {
            $snapshotText += "; this snapshot holds only $snapshotFileCount files (HEAD tracks $($State.Tracked)), so it was written after the files vanished - the previous snapshot is its second parent refs/wip/$name^2 when it has one, or the commit that a tag protects"
        }
    }
    $summary = "${name}: worktree contents vanished: $($State.Deleted)/$($State.Tracked) tracked files are missing from $($Target.Worktree) ($($State.Reason); more than $VanishedPercentThreshold percent); worktree HEAD=$($Target.Head); $snapshotText"
    $restore = "     [RESTORE] ${name}: expand ""git archive $($Target.Head)"" and then ""git archive refs/wip/$name"" in this order into the worktree folder (a new folder when the folder or its .git file is gone), then compare ""git hash-object <file>"" with the blob ids in ""git ls-tree -r refs/wip/$name"". git is only read, never written. See docs/rules/06-*.md section 10.7.26."
    return ($summary + [Environment]::NewLine + $restore)
}

function Save-Snapshot {
    param(
        [Parameter(Mandatory = $true)][string]$SnapshotName,
        [Parameter(Mandatory = $true)][string]$Worktree,
        [Parameter(Mandatory = $true)][bool]$IsRepositoryRoot
    )

    $indexFile = Join-Path ([IO.Path]::GetTempPath()) ("ori3-snapshot-" + [Guid]::NewGuid().ToString("N") + ".index")
    $pathspecFile = $null
    try {
        Invoke-Git -WorkingDirectory $Worktree -Arguments @("read-tree", "HEAD") -IndexFile $indexFile | Out-Null
        Invoke-Git -WorkingDirectory $Worktree -Arguments @("add", "-A", ".") -IndexFile $indexFile -AllowFailure | Out-Null
        foreach ($path in $ExcludedPaths) {
            Invoke-Git -WorkingDirectory $Worktree -Arguments @("rm", "-r", "-q", "--cached", "--ignore-unmatch", $path) -IndexFile $indexFile -AllowFailure | Out-Null
        }
        $scratchpad = Get-SnapshotScratchpadPaths -Worktree $Worktree -IsRepositoryRoot $IsRepositoryRoot -MaxFileBytes $ScratchpadMaxFileBytes
        Write-Output "[SNAPSHOT-SCRATCHPAD] $SnapshotName included=$($scratchpad.IncludedCount) bytes=$($scratchpad.IncludedBytes) skipped_dir=$($scratchpad.SkippedDirectory) skipped_ext=$($scratchpad.SkippedExtension) skipped_size=$($scratchpad.SkippedSize)"
        foreach ($item in $scratchpad.SizeSkipped) {
            Write-Output "[SNAPSHOT-SCRATCHPAD-SKIPPED-SIZE] $SnapshotName $($item.Path) bytes=$($item.Bytes) limit=$ScratchpadMaxFileBytes"
        }
        if ($scratchpad.Paths.Count -gt 0) {
            $pathspecFile = Join-Path ([IO.Path]::GetTempPath()) ("ori3-snapshot-" + [Guid]::NewGuid().ToString("N") + ".pathspec")
            $pathspecBytes = [Text.UTF8Encoding]::new($false).GetBytes(($scratchpad.Paths -join [char]0) + [char]0)
            [IO.File]::WriteAllBytes($pathspecFile, $pathspecBytes)
            Invoke-Git -WorkingDirectory $Worktree -Arguments @("add", "-f", "--pathspec-from-file=$pathspecFile", "--pathspec-file-nul") -IndexFile $indexFile | Out-Null
            # git add exits 0 but silently skips a file inside a nested git repository. Count
            # what reached the index so that included= is never an overstatement.
            $indexedCount = @(Invoke-Git -WorkingDirectory $Worktree -Arguments @("ls-files", "--", "scratchpad") -IndexFile $indexFile).Count
            if ($indexedCount -lt $scratchpad.Paths.Count) {
                $script:SnapshotFindingCount += 1
                Write-Host "[NG] ${SnapshotName}: $($scratchpad.Paths.Count) scratchpad files were selected but only $indexedCount reached the snapshot index (git silently skips files inside a nested git repository)" -ForegroundColor Red
            }
        }
        $tree = (Invoke-Git -WorkingDirectory $Worktree -Arguments @("write-tree") -IndexFile $indexFile) -join ""
        if ($tree -notmatch "^[0-9a-f]{40}$") { throw "write-tree did not return a tree id: $tree" }
        $head = (Invoke-Git -WorkingDirectory $Worktree -Arguments @("rev-parse", "HEAD")) -join ""

        $freshnessSelection = $null
        if (-not $IsRepositoryRoot) { $freshnessSelection = $scratchpad }
        $latestSource = Get-LatestSourceFile -Worktree $Worktree -ScratchpadSelection $freshnessSelection
        $snapshotSeconds = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
        if ($null -ne $latestSource) {
            # Commit timestamps are second-granularity. Record at least the second after the
            # latest source file so freshness remains a strict, reproducible comparison.
            $sourceSeconds = ([DateTimeOffset]$latestSource.LastWriteTimeUtc).ToUnixTimeSeconds()
            $snapshotSeconds = [Math]::Max($snapshotSeconds, $sourceSeconds + 1)
        }
        $snapshotDate = "@$snapshotSeconds +0000"
        $environment = @{ GIT_AUTHOR_DATE = $snapshotDate; GIT_COMMITTER_DATE = $snapshotDate }
        $message = "WIP snapshot $SnapshotName $([DateTime]::UtcNow.ToString('yyyy-MM-dd HH:mm')) UTC (no hooks; for resume)"
        # The previous snapshot becomes the second parent, so a snapshot that is replaced by a
        # bad one (2026-10-04: 19 worktrees were replaced by emptied snapshots) stays reachable
        # as refs/wip/<name>^2 and git gc cannot drop it. Without a previous snapshot, or when
        # it is the worktree HEAD itself, the only parent is HEAD as before.
        $commitArguments = @("commit-tree", $tree, "-p", $head)
        $previousSnapshot = Get-SnapshotCommitId -SnapshotName $SnapshotName
        if ($null -ne $previousSnapshot -and $previousSnapshot -ne $head) { $commitArguments += @("-p", $previousSnapshot) }
        $commitArguments += @("-m", $message)
        $commit = (Invoke-Git -WorkingDirectory $Worktree -Arguments $commitArguments -IndexFile $indexFile -Environment $environment) -join ""
        if ($commit -notmatch "^[0-9a-f]{40}$") { throw "commit-tree did not return a commit id: $commit" }
        Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("update-ref", "refs/wip/$SnapshotName", $commit) | Out-Null

        $summary = (Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("diff", "--shortstat", $head, $commit)) -join " "
        if ([string]::IsNullOrWhiteSpace($summary)) { $summary = "same tree as HEAD" }
        Write-Output "[OK] $SnapshotName -> $($commit.Substring(0,7)) (HEAD $($head.Substring(0,7))) $($summary.Trim())"
    }
    finally {
        if (Test-Path -LiteralPath $indexFile) { Remove-Item -LiteralPath $indexFile -Force }
        if ($null -ne $pathspecFile -and (Test-Path -LiteralPath $pathspecFile)) { Remove-Item -LiteralPath $pathspecFile -Force }
    }
}

function Test-SnapshotFreshness {
    param([Parameter(Mandatory = $true)][object[]]$Targets)

    $problems = New-Object System.Collections.Generic.List[string]
    foreach ($target in $Targets) {
        $freshnessSelection = $null
        if (-not $target.IsRepositoryRoot) {
            $vanished = Get-WorktreeVanishedState -Target $target
            if ($vanished.Vanished -or $vanished.Failed) {
                $problems.Add((Get-VanishedFindingText -Target $target -State $vanished))
                continue
            }
            $freshnessSelection = Get-SnapshotScratchpadPaths -Worktree $target.Worktree -IsRepositoryRoot $false -MaxFileBytes $ScratchpadMaxFileBytes
        }
        $latestSource = Get-LatestSourceFile -Worktree $target.Worktree -ScratchpadSelection $freshnessSelection
        if ($null -eq $latestSource) {
            Write-Host "[SKIP] $($target.Name): no source file in the monitored directories"
            continue
        }
        $snapshotTime = Get-SnapshotCommitTimeUtc -SnapshotName $target.Name
        if ($null -eq $snapshotTime) {
            $problems.Add("$($target.Name): refs/wip/$($target.Name) is missing; latest source is $($latestSource.FullName) at $($latestSource.LastWriteTimeUtc.ToString('o')) UTC")
            continue
        }
        if ($snapshotTime -le $latestSource.LastWriteTimeUtc) {
            $problems.Add("$($target.Name): refs/wip/$($target.Name) is stale; snapshot $($snapshotTime.ToString('o')) UTC <= source $($latestSource.FullName) $($latestSource.LastWriteTimeUtc.ToString('o')) UTC")
            continue
        }
        Write-Host "[OK] $($target.Name): snapshot $($snapshotTime.ToString('o')) UTC > latest source $($latestSource.LastWriteTimeUtc.ToString('o')) UTC"
    }
    # Write-Error obeys the script-wide Stop preference and would abort at the first
    # missing snapshot. Emit every affected worktree before returning a nonzero exit.
    foreach ($problem in $problems) { Write-Host "[NG] $problem" -ForegroundColor Red }
    Write-Host "[INFO] snapshot check completed: targets=$($Targets.Count), findings=$($problems.Count)"
    return $problems.Count
}

$inventory = Get-WorktreeInventory
$targets = @($inventory.Targets)
$excluded = @($inventory.Excluded)
foreach ($item in $excluded) {
    Write-Output "[EXCLUDE] $($item.Worktree): $($item.Reason)"
}
Write-Output "[INFO] snapshot targets=$($targets.Count), excluded=$($excluded.Count), mode=$(if ($Check) { 'check' } else { 'save' })"
if ($targets.Count -eq 0) {
    Write-Host "[NG] snapshot targets=0; worktree discovery produced no snapshot target" -ForegroundColor Red
    if ($Check) {
        Write-Output "[INFO] snapshot check completed: targets=0, findings=1"
    }
    exit 1
}
if ($Name) {
    $targets = @($targets | Where-Object { $_.Name -eq $Name })
    if ($targets.Count -ne 1) {
        $available = @($inventory.Targets | ForEach-Object Name) -join ', '
        throw "Unknown or ambiguous snapshot name '$Name'. Available: $available"
    }
}

if ($Check) {
    $problemCount = Test-SnapshotFreshness -Targets $targets
    if ($problemCount -gt 0) { exit 1 }
    exit 0
}

foreach ($target in $targets) {
    if (-not $target.IsRepositoryRoot) {
        # Saving a vanished worktree would overwrite its good snapshot: git add -A records the
        # missing files as deletions. Leave refs/wip/<name> untouched and report instead.
        $vanished = Get-WorktreeVanishedState -Target $target
        if ($vanished.Vanished -or $vanished.Failed) {
            $script:SnapshotFindingCount += 1
            Write-Host "[NG] $(Get-VanishedFindingText -Target $target -State $vanished)" -ForegroundColor Red
            Write-Output "[SKIP] $($target.Name): refs/wip/$($target.Name) was not updated"
            continue
        }
    }
    Save-Snapshot -SnapshotName $target.Name -Worktree $target.Worktree -IsRepositoryRoot $target.IsRepositoryRoot
}
if ($script:SnapshotFindingCount -gt 0) {
    Write-Host "[NG] snapshot save completed with findings=$($script:SnapshotFindingCount)" -ForegroundColor Red
    exit 1
}
