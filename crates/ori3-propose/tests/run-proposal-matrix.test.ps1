[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RunnerPath = Join-Path $PSScriptRoot "run-proposal-matrix.ps1"
$RealRepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$SandboxRoot = Join-Path ([IO.Path]::GetTempPath()) ("ori3-proposal-matrix-fingerprint-test-" + [Guid]::NewGuid().ToString("N"))
$script:AssertionCount = 0
$Utf8NoBom = [Text.UTF8Encoding]::new($false)

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

function Assert-NotEqual {
    param(
        [AllowNull()]$Actual,
        [AllowNull()]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )
    $script:AssertionCount += 1
    if ($Actual -eq $Expected) {
        throw "ASSERTION FAILED: $Message (both=$Actual)"
    }
}

function Write-SandboxFile {
    param(
        [Parameter(Mandatory = $true)][string]$RelativePath,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Content
    )
    $path = Join-Path $SandboxRoot $RelativePath
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $path))
    [IO.File]::WriteAllText($path, $Content.Replace("`r`n", "`n"), $Utf8NoBom)
}

function Get-SandboxFixturePath {
    param(
        [Parameter(Mandatory = $true)][string[]]$Segments
    )

    $path = Join-Path "crates/ori3-propose/tests" "fixtures"
    foreach ($segment in $Segments) {
        $path = Join-Path $path $segment
    }
    return $path.Replace("\", "/")
}

function Get-SandboxFingerprint {
    return Get-InputFingerprint
}

function Test-FingerprintContains {
    param(
        [Parameter(Mandatory = $true)]$Fingerprint,
        [Parameter(Mandatory = $true)][string]$RelativePath
    )
    return @($Fingerprint.files | Where-Object { $_.path -eq $RelativePath }).Count -eq 1
}

if (-not (Test-Path -LiteralPath $RunnerPath -PathType Leaf)) {
    throw "Required implementation is missing: $RunnerPath"
}

try {
    [void][IO.Directory]::CreateDirectory($SandboxRoot)
    $rootManifest = @'
[workspace]
resolver = "2"
members = [
    "crates/ori3-propose",
    "crates/propose-dep",
    "crates/target-dep",
    "crates/build-dep",
    "crates/desktop-only",
    "apps/desktop/src-tauri",
]

[workspace.dependencies]
propose-dep = { path = "crates/propose-dep" }
build-dep = { path = "crates/build-dep" }
'@
    $proposeManifest = @'
[package]
name = "ori3-propose"

[dependencies]
propose-dep.workspace = true

[target.'cfg(windows)'.dependencies]
target-dep = { path = "../target-dep" }
'@
    $desktopManifest = @'
[package]
name = "desktop"

[dependencies]
ori3-propose = { path = "../../../crates/ori3-propose" }
desktop-only = { path = "../../../crates/desktop-only" }

[build-dependencies]
build-dep = { workspace = true }
'@

    Write-SandboxFile "Cargo.toml" $rootManifest
    Write-SandboxFile "Cargo.lock" "# fake lock`n"
    Write-SandboxFile "crates/ori3-propose/Cargo.toml" $proposeManifest
    Write-SandboxFile "crates/ori3-propose/src/lib.rs" "pub fn proposal() {}`n"
    Write-SandboxFile "crates/ori3-propose/src/embedded.txt" "included data`n"
    Write-SandboxFile "crates/ori3-propose/tests/acceptance.rs" "#[test] fn acceptance() {}`n"
    Write-SandboxFile (Get-SandboxFixturePath -Segments @("corpus", "base.json")) "{`"shape`":`"crane`"}`n"
    foreach ($crate in @("propose-dep", "target-dep", "build-dep", "desktop-only")) {
        Write-SandboxFile "crates/$crate/Cargo.toml" "[package]`nname = `"$crate`"`n"
        Write-SandboxFile "crates/$crate/src/lib.rs" "pub fn marker() {}`n"
    }
    Write-SandboxFile "apps/desktop/src-tauri/Cargo.toml" $desktopManifest
    Write-SandboxFile "apps/desktop/src-tauri/src/lib.rs" "pub fn desktop() {}`n"
    Write-SandboxFile "apps/desktop/src-tauri/build.rs" "fn main() {}`n"
    Write-SandboxFile "apps/desktop/src-tauri/tauri.conf.json" "{}`n"
    Write-SandboxFile "apps/desktop/src-tauri/capabilities/default.json" "{}`n"
    Write-SandboxFile "apps/desktop/src/App.tsx" "export const App = () => null;`n"
    Write-SandboxFile "docs/note.md" "outside`n"

    . $RunnerPath -LoadFunctionsOnly -RepositoryRoot $SandboxRoot

    Write-Host "[1/8] Identical inputs are deterministic and file paths are ordered"
    $first = Get-SandboxFingerprint
    $second = Get-SandboxFingerprint
    Assert-Equal $first.aggregate_sha256 $second.aggregate_sha256 "same inputs must produce the same aggregate"
    Assert-Equal $first.files.Count $second.files.Count "same inputs must produce the same file count"
    $actualPaths = @($first.files | ForEach-Object { $_.path })
    for ($index = 1; $index -lt $actualPaths.Count; $index++) {
        Assert-True ([StringComparer]::Ordinal.Compare($actualPaths[$index - 1], $actualPaths[$index]) -lt 0) "file paths must use ordinal ordering"
    }
    Assert-Equal @($actualPaths | Sort-Object -Unique).Count $actualPaths.Count "file paths must be unique"

    Write-Host "[2/8] Both propose-only and desktop-only path dependencies affect the aggregate"
    Assert-True (Test-FingerprintContains $first "crates/propose-dep/src/lib.rs") "propose dependency source must be included"
    Assert-True (Test-FingerprintContains $first "crates/desktop-only/src/lib.rs") "desktop-only dependency source must be included"
    Assert-True (Test-FingerprintContains $first "crates/target-dep/src/lib.rs") "target cfg path dependency source must be included"
    Assert-True (Test-FingerprintContains $first "crates/build-dep/src/lib.rs") "build dependency source must be included"
    Write-SandboxFile "crates/propose-dep/src/lib.rs" "pub fn marker() { }`n"
    $changedProposeDependency = Get-SandboxFingerprint
    Assert-NotEqual $changedProposeDependency.aggregate_sha256 $first.aggregate_sha256 "one byte in propose dependency must change aggregate"
    Write-SandboxFile "crates/propose-dep/src/lib.rs" "pub fn marker() {}`n"
    Write-SandboxFile "crates/desktop-only/src/lib.rs" "pub fn marker() { }`n"
    $changedDesktopDependency = Get-SandboxFingerprint
    Assert-NotEqual $changedDesktopDependency.aggregate_sha256 $first.aggregate_sha256 "one byte in desktop-only dependency must change aggregate"
    Write-SandboxFile "crates/desktop-only/src/lib.rs" "pub fn marker() {}`n"

    Write-Host "[3/8] Nested corpus additions, deletions, and content changes affect the aggregate"
    $beforeCorpusChange = Get-SandboxFingerprint
    Write-SandboxFile (Get-SandboxFixturePath -Segments @("corpus", "base.json")) "{`"shape`":`"bird-base`"}`n"
    $changedCorpusContent = Get-SandboxFingerprint
    Assert-NotEqual $changedCorpusContent.aggregate_sha256 $beforeCorpusChange.aggregate_sha256 "nested corpus content change must change aggregate"
    Write-SandboxFile (Get-SandboxFixturePath -Segments @("corpus", "base.json")) "{`"shape`":`"crane`"}`n"
    $beforeCorpusAdd = Get-SandboxFingerprint
    Write-SandboxFile (Get-SandboxFixturePath -Segments @("corpus", "nested", "new.json")) "{`"shape`":`"frog`"}`n"
    $afterCorpusAdd = Get-SandboxFingerprint
    Assert-NotEqual $afterCorpusAdd.aggregate_sha256 $beforeCorpusAdd.aggregate_sha256 "nested corpus addition must change aggregate"
    Assert-True (Test-FingerprintContains $afterCorpusAdd (Get-SandboxFixturePath -Segments @("corpus", "nested", "new.json"))) "added corpus path must enter file set"
    Remove-Item -LiteralPath (Join-Path $SandboxRoot (Get-SandboxFixturePath -Segments @("corpus", "nested", "new.json"))) -Force
    $afterCorpusDelete = Get-SandboxFingerprint
    Assert-NotEqual $afterCorpusDelete.aggregate_sha256 $afterCorpusAdd.aggregate_sha256 "nested corpus deletion must change aggregate"
    Assert-Equal $afterCorpusDelete.aggregate_sha256 $beforeCorpusAdd.aggregate_sha256 "deleting the added corpus must restore aggregate"

    Write-Host "[4/8] Toolchain, lockfile, and build script inputs affect the aggregate"
    $beforeToolchain = Get-SandboxFingerprint
    Write-SandboxFile "rust-toolchain.toml" "[toolchain]`nchannel = `"stable`"`n"
    $afterToolchain = Get-SandboxFingerprint
    Assert-NotEqual $afterToolchain.aggregate_sha256 $beforeToolchain.aggregate_sha256 "adding rust-toolchain.toml must change aggregate"
    Assert-True (Test-FingerprintContains $afterToolchain "rust-toolchain.toml") "toolchain file must enter file set"
    Remove-Item -LiteralPath (Join-Path $SandboxRoot "rust-toolchain.toml") -Force
    Write-SandboxFile "Cargo.lock" "# changed fake lock`n"
    Assert-NotEqual (Get-SandboxFingerprint).aggregate_sha256 $beforeToolchain.aggregate_sha256 "Cargo.lock change must change aggregate"
    Write-SandboxFile "Cargo.lock" "# fake lock`n"
    Write-SandboxFile "apps/desktop/src-tauri/build.rs" "fn main() { }`n"
    Assert-NotEqual (Get-SandboxFingerprint).aggregate_sha256 $beforeToolchain.aggregate_sha256 "build.rs change must change aggregate"
    Write-SandboxFile "apps/desktop/src-tauri/build.rs" "fn main() {}`n"

    Write-Host "[5/8] A newly linked workspace path dependency is discovered without a crate allowlist"
    $rootWithDynamic = $rootManifest.Replace('    "crates/desktop-only",', "    `"crates/desktop-only`",`n    `"crates/dynamic`",")
    $desktopWithDynamic = $desktopManifest + "`ndynamic = { path = `"../../../crates/dynamic`" }`n"
    Write-SandboxFile "crates/dynamic/Cargo.toml" "[package]`nname = `"dynamic`"`n"
    Write-SandboxFile "crates/dynamic/src/lib.rs" "pub fn dynamic() {}`n"
    Write-SandboxFile "Cargo.toml" $rootWithDynamic
    Write-SandboxFile "apps/desktop/src-tauri/Cargo.toml" $desktopWithDynamic
    $withDynamic = Get-SandboxFingerprint
    Assert-True (Test-FingerprintContains $withDynamic "crates/dynamic/src/lib.rs") "new desktop path dependency source must enter file set"
    Write-SandboxFile "crates/dynamic/src/lib.rs" "pub fn dynamic() { }`n"
    Assert-NotEqual (Get-SandboxFingerprint).aggregate_sha256 $withDynamic.aggregate_sha256 "newly discovered dependency source must affect aggregate"
    Write-SandboxFile "Cargo.toml" $rootManifest
    Write-SandboxFile "apps/desktop/src-tauri/Cargo.toml" $desktopManifest

    Write-Host "[6/8] Non-Rust source files are included but frontend and docs are excluded"
    $withoutExcludedChanges = Get-SandboxFingerprint
    Assert-True (Test-FingerprintContains $withoutExcludedChanges "crates/ori3-propose/src/embedded.txt") "non-Rust source input must be included"
    Write-SandboxFile "crates/ori3-propose/src/embedded.txt" "changed included data`n"
    Assert-NotEqual (Get-SandboxFingerprint).aggregate_sha256 $withoutExcludedChanges.aggregate_sha256 "non-Rust source change must affect aggregate"
    Write-SandboxFile "crates/ori3-propose/src/embedded.txt" "included data`n"
    $beforeExcludedChanges = Get-SandboxFingerprint
    Write-SandboxFile "apps/desktop/src/App.tsx" "export const App = () => 1;`n"
    Write-SandboxFile "docs/note.md" "changed outside`n"
    $afterExcludedChanges = Get-SandboxFingerprint
    Assert-Equal $afterExcludedChanges.aggregate_sha256 $beforeExcludedChanges.aggregate_sha256 "frontend and docs changes must not affect aggregate"
    Assert-True (-not (Test-FingerprintContains $afterExcludedChanges "apps/desktop/src/App.tsx")) "frontend source must be excluded"
    Assert-True (-not (Test-FingerprintContains $afterExcludedChanges "docs/note.md")) "docs must be excluded"

    Write-Host "[7/8] File entries retain the existing path and sha256 shape"
    Assert-True ($first.files.Count -gt 0) "fingerprint must contain files"
    foreach ($entry in $first.files) {
        Assert-Equal ($entry.Keys -join ",") "path,sha256" "each file entry must retain path and sha256 only"
        Assert-True ([string]$entry.sha256 -match '^[0-9a-f]{64}$') "each file hash must be lowercase SHA-256"
    }

    Write-Host "[8/8] The real repository includes required backend inputs and excludes App.tsx"
    $RepositoryRoot = $RealRepositoryRoot
    $real = Get-InputFingerprint
    Assert-True (Test-FingerprintContains $real (Get-SandboxFixturePath -Segments @("corpus", "manifest.json"))) "real corpus manifest must be included"
    Assert-True (Test-FingerprintContains $real "crates/ori3-soft/Cargo.toml") "real desktop-only soft manifest must be included"
    Assert-True (Test-FingerprintContains $real "crates/ori3-export/src/lib.rs") "real export source must be included"
    Assert-True (Test-FingerprintContains $real "apps/desktop/src-tauri/build.rs") "real desktop build script must be included"
    Assert-True (-not (Test-FingerprintContains $real "apps/desktop/src/App.tsx")) "real frontend App.tsx must be excluded"

    Write-Host "run-proposal-matrix fingerprint self-test passed: $script:AssertionCount assertions"
}
finally {
    if (Test-Path -LiteralPath $SandboxRoot) {
        $resolvedSandbox = [IO.Path]::GetFullPath($SandboxRoot)
        $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
        if (-not $resolvedSandbox.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing unsafe self-test cleanup: $resolvedSandbox"
        }
        Remove-Item -LiteralPath $resolvedSandbox -Recurse -Force
    }
}
