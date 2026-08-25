[CmdletBinding()]
param(
    [string]$CargoLockPath,
    [string]$PackageLockPath,
    [Parameter(Mandatory = $true)]
    [string]$DestinationPath,
    [string]$ArtifactName,
    [string]$BuildId
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ScriptVersion = "1.1.0"

if ([string]::IsNullOrWhiteSpace($ArtifactName) -xor [string]::IsNullOrWhiteSpace($BuildId)) {
    throw "ArtifactName and BuildId must either both be supplied or both be omitted."
}

if ([string]::IsNullOrWhiteSpace($CargoLockPath)) {
    $CargoLockPath = Join-Path $PSScriptRoot "..\Cargo.lock"
}
if ([string]::IsNullOrWhiteSpace($PackageLockPath)) {
    $PackageLockPath = Join-Path $PSScriptRoot "..\apps\desktop\package-lock.json"
}

foreach ($inputPath in @($CargoLockPath, $PackageLockPath)) {
    if (-not (Test-Path -LiteralPath $inputPath -PathType Leaf)) {
        throw "Required lockfile not found: $inputPath"
    }
}

$outputDirectory = Split-Path -Path $DestinationPath -Parent
if ([string]::IsNullOrWhiteSpace($outputDirectory) -or -not (Test-Path -LiteralPath $outputDirectory -PathType Container)) {
    throw "The output directory does not exist: $outputDirectory"
}

function Get-RequiredCapture {
    param(
        [string]$Text,
        [string]$Pattern,
        [string]$Label
    )

    $match = [regex]::Match($Text, $Pattern)
    if (-not $match.Success) {
        throw "Cannot read $Label from Cargo.lock."
    }
    return $match.Groups[1].Value
}

function Get-CargoComponents {
    param([string]$LockPath)

    $lockText = Get-Content -LiteralPath $LockPath -Raw -Encoding UTF8
    $blocks = @([regex]::Split($lockText, '(?m)^\[\[package\]\]\s*$') | Select-Object -Skip 1)
    $components = New-Object System.Collections.Generic.List[object]
    foreach ($block in $blocks) {
        $name = Get-RequiredCapture -Text $block -Pattern '(?m)^name = "([^"]+)"' -Label "package name"
        $version = Get-RequiredCapture -Text $block -Pattern '(?m)^version = "([^"]+)"' -Label "package version"
        $sourceMatch = [regex]::Match($block, '(?m)^source = "([^"]+)"')
        $source = if ($sourceMatch.Success) { $sourceMatch.Groups[1].Value } else { "workspace" }
        $checksumMatch = [regex]::Match($block, '(?m)^checksum = "([0-9a-f]+)"')
        $properties = @(
            [ordered]@{ name = "ori3:ecosystem"; value = "cargo" },
            [ordered]@{ name = "ori3:lock-source"; value = $source }
        )
        if ($checksumMatch.Success) {
            $properties += [ordered]@{ name = "ori3:lock-checksum"; value = $checksumMatch.Groups[1].Value }
        }
        $components.Add([ordered]@{
            type = "library"
            name = $name
            version = $version
            "bom-ref" = "cargo:$name@$version"
            properties = $properties
        })
    }
    return $components.ToArray()
}

function Get-NpmComponents {
    param([string]$LockPath)

    $node = Get-Command node -ErrorAction SilentlyContinue
    if ($null -eq $node) {
        throw "Node.js is required to read package-lock.json without PowerShell 5.1 empty-key loss."
    }

    $nodeScript = @'
const fs = require("fs");
const lockPath = process.argv[2];
const lock = JSON.parse(fs.readFileSync(lockPath, "utf8"));
const encode = (value) => Buffer.from(String(value || ""), "utf8").toString("base64");
for (const [lockPathEntry, value] of Object.entries(lock.packages || {})) {
  if (!lockPathEntry) continue;
  const marker = "node_modules/";
  const markerAt = lockPathEntry.lastIndexOf(marker);
  const packageName = markerAt >= 0 ? lockPathEntry.slice(markerAt + marker.length) : lockPathEntry;
  const scope = value.dev === true || value.devOptional === true ? "developmentBuild" : "production";
  process.stdout.write(
    `${encode(lockPathEntry)}\t${encode(packageName)}\t${encode(value.version || "")}\t${scope}\t${encode(value.license || "")}\n`
  );
}
'@

    $encodedNodeScript = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($nodeScript))
    $nodeWrapper = "eval(Buffer.from(process.argv[1],'base64').toString('utf8'))"
    $output = @(& $node.Source --eval $nodeWrapper $encodedNodeScript $LockPath 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw "Node.js failed to read package-lock.json: $($output -join ' ')"
    }

    $components = New-Object System.Collections.Generic.List[object]
    foreach ($line in $output) {
        if ([string]::IsNullOrWhiteSpace([string]$line)) {
            continue
        }
        $parts = ([string]$line).Split("`t")
        if ($parts.Count -ne 5) {
            throw "Unexpected npm lockfile record: $line"
        }
        $lockLocation = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($parts[0]))
        $name = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($parts[1]))
        $version = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($parts[2]))
        $scope = $parts[3]
        $license = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($parts[4]))
        if ([string]::IsNullOrWhiteSpace($name) -or [string]::IsNullOrWhiteSpace($version)) {
            throw "npm lockfile entry '$lockLocation' is missing a package name or version."
        }
        $properties = @(
            [ordered]@{ name = "ori3:ecosystem"; value = "npm" },
            [ordered]@{ name = "ori3:dependency-class"; value = $scope },
            [ordered]@{ name = "ori3:lock-location"; value = $lockLocation }
        )
        if (-not [string]::IsNullOrWhiteSpace($license)) {
            $properties += [ordered]@{ name = "ori3:recorded-license"; value = $license }
        }
        $components.Add([ordered]@{
            type = "library"
            name = $name
            version = $version
            "bom-ref" = "npm:$lockLocation@$version"
            properties = $properties
        })
    }
    return $components.ToArray()
}

$cargoComponents = @(Get-CargoComponents -LockPath $CargoLockPath)
$npmComponents = @(Get-NpmComponents -LockPath $PackageLockPath)
$allComponents = @($cargoComponents + $npmComponents)
if ($cargoComponents.Count -eq 0 -or $npmComponents.Count -eq 0) {
    throw "Both lockfiles must contribute at least one component. Cargo=$($cargoComponents.Count), npm=$($npmComponents.Count)."
}

$cargoTomlPath = Join-Path $PSScriptRoot "..\Cargo.toml"
$cargoToml = Get-Content -LiteralPath $cargoTomlPath -Raw -Encoding UTF8
$workspaceVersionMatch = [regex]::Match($cargoToml, '(?ms)^\[workspace\.package\].*?^version = "([^"]+)"')
if (-not $workspaceVersionMatch.Success) {
    throw "Cannot read the workspace version from Cargo.toml."
}

$metadataProperties = @(
    [ordered]@{ name = "ori3:source"; value = "Cargo.lock and apps/desktop/package-lock.json only" },
    [ordered]@{ name = "ori3:external-network"; value = "not-used" },
    [ordered]@{ name = "ori3:cargo-component-count"; value = [string]$cargoComponents.Count },
    [ordered]@{ name = "ori3:npm-component-count"; value = [string]$npmComponents.Count }
)
if (-not [string]::IsNullOrWhiteSpace($ArtifactName)) {
    $metadataProperties += [ordered]@{ name = "ori3:artifact-name"; value = $ArtifactName }
    $metadataProperties += [ordered]@{ name = "ori3:build-id"; value = $BuildId }
}

$document = [ordered]@{
    bomFormat = "CycloneDX"
    specVersion = "1.6"
    version = 1
    metadata = [ordered]@{
        tools = [ordered]@{
            components = @(
                [ordered]@{
                    type = "application"
                    name = "ORIGAMI3 offline lockfile inventory generator"
                    version = $ScriptVersion
                }
            )
        }
        component = [ordered]@{
            type = "application"
            name = "ORIGAMI3"
            version = $workspaceVersionMatch.Groups[1].Value
        }
        properties = $metadataProperties
    }
    components = $allComponents
}

$json = $document | ConvertTo-Json -Depth 12
[IO.File]::WriteAllText($DestinationPath, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))

$written = Get-Content -LiteralPath $DestinationPath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($written.bomFormat -ne "CycloneDX" -or $written.specVersion -ne "1.6" -or @($written.components).Count -ne $allComponents.Count) {
    throw "Generated component inventory did not survive JSON round-trip validation."
}
$writtenComponents = @($written.components)
$missingNameOrVersion = @($writtenComponents | Where-Object {
    [string]::IsNullOrWhiteSpace([string]$_.name) -or
    [string]::IsNullOrWhiteSpace([string]$_.version)
})
if ($missingNameOrVersion.Count -ne 0) {
    throw "Generated component inventory has $($missingNameOrVersion.Count) component(s) without a name or version."
}
$duplicateReferences = @($writtenComponents | Group-Object { [string]$_.'bom-ref' } | Where-Object { $_.Count -gt 1 })
if ($duplicateReferences.Count -ne 0) {
    throw "Generated component inventory has $($duplicateReferences.Count) duplicate component reference(s)."
}
if (-not [string]::IsNullOrWhiteSpace($ArtifactName)) {
    $writtenProperties = @($written.metadata.properties)
    $writtenArtifactName = @($writtenProperties | Where-Object { $_.name -eq "ori3:artifact-name" })
    $writtenBuildId = @($writtenProperties | Where-Object { $_.name -eq "ori3:build-id" })
    if ($writtenArtifactName.Count -ne 1 -or $writtenArtifactName[0].value -ne $ArtifactName) {
        throw "Generated component inventory did not retain the artifact name."
    }
    if ($writtenBuildId.Count -ne 1 -or $writtenBuildId[0].value -ne $BuildId) {
        throw "Generated component inventory did not retain the build ID."
    }
}

$hash = (Get-FileHash -LiteralPath $DestinationPath -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Host "Generated offline component inventory: $DestinationPath"
Write-Host "Cargo components: $($cargoComponents.Count)"
Write-Host "npm components: $($npmComponents.Count)"
Write-Host "total components: $($allComponents.Count)"
if (-not [string]::IsNullOrWhiteSpace($ArtifactName)) {
    Write-Host "artifact: $ArtifactName (build ID: $BuildId)"
}
Write-Host "SHA-256: $hash"
