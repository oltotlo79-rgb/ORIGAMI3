[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$')]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$BuildId,

    [Parameter(Mandatory = $true)]
    [string]$SetupPath,

    [Parameter(Mandatory = $true)]
    [string]$MsiPath,

    [Parameter(Mandatory = $true)]
    [string]$PortablePath,

    [Parameter(Mandatory = $true)]
    [string]$ManualPath,

    [Parameter(Mandatory = $true)]
    [string]$DestinationDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $DestinationDirectory -PathType Container)) {
    throw "Destination directory does not exist: $DestinationDirectory"
}

$expected = @(
    [ordered]@{
        sourcePath = $SetupPath
        artifactName = "ORIGAMI3_${Version}_setup.exe"
    },
    [ordered]@{
        sourcePath = $MsiPath
        artifactName = "ORIGAMI3_${Version}_x64.msi"
    },
    [ordered]@{
        sourcePath = $PortablePath
        artifactName = "ORIGAMI3_${Version}_portable.exe"
    },
    [ordered]@{
        sourcePath = $ManualPath
        artifactName = "ORIGAMI3.pdf"
    }
)

foreach ($artifact in $expected) {
    if (-not (Test-Path -LiteralPath $artifact.sourcePath -PathType Leaf)) {
        throw "Release input is missing: $($artifact.sourcePath)"
    }
}

$artifactNames = @($expected | ForEach-Object { [string]$_.artifactName })
if (($artifactNames | Select-Object -Unique).Count -ne 4) {
    throw "Release artifact names must be four unique values."
}

$records = New-Object System.Collections.Generic.List[object]
$manifestLines = New-Object System.Collections.Generic.List[string]
foreach ($artifact in $expected) {
    $hash = (Get-FileHash -LiteralPath $artifact.sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $sidecarPath = Join-Path $DestinationDirectory ("{0}.sha256" -f $artifact.artifactName)
    $line = "{0} *{1}" -f $hash, $artifact.artifactName
    [IO.File]::WriteAllText($sidecarPath, $line + [Environment]::NewLine, [Text.Encoding]::ASCII)
    $manifestLines.Add($line)
    $records.Add([ordered]@{
        artifactName = $artifact.artifactName
        version = $Version
        buildId = $BuildId
        sha256 = $hash
        sourceFile = [IO.Path]::GetFileName($artifact.sourcePath)
        sidecarFile = [IO.Path]::GetFileName($sidecarPath)
    })
}

$manifestPath = Join-Path $DestinationDirectory ("ORIGAMI3_{0}_SHA256SUMS.txt" -f $Version)
[IO.File]::WriteAllLines($manifestPath, $manifestLines, [Text.Encoding]::ASCII)
$mappingPath = Join-Path $DestinationDirectory ("ORIGAMI3_{0}_artifact-record.json" -f $Version)
$mapping = [ordered]@{
    schema = 1
    version = $Version
    buildId = $BuildId
    hashAlgorithm = "SHA-256"
    artifacts = $records.ToArray()
}
[IO.File]::WriteAllText($mappingPath, ($mapping | ConvertTo-Json -Depth 6) + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))

$writtenManifest = @(Get-Content -LiteralPath $manifestPath -Encoding ASCII)
if ($writtenManifest.Count -ne 4 -or ($writtenManifest | Select-Object -Unique).Count -ne 4) {
    throw "Hash manifest must contain exactly four unique lines."
}
foreach ($record in $records) {
    $expectedLine = "{0} *{1}" -f $record.sha256, $record.artifactName
    if ($writtenManifest -notcontains $expectedLine) {
        throw "Hash manifest is missing $($record.artifactName)."
    }
    $sidecarPath = Join-Path $DestinationDirectory $record.sidecarFile
    if ((Get-Content -LiteralPath $sidecarPath -Raw -Encoding ASCII).TrimEnd() -ne $expectedLine) {
        throw "Hash sidecar does not match the manifest: $($record.artifactName)"
    }
}

$writtenMapping = Get-Content -LiteralPath $mappingPath -Raw -Encoding UTF8 | ConvertFrom-Json
if (@($writtenMapping.artifacts).Count -ne 4 -or @($writtenMapping.artifacts | Select-Object -ExpandProperty artifactName -Unique).Count -ne 4) {
    throw "Artifact record did not retain four unique identities."
}

Write-Host "Generated four hash sidecars: $DestinationDirectory"
Write-Host "Manifest: $manifestPath"
Write-Host "Artifact record: $mappingPath"
