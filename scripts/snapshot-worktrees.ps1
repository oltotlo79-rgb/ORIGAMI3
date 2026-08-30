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

.PARAMETER Name
The derived snapshot name to operate on. Omit it to operate on every target.

.PARAMETER Check
Verify freshness only. Exit nonzero for a missing or stale refs/wip snapshot.
#>
[CmdletBinding()]
param(
    [string]$Name,
    [switch]$Check,
    [string]$RepositoryRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

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
        Push-Location -LiteralPath $WorkingDirectory
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
    foreach ($line in $lines) {
        if (-not $line.StartsWith("worktree ", [StringComparison]::Ordinal)) { continue }
        $worktree = Normalize-DirectoryPath $line.Substring("worktree ".Length)
        if ([string]::Equals($worktree, $RepositoryRoot, [StringComparison]::OrdinalIgnoreCase)) {
            $snapshotName = "main"
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
        $targets.Add([PSCustomObject]@{ Name = $snapshotName; Worktree = $worktree })
    }
    return [PSCustomObject]@{ Targets = $targets.ToArray(); Excluded = $excluded.ToArray() }
}

function Get-LatestSourceFile {
    param([Parameter(Mandatory = $true)][string]$Worktree)

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
    return $latest
}

function Get-SnapshotCommitTimeUtc {
    param([Parameter(Mandatory = $true)][string]$SnapshotName)

    $ref = "refs/wip/$SnapshotName"
    $commit = (Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("rev-parse", "--verify", "--quiet", "${ref}^{commit}") -AllowFailure) -join ""
    if ($commit -notmatch '^[0-9a-f]{40}$') { return $null }
    $secondsText = (Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("show", "-s", "--format=%ct", $commit)) -join ""
    [long]$seconds = 0
    if (-not [long]::TryParse($secondsText.Trim(), [ref]$seconds)) {
        throw "Cannot read commit timestamp for ${ref}: $secondsText"
    }
    return [DateTimeOffset]::FromUnixTimeSeconds($seconds).UtcDateTime
}

function Get-SnapshotScratchpadPaths {
    param([Parameter(Mandatory = $true)][string]$Worktree)

    $scratchpad = Join-Path $Worktree "scratchpad"
    if (-not (Test-Path -LiteralPath $scratchpad -PathType Container)) {
        return @()
    }

    $paths = New-Object System.Collections.Generic.List[string]
    foreach ($file in @(Get-ChildItem -LiteralPath $scratchpad -File -Recurse -Force -ErrorAction Stop)) {
        $relative = $file.FullName.Substring($scratchpad.Length).TrimStart([char[]]"\\/")
        $isDirectChild = $relative -notmatch '[\\/]'
        $extension = $file.Extension
        if ($extension -ieq ".md" -or ($isDirectChild -and ($extension -ieq ".patch" -or $extension -ieq ".txt"))) {
            $paths.Add((Join-Path "scratchpad" $relative))
        }
    }
    return $paths.ToArray()
}

function Save-Snapshot {
    param(
        [Parameter(Mandatory = $true)][string]$SnapshotName,
        [Parameter(Mandatory = $true)][string]$Worktree
    )

    $indexFile = Join-Path ([IO.Path]::GetTempPath()) ("ori3-snapshot-" + [Guid]::NewGuid().ToString("N") + ".index")
    try {
        Invoke-Git -WorkingDirectory $Worktree -Arguments @("read-tree", "HEAD") -IndexFile $indexFile | Out-Null
        Invoke-Git -WorkingDirectory $Worktree -Arguments @("add", "-A", ".") -IndexFile $indexFile -AllowFailure | Out-Null
        foreach ($path in $ExcludedPaths) {
            Invoke-Git -WorkingDirectory $Worktree -Arguments @("rm", "-r", "-q", "--cached", "--ignore-unmatch", $path) -IndexFile $indexFile -AllowFailure | Out-Null
        }
        $scratchpadPaths = @(Get-SnapshotScratchpadPaths -Worktree $Worktree)
        if ($scratchpadPaths.Count -gt 0) {
            Invoke-Git -WorkingDirectory $Worktree -Arguments (@("add", "-f", "--") + $scratchpadPaths) -IndexFile $indexFile | Out-Null
        }
        $tree = (Invoke-Git -WorkingDirectory $Worktree -Arguments @("write-tree") -IndexFile $indexFile) -join ""
        if ($tree -notmatch "^[0-9a-f]{40}$") { throw "write-tree did not return a tree id: $tree" }
        $head = (Invoke-Git -WorkingDirectory $Worktree -Arguments @("rev-parse", "HEAD")) -join ""

        $latestSource = Get-LatestSourceFile -Worktree $Worktree
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
        $commit = (Invoke-Git -WorkingDirectory $Worktree -Arguments @("commit-tree", $tree, "-p", $head, "-m", $message) -IndexFile $indexFile -Environment $environment) -join ""
        if ($commit -notmatch "^[0-9a-f]{40}$") { throw "commit-tree did not return a commit id: $commit" }
        Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("update-ref", "refs/wip/$SnapshotName", $commit) | Out-Null

        $summary = (Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("diff", "--shortstat", $head, $commit)) -join " "
        if ([string]::IsNullOrWhiteSpace($summary)) { $summary = "same tree as HEAD" }
        Write-Output "[OK] $SnapshotName -> $($commit.Substring(0,7)) (HEAD $($head.Substring(0,7))) $($summary.Trim())"
    }
    finally {
        if (Test-Path -LiteralPath $indexFile) { Remove-Item -LiteralPath $indexFile -Force }
    }
}

function Test-SnapshotFreshness {
    param([Parameter(Mandatory = $true)][object[]]$Targets)

    $problems = New-Object System.Collections.Generic.List[string]
    foreach ($target in $Targets) {
        $latestSource = Get-LatestSourceFile -Worktree $target.Worktree
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

foreach ($target in $targets) { Save-Snapshot -SnapshotName $target.Name -Worktree $target.Worktree }
