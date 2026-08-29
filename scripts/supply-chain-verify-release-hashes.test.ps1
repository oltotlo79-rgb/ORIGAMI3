[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# This is an isolated test that proves post-publish verification works without cutting an
# actual release. It targets scripts/supply-chain-verify-release-hashes.ps1 and builds its
# fixtures with the real scripts/supply-chain-generate-release-hashes.ps1, instead of a
# hand-written manifest/record that could silently drift from the real generator's format.
# Using the same generator release.yml uses also catches any mismatch between the two.
#
# All comments in this file are kept in plain ASCII on purpose. During authoring, this
# script failed to parse when invoked as `powershell.exe -File <path>` (Windows PowerShell
# 5.1, not pwsh) on a machine whose system codepage is 932 (Shift-JIS), even though the file
# itself was valid UTF-8 without a BOM. Adding a UTF-8 BOM made it parse correctly, which
# confirms encoding auto-detection was the cause; several existing repository scripts with
# Japanese comments and no BOM did not reproduce the failure, so the exact trigger was not
# fully isolated. Because this file is meant to be run directly and repeatedly on that same
# machine, staying ASCII-only sidesteps the risk entirely rather than relying on a BOM that
# would make this file inconsistent with the no-BOM convention used elsewhere in scripts/.

$VerifyScriptPath = Join-Path $PSScriptRoot "supply-chain-verify-release-hashes.ps1"
$GenerateScriptPath = Join-Path $PSScriptRoot "supply-chain-generate-release-hashes.ps1"
foreach ($requiredScript in @($VerifyScriptPath, $GenerateScriptPath)) {
    if (-not (Test-Path -LiteralPath $requiredScript -PathType Leaf)) {
        throw "Required implementation is missing: $requiredScript"
    }
}

$SandboxRoot = Join-Path ([IO.Path]::GetTempPath()) ("ori3-supply-chain-verify-test-" + [Guid]::NewGuid().ToString("N"))
$script:AssertionCount = 0
$Utf8NoBom = [Text.UTF8Encoding]::new($false)
$FixtureVersion = "9.9.9"
$FixtureBuildId = "isolated-test-build-1"

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $script:AssertionCount += 1
    if (-not $Condition) {
        throw "ASSERTION FAILED: $Message"
    }
}

function Assert-Equal {
    param(
        [AllowNull()]$Actual,
        [AllowNull()]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $script:AssertionCount += 1
    if ($Actual -ne $Expected) {
        throw "ASSERTION FAILED: $Message (expected=$Expected, actual=$Actual)"
    }
}

function New-BaselineFixture {
    # Builds four fake distribution artifacts and a real-format manifest/record pair in an
    # isolated, disposable folder. To simulate "re-downloaded", the artifacts are copied
    # (under their published names) into a separate downloaded\ directory rather than reused
    # from the source files the generator hashed.
    param([Parameter(Mandatory = $true)][string]$Name)

    $root = Join-Path $SandboxRoot $Name
    $sourceDir = Join-Path $root "source"
    $recordsDir = Join-Path $root "records"
    $downloadedDir = Join-Path $root "downloaded"
    foreach ($dir in @($root, $sourceDir, $recordsDir, $downloadedDir)) {
        [void][IO.Directory]::CreateDirectory($dir)
    }

    $setupSource = Join-Path $sourceDir "setup-source.bin"
    $msiSource = Join-Path $sourceDir "msi-source.bin"
    $portableSource = Join-Path $sourceDir "portable-source.bin"
    $manualSource = Join-Path $sourceDir "manual-source.bin"
    [IO.File]::WriteAllText($setupSource, "fake setup installer bytes $([Guid]::NewGuid())", $Utf8NoBom)
    [IO.File]::WriteAllText($msiSource, "fake msi installer bytes $([Guid]::NewGuid())", $Utf8NoBom)
    [IO.File]::WriteAllText($portableSource, "fake portable exe bytes $([Guid]::NewGuid())", $Utf8NoBom)
    [IO.File]::WriteAllText($manualSource, "fake manual pdf bytes $([Guid]::NewGuid())", $Utf8NoBom)

    $generateStdOut = Join-Path $root "generate-stdout.txt"
    $generateStdErr = Join-Path $root "generate-stderr.txt"
    $generateArguments = @(
        "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ('"{0}"' -f $GenerateScriptPath),
        "-Version", $FixtureVersion,
        "-BuildId", $FixtureBuildId,
        "-SetupPath", ('"{0}"' -f $setupSource),
        "-MsiPath", ('"{0}"' -f $msiSource),
        "-PortablePath", ('"{0}"' -f $portableSource),
        "-ManualPath", ('"{0}"' -f $manualSource),
        "-DestinationDirectory", ('"{0}"' -f $recordsDir)
    ) -join ' '
    $generateProcess = Start-Process -FilePath "powershell.exe" -ArgumentList $generateArguments `
        -NoNewWindow -PassThru -Wait `
        -RedirectStandardOutput $generateStdOut -RedirectStandardError $generateStdErr
    if ($generateProcess.ExitCode -ne 0) {
        throw "Baseline fixture generation failed unexpectedly (exit $($generateProcess.ExitCode)); see $generateStdErr"
    }

    $manifestPath = Join-Path $recordsDir ("ORIGAMI3_{0}_SHA256SUMS.txt" -f $FixtureVersion)
    $recordPath = Join-Path $recordsDir ("ORIGAMI3_{0}_artifact-record.json" -f $FixtureVersion)

    # Simulate "re-fetched from the publication target": copy the exact bytes the generator
    # hashed into downloaded\ under their published (artifactName) names.
    Copy-Item -LiteralPath $setupSource -Destination (Join-Path $downloadedDir ("ORIGAMI3_{0}_setup.exe" -f $FixtureVersion))
    Copy-Item -LiteralPath $msiSource -Destination (Join-Path $downloadedDir ("ORIGAMI3_{0}_x64.msi" -f $FixtureVersion))
    Copy-Item -LiteralPath $portableSource -Destination (Join-Path $downloadedDir ("ORIGAMI3_{0}_portable.exe" -f $FixtureVersion))
    Copy-Item -LiteralPath $manualSource -Destination (Join-Path $downloadedDir "ORIGAMI3.pdf")

    [pscustomobject]@{
        Root          = $root
        RecordsDir    = $recordsDir
        DownloadedDir = $downloadedDir
        ManifestPath  = $manifestPath
        RecordPath    = $recordPath
    }
}

function Invoke-VerifyScript {
    # Runs the script under test as a genuinely new process and returns that process's real
    # exit code (Start-Process -PassThru's ExitCode), not this session's $? or a function
    # return value.
    param(
        [Parameter(Mandatory = $true)][string]$ManifestPath,
        [Parameter(Mandatory = $true)][string]$DownloadedDirectory,
        [Parameter(Mandatory = $true)][string]$RecordPath,
        [Parameter(Mandatory = $true)][string]$ResultPath,
        [Parameter(Mandatory = $true)][string]$StdOutPath,
        [Parameter(Mandatory = $true)][string]$StdErrPath
    )

    $argumentList = @(
        "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ('"{0}"' -f $VerifyScriptPath),
        "-ManifestPath", ('"{0}"' -f $ManifestPath),
        "-DownloadedDirectory", ('"{0}"' -f $DownloadedDirectory),
        "-RecordPath", ('"{0}"' -f $RecordPath),
        "-ResultPath", ('"{0}"' -f $ResultPath)
    ) -join ' '

    $process = Start-Process -FilePath "powershell.exe" -ArgumentList $argumentList `
        -NoNewWindow -PassThru -Wait `
        -RedirectStandardOutput $StdOutPath -RedirectStandardError $StdErrPath
    return $process.ExitCode
}

function Get-MutatedRecordPath {
    param(
        [Parameter(Mandatory = $true)]$Fixture,
        [Parameter(Mandatory = $true)][string]$Suffix,
        [Parameter(Mandatory = $true)][scriptblock]$Mutate
    )

    $record = Get-Content -LiteralPath $Fixture.RecordPath -Raw -Encoding UTF8 | ConvertFrom-Json
    & $Mutate $record
    $mutatedPath = Join-Path $Fixture.Root ("mutated-record-{0}.json" -f $Suffix)
    [IO.File]::WriteAllText($mutatedPath, (($record | ConvertTo-Json -Depth 6) + [Environment]::NewLine), $Utf8NoBom)
    return $mutatedPath
}

function Get-MutatedManifestPath {
    param(
        [Parameter(Mandatory = $true)]$Fixture,
        [Parameter(Mandatory = $true)][string]$Suffix,
        [Parameter(Mandatory = $true)][scriptblock]$Mutate
    )

    $lines = New-Object System.Collections.Generic.List[string]
    $lines.AddRange([string[]](Get-Content -LiteralPath $Fixture.ManifestPath -Encoding ASCII))
    $mutatedLines = & $Mutate $lines
    $mutatedPath = Join-Path $Fixture.Root ("mutated-manifest-{0}.txt" -f $Suffix)
    [IO.File]::WriteAllLines($mutatedPath, $mutatedLines, [Text.Encoding]::ASCII)
    return $mutatedPath
}

[void][IO.Directory]::CreateDirectory($SandboxRoot)

try {
    Write-Host "[1/9] four re-downloaded artifacts that match the published record must pass (exit 0)"
    $fixture = New-BaselineFixture "pass-baseline"
    $resultPath = Join-Path $fixture.Root "result.json"
    $exitCode = Invoke-VerifyScript -ManifestPath $fixture.ManifestPath -DownloadedDirectory $fixture.DownloadedDir `
        -RecordPath $fixture.RecordPath -ResultPath $resultPath `
        -StdOutPath (Join-Path $fixture.Root "stdout.txt") -StdErrPath (Join-Path $fixture.Root "stderr.txt")
    Write-Host "  new-process exit code: $exitCode"
    Assert-Equal $exitCode 0 "matching downloaded artifacts must exit 0 in a new process"
    Assert-True (Test-Path -LiteralPath $resultPath -PathType Leaf) "a passing run must write the result file"
    # Bind the parsed JSON to a plain variable first, then wrap with @(). ConvertFrom-Json
    # writes its parsed value as a single pipeline object; wrapping the pipe expression
    # itself directly in @(...) would nest a 4-element array inside a 1-element array
    # instead of flattening it (this was caught by actually running this test, not by
    # reading the code: an earlier version used the one-line form and asserted Count -eq 4,
    # which failed with actual=1 even though the underlying result.json correctly held four
    # entries).
    $parsedResult = Get-Content -LiteralPath $resultPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $result = @($parsedResult)
    Assert-Equal $result.Count 4 "the result must list four artifacts"
    Assert-Equal (@($result | Where-Object { $_.result -ne "match" }).Count) 0 "all four artifacts must report result=match"
    Assert-Equal (@($result | Select-Object -ExpandProperty artifactName -Unique).Count) 4 "the result must name four distinct artifacts"

    Write-Host "[2/9] a single changed byte in one downloaded artifact must fail (non-zero exit)"
    $fixture = New-BaselineFixture "fail-byte-changed"
    $targetFile = Join-Path $fixture.DownloadedDir ("ORIGAMI3_{0}_setup.exe" -f $FixtureVersion)
    $bytes = [IO.File]::ReadAllBytes($targetFile)
    $bytes[$bytes.Length - 1] = $bytes[$bytes.Length - 1] -bxor 0xFF
    [IO.File]::WriteAllBytes($targetFile, $bytes)
    $resultPath = Join-Path $fixture.Root "result.json"
    $exitCode = Invoke-VerifyScript -ManifestPath $fixture.ManifestPath -DownloadedDirectory $fixture.DownloadedDir `
        -RecordPath $fixture.RecordPath -ResultPath $resultPath `
        -StdOutPath (Join-Path $fixture.Root "stdout.txt") -StdErrPath (Join-Path $fixture.Root "stderr.txt")
    Write-Host "  new-process exit code: $exitCode"
    Assert-True ($exitCode -ne 0) "a single byte change in a downloaded artifact must fail verification"
    Assert-True (-not (Test-Path -LiteralPath $resultPath)) "a failed run must not write a false result file"
    $stderrText = Get-Content -LiteralPath (Join-Path $fixture.Root "stderr.txt") -Raw -Encoding UTF8
    Assert-True ($stderrText -match [regex]::Escape("Downloaded artifact hash does not match")) "the failure reason must name a hash mismatch"

    Write-Host "[3/9] a missing downloaded artifact must fail (non-zero exit)"
    $fixture = New-BaselineFixture "fail-missing-file"
    Remove-Item -LiteralPath (Join-Path $fixture.DownloadedDir ("ORIGAMI3_{0}_x64.msi" -f $FixtureVersion)) -Force
    $resultPath = Join-Path $fixture.Root "result.json"
    $exitCode = Invoke-VerifyScript -ManifestPath $fixture.ManifestPath -DownloadedDirectory $fixture.DownloadedDir `
        -RecordPath $fixture.RecordPath -ResultPath $resultPath `
        -StdOutPath (Join-Path $fixture.Root "stdout.txt") -StdErrPath (Join-Path $fixture.Root "stderr.txt")
    Write-Host "  new-process exit code: $exitCode"
    Assert-True ($exitCode -ne 0) "a missing downloaded artifact must fail verification"
    Assert-True (-not (Test-Path -LiteralPath $resultPath)) "a failed run must not write a false result file"
    $stderrText = Get-Content -LiteralPath (Join-Path $fixture.Root "stderr.txt") -Raw -Encoding UTF8
    Assert-True ($stderrText -match [regex]::Escape("Downloaded artifact is missing")) "the failure reason must name the missing artifact"

    Write-Host "[4/9] a duplicated artifact name inside the record (4 entries, 3 unique) must fail"
    $fixture = New-BaselineFixture "fail-duplicate-record-name"
    $mutatedRecordPath = Get-MutatedRecordPath -Fixture $fixture -Suffix "dup-name" -Mutate {
        param($record)
        $record.artifacts[1].artifactName = $record.artifacts[0].artifactName
    }
    $resultPath = Join-Path $fixture.Root "result.json"
    $exitCode = Invoke-VerifyScript -ManifestPath $fixture.ManifestPath -DownloadedDirectory $fixture.DownloadedDir `
        -RecordPath $mutatedRecordPath -ResultPath $resultPath `
        -StdOutPath (Join-Path $fixture.Root "stdout.txt") -StdErrPath (Join-Path $fixture.Root "stderr.txt")
    Write-Host "  new-process exit code: $exitCode"
    Assert-True ($exitCode -ne 0) "a duplicated artifact name in the record must fail verification"
    Assert-True (-not (Test-Path -LiteralPath $resultPath)) "a failed run must not write a false result file"
    $stderrText = Get-Content -LiteralPath (Join-Path $fixture.Root "stderr.txt") -Raw -Encoding UTF8
    Assert-True ($stderrText -match [regex]::Escape("Artifact record has duplicate names")) "the failure reason must name the duplication"

    Write-Host "[5/9] a record with only three artifacts must fail"
    $fixture = New-BaselineFixture "fail-record-count-three"
    $mutatedRecordPath = Get-MutatedRecordPath -Fixture $fixture -Suffix "count-three" -Mutate {
        param($record)
        $record.artifacts = @($record.artifacts | Select-Object -First 3)
    }
    $resultPath = Join-Path $fixture.Root "result.json"
    $exitCode = Invoke-VerifyScript -ManifestPath $fixture.ManifestPath -DownloadedDirectory $fixture.DownloadedDir `
        -RecordPath $mutatedRecordPath -ResultPath $resultPath `
        -StdOutPath (Join-Path $fixture.Root "stdout.txt") -StdErrPath (Join-Path $fixture.Root "stderr.txt")
    Write-Host "  new-process exit code: $exitCode"
    Assert-True ($exitCode -ne 0) "a record with three artifacts must fail verification"
    Assert-True (-not (Test-Path -LiteralPath $resultPath)) "a failed run must not write a false result file"
    $stderrText = Get-Content -LiteralPath (Join-Path $fixture.Root "stderr.txt") -Raw -Encoding UTF8
    Assert-True ($stderrText -match [regex]::Escape("must contain exactly four artifacts")) "the failure reason must name the wrong count"

    Write-Host "[6/9] a record with five artifacts must fail"
    $fixture = New-BaselineFixture "fail-record-count-five"
    $mutatedRecordPath = Get-MutatedRecordPath -Fixture $fixture -Suffix "count-five" -Mutate {
        param($record)
        $extra = $record.artifacts[0] | Select-Object *
        $extra.artifactName = "ORIGAMI3_extra_fifth_artifact.bin"
        $record.artifacts = @($record.artifacts) + @($extra)
    }
    $resultPath = Join-Path $fixture.Root "result.json"
    $exitCode = Invoke-VerifyScript -ManifestPath $fixture.ManifestPath -DownloadedDirectory $fixture.DownloadedDir `
        -RecordPath $mutatedRecordPath -ResultPath $resultPath `
        -StdOutPath (Join-Path $fixture.Root "stdout.txt") -StdErrPath (Join-Path $fixture.Root "stderr.txt")
    Write-Host "  new-process exit code: $exitCode"
    Assert-True ($exitCode -ne 0) "a record with five artifacts must fail verification"
    Assert-True (-not (Test-Path -LiteralPath $resultPath)) "a failed run must not write a false result file"
    $stderrText = Get-Content -LiteralPath (Join-Path $fixture.Root "stderr.txt") -Raw -Encoding UTF8
    Assert-True ($stderrText -match [regex]::Escape("must contain exactly four artifacts")) "the failure reason must name the wrong count"

    Write-Host "[7/9] (bonus) a manifest hash that disagrees with the record must fail"
    $fixture = New-BaselineFixture "fail-manifest-hash-mismatch"
    $mutatedManifestPath = Get-MutatedManifestPath -Fixture $fixture -Suffix "hash-mismatch" -Mutate {
        param($lines)
        $first = $lines[0]
        if ($first -notmatch '^(?<hash>[0-9a-f]{64}) \*(?<name>.+)$') {
            throw "unexpected manifest line format in fixture: $first"
        }
        $flippedHash = ($Matches.hash.Substring(0, 63)) + $(if ($Matches.hash.Substring(63,1) -eq "0") { "1" } else { "0" })
        $lines[0] = "$flippedHash *$($Matches.name)"
        return $lines
    }
    $resultPath = Join-Path $fixture.Root "result.json"
    $exitCode = Invoke-VerifyScript -ManifestPath $mutatedManifestPath -DownloadedDirectory $fixture.DownloadedDir `
        -RecordPath $fixture.RecordPath -ResultPath $resultPath `
        -StdOutPath (Join-Path $fixture.Root "stdout.txt") -StdErrPath (Join-Path $fixture.Root "stderr.txt")
    Write-Host "  new-process exit code: $exitCode"
    Assert-True ($exitCode -ne 0) "a manifest/record hash disagreement must fail verification"
    Assert-True (-not (Test-Path -LiteralPath $resultPath)) "a failed run must not write a false result file"
    $stderrText = Get-Content -LiteralPath (Join-Path $fixture.Root "stderr.txt") -Raw -Encoding UTF8
    Assert-True ($stderrText -match [regex]::Escape("disagree")) "the failure reason must name the disagreement"

    Write-Host "[8/9] (bonus) a duplicated artifact name inside the manifest must fail"
    $fixture = New-BaselineFixture "fail-manifest-duplicate-name"
    $mutatedManifestPath = Get-MutatedManifestPath -Fixture $fixture -Suffix "dup-name" -Mutate {
        param($lines)
        if ($lines[0] -notmatch '^(?<hash>[0-9a-f]{64}) \*(?<name>.+)$') {
            throw "unexpected manifest line format in fixture: $($lines[0])"
        }
        $firstName = $Matches.name
        if ($lines[1] -notmatch '^(?<hash>[0-9a-f]{64}) \*(?<name>.+)$') {
            throw "unexpected manifest line format in fixture: $($lines[1])"
        }
        $lines[1] = "$($Matches.hash) *$firstName"
        return $lines
    }
    $resultPath = Join-Path $fixture.Root "result.json"
    $exitCode = Invoke-VerifyScript -ManifestPath $mutatedManifestPath -DownloadedDirectory $fixture.DownloadedDir `
        -RecordPath $fixture.RecordPath -ResultPath $resultPath `
        -StdOutPath (Join-Path $fixture.Root "stdout.txt") -StdErrPath (Join-Path $fixture.Root "stderr.txt")
    Write-Host "  new-process exit code: $exitCode"
    Assert-True ($exitCode -ne 0) "a duplicated artifact name in the manifest must fail verification"
    Assert-True (-not (Test-Path -LiteralPath $resultPath)) "a failed run must not write a false result file"
    $stderrText = Get-Content -LiteralPath (Join-Path $fixture.Root "stderr.txt") -Raw -Encoding UTF8
    Assert-True ($stderrText -match [regex]::Escape("Hash manifest contains a duplicate artifact")) "the failure reason must name the duplication"

    Write-Host "[9/9] (bonus) a manifest with only three lines must fail"
    $fixture = New-BaselineFixture "fail-manifest-count-three"
    $mutatedManifestPath = Get-MutatedManifestPath -Fixture $fixture -Suffix "count-three" -Mutate {
        param($lines)
        return @($lines | Select-Object -First 3)
    }
    $resultPath = Join-Path $fixture.Root "result.json"
    $exitCode = Invoke-VerifyScript -ManifestPath $mutatedManifestPath -DownloadedDirectory $fixture.DownloadedDir `
        -RecordPath $fixture.RecordPath -ResultPath $resultPath `
        -StdOutPath (Join-Path $fixture.Root "stdout.txt") -StdErrPath (Join-Path $fixture.Root "stderr.txt")
    Write-Host "  new-process exit code: $exitCode"
    Assert-True ($exitCode -ne 0) "a manifest with three lines must fail verification"
    Assert-True (-not (Test-Path -LiteralPath $resultPath)) "a failed run must not write a false result file"
    $stderrText = Get-Content -LiteralPath (Join-Path $fixture.Root "stderr.txt") -Raw -Encoding UTF8
    Assert-True ($stderrText -match [regex]::Escape("Hash manifest must contain exactly four artifacts")) "the failure reason must name the wrong count"

    Write-Host "supply-chain-verify-release-hashes self-test passed: $script:AssertionCount assertions"
}
finally {
    if (Test-Path -LiteralPath $SandboxRoot) {
        $fullSandbox = [IO.Path]::GetFullPath($SandboxRoot).TrimEnd([char[]]"\/")
        $fullTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
        $leaf = [IO.Path]::GetFileName($fullSandbox)
        if (([IO.Path]::GetDirectoryName($fullSandbox) -eq $fullTemp) -and
            ([regex]::IsMatch($leaf, "^ori3-supply-chain-verify-test-[0-9a-f]{32}$", [Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
            Remove-Item -LiteralPath $fullSandbox -Recurse -Force
        }
    }
}
