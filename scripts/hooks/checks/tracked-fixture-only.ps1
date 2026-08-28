[CmdletBinding()]
param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [AllowEmptyCollection()]
    [string[]]$Files = @()
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

function Get-NormalizedGitPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $normalized = $Path.Replace("\", "/")
    while ($normalized.StartsWith("./", [System.StringComparison]::Ordinal)) {
        $normalized = $normalized.Substring(2)
    }
    return $normalized
}

function Test-IsTestSource {
    param([Parameter(Mandatory = $true)][string]$Path)

    $normalized = Get-NormalizedGitPath $Path
    if ($normalized -notmatch '(?i)(\.rs|\.ts|\.tsx|\.js|\.jsx|\.mjs|\.cjs|\.ps1|\.py|\.cs|\.c|\.cc|\.cpp|\.h|\.hpp|\.java|\.kt|\.swift)$') {
        return $false
    }

    if ($normalized -match '(?i)(^|/)(tests?|__tests__)(/|$)') {
        return $true
    }

    $leaf = @($normalized -split '/')[-1]
    return $leaf -match '(?i)(^|[._-])(tests?|spec)([._-]|$)'
}

function Get-AddedLines {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $global:LASTEXITCODE = 0
    $diff = @(& git -C $RepositoryRoot -c core.excludesFile= diff --cached --no-color --no-ext-diff --no-textconv --unified=0 -- $Path)
    $status = $LASTEXITCODE
    if ($status -ne 0) {
        throw "git diff --cached に失敗しました（終了コード: $status、対象: $Path）"
    }

    $added = New-Object System.Collections.Generic.List[string]
    foreach ($line in $diff) {
        if ($line.StartsWith("+", [System.StringComparison]::Ordinal) -and
            -not $line.StartsWith("+++", [System.StringComparison]::Ordinal)) {
            $added.Add($line.Substring(1))
        }
    }
    return $added.ToArray()
}

function Test-IsInsideRepository {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$Candidate
    )

    $root = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd([char[]]"\/")
    $full = [IO.Path]::GetFullPath($Candidate)
    if ($full.Equals($root, [System.StringComparison]::OrdinalIgnoreCase)) {
        return $true
    }
    $prefix = $root + [IO.Path]::DirectorySeparatorChar
    return $full.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)
}

function Get-RepositoryRelativeCandidates {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$SourcePath,
        [Parameter(Mandatory = $true)][string]$LiteralPath
    )

    $repositoryFull = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd([char[]]"\/")
    $sourceNormalized = Get-NormalizedGitPath $SourcePath
    $sourceFull = [IO.Path]::GetFullPath((Join-Path $repositoryFull ($sourceNormalized.Replace("/", "\"))))
    $sourceDirectory = [IO.Path]::GetDirectoryName($sourceFull)

    $segments = @($sourceNormalized -split '/')
    $componentRoot = $repositoryFull
    if ($segments.Count -ge 2 -and ($segments[0] -eq "crates" -or $segments[0] -eq "apps")) {
        $componentRoot = Join-Path (Join-Path $repositoryFull $segments[0]) $segments[1]
    }

    $literalSystemPath = $LiteralPath.Replace("/", "\")
    $trimmedLiteral = $literalSystemPath.TrimStart([char[]]"\/")
    $baseCandidates = New-Object System.Collections.Generic.List[string]

    if ([IO.Path]::IsPathRooted($literalSystemPath) -and -not $literalSystemPath.StartsWith("\") -and
        -not $literalSystemPath.StartsWith("/")) {
        $baseCandidates.Add($literalSystemPath)
    }
    elseif ($literalSystemPath.StartsWith("\") -or $literalSystemPath.StartsWith("/")) {
        $baseCandidates.Add((Join-Path $componentRoot $trimmedLiteral))
        $baseCandidates.Add((Join-Path $repositoryFull $trimmedLiteral))
    }
    else {
        $baseCandidates.Add((Join-Path $sourceDirectory $literalSystemPath))
        $baseCandidates.Add((Join-Path $componentRoot $literalSystemPath))
        $baseCandidates.Add((Join-Path $repositoryFull $literalSystemPath))
    }

    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
    $relativeCandidates = New-Object System.Collections.Generic.List[string]
    foreach ($candidate in $baseCandidates) {
        try {
            $fullCandidate = [IO.Path]::GetFullPath($candidate)
        }
        catch {
            continue
        }
        if (-not (Test-IsInsideRepository -RepositoryRoot $repositoryFull -Candidate $fullCandidate)) {
            continue
        }

        $relative = $fullCandidate.Substring($repositoryFull.Length).TrimStart([char[]]"\/").Replace("\", "/")
        if ($seen.Add($relative)) {
            $relativeCandidates.Add($relative)
        }
    }
    return $relativeCandidates.ToArray()
}

function Test-IsTrackedPath {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $global:LASTEXITCODE = 0
    $listed = @(& git -C $RepositoryRoot -c core.quotePath=false -c core.excludesFile= ls-files --cached -- $Path)
    $status = $LASTEXITCODE
    if ($status -ne 0) {
        throw "git ls-files に失敗しました（終了コード: $status、対象: $Path）"
    }

    foreach ($listedPath in $listed) {
        if ((Get-NormalizedGitPath ([string]$listedPath)) -ceq $Path) {
            return $true
        }
    }
    return $false
}

try {
    $repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\..\.."))
    $testFiles = New-Object System.Collections.Generic.List[string]
    $seenFiles = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($file in @($Files)) {
        if ([string]::IsNullOrWhiteSpace($file)) {
            continue
        }
        $normalized = Get-NormalizedGitPath $file
        if ((Test-IsTestSource $normalized) -and $seenFiles.Add($normalized)) {
            $testFiles.Add($normalized)
        }
    }

    $referenceCount = 0
    $violations = New-Object System.Collections.Generic.List[object]
    $literalPattern = [regex]::new(
        '(?<quote>["''])(?<path>[^"'']*(?<![A-Za-z0-9_])(?:__fixtures__|fixtures?)[\/][^"'']*)\k<quote>',
        [Text.RegularExpressions.RegexOptions]::IgnoreCase
    )

    foreach ($testFile in $testFiles) {
        $addedLines = @(Get-AddedLines -RepositoryRoot $repositoryRoot -Path $testFile)
        foreach ($line in $addedLines) {
            $trimmed = $line.TrimStart()
            if ($trimmed.StartsWith("//") -or $trimmed.StartsWith("///") -or
                $trimmed.StartsWith("/*") -or $trimmed.StartsWith("*") -or
                $trimmed.StartsWith("# ")) {
                continue
            }

            foreach ($match in $literalPattern.Matches($line)) {
                $literalPath = $match.Groups["path"].Value.Trim()
                if ([string]::IsNullOrWhiteSpace($literalPath)) {
                    continue
                }
                $referenceCount += 1

                if ($literalPath.IndexOfAny([char[]]"{}*") -ge 0) {
                    $violations.Add([pscustomobject]@{
                        Source = $testFile
                        Literal = $literalPath
                        Candidates = @()
                        Reason = "動的・ワイルドカードのパスで、追跡対象の実ファイルを確定できません"
                    })
                    continue
                }

                $candidates = @(Get-RepositoryRelativeCandidates -RepositoryRoot $repositoryRoot -SourcePath $testFile -LiteralPath $literalPath)
                $tracked = $false
                foreach ($candidate in $candidates) {
                    if (Test-IsTrackedPath -RepositoryRoot $repositoryRoot -Path $candidate) {
                        $tracked = $true
                        break
                    }
                }
                if (-not $tracked) {
                    $violations.Add([pscustomobject]@{
                        Source = $testFile
                        Literal = $literalPath
                        Candidates = $candidates
                        Reason = "参照先が git の追跡対象ではありません"
                    })
                }
            }
        }
    }

    if ($violations.Count -gt 0) {
        Write-Output "[NG] fixture参照検査: 未追跡または検査不能な参照が $($violations.Count) 件あります。"
        foreach ($violation in $violations) {
            Write-Output "  対象: $($violation.Source)"
            Write-Output "  参照: $($violation.Literal)"
            Write-Output "  理由: $($violation.Reason)"
            if (@($violation.Candidates).Count -gt 0) {
                Write-Output "  解決候補: $(@($violation.Candidates) -join ', ')"
            }
        }
        Write-Output "  fixture をコミット対象に追加し、git ls-files で追跡対象になったことを確認してください。"
        exit 1
    }

    Write-Output "[OK] fixture参照検査: 対象テストソース $($testFiles.Count) 件、追加fixture参照 $referenceCount 件、未追跡 0件"
    exit 0
}
catch {
    Write-Output "[NG] fixture参照検査を完了できませんでした: $($_.Exception.Message)"
    exit 2
}
