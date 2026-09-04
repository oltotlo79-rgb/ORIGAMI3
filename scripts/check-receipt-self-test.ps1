# Receipt logic self-test. It never creates rust-w4/check-all pass records and
# never runs cargo test/npm. The synthetic record is removed in finally.

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "check-receipt.ps1")

function Assert-Ori3SelfTest {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) {
        throw $Message
    }
}

$ReceiptTempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\\/")
$ReceiptSandbox = Join-Path $ReceiptTempBase ("ori3-receipt-self-test-" + [Guid]::NewGuid().ToString("N"))
$ReceiptSandboxRoot = Join-Path $ReceiptSandbox "repo"

function Remove-Ori3ReceiptSelfTestSandbox {
    if (-not (Test-Path -LiteralPath $ReceiptSandbox)) { return }
    $fullSandbox = [IO.Path]::GetFullPath($ReceiptSandbox).TrimEnd([char[]]"\\/")
    if ([IO.Path]::GetDirectoryName($fullSandbox) -ne $ReceiptTempBase -or
        [IO.Path]::GetFileName($fullSandbox) -notmatch '^ori3-receipt-self-test-[0-9a-f]{32}$') {
        throw "Refusing unsafe receipt self-test cleanup: $fullSandbox"
    }
    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

$arguments = @(Get-Ori3RustW4Arguments)
$expectedArguments = @(
    "test", "--workspace", "--no-fail-fast", "--",
    "--skip", "completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten",
    "--skip", "named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten",
    "--skip", "a_safe_coincident_partial_network_appears_after_the_first_fold",
    "--skip", "the_heaviest_proposal_never_hits_the_time_limit"
)
Assert-Ori3SelfTest (($arguments -join [char]0) -eq ($expectedArguments -join [char]0)) "W4 argv mismatch"
Assert-Ori3SelfTest (((Get-Ori3ClippyArguments) -join [char]0) -eq (@("clippy", "--workspace", "--all-targets", "--", "-D", "warnings") -join [char]0)) "clippy argv mismatch"
Assert-Ori3SelfTest (((Get-Ori3NpmBuildArguments) -join [char]0) -eq (@("run", "build") -join [char]0)) "npm build argv mismatch"
Assert-Ori3SelfTest (((Get-Ori3NpmLintArguments) -join [char]0) -eq (@("run", "lint") -join [char]0)) "npm lint argv mismatch"
Assert-Ori3SelfTest (((Get-Ori3NpmTestArguments) -join [char]0) -eq (@("run", "test") -join [char]0)) "npm test argv mismatch"
Write-Host "[OK] exact W4 argv"

$listedPaths = @(Invoke-Ori3GitFileList $root)
Assert-Ori3SelfTest ($listedPaths.Count -gt 0) "git-visible worktree list is empty"
Assert-Ori3SelfTest (-not @($listedPaths | Where-Object { $_ -like ".origami3/*" }).Count) "receipt directory is not ignored"
for ($pathIndex = 1; $pathIndex -lt $listedPaths.Count; $pathIndex++) {
    $pathComparison = [string]::CompareOrdinal($listedPaths[$pathIndex - 1], $listedPaths[$pathIndex])
    if ($pathComparison -ge 0) {
        $previousPathDigest = Get-Ori3Sha256HexFromText ([string]$listedPaths[$pathIndex - 1])
        $currentPathDigest = Get-Ori3Sha256HexFromText ([string]$listedPaths[$pathIndex])
        throw "worktree paths are not unique ordinal order: index=$pathIndex comparison=$pathComparison previous=$previousPathDigest current=$currentPathDigest"
    }
}
Write-Host "[OK] deterministic Git-visible worktree path list; receipt directory excluded"

$machine = Get-Ori3MachineCondition $root
Assert-Ori3SelfTest (-not [string]::IsNullOrWhiteSpace($machine.Os)) "OS condition is empty"
Assert-Ori3SelfTest (-not [string]::IsNullOrWhiteSpace($machine.Cpu)) "CPU condition is empty"
Assert-Ori3SelfTest ($machine.MachineHash.Length -eq 64) "machine digest is incomplete"
Write-Host "[OK] machine / OS / CPU conditions"

$oldCi = $env:CI
try {
    $env:CI = "self-test"
    $ciDenied = $false
    try {
        Test-Ori3ReceiptEnvironmentAllowed
    }
    catch {
        $ciDenied = $_.Exception.Message.Contains("CI")
    }
    Assert-Ori3SelfTest $ciDenied "CI did not disable receipt reuse"
    Write-Host "[OK] CI environment => receipt disabled"
}
finally {
    if ($null -eq $oldCi) {
        Remove-Item Env:CI -ErrorAction SilentlyContinue
    }
    else {
        $env:CI = $oldCi
    }
}

$rustConditions = Get-Ori3ReceiptConditions "rust-w4" $root
Assert-Ori3SelfTest ($rustConditions.Sha256.Length -eq 64) "condition digest is incomplete"
Assert-Ori3SelfTest ($rustConditions.SafeDisplay.Rustc.StartsWith("rustc ")) "rustc version is absent"
Assert-Ori3SelfTest ($rustConditions.SafeDisplay.Cargo.StartsWith("cargo ")) "cargo version is absent"
Write-Host "[OK] Rust toolchain conditions"

$fullConditions = Get-Ori3ReceiptConditions "check-all" $root
Assert-Ori3SelfTest (-not [string]::IsNullOrWhiteSpace($fullConditions.SafeDisplay.Node)) "Node version is absent"
Assert-Ori3SelfTest (-not [string]::IsNullOrWhiteSpace($fullConditions.SafeDisplay.Npm)) "npm version is absent"
Assert-Ori3SelfTest (-not [string]::IsNullOrWhiteSpace($fullConditions.SafeDisplay.Clippy)) "clippy version is absent"
Assert-Ori3SelfTest (-not [string]::IsNullOrWhiteSpace($fullConditions.SafeDisplay.PowerShell)) "PowerShell condition is absent"
Assert-Ori3SelfTest (-not [string]::IsNullOrWhiteSpace($fullConditions.SafeDisplay.Locale)) "locale/timezone condition is absent"
Write-Host "[OK] Node / npm / clippy conditions"

$oldMarker = $env:ORI3_RECEIPT_TEST_MARKER
$oldVitestWorkers = $env:VITEST_MAX_WORKERS
try {
    Remove-Item Env:ORI3_RECEIPT_TEST_MARKER -ErrorAction SilentlyContinue
    $environmentA = (Get-Ori3RelevantEnvironment).Sha256
    $env:ORI3_RECEIPT_TEST_MARKER = "changed"
    $environmentB = (Get-Ori3RelevantEnvironment).Sha256
    Assert-Ori3SelfTest ($environmentA -ne $environmentB) "relevant environment change was not detected"
    Write-Host "[OK] relevant environment change => different condition"

    Remove-Item Env:ORI3_RECEIPT_TEST_MARKER -ErrorAction SilentlyContinue
    Remove-Item Env:VITEST_MAX_WORKERS -ErrorAction SilentlyContinue
    $vitestEnvironmentA = (Get-Ori3RelevantEnvironment).Sha256
    $env:VITEST_MAX_WORKERS = "1"
    $vitestEnvironmentB = (Get-Ori3RelevantEnvironment).Sha256
    Assert-Ori3SelfTest ($vitestEnvironmentA -ne $vitestEnvironmentB) "VITEST_MAX_WORKERS change was not detected"
    Write-Host "[OK] VITEST_MAX_WORKERS change => different condition"
}
finally {
    if ($null -eq $oldMarker) {
        Remove-Item Env:ORI3_RECEIPT_TEST_MARKER -ErrorAction SilentlyContinue
    }
    else {
        $env:ORI3_RECEIPT_TEST_MARKER = $oldMarker
    }
    if ($null -eq $oldVitestWorkers) {
        Remove-Item Env:VITEST_MAX_WORKERS -ErrorAction SilentlyContinue
    }
    else {
        $env:VITEST_MAX_WORKERS = $oldVitestWorkers
    }
}

$store = Get-Ori3ReceiptStorePath $root
[System.IO.Directory]::CreateDirectory($store) | Out-Null
$gateStatus = Join-Path $store ("gate-self-test-" + [Guid]::NewGuid().ToString("N") + ".status")
try {
    Set-Ori3GateStatus $root $gateStatus "helper-ready"
    Assert-Ori3SelfTest (([System.IO.File]::ReadAllText($gateStatus)).Trim() -eq "helper-ready") "helper-ready marker mismatch"
    Set-Ori3GateStatus $root $gateStatus "cargo-started"
    Assert-Ori3SelfTest (([System.IO.File]::ReadAllText($gateStatus)).Trim() -eq "cargo-started") "cargo-started marker mismatch"
    Write-Host "[OK] pre-commit fallback marker states"
}
finally {
    if (Test-Path -LiteralPath $gateStatus -PathType Leaf) {
        Remove-Item -LiteralPath $gateStatus -Force
    }
}

$temporaryInput = Join-Path $store ("hash-self-test-" + [Guid]::NewGuid().ToString("N") + ".bin")
try {
    [System.IO.File]::WriteAllBytes($temporaryInput, [byte[]](0, 1, 2, 3, 255))
    $hashA = Get-Ori3FileSha256Hex $temporaryInput
    [System.IO.File]::WriteAllBytes($temporaryInput, [byte[]](0, 1, 2, 4, 255))
    $hashB = Get-Ori3FileSha256Hex $temporaryInput
    Assert-Ori3SelfTest ($hashA -ne $hashB) "raw byte change was not detected"
    Write-Host "[OK] raw byte change => different content"
}
finally {
    if (Test-Path -LiteralPath $temporaryInput -PathType Leaf) {
        Remove-Item -LiteralPath $temporaryInput -Force
    }
}

$kind = "self-test-" + [Guid]::NewGuid().ToString("N")
$content = [pscustomobject]@{ Sha256 = ("a" * 64); FileCount = 2; TotalBytes = 3 }
$recipe = [pscustomobject]@{ Sha256 = ("b" * 64); Checks = @("synthetic check only") }
$safeDisplay = [ordered]@{
    Os = $machine.Os
    Cpu = $machine.Cpu
    Machine = $machine.MachineHash.Substring(0, 12)
    PowerShell = $fullConditions.SafeDisplay.PowerShell
    Locale = $fullConditions.SafeDisplay.Locale
    Rustc = $rustConditions.SafeDisplay.Rustc
    Cargo = $rustConditions.SafeDisplay.Cargo
    Node = "(not used)"
    Npm = "(not used)"
    Clippy = "(not used)"
    EnvironmentNames = "ORI3_RECEIPT_TEST_MARKER"
}
$conditions = [pscustomobject]@{ Sha256 = ("c" * 64); SafeDisplay = $safeDisplay }
$context = $null
$receiptPath = $null
$key = $null

function New-Ori3SyntheticReceipt {
    param([DateTime]$PassedAt, [DateTime]$ExpiresAt)
    $value = [ordered]@{
        schemaVersion = 1
        checkId = $kind
        result = "passed"
        passedAtUtc = $PassedAt.ToUniversalTime().ToString("o", [Globalization.CultureInfo]::InvariantCulture)
        expiresAtUtc = $ExpiresAt.ToUniversalTime().ToString("o", [Globalization.CultureInfo]::InvariantCulture)
        contentSha256 = $content.Sha256
        contentFileCount = $content.FileCount
        recipeSha256 = $recipe.Sha256
        conditionsSha256 = $conditions.Sha256
        eligibilitySha256 = $context.EligibilitySha256
        checks = $recipe.Checks
        conditions = [ordered]@{
            os = $safeDisplay.Os
            cpu = $safeDisplay.Cpu
            machine = $safeDisplay.Machine
            powerShell = $safeDisplay.PowerShell
            locale = $safeDisplay.Locale
            rustc = $safeDisplay.Rustc
            cargo = $safeDisplay.Cargo
            node = $safeDisplay.Node
            npm = $safeDisplay.Npm
            clippy = $safeDisplay.Clippy
            environmentNames = $safeDisplay.EnvironmentNames
        }
        headAtPass = "synthetic-display-only"
        reusedComponentCheckId = ""
        reusedComponentPassedAtUtc = ""
        reusedComponentExpiresAtUtc = ""
        signatureHmacSha256 = ""
    }
    $value.signatureHmacSha256 = Get-Ori3ReceiptSignature ([pscustomobject]$value) $key
    return $value
}

try {
    [void][IO.Directory]::CreateDirectory($ReceiptSandboxRoot)
    $global:LASTEXITCODE = 0
    & git init --quiet $ReceiptSandboxRoot
    if ($LASTEXITCODE -ne 0) { throw "receipt self-test temporary repository initialization failed: exit=$LASTEXITCODE" }
    [IO.File]::WriteAllText((Join-Path $ReceiptSandboxRoot ".gitignore"), ".origami/`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $ReceiptSandboxRoot "fixture.txt"), "receipt self-test fixture`n", [Text.UTF8Encoding]::new($false))

    $context = [pscustomobject]@{
        Kind = $kind
        Root = $ReceiptSandboxRoot
        Content = $content
        Recipe = $recipe
        Conditions = $conditions
        EligibilitySha256 = ("d" * 64)
    }
    $receiptPath = Get-Ori3ReceiptPath $context
    $key = [byte[]](Get-Ori3SigningKey $ReceiptSandboxRoot -Create)
    $now = [DateTime]::UtcNow
    $valid = New-Ori3SyntheticReceipt $now $now.AddHours(24)
    Invoke-Ori3AtomicJsonWrite $receiptPath $valid
    $hit = Find-Ori3CheckReceipt $context
    Assert-Ori3SelfTest $hit.IsHit ("valid signed receipt missed: " + $hit.Reason)
    Write-Host "[OK] signed valid receipt => hit"

    $bounded = New-Ori3SyntheticReceipt $now $now.AddHours(1)
    Invoke-Ori3AtomicJsonWrite $receiptPath $bounded
    $boundedHit = Find-Ori3CheckReceipt $context
    Assert-Ori3SelfTest $boundedHit.IsHit ("bounded composite receipt missed: " + $boundedHit.Reason)
    Write-Host "[OK] inherited earlier expiry => hit without extending TTL"

    $component = New-Ori3SyntheticReceipt $now $now.AddHours(1)
    $component.reusedComponentCheckId = "rust-w4"
    $component.reusedComponentPassedAtUtc = $now.AddMinutes(-1).ToString("o", [Globalization.CultureInfo]::InvariantCulture)
    $component.reusedComponentExpiresAtUtc = $now.AddHours(1).ToString("o", [Globalization.CultureInfo]::InvariantCulture)
    $component.signatureHmacSha256 = Get-Ori3ReceiptSignature ([pscustomobject]$component) $key
    Invoke-Ori3AtomicJsonWrite $receiptPath $component
    $componentHit = Find-Ori3CheckReceipt $context
    Assert-Ori3SelfTest $componentHit.IsHit ("component provenance receipt missed: " + $componentHit.Reason)
    Write-Host "[OK] reused W4 provenance and inherited expiry"

    Invoke-Ori3AtomicJsonWrite $receiptPath $valid

    $signedWrongChecks = New-Ori3SyntheticReceipt $now $now.AddHours(24)
    $signedWrongChecks.checks = @("wrong display")
    $signedWrongChecks.signatureHmacSha256 = Get-Ori3ReceiptSignature ([pscustomobject]$signedWrongChecks) $key
    Invoke-Ori3AtomicJsonWrite $receiptPath $signedWrongChecks
    $wrongChecksMiss = Find-Ori3CheckReceipt $context
    Assert-Ori3SelfTest (-not $wrongChecksMiss.IsHit -and $wrongChecksMiss.Reason.Contains("表示")) "signed wrong check list did not miss"
    Write-Host "[OK] signed but wrong check list => miss"

    Invoke-Ori3AtomicJsonWrite $receiptPath $valid

    $changedContext = [pscustomobject]@{
        Kind = $kind
        Root = $ReceiptSandboxRoot
        Content = [pscustomobject]@{ Sha256 = ("e" * 64); FileCount = 2; TotalBytes = 3 }
        Recipe = $recipe
        Conditions = $conditions
        EligibilitySha256 = ("f" * 64)
    }
    $miss = Find-Ori3CheckReceipt $changedContext
    Assert-Ori3SelfTest (-not $miss.IsHit -and $miss.Reason.Contains("作業内容")) "content mismatch did not miss"
    Write-Host "[OK] content change => miss"

    $tampered = Read-Ori3ReceiptJson $receiptPath
    $tampered.headAtPass = "tampered"
    Invoke-Ori3AtomicJsonWrite $receiptPath $tampered
    $miss = Find-Ori3CheckReceipt $context
    Assert-Ori3SelfTest (-not $miss.IsHit -and $miss.Reason.Contains("署名")) "tampering did not fail HMAC"
    Write-Host "[OK] JSON tampering => HMAC miss"

    $oldPassed = [DateTime]::UtcNow.AddHours(-30)
    $expired = New-Ori3SyntheticReceipt $oldPassed $oldPassed.AddHours(24)
    Invoke-Ori3AtomicJsonWrite $receiptPath $expired
    $miss = Find-Ori3CheckReceipt $context
    Assert-Ori3SelfTest (-not $miss.IsHit -and $miss.Reason.Contains("期限")) "expired receipt hit"
    Write-Host "[OK] expired receipt => miss"

    $future = [DateTime]::UtcNow.AddMinutes(10)
    $futureReceipt = New-Ori3SyntheticReceipt $future $future.AddHours(24)
    Invoke-Ori3AtomicJsonWrite $receiptPath $futureReceipt
    $miss = Find-Ori3CheckReceipt $context
    Assert-Ori3SelfTest (-not $miss.IsHit -and $miss.Reason.Contains("未来")) "future receipt hit"
    Write-Host "[OK] future receipt => miss"

    $fallbackContext = New-Ori3ReceiptContext "rust-w4" $ReceiptSandboxRoot $null
    [void](Write-Ori3CheckReceipt $fallbackContext)
    $invalidKeyPath = Join-Path (Get-Ori3ReceiptStorePath $ReceiptSandboxRoot) "local-signing-key.dpapi"
    [IO.File]::WriteAllBytes($invalidKeyPath, [byte[]](1, 2, 3, 4, 5, 6, 7, 8))
    $unreadableHit = Find-Ori3CheckReceipt $fallbackContext
    Assert-Ori3SelfTest (-not $unreadableHit.IsHit) "unreadable signing key must not reuse a receipt"
    Assert-Ori3SelfTest ($unreadableHit.Reason -eq "署名鍵を復号できないためreceiptを再利用せず、通常検査を実行します") "unreadable signing key must report the stable fallback reason"
    $normalCheckRequired = -not $unreadableHit.IsHit
    Assert-Ori3SelfTest $normalCheckRequired "unreadable signing key must require the normal check path"
    Write-Ori3ReceiptMissMessage "synthetic unreadable-key fallback" $unreadableHit
    Write-Host "[OK] unreadable signing key => visible receipt miss and normal-check required"

    $repairScript = Join-Path $PSScriptRoot "check-receipt.ps1"
    $repairOutput = @(& powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $repairScript -RepairSigningKey -RepoRoot $ReceiptSandboxRoot 2>&1)
    $repairExit = $LASTEXITCODE
    Assert-Ori3SelfTest ($repairExit -eq 0) ("explicit key repair failed: exit=" + $repairExit + " output=" + ($repairOutput -join "`n"))
    Assert-Ori3SelfTest (($repairOutput -join "`n").Contains("旧領収書は再利用できず")) "explicit key repair did not warn that old receipts cannot be reused"
    $backups = @(Get-ChildItem -LiteralPath (Get-Ori3ReceiptStorePath $ReceiptSandboxRoot) -Filter "local-signing-key.dpapi.invalid-*.dpapi" -File)
    Assert-Ori3SelfTest ($backups.Count -eq 1) "explicit key repair did not retain exactly one invalid-key backup"
    Assert-Ori3SelfTest ($backups[0].Length -eq 8) "invalid signing key backup did not retain the original bytes"
    $repairedKey = [byte[]](Get-Ori3SigningKey $ReceiptSandboxRoot)
    Assert-Ori3SelfTest ($repairedKey.Length -eq 32) "explicit key repair did not create a usable 32-byte signing key"
    $postRepairHit = Find-Ori3CheckReceipt $fallbackContext
    Assert-Ori3SelfTest (-not $postRepairHit.IsHit) "receipt signed by the replaced key must not be reused"
    Assert-Ori3SelfTest ($postRepairHit.Reason.Contains("署名")) "replaced-key receipt miss did not explain the signature mismatch"
    Write-Host "[OK] explicit unreadable-key repair => retained backup, usable replacement, and old receipt miss"
}
finally {
    if ($null -ne $receiptPath -and (Test-Path -LiteralPath $receiptPath -PathType Leaf)) {
        Remove-Item -LiteralPath $receiptPath -Force
    }
    Remove-Ori3ReceiptSelfTestSandbox
}

Write-Host "[OK] synthetic receipt removed; no rust-w4/check-all pass receipt created"

# --- Negative examples for the changed-.rs selector used by the W4 gate's
# timestamp alignment. `git status --porcelain -z` entries are
# `XY <space><path>NUL`, except renames/copies which append the OLD path as a
# second NUL-terminated field with no status letters. The old side must be
# skipped (it no longer exists under that name), deletions must be skipped
# (nothing to touch), and non-.rs paths must never be touched.
$nul = ([char]0).ToString()
$porcelainEntries = @(
    "A  crates/ori3-rigid/src/staged.rs",
    " M apps/desktop/src-tauri/src/store.rs",
    "?? crates/ori3-layers/src/replay.rs",
    "D  crates/ori3-rigid/src/removed_from_index.rs",
    " D crates/ori3-rigid/src/removed_in_worktree.rs",
    "R  crates/ori3-rigid/src/new_name.rs",
    "crates/ori3-rigid/src/old_name.rs",
    "MM crates/ori3-cp/src/curve.rs",
    "?? crates/ori3-rigid/src/notes.md",
    "?? crates/ori3-rigid/build.ps1"
)
$porcelainText = ($porcelainEntries -join $nul) + $nul
$selectedRustPaths = @(Get-Ori3ChangedRustPathsFromPorcelainZ $porcelainText)
$expectedRustPaths = @(
    "crates/ori3-rigid/src/staged.rs",
    "apps/desktop/src-tauri/src/store.rs",
    "crates/ori3-layers/src/replay.rs",
    "crates/ori3-rigid/src/new_name.rs",
    "crates/ori3-cp/src/curve.rs"
)
Assert-Ori3SelfTest (($selectedRustPaths -join $nul) -eq ($expectedRustPaths -join $nul)) ("changed-.rs selection mismatch: got " + ($selectedRustPaths -join ", "))
Assert-Ori3SelfTest (-not ($selectedRustPaths -contains "crates/ori3-rigid/src/old_name.rs")) "rename old side was treated as its own entry"
Assert-Ori3SelfTest (-not ($selectedRustPaths -contains "crates/ori3-rigid/src/removed_from_index.rs")) "index-deleted path was selected"
Assert-Ori3SelfTest (-not ($selectedRustPaths -contains "crates/ori3-rigid/src/removed_in_worktree.rs")) "worktree-deleted path was selected"
Assert-Ori3SelfTest ((@(Get-Ori3ChangedRustPathsFromPorcelainZ "")).Count -eq 0) "empty git status did not select zero paths"
$malformedRejected = $false
try {
    [void](Get-Ori3ChangedRustPathsFromPorcelainZ ("A" + $nul))
}
catch {
    $malformedRejected = $true
}
Assert-Ori3SelfTest $malformedRejected "malformed git status entry was accepted"
Write-Host "[OK] changed-.rs selection: staged/modified/untracked/rename-new only; deletions, rename-old, .md and .ps1 excluded"

# --- Negative-example regression: the -RunRustW4 helper's own process exit
# code must equal cargo's real exit code, never an array whose last element
# happens to be that code.
#
# Real incident (2026-09-04 13:35-15:1x): `git commit` (touching .rs files)
# ran the helper, cargo failed inside `-p desktop --lib` (exit 101), the
# helper printed "[NG] ... 終了コード: 101", and the commit was created
# anyway. Cause: inside Invoke-Ori3RustW4Gate, `& $cargoPath @rustW4Arguments`
# let cargo's stdout flow into the function's own output stream. When the
# caller does `$exitCode = Invoke-Ori3RustW4Gate ...`, every leaked line
# becomes part of the function's return value, so `return $status` became the
# last element of an array. `exit $exitCode` on that array produced process
# exit code 0, and `scripts/hooks/pre-commit` treated 0 as success.
#
# This never invokes the real cargo (never runs cargo test/npm): it puts a
# stub `cargo.cmd` at the front of PATH and drives the real, unmodified
# `scripts/check-receipt.ps1 -RunRustW4` as a genuine child process, then
# asserts the CHILD PROCESS'S OWN exit code (not any value captured inside
# this PowerShell session) equals the stub's exit code.

function New-Ori3StubCargoDirectory {
    param([int]$ExitCode, [string]$ResultLine)
    $directory = Join-Path $ReceiptTempBase ("ori3-stub-cargo-" + [Guid]::NewGuid().ToString("N"))
    [void][IO.Directory]::CreateDirectory($directory)
    $stubPath = Join-Path $directory "cargo.cmd"
    $stubLines = @(
        "@echo off",
        "echo running 3 tests",
        "echo $ResultLine",
        "exit /b $ExitCode"
    )
    [IO.File]::WriteAllText($stubPath, (($stubLines -join "`r`n") + "`r`n"), [Text.ASCIIEncoding]::new())
    return $directory
}

function Remove-Ori3StubCargoDirectory {
    param([string]$Directory)
    if ([string]::IsNullOrWhiteSpace($Directory)) { return }
    if (-not (Test-Path -LiteralPath $Directory)) { return }
    $fullDirectory = [IO.Path]::GetFullPath($Directory).TrimEnd([char[]]"\\/")
    if ([IO.Path]::GetDirectoryName($fullDirectory) -ne $ReceiptTempBase -or
        [IO.Path]::GetFileName($fullDirectory) -notmatch '^ori3-stub-cargo-[0-9a-f]{32}$') {
        throw "Refusing unsafe stub cargo cleanup: $fullDirectory"
    }
    Remove-Item -LiteralPath $fullDirectory -Recurse -Force
}

function Invoke-Ori3RustW4GateChildProcess {
    param([string]$RepoRoot, [string]$GateStatusPath, [string]$StubCargoDirectory)

    $checkReceiptScript = Join-Path $PSScriptRoot "check-receipt.ps1"
    $originalPath = $env:Path
    try {
        $env:Path = $StubCargoDirectory + [IO.Path]::PathSeparator + $originalPath
        $global:LASTEXITCODE = 0
        $childOutput = @(& powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $checkReceiptScript -RunRustW4 -RepoRoot $RepoRoot -GateStatusPath $GateStatusPath 2>&1)
        $childExitCode = $LASTEXITCODE
    }
    finally {
        $env:Path = $originalPath
    }
    return [pscustomobject]@{ ExitCode = $childExitCode; Output = ($childOutput -join "`n") }
}

$ExitCodeSandbox = Join-Path $ReceiptTempBase ("ori3-exit-code-self-test-" + [Guid]::NewGuid().ToString("N"))
$ExitCodeRepo = Join-Path $ExitCodeSandbox "repo"

function Remove-Ori3ExitCodeSelfTestSandbox {
    if (-not (Test-Path -LiteralPath $ExitCodeSandbox)) { return }
    $fullSandbox = [IO.Path]::GetFullPath($ExitCodeSandbox).TrimEnd([char[]]"\\/")
    if ([IO.Path]::GetDirectoryName($fullSandbox) -ne $ReceiptTempBase -or
        [IO.Path]::GetFileName($fullSandbox) -notmatch '^ori3-exit-code-self-test-[0-9a-f]{32}$') {
        throw "Refusing unsafe exit-code self-test cleanup: $fullSandbox"
    }
    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

$stubFailDirectory = $null
$stubOkDirectory = $null
try {
    [void][IO.Directory]::CreateDirectory($ExitCodeRepo)
    $global:LASTEXITCODE = 0
    & git init --quiet $ExitCodeRepo
    if ($LASTEXITCODE -ne 0) { throw "exit-code self-test temporary repository initialization failed: exit=$LASTEXITCODE" }
    [IO.File]::WriteAllText((Join-Path $ExitCodeRepo "fixture.txt"), "exit-code self-test fixture`n", [Text.UTF8Encoding]::new($false))

    $exitCodeReceiptStore = Join-Path $ExitCodeRepo ".origami3\check-receipts"
    [void][IO.Directory]::CreateDirectory($exitCodeReceiptStore)

    $stubFailDirectory = New-Ori3StubCargoDirectory -ExitCode 101 -ResultLine "test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out"
    $failGateStatusPath = Join-Path $exitCodeReceiptStore ("gate-exit-fail-" + [Guid]::NewGuid().ToString("N") + ".status")
    $failResult = Invoke-Ori3RustW4GateChildProcess -RepoRoot $ExitCodeRepo -GateStatusPath $failGateStatusPath -StubCargoDirectory $stubFailDirectory
    Assert-Ori3SelfTest ($failResult.ExitCode -eq 101) ("stub cargo exit 101 did not propagate to the helper's own process exit code: got " + $failResult.ExitCode + " output=" + $failResult.Output)
    Assert-Ori3SelfTest (Test-Path -LiteralPath $failGateStatusPath -PathType Leaf) "gate status file was not written for the failing stub"
    Assert-Ori3SelfTest ((([IO.File]::ReadAllText($failGateStatusPath)).Trim()) -eq "cargo-started") "gate status was not left at cargo-started for the failing stub"
    Assert-Ori3SelfTest ($failResult.Output.Contains("running 3 tests")) "stub cargo stdout (running 3 tests) did not reach the helper's own output"
    Assert-Ori3SelfTest ($failResult.Output.Contains("test result: FAILED")) "stub cargo stdout (test result: FAILED) did not reach the helper's own output"
    # This sandbox has no crates/ or apps/, so the timestamp alignment must
    # report zero and the gate must behave exactly as before.
    Assert-Ori3SelfTest ($failResult.Output.Contains("更新時刻の揃えは 0 件")) "zero changed .rs did not report a zero alignment line"
    Write-Host "[OK] stub cargo exit 101 => helper process exit code 101, gate status cargo-started, stub stdout visible"

    $stubOkDirectory = New-Ori3StubCargoDirectory -ExitCode 0 -ResultLine "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"
    $okGateStatusPath = Join-Path $exitCodeReceiptStore ("gate-exit-ok-" + [Guid]::NewGuid().ToString("N") + ".status")
    $okResult = Invoke-Ori3RustW4GateChildProcess -RepoRoot $ExitCodeRepo -GateStatusPath $okGateStatusPath -StubCargoDirectory $stubOkDirectory
    Assert-Ori3SelfTest ($okResult.ExitCode -eq 0) ("stub cargo exit 0 did not produce helper process exit code 0: got " + $okResult.ExitCode + " output=" + $okResult.Output)
    Assert-Ori3SelfTest ($okResult.Output.Contains("[OK]")) "stub cargo exit 0 did not produce an [OK] line from the helper"
    Assert-Ori3SelfTest ($okResult.Output.Contains("更新時刻の揃えは 0 件")) "zero changed .rs did not report a zero alignment line on the passing path"
    Write-Host "[OK] stub cargo exit 0 => helper process exit code 0 with [OK]"
}
finally {
    Remove-Ori3StubCargoDirectory $stubFailDirectory
    Remove-Ori3StubCargoDirectory $stubOkDirectory
    Remove-Ori3ExitCodeSelfTestSandbox
}

Write-Host "[OK] Rust W4 gate child-process exit code matches stub cargo exit code (no real cargo invoked)"

# --- Negative-example regression: cargo decides "unchanged" from file
# modification times, not from content hashes.
#
# Real incident (2026-09-04 22:37): four .rs files were copied into the
# repository from a separate work copy with Copy-Item. Their content was
# correct and `git diff --cached` matched, but their LastWriteTime stayed at
# the time they had been edited in the copy (19:06-19:26), while the root
# target/ already held ori3-rigid artifacts built at 20:3x-21:4x. cargo judged
# ori3-rigid "unchanged", relinked the stale rlib, and the commit stopped with
# `error[E0425]: cannot find function to_frame3d_geometry_only in crate
# ori3_rigid`. Setting the four files' LastWriteTime to now made the same
# content pass.
#
# Like the exit-code test above, this never invokes the real cargo: it drives
# the real `scripts/check-receipt.ps1 -RunRustW4` as a child process with a
# stub cargo on PATH, and checks the files on disk before and after.

$MtimeSandbox = Join-Path $ReceiptTempBase ("ori3-mtime-self-test-" + [Guid]::NewGuid().ToString("N"))
$MtimeRepo = Join-Path $MtimeSandbox "repo"

function Remove-Ori3MtimeSelfTestSandbox {
    if (-not (Test-Path -LiteralPath $MtimeSandbox)) { return }
    $fullSandbox = [IO.Path]::GetFullPath($MtimeSandbox).TrimEnd([char[]]"\\/")
    if ([IO.Path]::GetDirectoryName($fullSandbox) -ne $ReceiptTempBase -or
        [IO.Path]::GetFileName($fullSandbox) -notmatch '^ori3-mtime-self-test-[0-9a-f]{32}$') {
        throw "Refusing unsafe mtime self-test cleanup: $fullSandbox"
    }
    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

$stubMtimeDirectory = $null
try {
    [void][IO.Directory]::CreateDirectory((Join-Path $MtimeRepo "crates\ori3-stub\src"))
    $global:LASTEXITCODE = 0
    & git init --quiet $MtimeRepo
    if ($LASTEXITCODE -ne 0) { throw "mtime self-test temporary repository initialization failed: exit=$LASTEXITCODE" }

    $stagedRustPath = Join-Path $MtimeRepo "crates\ori3-stub\src\staged.rs"
    $untrackedRustPath = Join-Path $MtimeRepo "crates\ori3-stub\src\untracked.rs"
    $keptMarkdownPath = Join-Path $MtimeRepo "crates\ori3-stub\src\notes.md"
    $keptScriptPath = Join-Path $MtimeRepo "crates\ori3-stub\build.ps1"
    # Negative control: a .rs outside the crates/apps pathspec. Nothing else in
    # the gate touches files, so if this one also moved, the assertions below
    # would be proving something other than the alignment step.
    [void][IO.Directory]::CreateDirectory((Join-Path $MtimeRepo "docs"))
    $outOfScopeRustPath = Join-Path $MtimeRepo "docs\outside.rs"
    $utf8NoBom = [Text.UTF8Encoding]::new($false)
    [IO.File]::WriteAllText($stagedRustPath, "pub fn to_frame3d_geometry_only() {}`n", $utf8NoBom)
    [IO.File]::WriteAllText($untrackedRustPath, "pub fn replay() {}`n", $utf8NoBom)
    [IO.File]::WriteAllText($keptMarkdownPath, "notes`n", $utf8NoBom)
    [IO.File]::WriteAllText($keptScriptPath, "Write-Host 'stub'`n", $utf8NoBom)
    [IO.File]::WriteAllText($outOfScopeRustPath, "pub fn outside() {}`n", $utf8NoBom)

    # Sandbox index only. The real repository's .git is never touched.
    $global:LASTEXITCODE = 0
    & git -C $MtimeRepo add -- "crates/ori3-stub/src/staged.rs"
    if ($LASTEXITCODE -ne 0) { throw "mtime self-test staging failed: exit=$LASTEXITCODE" }

    $staleStamp = (Get-Date).AddHours(-2)
    foreach ($stalePath in @($stagedRustPath, $untrackedRustPath, $keptMarkdownPath, $keptScriptPath, $outOfScopeRustPath)) {
        [IO.File]::SetLastWriteTime($stalePath, $staleStamp)
    }
    $outOfScopeStampBefore = [IO.File]::GetLastWriteTime($outOfScopeRustPath)
    $stagedHashBefore = (Get-FileHash -LiteralPath $stagedRustPath -Algorithm SHA256).Hash
    $untrackedHashBefore = (Get-FileHash -LiteralPath $untrackedRustPath -Algorithm SHA256).Hash
    $markdownStampBefore = [IO.File]::GetLastWriteTime($keptMarkdownPath)
    $scriptStampBefore = [IO.File]::GetLastWriteTime($keptScriptPath)

    $mtimeReceiptStore = Join-Path $MtimeRepo ".origami3\check-receipts"
    [void][IO.Directory]::CreateDirectory($mtimeReceiptStore)
    $mtimeGateStatusPath = Join-Path $mtimeReceiptStore ("gate-mtime-" + [Guid]::NewGuid().ToString("N") + ".status")

    $stubMtimeDirectory = New-Ori3StubCargoDirectory -ExitCode 0 -ResultLine "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"
    $gateStartedAt = Get-Date
    $mtimeResult = Invoke-Ori3RustW4GateChildProcess -RepoRoot $MtimeRepo -GateStatusPath $mtimeGateStatusPath -StubCargoDirectory $stubMtimeDirectory

    Assert-Ori3SelfTest ($mtimeResult.ExitCode -eq 0) ("stub cargo exit 0 with stale .rs did not produce helper process exit code 0: got " + $mtimeResult.ExitCode + " output=" + $mtimeResult.Output)
    $stagedStampAfter = [IO.File]::GetLastWriteTime($stagedRustPath)
    $untrackedStampAfter = [IO.File]::GetLastWriteTime($untrackedRustPath)
    Assert-Ori3SelfTest ($stagedStampAfter -ge $gateStartedAt) ("staged .rs kept its stale timestamp: " + $stagedStampAfter.ToString("o"))
    Assert-Ori3SelfTest ($untrackedStampAfter -ge $gateStartedAt) ("untracked .rs kept its stale timestamp: " + $untrackedStampAfter.ToString("o"))
    Assert-Ori3SelfTest ((Get-FileHash -LiteralPath $stagedRustPath -Algorithm SHA256).Hash -eq $stagedHashBefore) "staged .rs content changed while aligning its timestamp"
    Assert-Ori3SelfTest ((Get-FileHash -LiteralPath $untrackedRustPath -Algorithm SHA256).Hash -eq $untrackedHashBefore) "untracked .rs content changed while aligning its timestamp"
    Assert-Ori3SelfTest ([IO.File]::GetLastWriteTime($keptMarkdownPath) -eq $markdownStampBefore) "a .md file's timestamp was changed"
    Assert-Ori3SelfTest ([IO.File]::GetLastWriteTime($keptScriptPath) -eq $scriptStampBefore) "a .ps1 file's timestamp was changed"
    Assert-Ori3SelfTest ([IO.File]::GetLastWriteTime($outOfScopeRustPath) -eq $outOfScopeStampBefore) "a .rs outside crates/apps was touched, so the two aligned files prove nothing about the alignment step"
    Assert-Ori3SelfTest ($mtimeResult.Output.Contains("2件")) ("timestamp alignment did not report two files: output=" + $mtimeResult.Output)
    Assert-Ori3SelfTest ($mtimeResult.Output.Contains("running 3 tests")) "stub cargo did not run after the timestamp alignment"
    Write-Host "[OK] stale changed .rs => timestamps moved to now with identical SHA-256; .md and .ps1 untouched; stub cargo still ran"
}
finally {
    Remove-Ori3StubCargoDirectory $stubMtimeDirectory
    Remove-Ori3MtimeSelfTestSandbox
}
