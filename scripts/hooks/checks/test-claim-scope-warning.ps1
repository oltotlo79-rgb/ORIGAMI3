<#
.SYNOPSIS
Warns when staged test changes appear to narrow the tested claim.

.DESCRIPTION
This is deliberately nonblocking: it reports heuristic warning candidates and
always exits zero after a successful scan. It examines staged test files for two
signals: removed assertion calls, and a removed dynamic loop paired with an added
fixed collection literal. It cannot prove that a test contract was weakened.
#>
[CmdletBinding()]
param(
    [string]$RepositoryRoot = (Join-Path $PSScriptRoot "..\..\.."),
    [string[]]$Files = @()
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$repositoryRoot = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd([char[]]"\\/")
$assertionPattern = [regex]::new('(?:\bassert(?:_eq|_ne)?!\s*\(|\bexpect\s*\(|\bAssert-[A-Za-z][A-Za-z-]*)')
$dynamicLoopPattern = [regex]::new('(?:\bfor\b[^\r\n]*(?:\.iter\s*\(|\.collect\s*\(|\.values\s*\(|\.keys\s*\(|\.entries\s*\(|read_dir\s*\(|glob\s*\())')
$literalLoopPattern = [regex]::new('(?:\bfor\b[^\r\n]*\bin\s*\[[^\]\r\n]*\]|\b(?:let|const|var)\s+[A-Za-z_][A-Za-z0-9_]*\s*=\s*\[[^\]\r\n]*\])')

function Invoke-GitText {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)

    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = @(& git -C $repositoryRoot -c core.quotePath=false @Arguments)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($exitCode -ne 0) {
        throw "git $($Arguments -join ' ') failed with exit code $exitCode"
    }
    return @($output | ForEach-Object { [string]$_ })
}

function Test-IsTestPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $normalized = $Path.Replace("\\", "/")
    return ($normalized -match '(^|/)(tests?|__tests__|spec)(/|$)' -or
        $normalized -match '(?:^|/)[^/]+(?:\.test|\.spec)\.(?:rs|ts|tsx|js|jsx|ps1)$' -or
        $normalized -match '(?:^|/)[^/]+_test\.(?:rs|ps1)$')
}

function Get-StagedTestPaths {
    if ($Files.Count -gt 0) {
        return @($Files | Where-Object { -not [string]::IsNullOrWhiteSpace($_) -and (Test-IsTestPath $_) } | Select-Object -Unique)
    }
    return @(Invoke-GitText -Arguments @("diff", "--cached", "--name-only", "--diff-filter=ACMR") |
        Where-Object { Test-IsTestPath $_ } |
        Select-Object -Unique)
}

$findings = New-Object System.Collections.Generic.List[object]
foreach ($path in @(Get-StagedTestPaths)) {
    $diffLines = Invoke-GitText -Arguments @("diff", "--cached", "--no-color", "--no-ext-diff", "--unified=0", "--", $path)
    $addedAssertions = 0
    $removedAssertions = 0
    $removedDynamicLoop = $false
    $addedLiteralLoop = $false

    foreach ($line in $diffLines) {
        if ($line.StartsWith("+++", [StringComparison]::Ordinal) -or $line.StartsWith("---", [StringComparison]::Ordinal)) { continue }
        if ($line.StartsWith("-", [StringComparison]::Ordinal)) {
            $removedAssertions += $assertionPattern.Matches($line.Substring(1)).Count
            if ($dynamicLoopPattern.IsMatch($line.Substring(1))) { $removedDynamicLoop = $true }
        }
        elseif ($line.StartsWith("+", [StringComparison]::Ordinal)) {
            $addedAssertions += $assertionPattern.Matches($line.Substring(1)).Count
            if ($literalLoopPattern.IsMatch($line.Substring(1))) { $addedLiteralLoop = $true }
        }
    }

    if ($removedAssertions -gt $addedAssertions) {
        $findings.Add([PSCustomObject]@{
            Path = $path
            Signal = "assertion calls decreased"
            Detail = "removed=$removedAssertions added=$addedAssertions"
        })
    }
    if ($removedDynamicLoop -and $addedLiteralLoop) {
        $findings.Add([PSCustomObject]@{
            Path = $path
            Signal = "dynamic iteration may have become a fixed literal"
            Detail = "removed dynamic loop signal and added literal collection signal"
        })
    }
}

if ($findings.Count -eq 0) {
    Write-Host "[OK] No staged test-claim narrowing signal was found."
    exit 0
}

foreach ($finding in $findings) {
    Write-Warning "Test claim scope may have narrowed: $($finding.Path) - $($finding.Signal) ($($finding.Detail))"
}
Write-Warning "This is nonblocking. Review the staged diff and confirm that every removed assertion or narrowed iteration is intentional."
exit 0
