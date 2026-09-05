<#
.SYNOPSIS
Warns when staged test changes appear to narrow the tested claim.

.DESCRIPTION
This is deliberately nonblocking: it reports heuristic warning candidates and
always exits zero after a successful scan. It examines staged test files for three
signals: removed assertion calls, a removed dynamic loop paired with an added
fixed collection literal, and a newly added Rust #[test] body with no direct
failure signal. It cannot prove that a test contract was weakened.
#>
[CmdletBinding()]
param(
    [string]$RepositoryRoot = "",
    [string[]]$Files = @(),
    [switch]$FailOnVacuousRustTest
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $scriptDirectory = [string]$PSScriptRoot
    $scriptDirectorySource = '$PSScriptRoot'
    if ([string]::IsNullOrWhiteSpace($scriptDirectory)) {
        $invocationPath = [string]$MyInvocation.MyCommand.Path
        if (-not [string]::IsNullOrWhiteSpace($invocationPath)) {
            $scriptDirectory = Split-Path -Parent ([IO.Path]::GetFullPath($invocationPath))
            $scriptDirectorySource = '$MyInvocation.MyCommand.Path'
        }
    }
    if ([string]::IsNullOrWhiteSpace($scriptDirectory)) {
        throw "RepositoryRoot was not supplied and the script directory could not be determined."
    }
    $RepositoryRoot = Join-Path $scriptDirectory "..\..\.."
    Write-Verbose "Default RepositoryRoot resolved through $scriptDirectorySource."
}
$repositoryRoot = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd([char[]]"\\/")
$assertionPattern = [regex]::new('(?:\bassert(?:_eq|_ne)?!\s*\(|\bexpect\s*\(|\bAssert-[A-Za-z][A-Za-z-]*)')
$dynamicLoopPattern = [regex]::new('(?:\bfor\b[^\r\n]*(?:\.iter\s*\(|\.collect\s*\(|\.values\s*\(|\.keys\s*\(|\.entries\s*\(|read_dir\s*\(|glob\s*\())')
$literalLoopPattern = [regex]::new('(?:\bfor\b[^\r\n]*\bin\s*\[[^\]\r\n]*\]|\b(?:let|const|var)\s+[A-Za-z_][A-Za-z0-9_]*\s*=\s*\[[^\]\r\n]*\])')
$rustTestAttributePattern = [regex]::new('^\s*#\s*\[\s*test\s*\]\s*$')
$rustFunctionPattern = [regex]::new('^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\b')
$rustFailureSignalPattern = [regex]::new('(?:\b(?:debug_)?assert(?:_[A-Za-z0-9_]+)?!\s*\(|\b(?:panic|unreachable|todo|bail|ensure)!\s*\(|\.(?:expect|unwrap(?:_err)?)\s*\(|\?|\bErr\s*\()')

function Get-RustTestDefinitions {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][AllowEmptyString()][string[]]$Lines)

    $definitions = @()
    for ($index = 0; $index -lt $lines.Count; $index += 1) {
        if (-not $rustTestAttributePattern.IsMatch($lines[$index])) { continue }

        $cursor = $index + 1
        $hasShouldPanic = $false
        while ($cursor -lt $lines.Count -and -not $rustFunctionPattern.IsMatch($lines[$cursor])) {
            if ($lines[$cursor] -match '^\s*#\s*\[\s*should_panic(?:\s*\([^\]]*\))?\s*\]\s*$') {
                $hasShouldPanic = $true
            }
            $cursor += 1
        }
        if ($cursor -ge $lines.Count) { continue }

        $functionMatch = $rustFunctionPattern.Match($lines[$cursor])
        $functionName = $functionMatch.Groups[1].Value
        $bodyLines = New-Object System.Collections.Generic.List[string]
        $braceDepth = 0
        $bodyStarted = $false
        for ($bodyIndex = $cursor; $bodyIndex -lt $lines.Count; $bodyIndex += 1) {
            $bodyLine = $lines[$bodyIndex]
            if (-not $bodyStarted) {
                $openingBrace = $bodyLine.IndexOf('{')
                if ($openingBrace -lt 0) { continue }
                $bodyStarted = $true
                $bodyLine = $bodyLine.Substring($openingBrace + 1)
                $braceDepth = 1
            }
            [void]$bodyLines.Add($bodyLine)
            $braceDepth += ([regex]::Matches($bodyLine, '\{').Count - [regex]::Matches($bodyLine, '\}').Count)
            if ($braceDepth -le 0) { break }
        }
        if (-not $bodyStarted) { continue }
        $definitions += [PSCustomObject]@{
            Name = $functionName
            Body = $bodyLines -join "`n"
            HasShouldPanic = $hasShouldPanic
        }
    }
    return @($definitions)
}

function Get-AddedRustTestNamesWithoutDirectFailureSignal {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not $Path.EndsWith('.rs', [StringComparison]::OrdinalIgnoreCase)) {
        return @()
    }

    $normalizedPath = $Path.Replace('\', '/')
    $headTestNames = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal)
    $headPaths = @(Invoke-GitText -Arguments @('ls-tree', '--name-only', 'HEAD', '--', $normalizedPath))
    if ($headPaths -contains $normalizedPath) {
        $headContent = @(Invoke-GitText -Arguments @('show', "HEAD:$normalizedPath"))
        foreach ($definition in Get-RustTestDefinitions -Lines $headContent) {
            [void]$headTestNames.Add([string]$definition.Name)
        }
    }

    # The index is the exact changed snapshot that pre-commit will commit. Using
    # the unstaged file here would let an unstaged assertion hide a vacuous test
    # that remains in the index.
    $stagedContent = @(Invoke-GitText -Arguments @('show', ":$normalizedPath"))
    # Deliberately not named $findings: that name belongs to the outer warning
    # list. Reusing it here made the known-defect-shapes ratchet, which counts
    # "$findings.Add(" as the number of warning signals, count this inner
    # accumulator as a fourth signal that does not exist.
    $vacuousTestNames = New-Object System.Collections.Generic.List[string]
    foreach ($definition in Get-RustTestDefinitions -Lines $stagedContent) {
        if ($headTestNames.Contains([string]$definition.Name)) { continue }
        if ([bool]$definition.HasShouldPanic) { continue }
        if (-not $rustFailureSignalPattern.IsMatch([string]$definition.Body)) {
            $vacuousTestNames.Add([string]$definition.Name)
        }
    }
    return @($vacuousTestNames)
}

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
$vacuousRustTestFindings = New-Object System.Collections.Generic.List[object]
$stagedTestPaths = @(Get-StagedTestPaths)
foreach ($path in $stagedTestPaths) {
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
    foreach ($functionName in Get-AddedRustTestNamesWithoutDirectFailureSignal -Path $path) {
        $finding = [PSCustomObject]@{
            Path = $path
            Signal = 'new Rust test has no direct failure signal'
            Detail = "function=$functionName"
        }
        $findings.Add($finding)
        $vacuousRustTestFindings.Add($finding)
    }
}

if ($findings.Count -eq 0) {
    Write-Host "[OK] test-claim-scope scan completed: targets=$($stagedTestPaths.Count), findings=0"
    exit 0
}

foreach ($finding in $findings) {
    Write-Warning "Test claim scope may have narrowed: $($finding.Path) - $($finding.Signal) ($($finding.Detail))"
}
if ($FailOnVacuousRustTest -and $vacuousRustTestFindings.Count -gt 0) {
    Write-Host "[NG] new Rust #[test] without a direct failure signal blocks the commit. Add a failure assertion to the named test."
    exit 2
}
Write-Warning "This is nonblocking. Review the staged diff and confirm that every removed assertion or narrowed iteration is intentional."
Write-Host "[OK] test-claim-scope scan completed: targets=$($stagedTestPaths.Count), findings=$($findings.Count)"
exit 0
