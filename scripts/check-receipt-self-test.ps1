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

$arguments = @(Get-Ori3RustW4Arguments)
$expectedArguments = @(
    "test", "--workspace", "--",
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
$context = [pscustomobject]@{
    Kind = $kind
    Root = $root
    Content = $content
    Recipe = $recipe
    Conditions = $conditions
    EligibilitySha256 = ("d" * 64)
}
$receiptPath = Get-Ori3ReceiptPath $context
$key = [byte[]](Get-Ori3SigningKey $root -Create)

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
        Root = $root
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
}
finally {
    if (Test-Path -LiteralPath $receiptPath -PathType Leaf) {
        Remove-Item -LiteralPath $receiptPath -Force
    }
}

Write-Host "[OK] synthetic receipt removed; no rust-w4/check-all pass receipt created"
