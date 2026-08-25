[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ManifestPath,

    [Parameter(Mandatory = $true)]
    [string]$DownloadedDirectory,

    [Parameter(Mandatory = $true)]
    [string]$RecordPath,

    [Parameter(Mandatory = $true)]
    [string]$ResultPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

foreach ($inputPath in @($ManifestPath, $RecordPath)) {
    if (-not (Test-Path -LiteralPath $inputPath -PathType Leaf)) {
        throw "Required record is missing: $inputPath"
    }
}
if (-not (Test-Path -LiteralPath $DownloadedDirectory -PathType Container)) {
    throw "Downloaded directory does not exist: $DownloadedDirectory"
}

$record = Get-Content -LiteralPath $RecordPath -Raw -Encoding UTF8 | ConvertFrom-Json
$artifacts = @($record.artifacts)
if ($artifacts.Count -ne 4) {
    throw "Artifact record must contain exactly four artifacts."
}
$names = @($artifacts | ForEach-Object { [string]$_.artifactName })
if (($names | Select-Object -Unique).Count -ne 4) {
    throw "Artifact record has duplicate names."
}

$manifestByName = @{}
foreach ($line in @(Get-Content -LiteralPath $ManifestPath -Encoding ASCII)) {
    if ($line -notmatch '^(?<hash>[0-9a-f]{64}) \*(?<name>[^\\/]+)$') {
        throw "Hash manifest line has an invalid format: $line"
    }
    if ($manifestByName.ContainsKey($Matches.name)) {
        throw "Hash manifest contains a duplicate artifact: $($Matches.name)"
    }
    $manifestByName[$Matches.name] = $Matches.hash
}
if ($manifestByName.Count -ne 4) {
    throw "Hash manifest must contain exactly four artifacts."
}

$results = New-Object System.Collections.Generic.List[object]
foreach ($artifact in $artifacts) {
    $artifactName = [string]$artifact.artifactName
    $expectedHash = [string]$artifact.sha256
    if (-not $manifestByName.ContainsKey($artifactName) -or $manifestByName[$artifactName] -ne $expectedHash) {
        throw "Artifact record and hash manifest disagree for $artifactName."
    }
    $downloadedPath = Join-Path $DownloadedDirectory $artifactName
    if (-not (Test-Path -LiteralPath $downloadedPath -PathType Leaf)) {
        throw "Downloaded artifact is missing: $downloadedPath"
    }
    $actualHash = (Get-FileHash -LiteralPath $downloadedPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedHash) {
        throw "Downloaded artifact hash does not match: $artifactName"
    }
    $results.Add([ordered]@{
        artifactName = $artifactName
        sha256 = $actualHash
        result = "match"
    })
}

$resultDirectory = Split-Path -Parent $ResultPath
if ([string]::IsNullOrWhiteSpace($resultDirectory) -or -not (Test-Path -LiteralPath $resultDirectory -PathType Container)) {
    throw "Result directory does not exist: $resultDirectory"
}
[IO.File]::WriteAllText($ResultPath, (($results.ToArray() | ConvertTo-Json -Depth 4) + [Environment]::NewLine), [Text.UTF8Encoding]::new($false))
Write-Host "All four downloaded artifacts match their published hashes."
