[CmdletBinding()]
param(
    [ValidateSet("Staged", "Tree")]
    [string]$Mode = "Tree",

    [string]$RepositoryRoot = ""
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$script:CheckNames = @(
    "no-allow-attribute",
    "ignore-reason-has-number",
    "tracked-fixture-only",
    "no-prohibited-doc",
    "known-defect-shapes"
)
$script:ProhibitedPath = "docs/competitive-review-2026-08-20.md"
$script:Utf8NoBom = New-Object Text.UTF8Encoding($false)

function ConvertTo-RepositoryPath {
    param([AllowNull()][string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) { return "" }
    $normalized = $Path.Replace("\", "/")
    while ($normalized.StartsWith("./", [StringComparison]::Ordinal)) {
        $normalized = $normalized.Substring(2)
    }
    return $normalized
}

function Invoke-GitLines {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        $output = @(& git -c core.excludesFile=NUL -c core.quotePath=false -C $Root @Arguments 2>$null)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($exitCode -ne 0) {
        throw "git $($Arguments -join ' ') failed (exit=$exitCode): $($output -join ' ')"
    }
    return @($output | ForEach-Object { [string]$_ })
}

function Test-PowerShellSyntax {
    param([Parameter(Mandatory = $true)][string]$Path)

    $tokens = $null
    $errors = $null
    [void][Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$errors)
    return @($errors)
}

function Get-CheckPaths {
    param([Parameter(Mandatory = $true)][string]$ChecksDirectory)

    $result = @{}
    foreach ($name in $script:CheckNames) {
        $result[$name] = Join-Path $ChecksDirectory "$name.ps1"
    }
    return $result
}

function Assert-ChecksAvailable {
    param([Parameter(Mandatory = $true)][hashtable]$CheckPaths)

    $unavailable = 0
    foreach ($name in $script:CheckNames) {
        $path = [string]$CheckPaths[$name]
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            [Console]::Error.WriteLine("[NG] HOOK_CHECK_UNAVAILABLE check=$name reason=missing path=$path")
            $unavailable += 1
            continue
        }
        $parseErrors = @(Test-PowerShellSyntax -Path $path)
        if ($parseErrors.Count -gt 0) {
            $message = ([string]$parseErrors[0].Message) -replace '\s+', ' '
            [Console]::Error.WriteLine("[NG] HOOK_CHECK_UNAVAILABLE check=$name reason=syntax-error detail=$message")
            $unavailable += 1
        }
    }
    if ($unavailable -gt 0) {
        throw "$unavailable required hook check script(s) are unavailable"
    }
}

function Invoke-OneCheck {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$ScriptPath,
        [Parameter(Mandatory = $true)][AllowNull()][AllowEmptyCollection()][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$PowerShellPath
    )

    Write-Host "--- hook check: $Name ---"
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        $output = @(& $PowerShellPath -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $ScriptPath @Arguments 2>&1)
        $exitCode = $LASTEXITCODE
    }
    catch {
        [Console]::Error.WriteLine("[NG] HOOK_CHECK_UNAVAILABLE check=$Name reason=start-failed detail=$($_.Exception.Message)")
        return 2
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    foreach ($line in $output) { Write-Host ([string]$line) }
    if ($exitCode -eq 0) { return 0 }
    if ($exitCode -eq 1) {
        [Console]::Error.WriteLine("[NG] HOOK_CHECK_VIOLATION check=$Name exit=1")
        return 1
    }
    [Console]::Error.WriteLine("[NG] HOOK_CHECK_UNAVAILABLE check=$Name reason=unclassified-exit exit=$exitCode")
    return 2
}

function Invoke-KnownDefectMeasurement {
    param(
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][string]$ScriptPath,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$PowerShellPath
    )

    Write-Host "--- hook check: known-defect-shapes ($Label) ---"
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        $output = @(& $PowerShellPath -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $ScriptPath -RepositoryRoot $Root -FailOnDecrease 2>&1)
        $exitCode = $LASTEXITCODE
    }
    catch {
        [Console]::Error.WriteLine("[NG] HOOK_CHECK_UNAVAILABLE check=known-defect-shapes reason=start-failed label=$Label detail=$($_.Exception.Message)")
        return [pscustomobject]@{ ExitCode = 2; Increase = $null; Decrease = $null; InventoryDrift = $null }
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    foreach ($line in $output) { Write-Host ([string]$line) }
    if ($exitCode -notin @(0, 1)) {
        [Console]::Error.WriteLine("[NG] HOOK_CHECK_UNAVAILABLE check=known-defect-shapes reason=unclassified-exit label=$Label exit=$exitCode")
        return [pscustomobject]@{ ExitCode = 2; Increase = $null; Decrease = $null; InventoryDrift = $null }
    }

    $metrics = @(
        $output | ForEach-Object {
            $line = [string]$_
            $numbers = @([regex]::Matches($line, '\d+') | ForEach-Object { [int]$_.Value })
            if ($line -match '^\[(?:OK|NG)\] [^'']+?:' -and $numbers.Count -in @(3, 4)) {
                [pscustomobject]@{
                    Increase = $numbers[$numbers.Count - 3]
                    Decrease = $numbers[$numbers.Count - 2]
                    InventoryDrift = $numbers[$numbers.Count - 1]
                }
            }
        }
    )
    if ($metrics.Count -ne 1) {
        [Console]::Error.WriteLine("[NG] HOOK_CHECK_UNAVAILABLE check=known-defect-shapes reason=metrics-unreadable label=$Label summaries=$($metrics.Count)")
        return [pscustomobject]@{ ExitCode = 2; Increase = $null; Decrease = $null; InventoryDrift = $null }
    }
    return [pscustomobject]@{
        ExitCode = $exitCode
        Increase = $metrics[0].Increase
        Decrease = $metrics[0].Decrease
        InventoryDrift = $metrics[0].InventoryDrift
    }
}

function Compare-StagedKnownDefects {
    param(
        [Parameter(Mandatory = $true)]$Head,
        [Parameter(Mandatory = $true)]$Staged
    )

    if ($Head.ExitCode -eq 2 -or $Staged.ExitCode -eq 2) { return 2 }
    $changes = New-Object System.Collections.Generic.List[string]
    foreach ($field in @('Increase', 'Decrease', 'InventoryDrift')) {
        if ([int]$Staged.$field -gt [int]$Head.$field) {
            $label = switch ($field) {
                'Increase' { 'increase-kinds' }
                'Decrease' { 'decrease-kinds' }
                'InventoryDrift' { 'inventory-drift' }
            }
            $changes.Add("$label $($Head.$field) -> $($Staged.$field)")
        }
    }
    if ($changes.Count -gt 0) {
        [Console]::Error.WriteLine("[NG] STAGED_KNOWN_DEFECT_WORSENED $($changes -join ', ')")
        return 1
    }
    if ($Head.ExitCode -eq 1) {
        $inheritedFormat = [regex]::Unescape('[\u7d99\u627f] HEAD \u3067\u65e2\u306b\u8d64: \u5897\u52a0 {0} \u7a2e\u30fb\u6e1b\u5c11 {1} \u7a2e\u30fb\u53f0\u5e33\u79fb\u52d5 {2} \u4ef6\uff08check.ps1 / check-ci / CI \u3067\u306f\u7d76\u5bfe\u5024\u3067\u8d64\u306e\u307e\u307e\uff09 [INHERITED head-red increase={0} decrease={1} inventory-drift={2}]')
        Write-Host ($inheritedFormat -f $Head.Increase, $Head.Decrease, $Head.InventoryDrift)
    }
    Write-Host "[OK] staged known-defect ratchet: no regression from HEAD"
    return 0
}

function Remove-SafeTemporaryDirectory {
    param([AllowNull()][string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path) -or -not (Test-Path -LiteralPath $Path)) { return }
    $full = [IO.Path]::GetFullPath($Path).TrimEnd([char[]]"\/")
    $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
    if ([IO.Path]::GetDirectoryName($full) -cne $temp -or
        [IO.Path]::GetFileName($full) -notmatch '^ori3-hook-checks-[0-9a-f]{32}$') {
        throw "Refusing unsafe temporary cleanup: $full"
    }
    $item = Get-Item -LiteralPath $full -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Refusing reparse-point temporary cleanup: $full"
    }
    Remove-Item -LiteralPath $full -Recurse -Force
}

function Copy-WorkingTreePaths {
    param(
        [Parameter(Mandatory = $true)][string]$SourceRoot,
        [Parameter(Mandatory = $true)][string]$DestinationRoot,
        [Parameter(Mandatory = $true)][string[]]$Paths
    )

    $sourcePrefix = $SourceRoot.TrimEnd([char[]]"\/") + [IO.Path]::DirectorySeparatorChar
    $destinationPrefix = $DestinationRoot.TrimEnd([char[]]"\/") + [IO.Path]::DirectorySeparatorChar
    foreach ($relativePath in $Paths) {
        $normalized = ConvertTo-RepositoryPath $relativePath
        if ([string]::IsNullOrWhiteSpace($normalized) -or @($normalized -split '/') -contains "..") {
            throw "Unsafe working-tree path: $relativePath"
        }
        $source = [IO.Path]::GetFullPath((Join-Path $SourceRoot $normalized.Replace('/', [IO.Path]::DirectorySeparatorChar)))
        $destination = [IO.Path]::GetFullPath((Join-Path $DestinationRoot $normalized.Replace('/', [IO.Path]::DirectorySeparatorChar)))
        if (-not $source.StartsWith($sourcePrefix, [StringComparison]::OrdinalIgnoreCase) -or
            -not $destination.StartsWith($destinationPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Working-tree path escaped its root: $relativePath"
        }
        if (Test-Path -LiteralPath $source -PathType Leaf) {
            [void][IO.Directory]::CreateDirectory((Split-Path -Parent $destination))
            [IO.File]::Copy($source, $destination, $true)
        }
        elseif (Test-Path -LiteralPath $destination -PathType Leaf) {
            Remove-Item -LiteralPath $destination -Force
        }
    }
}

function New-TreeSnapshot {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$TemporaryRoot,
        [Parameter(Mandatory = $true)][string[]]$Paths
    )

    $snapshot = Join-Path $TemporaryRoot "tree-repository"
    $headExists = $true
    try { [void](Invoke-GitLines -Root $Root -Arguments @("rev-parse", "--verify", "HEAD")) }
    catch { $headExists = $false }
    if ($headExists) {
        [void](Invoke-GitLines -Root $TemporaryRoot -Arguments @("clone", "--quiet", "--no-hardlinks", $Root, $snapshot))
    }
    else {
        [void][IO.Directory]::CreateDirectory($snapshot)
        [void](Invoke-GitLines -Root $snapshot -Arguments @("init", "--quiet"))
    }
    Copy-WorkingTreePaths -SourceRoot $Root -DestinationRoot $snapshot -Paths $Paths
    [void](Invoke-GitLines -Root $snapshot -Arguments @("add", "-A"))
    return $snapshot
}

function New-StagedSnapshot {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$TemporaryRoot,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$DeletedPaths
    )

    $snapshot = Join-Path $TemporaryRoot "staged-repository"
    $headExists = $true
    try { [void](Invoke-GitLines -Root $Root -Arguments @("rev-parse", "--verify", "HEAD")) }
    catch { $headExists = $false }

    if ($headExists) {
        [void](Invoke-GitLines -Root $TemporaryRoot -Arguments @("clone", "--quiet", "--no-hardlinks", $Root, $snapshot))
    }
    else {
        [void][IO.Directory]::CreateDirectory($snapshot)
        [void](Invoke-GitLines -Root $snapshot -Arguments @("init", "--quiet"))
    }

    $prefix = $snapshot.TrimEnd([char[]]"\/") + [IO.Path]::DirectorySeparatorChar
    [void](Invoke-GitLines -Root $Root -Arguments @("checkout-index", "--all", "--force", "--prefix=$prefix"))
    foreach ($relativePath in $DeletedPaths) {
        $normalized = ConvertTo-RepositoryPath $relativePath
        if ([string]::IsNullOrWhiteSpace($normalized) -or @($normalized -split '/') -contains "..") {
            throw "Unsafe staged deletion path: $relativePath"
        }
        $candidate = [IO.Path]::GetFullPath((Join-Path $snapshot $normalized.Replace('/', [IO.Path]::DirectorySeparatorChar)))
        $snapshotPrefix = $snapshot.TrimEnd([char[]]"\/") + [IO.Path]::DirectorySeparatorChar
        if (-not $candidate.StartsWith($snapshotPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Staged deletion escaped the snapshot: $relativePath"
        }
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            Remove-Item -LiteralPath $candidate -Force
        }
    }
    [void](Invoke-GitLines -Root $snapshot -Arguments @("add", "-A"))
    return $snapshot
}

function New-HeadSnapshot {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$TemporaryRoot
    )

    $snapshot = Join-Path $TemporaryRoot "head-repository"
    [void](Invoke-GitLines -Root $TemporaryRoot -Arguments @("clone", "--quiet", "--no-hardlinks", $Root, $snapshot))
    return $snapshot
}

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = Join-Path $PSScriptRoot "..\..\.."
}
$repository = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd([char[]]"\/")
$powerShellPath = (Get-Process -Id $PID).Path
$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) ("ori3-hook-checks-{0}" -f [Guid]::NewGuid().ToString("N"))
$results = New-Object System.Collections.Generic.List[object]

try {
    [void][IO.Directory]::CreateDirectory($temporaryRoot)
    $trackedPaths = @(Invoke-GitLines -Root $repository -Arguments @("ls-files", "--cached") |
        ForEach-Object { ConvertTo-RepositoryPath $_ } |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) })

    if ($Mode -ceq "Staged") {
        $stagedPaths = @(Invoke-GitLines -Root $repository -Arguments @("diff", "--cached", "--name-only", "--diff-filter=ACMR") |
            ForEach-Object { ConvertTo-RepositoryPath $_ } |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        $deletedPaths = @(Invoke-GitLines -Root $repository -Arguments @("diff", "--cached", "--name-only", "--diff-filter=D") |
            ForEach-Object { ConvertTo-RepositoryPath $_ } |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) })

        # Reject the prohibited path before checkout-index can materialize its body.
        $originalCheckPaths = Get-CheckPaths -ChecksDirectory $PSScriptRoot
        Assert-ChecksAvailable -CheckPaths $originalCheckPaths
        $prohibitedExit = Invoke-OneCheck -Name "no-prohibited-doc" -ScriptPath $originalCheckPaths["no-prohibited-doc"] -Arguments $stagedPaths -PowerShellPath $powerShellPath
        $results.Add([pscustomobject]@{ Name = "no-prohibited-doc"; ExitCode = $prohibitedExit })
        if ($prohibitedExit -ne 0) {
            throw "prohibited document path rejected before staged snapshot creation"
        }

        $executionRoot = New-StagedSnapshot -Root $repository -TemporaryRoot $temporaryRoot -DeletedPaths $deletedPaths
        $checkPaths = Get-CheckPaths -ChecksDirectory (Join-Path $executionRoot "scripts\hooks\checks")
        Assert-ChecksAvailable -CheckPaths $checkPaths
        $targetPaths = $stagedPaths

        $headRoot = New-HeadSnapshot -Root $repository -TemporaryRoot $temporaryRoot
        $headKnownDefect = Invoke-KnownDefectMeasurement -Label "HEAD" -ScriptPath $originalCheckPaths["known-defect-shapes"] -Root $headRoot -PowerShellPath $powerShellPath
        $stagedKnownDefect = Invoke-KnownDefectMeasurement -Label "HEAD+index" -ScriptPath $originalCheckPaths["known-defect-shapes"] -Root $executionRoot -PowerShellPath $powerShellPath
        $knownDefectExit = Compare-StagedKnownDefects -Head $headKnownDefect -Staged $stagedKnownDefect
        $results.Add([pscustomobject]@{ Name = "known-defect-shapes"; ExitCode = $knownDefectExit })
    }
    else {
        $untrackedPaths = @(Invoke-GitLines -Root $repository -Arguments @("ls-files", "--others", "--exclude-standard") |
            ForEach-Object { ConvertTo-RepositoryPath $_ } |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        $treePaths = @($trackedPaths + $untrackedPaths | Select-Object -Unique)

        # Only tracked instances are violations. Untracked instances stay outside the snapshot and are reported.
        $originalCheckPaths = Get-CheckPaths -ChecksDirectory $PSScriptRoot
        Assert-ChecksAvailable -CheckPaths $originalCheckPaths
        $prohibitedCandidates = @($trackedPaths | Where-Object {
            [string]::Equals($_, $script:ProhibitedPath, [StringComparison]::OrdinalIgnoreCase)
        })
        $excludedProhibitedPaths = @($untrackedPaths | Where-Object {
            [string]::Equals($_, $script:ProhibitedPath, [StringComparison]::OrdinalIgnoreCase)
        })
        $excludedFormat = [regex]::Unescape('[\u9664\u5916] \u672a\u8ffd\u8de1\u306e\u7981\u6b62 path {0} \u4ef6\uff08\u672c\u6587\u672a\u8aad\uff09 [EXCLUDED untracked-prohibited-paths={0} body-read=0]')
        Write-Host ($excludedFormat -f $excludedProhibitedPaths.Count)
        $prohibitedExit = Invoke-OneCheck -Name "no-prohibited-doc" -ScriptPath $originalCheckPaths["no-prohibited-doc"] -Arguments $prohibitedCandidates -PowerShellPath $powerShellPath
        $results.Add([pscustomobject]@{ Name = "no-prohibited-doc"; ExitCode = $prohibitedExit })

        $targetPaths = @($treePaths |
            Where-Object { -not [string]::Equals($_, $script:ProhibitedPath, [StringComparison]::OrdinalIgnoreCase) } |
            Select-Object -Unique)
        $executionRoot = New-TreeSnapshot -Root $repository -TemporaryRoot $temporaryRoot -Paths $targetPaths
        $checkPaths = Get-CheckPaths -ChecksDirectory (Join-Path $executionRoot "scripts\hooks\checks")
        Assert-ChecksAvailable -CheckPaths $checkPaths
    }

    foreach ($name in $script:CheckNames) {
        if ($name -ceq "no-prohibited-doc" -or ($Mode -ceq "Staged" -and $name -ceq "known-defect-shapes")) { continue }
        $arguments = switch ($name) {
            "no-allow-attribute" {
                @("-RepositoryRoot", $executionRoot) + @($targetPaths | Where-Object { $_ -match '(?i)\.rs$' })
            }
            "ignore-reason-has-number" {
                @($targetPaths | Where-Object { $_ -match '(?i)\.rs$' })
            }
            "tracked-fixture-only" {
                @($targetPaths | Where-Object {
                    $_ -match '(?i)(^|/)(tests?|__tests__)(/|$)' -or
                    $_ -match '(?i)(?:^|/)[^/]+(?:[._-](?:test|tests|spec))\.(?:rs|ts|tsx|js|jsx|mjs|cjs|ps1|py|cs|c|cc|cpp|h|hpp|java|kt|swift)$'
                })
            }
            "known-defect-shapes" { @("-RepositoryRoot", $executionRoot, "-FailOnDecrease") }
            default { @($targetPaths) }
        }
        $exitCode = Invoke-OneCheck -Name $name -ScriptPath $checkPaths[$name] -Arguments $arguments -PowerShellPath $powerShellPath
        $results.Add([pscustomobject]@{ Name = $name; ExitCode = $exitCode })
    }
}
catch {
    [Console]::Error.WriteLine("[NG] HOOK_CHECKS_FAILED mode=$Mode reason=$($_.Exception.Message)")
    if ($results.Count -eq 0 -or @($results | Where-Object { $_.ExitCode -ne 0 }).Count -eq 0) {
        $results.Add([pscustomobject]@{ Name = "runner"; ExitCode = 2 })
    }
}
finally {
    Remove-SafeTemporaryDirectory -Path $temporaryRoot
}

$failed = @($results | Where-Object { $_.ExitCode -ne 0 })
if ($failed.Count -gt 0) {
    $unavailable = @($failed | Where-Object { $_.ExitCode -ne 1 }).Count
    $violations = @($failed | Where-Object { $_.ExitCode -eq 1 }).Count
    [Console]::Error.WriteLine("[NG] HOOK_CHECKS_SUMMARY mode=$Mode checks=$($script:CheckNames.Count) violations=$violations unavailable=$unavailable")
    if ($unavailable -gt 0) { exit 2 }
    exit 1
}

Write-Host "[OK] HOOK_CHECKS_SUMMARY mode=$Mode checks=$($script:CheckNames.Count) violations=0 unavailable=0"
exit 0
