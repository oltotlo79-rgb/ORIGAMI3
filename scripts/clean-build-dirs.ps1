<#
.SYNOPSIS
Inventories ORIGAMI3 build directories and deletes only explicitly selected safe candidates.

.DESCRIPTION
With no -DeletePath this script is preview-only and deletes zero directories. Directory
timestamps are never used. To delete, pass an absolute candidate path and PowerShell's
standard -Confirm switch, for example:

  .\scripts\clean-build-dirs.ps1 `
      -DeletePath C:\Users\name\AppData\Local\Temp\ori3-target-example `
      -Confirm

The exact path is checked against a fresh direct-child allowlist, then re-scanned after
confirmation. Recent outputs, running executables, active file handles, .git metadata,
registered worktrees, reparse points, scan failures, and verification evidence all fail
closed. verification/push-tree is inventory-only and must use the git worktree workflow.
#>
[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = "High")]
param(
    [ValidateRange(1, 8760)]
    [int]$ProtectHours = 6,

    [string[]]$DeletePath = @(),

    [string]$TempRoot = [IO.Path]::GetTempPath(),

    [string]$RepositoryRoot = "",

    [string]$TestSandboxRoot
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$scriptDirectory = [string]$PSScriptRoot
if ([string]::IsNullOrWhiteSpace($scriptDirectory)) {
    $invocationPath = [string]$MyInvocation.MyCommand.Path
    if (-not [string]::IsNullOrWhiteSpace($invocationPath)) {
        $scriptDirectory = Split-Path -Parent ([IO.Path]::GetFullPath($invocationPath))
    }
}
if ([string]::IsNullOrWhiteSpace($scriptDirectory)) {
    throw "RepositoryRoot was not supplied and the script directory could not be determined."
}
$defaultRepositoryRootCandidate = [IO.Path]::GetFullPath((Join-Path $scriptDirectory ".."))
if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = $defaultRepositoryRootCandidate
}

function Get-NormalizedPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $fullPath = [IO.Path]::GetFullPath($Path)
    $root = [IO.Path]::GetPathRoot($fullPath)
    if ($fullPath -eq $root) {
        return $fullPath
    }
    $fullPath.TrimEnd([char[]]"\/")
}

function Test-PathEqual {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Left,

        [Parameter(Mandatory = $true)]
        [string]$Right
    )

    [string]::Equals(
        (Get-NormalizedPath $Left),
        (Get-NormalizedPath $Right),
        [StringComparison]::OrdinalIgnoreCase
    )
}

function Test-PathInside {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Directory
    )

    $normalizedPath = Get-NormalizedPath $Path
    $normalizedDirectory = Get-NormalizedPath $Directory
    $prefix = $normalizedDirectory + [IO.Path]::DirectorySeparatorChar
    $normalizedPath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
}

function Get-AllowedCandidateSource {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CandidatePath,

        [Parameter(Mandatory = $true)]
        [string]$NormalizedTempRoot,

        [Parameter(Mandatory = $true)]
        [string]$NormalizedVerificationRoot
    )

    $normalizedCandidate = Get-NormalizedPath $CandidatePath
    $parent = [IO.Path]::GetDirectoryName($normalizedCandidate)
    $leaf = [IO.Path]::GetFileName($normalizedCandidate)

    if ((Test-PathEqual $parent $NormalizedTempRoot) -and
        [regex]::IsMatch($leaf, "^ori3-target-.+$", [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        return "TempTarget"
    }
    if (Test-PathEqual $parent $NormalizedVerificationRoot) {
        if ([regex]::IsMatch($leaf, "^target-.+$", [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
            return "VerificationTarget"
        }
        if ([regex]::IsMatch($leaf, "^ci-.+$", [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
            return "VerificationCi"
        }
        if ([string]::Equals($leaf, "push-tree", [StringComparison]::OrdinalIgnoreCase)) {
            return "PushTree"
        }
    }
    $null
}

function Assert-AllowedCandidatePath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CandidatePath,

        [Parameter(Mandatory = $true)]
        [string]$ExpectedSource,

        [Parameter(Mandatory = $true)]
        [string]$NormalizedTempRoot,

        [Parameter(Mandatory = $true)]
        [string]$NormalizedVerificationRoot
    )

    $normalizedCandidate = Get-NormalizedPath $CandidatePath
    if (-not (Test-Path -LiteralPath $normalizedCandidate -PathType Container)) {
        throw "Deletion target no longer exists as a directory: $normalizedCandidate"
    }

    $actualSource = Get-AllowedCandidateSource $normalizedCandidate $NormalizedTempRoot $NormalizedVerificationRoot
    if (($null -eq $actualSource) -or ($actualSource -ne $ExpectedSource)) {
        throw "Refusing an unapproved deletion target: $normalizedCandidate"
    }

    $rootItem = Get-Item -LiteralPath $normalizedCandidate -Force -ErrorAction Stop
    if (($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Refusing a reparse-point deletion target: $normalizedCandidate"
    }
    $normalizedCandidate
}

function Assert-RootIsNotReparsePoint {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        return
    }
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Refusing a candidate root that is a reparse point: $Path"
    }
}

function Get-Candidates {
    param(
        [Parameter(Mandatory = $true)]
        [string]$NormalizedTempRoot,

        [Parameter(Mandatory = $true)]
        [string]$NormalizedVerificationRoot
    )

    $byPath = @{}
    $definitions = @(
        [pscustomobject]@{ Root = $NormalizedTempRoot; Pattern = "^ori3-target-.+$"; Source = "TempTarget" },
        [pscustomobject]@{ Root = $NormalizedVerificationRoot; Pattern = "^target-.+$"; Source = "VerificationTarget" },
        [pscustomobject]@{ Root = $NormalizedVerificationRoot; Pattern = "^ci-.+$"; Source = "VerificationCi" },
        [pscustomobject]@{ Root = $NormalizedVerificationRoot; Pattern = "^push-tree$"; Source = "PushTree" }
    )

    foreach ($definition in $definitions) {
        if (-not (Test-Path -LiteralPath $definition.Root -PathType Container)) {
            continue
        }
        $children = @(Get-ChildItem -LiteralPath $definition.Root -Directory -Force -ErrorAction Stop)
        foreach ($child in $children) {
            if (-not [regex]::IsMatch($child.Name, $definition.Pattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
                continue
            }
            $path = Get-NormalizedPath $child.FullName
            $key = $path.ToUpperInvariant()
            if (-not $byPath.ContainsKey($key)) {
                $byPath[$key] = [pscustomobject]@{
                    Name = $child.Name
                    Path = $path
                    Source = $definition.Source
                }
            }
        }
    }

    @($byPath.Values | Sort-Object Path)
}

function Get-RunningProcessesUnder {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CandidatePath
    )

    $matches = New-Object System.Collections.Generic.List[string]
    foreach ($process in @(Get-Process -ErrorAction SilentlyContinue)) {
        $executablePath = $null
        try {
            $executablePath = $process.Path
        }
        catch {
            $executablePath = $null
        }
        if ([string]::IsNullOrWhiteSpace($executablePath)) {
            continue
        }
        try {
            if (Test-PathInside $executablePath $CandidatePath) {
                [void]$matches.Add(("{0} (PID {1})" -f $process.ProcessName, $process.Id))
            }
        }
        catch {
            # A process can exit between enumeration and path normalization.
        }
    }
    @($matches.ToArray())
}

function Get-WorktreeStatus {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CandidatePath,

        [Parameter(Mandatory = $true)]
        [string]$NormalizedRepositoryRoot
    )

    $git = Get-Command git -ErrorAction SilentlyContinue
    if ($null -eq $git) {
        return [pscustomobject]@{ Status = "Unknown"; Detail = "git command is unavailable" }
    }

    try {
        $lines = @(& $git.Source -C $NormalizedRepositoryRoot worktree list --porcelain 2>$null)
        if ($LASTEXITCODE -ne 0) {
            return [pscustomobject]@{ Status = "Unknown"; Detail = "git worktree list failed" }
        }
        foreach ($line in $lines) {
            if (-not ([string]$line).StartsWith("worktree ", [StringComparison]::Ordinal)) {
                continue
            }
            $listedPath = ([string]$line).Substring(9)
            try {
                if (Test-PathEqual $listedPath $CandidatePath) {
                    return [pscustomobject]@{ Status = "Registered"; Detail = "listed by git worktree list" }
                }
            }
            catch {
                return [pscustomobject]@{ Status = "Unknown"; Detail = "could not normalize a git worktree path" }
            }
        }
        [pscustomobject]@{ Status = "NotRegistered"; Detail = "not listed by git worktree list" }
    }
    catch {
        [pscustomobject]@{ Status = "Unknown"; Detail = $_.Exception.Message }
    }
}

function Get-DirectorySnapshot {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CandidatePath,

        [Parameter(Mandatory = $true)]
        [bool]$ProtectEvidence
    )

    $stack = New-Object System.Collections.Stack
    $sizeBytes = [long]0
    $fileCount = [long]0
    $directoryCount = [long]0
    $newestFilePath = $null
    $newestFileLastWriteUtc = $null
    $containsGit = $false
    $containsReparsePoint = $false
    $containsEvidenceFiles = $false
    $scanError = $null

    try {
        $root = New-Object IO.DirectoryInfo($CandidatePath)
        $root.Refresh()
        if (($root.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            $containsReparsePoint = $true
        }
        else {
            $stack.Push($root)
        }
        while ($stack.Count -gt 0) {
            $directory = [IO.DirectoryInfo]$stack.Pop()
            $directoryCount += 1
            foreach ($entry in @($directory.GetFileSystemInfos())) {
                if ($entry.Name -ieq ".git") {
                    $containsGit = $true
                }
                if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                    $containsReparsePoint = $true
                    continue
                }
                if ($entry -is [IO.DirectoryInfo]) {
                    $stack.Push($entry)
                    continue
                }

                $file = [IO.FileInfo]$entry
                $fileCount += 1
                $sizeBytes += [long]$file.Length
                if (($null -eq $newestFileLastWriteUtc) -or ($file.LastWriteTimeUtc -gt $newestFileLastWriteUtc)) {
                    $newestFilePath = $file.FullName
                    $newestFileLastWriteUtc = $file.LastWriteTimeUtc
                }
                if ($ProtectEvidence -and (Test-IsProtectedEvidenceFile $file)) {
                    $containsEvidenceFiles = $true
                }
            }
        }
    }
    catch {
        $scanError = $_.Exception.Message
    }

    [pscustomobject]@{
        SizeBytes = $sizeBytes
        FileCount = $fileCount
        DirectoryCount = $directoryCount
        NewestFilePath = $newestFilePath
        NewestFileLastWriteUtc = $newestFileLastWriteUtc
        ContainsGitMetadata = $containsGit
        ContainsReparsePoint = $containsReparsePoint
        ContainsEvidenceFiles = $containsEvidenceFiles
        ScanError = $scanError
    }
}

function Test-DirectoryFilesExclusivelyAvailable {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CandidatePath
    )

    $stack = New-Object System.Collections.Stack
    try {
        $root = New-Object IO.DirectoryInfo($CandidatePath)
        $root.Refresh()
        if (($root.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            return [pscustomobject]@{
                Available = $false
                Path = $CandidatePath
                Detail = "candidate root is a reparse point"
            }
        }
        $stack.Push($root)
        while ($stack.Count -gt 0) {
            $directory = [IO.DirectoryInfo]$stack.Pop()
            foreach ($entry in @($directory.GetFileSystemInfos())) {
                if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                    return [pscustomobject]@{
                        Available = $false
                        Path = $entry.FullName
                        Detail = "reparse point appeared during the exclusive probe"
                    }
                }
                if ($entry -is [IO.DirectoryInfo]) {
                    $stack.Push($entry)
                    continue
                }

                $stream = $null
                try {
                    $stream = [IO.File]::Open(
                        $entry.FullName,
                        [IO.FileMode]::Open,
                        [IO.FileAccess]::Read,
                        [IO.FileShare]::None
                    )
                }
                catch {
                    return [pscustomobject]@{
                        Available = $false
                        Path = $entry.FullName
                        Detail = $_.Exception.Message
                    }
                }
                finally {
                    if ($null -ne $stream) {
                        $stream.Dispose()
                    }
                }
            }
        }
    }
    catch {
        return [pscustomobject]@{
            Available = $false
            Path = $CandidatePath
            Detail = $_.Exception.Message
        }
    }

    [pscustomobject]@{ Available = $true; Path = $null; Detail = $null }
}

function Get-ReasonText {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ReasonCode
    )

    switch ($ReasonCode) {
        "RecentDesktopExecutable" { "release\desktop.exe was updated within the protection window"; break }
        "RecentInnerFile" { "the newest inner file was updated within the protection window"; break }
        "RunningProcess" { "a running process uses an executable under this directory"; break }
        "ActiveWriteOrLock" { "an inner file is being written or is exclusively in use"; break }
        "ContainsGitMetadata" { "the directory contains .git metadata"; break }
        "ContainsReparsePoint" { "a reparse point makes the deletion boundary unsafe"; break }
        "ContainsEvidenceFiles" { "the verification directory contains protected evidence files"; break }
        "FileScanFailed" { "the complete inner-file scan failed"; break }
        "RegisteredGitWorktree" { "the directory is registered by git worktree list"; break }
        "WorktreeStatusUnknown" { "the git worktree status could not be verified"; break }
        "PushTreeRequiresGitRemoval" { "verification/push-tree must be removed with the git worktree workflow"; break }
        "TreeChangedDuringCheck" { "the directory contents changed between the inventory and final check"; break }
        "OlderThanProtectionWindow" { "no protection applies and every inner file is older than the cutoff"; break }
        default { $ReasonCode }
    }
}

function Test-IsProtectedEvidenceFile {
    param(
        [Parameter(Mandatory = $true)]
        $File
    )

    if ($File.Name -ieq "live3-frame.json") {
        return $true
    }
    $extension = $File.Extension.ToLowerInvariant()
    @(".tsv", ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp", ".tif", ".tiff", ".svg") -contains $extension
}

function Get-CandidateAssessment {
    param(
        [Parameter(Mandatory = $true)]
        $Candidate,

        [Parameter(Mandatory = $true)]
        [DateTime]$CutoffUtc,

        [Parameter(Mandatory = $true)]
        [string]$NormalizedRepositoryRoot,

        [Parameter(Mandatory = $true)]
        [bool]$ProbeLocks
    )

    $reasonCodes = @()
    $snapshot = Get-DirectorySnapshot $Candidate.Path ($Candidate.Source -ne "TempTarget")
    if ($null -ne $snapshot.ScanError) {
        $reasonCodes += "FileScanFailed"
    }

    $desktopPath = Join-Path $Candidate.Path "release\desktop.exe"
    $desktopFile = $null
    if (Test-Path -LiteralPath $desktopPath -PathType Leaf) {
        try {
            $desktopFile = Get-Item -LiteralPath $desktopPath -Force -ErrorAction Stop
        }
        catch {
            if (-not ($reasonCodes -contains "FileScanFailed")) {
                $reasonCodes += "FileScanFailed"
            }
            $snapshot.ScanError = $_.Exception.Message
        }
    }
    if (($null -ne $desktopFile) -and ($desktopFile.LastWriteTimeUtc -ge $CutoffUtc)) {
        $reasonCodes += "RecentDesktopExecutable"
    }
    if (($null -ne $snapshot.NewestFileLastWriteUtc) -and ($snapshot.NewestFileLastWriteUtc -ge $CutoffUtc)) {
        $reasonCodes += "RecentInnerFile"
    }

    $runningProcesses = @(Get-RunningProcessesUnder $Candidate.Path)
    if ($runningProcesses.Count -gt 0) {
        $reasonCodes += "RunningProcess"
    }
    if ($snapshot.ContainsGitMetadata) {
        $reasonCodes += "ContainsGitMetadata"
    }
    if ($snapshot.ContainsReparsePoint) {
        $reasonCodes += "ContainsReparsePoint"
    }
    if ($snapshot.ContainsEvidenceFiles) {
        $reasonCodes += "ContainsEvidenceFiles"
    }

    $worktreeStatus = "NotApplicable"
    $worktreeDetail = $null
    $worktree = Get-WorktreeStatus $Candidate.Path $NormalizedRepositoryRoot
    $worktreeStatus = $worktree.Status
    $worktreeDetail = $worktree.Detail
    if ($worktree.Status -eq "Registered") {
        $reasonCodes += "RegisteredGitWorktree"
    }
    elseif (($worktree.Status -eq "Unknown") -and ($Candidate.Source -ne "TempTarget")) {
        $reasonCodes += "WorktreeStatusUnknown"
    }
    if ($Candidate.Source -eq "PushTree") {
        $reasonCodes += "PushTreeRequiresGitRemoval"
    }

    $writeProbeStatus = if ($reasonCodes.Count -gt 0) { "NotNeededAlreadyProtected" } else { "DeferredUntilDeletion" }
    $writeProbePath = $null
    $writeProbeDetail = $null
    if (($reasonCodes.Count -eq 0) -and $ProbeLocks) {
        $probe = Test-DirectoryFilesExclusivelyAvailable $Candidate.Path
        if ($probe.Available) {
            $writeProbeStatus = "Available"
        }
        else {
            $writeProbeStatus = "ActiveWriteOrLock"
            $writeProbePath = $probe.Path
            $writeProbeDetail = $probe.Detail
            $reasonCodes += "ActiveWriteOrLock"
        }
    }

    $protected = $reasonCodes.Count -gt 0
    if (-not $protected) {
        $reasonCodes += "OlderThanProtectionWindow"
    }
    $reasonTexts = @($reasonCodes | ForEach-Object { Get-ReasonText $_ })

    [pscustomobject]@{
        Name = $Candidate.Name
        Path = $Candidate.Path
        Source = $Candidate.Source
        SizeBytes = $snapshot.SizeBytes
        FileCount = $snapshot.FileCount
        DirectoryCount = $snapshot.DirectoryCount
        DesktopExeExists = $null -ne $desktopFile
        DesktopExeLastWriteUtc = if ($null -ne $desktopFile) { $desktopFile.LastWriteTimeUtc } else { $null }
        NewestFilePath = $snapshot.NewestFilePath
        NewestFileLastWriteUtc = $snapshot.NewestFileLastWriteUtc
        RunningProcesses = $runningProcesses
        WriteProbeStatus = $writeProbeStatus
        WriteProbePath = $writeProbePath
        WriteProbeDetail = $writeProbeDetail
        ContainsGitMetadata = $snapshot.ContainsGitMetadata
        ContainsReparsePoint = $snapshot.ContainsReparsePoint
        ContainsEvidenceFiles = $snapshot.ContainsEvidenceFiles
        WorktreeStatus = $worktreeStatus
        WorktreeDetail = $worktreeDetail
        ScanError = $snapshot.ScanError
        Protected = $protected
        ReasonCodes = @($reasonCodes)
        Reason = $reasonTexts -join " / "
        Decision = if ($protected) { "Keep" } else { "WouldDelete" }
        Deleted = $false
    }
}

function Format-UtcTimestamp {
    param([AllowNull()]$Timestamp)

    if ($null -eq $Timestamp) {
        return "none"
    }
    ([DateTime]$Timestamp).ToUniversalTime().ToString("yyyy-MM-dd HH:mm:ss 'UTC'")
}

function Show-CandidateAssessment {
    param(
        [Parameter(Mandatory = $true)]
        $Assessment,

        [Parameter(Mandatory = $true)]
        [string]$DecisionLabel
    )

    $sizeMiB = [Math]::Round($Assessment.SizeBytes / 1MB, 2)
    $desktop = if ($Assessment.DesktopExeExists) {
        "yes / {0}" -f (Format-UtcTimestamp $Assessment.DesktopExeLastWriteUtc)
    }
    else {
        "no"
    }
    $newest = if ($null -ne $Assessment.NewestFilePath) {
        "{0} / {1}" -f $Assessment.NewestFilePath, (Format-UtcTimestamp $Assessment.NewestFileLastWriteUtc)
    }
    else {
        "none"
    }
    $processes = if (@($Assessment.RunningProcesses).Count -gt 0) {
        @($Assessment.RunningProcesses) -join ", "
    }
    else {
        "none"
    }
    $writeStatus = switch ($Assessment.WriteProbeStatus) {
        "Available" { "exclusive probe succeeded; no active write/use detected" }
        "ActiveWriteOrLock" { "active write or exclusive use: $($Assessment.WriteProbePath)" }
        "DeferredUntilDeletion" { "deferred; exact-path deletion was not requested" }
        default { "not needed because another protection already applies" }
    }

    Write-Host ""
    Write-Host ("[{0}] {1}" -f $Assessment.Source, $Assessment.Name)
    Write-Host ("  Path                 : {0}" -f $Assessment.Path)
    Write-Host ("  Size                 : {0:N0} bytes ({1:N2} MiB)" -f $Assessment.SizeBytes, $sizeMiB)
    Write-Host ("  release\desktop.exe : {0}" -f $desktop)
    Write-Host ("  Newest inner file    : {0}" -f $newest)
    Write-Host ("  Running process      : {0}" -f $processes)
    Write-Host ("  Write/lock probe     : {0}" -f $writeStatus)
    Write-Host ("  Decision             : {0}" -f $DecisionLabel)
    Write-Host ("  Reason               : {0}" -f $Assessment.Reason)
}

$defaultTempRoot = Get-NormalizedPath ([IO.Path]::GetTempPath())
$defaultRepositoryRoot = Get-NormalizedPath $defaultRepositoryRootCandidate
$normalizedTempRoot = Get-NormalizedPath $TempRoot
$normalizedRepositoryRoot = Get-NormalizedPath $RepositoryRoot
$normalizedVerificationRoot = Get-NormalizedPath (Join-Path $normalizedRepositoryRoot "verification")

if ([string]::IsNullOrWhiteSpace($TestSandboxRoot)) {
    if ((-not (Test-PathEqual $normalizedTempRoot $defaultTempRoot)) -or
        (-not (Test-PathEqual $normalizedRepositoryRoot $defaultRepositoryRoot))) {
        throw "TempRoot and RepositoryRoot can only be overridden inside an explicit test sandbox."
    }
}
else {
    $normalizedTestSandbox = Get-NormalizedPath $TestSandboxRoot
    $sandboxParent = [IO.Path]::GetDirectoryName($normalizedTestSandbox)
    $sandboxLeaf = [IO.Path]::GetFileName($normalizedTestSandbox)
    if ((-not (Test-PathEqual $sandboxParent $defaultTempRoot)) -or
        (-not [regex]::IsMatch($sandboxLeaf, "^ori3-clean-build-dirs-test-[0-9a-f]{32}$", [Text.RegularExpressions.RegexOptions]::IgnoreCase)) -or
        (-not (Test-Path -LiteralPath $normalizedTestSandbox -PathType Container))) {
        throw "The test sandbox must be an existing GUID-named direct child of the system temp directory."
    }
    if ((-not (Test-PathInside $normalizedTempRoot $normalizedTestSandbox)) -or
        (-not (Test-PathInside $normalizedRepositoryRoot $normalizedTestSandbox))) {
        throw "All overridden roots must stay inside the validated test sandbox."
    }
}

Assert-RootIsNotReparsePoint $normalizedTempRoot
Assert-RootIsNotReparsePoint $normalizedRepositoryRoot
Assert-RootIsNotReparsePoint $normalizedVerificationRoot

$cutoffUtc = [DateTime]::UtcNow.AddHours(-$ProtectHours)
$candidates = @(Get-Candidates $normalizedTempRoot $normalizedVerificationRoot)
$selectedPaths = @{}
foreach ($requestedPath in @($DeletePath)) {
    if ([string]::IsNullOrWhiteSpace($requestedPath) -or (-not [IO.Path]::IsPathRooted($requestedPath))) {
        throw "Every -DeletePath must be a non-empty absolute path."
    }
    $normalizedRequested = Get-NormalizedPath $requestedPath
    $matches = @($candidates | Where-Object { Test-PathEqual $_.Path $normalizedRequested })
    if ($matches.Count -ne 1) {
        throw "The requested deletion path is not exactly one current build-directory candidate: $normalizedRequested"
    }
    $source = Get-AllowedCandidateSource $normalizedRequested $normalizedTempRoot $normalizedVerificationRoot
    if (($null -eq $source) -or ($source -ne $matches[0].Source)) {
        throw "The requested deletion path failed the direct-child and name allowlist: $normalizedRequested"
    }
    $selectedPaths[$normalizedRequested.ToUpperInvariant()] = $true
}
$deleteRequested = $selectedPaths.Count -gt 0

Write-Host "ORIGAMI3 build-directory cleanup"
Write-Host ("Mode                 : {0}" -f $(if ($deleteRequested) { "EXACT-PATH DELETE REQUEST" } else { "PREVIEW ONLY; deleted=0" }))
Write-Host ("Protection window    : {0} hours (cutoff {1})" -f $ProtectHours, (Format-UtcTimestamp $cutoffUtc))
Write-Host "Directory timestamps : NOT USED"
Write-Host ("Temp ori3-target-*   : {0}" -f @($candidates | Where-Object Source -eq "TempTarget").Count)
Write-Host ("verification/target-*: {0}" -f @($candidates | Where-Object Source -eq "VerificationTarget").Count)
Write-Host ("verification/ci-*    : {0}" -f @($candidates | Where-Object Source -eq "VerificationCi").Count)
Write-Host ("verification/push-tree: {0}" -f @($candidates | Where-Object Source -eq "PushTree").Count)

$results = New-Object System.Collections.Generic.List[object]
foreach ($candidate in $candidates) {
    $isSelected = $selectedPaths.ContainsKey($candidate.Path.ToUpperInvariant())
    $assessment = Get-CandidateAssessment $candidate $cutoffUtc $normalizedRepositoryRoot $isSelected
    if ($assessment.Protected) {
        $assessment.Decision = "Keep"
        Show-CandidateAssessment $assessment "KEEP"
        [void]$results.Add($assessment)
        continue
    }

    if (-not $isSelected) {
        $assessment.Decision = "WouldDelete"
        Show-CandidateAssessment $assessment "WOULD DELETE (preview only; no deletion performed)"
        [void]$results.Add($assessment)
        continue
    }

    Show-CandidateAssessment $assessment "EXACT PATH SELECTED; waiting for confirmation and a fresh safety check"
    if (-not $PSCmdlet.ShouldProcess($candidate.Path, "Recursively delete this exact build directory")) {
        $assessment.Decision = "WouldDelete"
        [void]$results.Add($assessment)
        continue
    }

    $freshCandidates = @(Get-Candidates $normalizedTempRoot $normalizedVerificationRoot)
    $freshMatches = @($freshCandidates | Where-Object { Test-PathEqual $_.Path $candidate.Path })
    if (($freshMatches.Count -ne 1) -or ($freshMatches[0].Source -ne $candidate.Source)) {
        throw "The selected candidate changed or left the allowlist before deletion: $($candidate.Path)"
    }
    $validatedPath = Assert-AllowedCandidatePath $freshMatches[0].Path $freshMatches[0].Source $normalizedTempRoot $normalizedVerificationRoot
    $freshCandidate = $freshMatches[0]
    $fresh = Get-CandidateAssessment $freshCandidate $cutoffUtc $normalizedRepositoryRoot $true
    if ($fresh.Protected) {
        $fresh.Decision = "Keep"
        Show-CandidateAssessment $fresh "KEEP (fresh safety check found a protection)"
        [void]$results.Add($fresh)
        continue
    }

    $sameNewestPath = (($null -eq $assessment.NewestFilePath) -and ($null -eq $fresh.NewestFilePath))
    if (($null -ne $assessment.NewestFilePath) -and ($null -ne $fresh.NewestFilePath)) {
        $sameNewestPath = Test-PathEqual $assessment.NewestFilePath $fresh.NewestFilePath
    }
    $sameNewestTime = $assessment.NewestFileLastWriteUtc -eq $fresh.NewestFileLastWriteUtc
    if (($assessment.SizeBytes -ne $fresh.SizeBytes) -or
        ($assessment.FileCount -ne $fresh.FileCount) -or
        ($assessment.DirectoryCount -ne $fresh.DirectoryCount) -or
        (-not $sameNewestPath) -or
        (-not $sameNewestTime)) {
        $fresh.Protected = $true
        $fresh.ReasonCodes = @($fresh.ReasonCodes) + "TreeChangedDuringCheck"
        $fresh.Reason = (@($fresh.ReasonCodes | ForEach-Object { Get-ReasonText $_ })) -join " / "
        $fresh.Decision = "Keep"
        Show-CandidateAssessment $fresh "KEEP (directory changed during the final check)"
        [void]$results.Add($fresh)
        continue
    }

    $validatedPath = Assert-AllowedCandidatePath $fresh.Path $fresh.Source $normalizedTempRoot $normalizedVerificationRoot
    try {
        Remove-Item -LiteralPath $validatedPath -Recurse -Force -ErrorAction Stop
    }
    catch {
        throw "Failed to delete the validated build directory '$validatedPath': $($_.Exception.Message)"
    }
    if (Test-Path -LiteralPath $validatedPath) {
        throw "Deletion returned without an error, but the directory still exists: $validatedPath"
    }
    $fresh.Decision = "Deleted"
    $fresh.Deleted = $true
    Write-Host ("  Deleted              : {0}" -f $validatedPath)
    [void]$results.Add($fresh)
}

$keptCount = @($results | Where-Object Decision -eq "Keep").Count
$wouldDeleteCount = @($results | Where-Object Decision -eq "WouldDelete").Count
$deletedCount = @($results | Where-Object Decision -eq "Deleted").Count
$totalBytes = [long](($results | Measure-Object -Property SizeBytes -Sum).Sum)
Write-Host ""
Write-Host ("Summary: candidates={0}, bytes={1}, kept={2}, would-delete={3}, deleted={4}" -f $results.Count, $totalBytes, $keptCount, $wouldDeleteCount, $deletedCount)

@($results.ToArray())
