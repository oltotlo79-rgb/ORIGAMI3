[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$RunnerPath = Join-Path $PSScriptRoot "run-hook-checks.ps1"
$WarningPath = Join-Path $PSScriptRoot "test-claim-scope-warning.ps1"
$PowerShellPath = (Get-Process -Id $PID).Path
$TempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$SandboxRoot = Join-Path $TempRoot ("ori3-run-hook-checks-test-{0}" -f [Guid]::NewGuid().ToString("N"))
$script:AssertionCount = 0
$script:Utf8NoBom = New-Object Text.UTF8Encoding($false)
$CheckFiles = @(
    "no-allow-attribute.ps1",
    "ignore-reason-has-number.ps1",
    "tracked-fixture-only.ps1",
    "no-prohibited-doc.ps1",
    "known-defect-shapes.ps1",
    "run-hook-checks.ps1",
    "test-claim-scope-warning.ps1"
)

function Assert-True {
    param([bool]$Condition, [string]$Message)
    $script:AssertionCount += 1
    if (-not $Condition) { throw "ASSERTION FAILED: $Message" }
}

function Write-TestFile {
    param([string]$Path, [string]$Content)
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $Path))
    [IO.File]::WriteAllText($Path, $Content, $script:Utf8NoBom)
}

function Invoke-Git {
    param([string]$Repository, [string[]]$Arguments)
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        & git -c core.excludesFile=NUL -C $Repository @Arguments 1>$null 2>$null
        $exitCode = $LASTEXITCODE
    }
    finally { $ErrorActionPreference = $previousPreference }
    if ($exitCode -ne 0) { throw "temporary git failed: $($Arguments -join ' ') (exit=$exitCode)" }
}

function Invoke-GitResult {
    param([string]$Repository, [string[]]$Arguments)
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        $output = @(& git -c core.excludesFile=NUL -C $Repository @Arguments 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally { $ErrorActionPreference = $previousPreference }
    return [pscustomobject]@{
        ExitCode = $exitCode
        Output = (($output | ForEach-Object { [string]$_ }) -join "`n")
    }
}

function Install-PreCommit {
    param([Parameter(Mandatory = $true)][string]$Repository)
    $source = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\pre-commit"))
    $destination = Join-Path $Repository ".git\hooks\pre-commit"
    [IO.File]::Copy($source, $destination, $true)
}

function New-TestRepository {
    param([string]$Name)

    $repository = Join-Path $SandboxRoot $Name
    [void][IO.Directory]::CreateDirectory($repository)
    Invoke-Git $repository @("init", "--quiet")
    Invoke-Git $repository @("config", "user.email", "hook-checks@example.invalid")
    Invoke-Git $repository @("config", "user.name", "Hook Checks Test")

    $checksDirectory = Join-Path $repository "scripts\hooks\checks"
    [void][IO.Directory]::CreateDirectory($checksDirectory)
    foreach ($file in $CheckFiles) {
        [IO.File]::Copy((Join-Path $PSScriptRoot $file), (Join-Path $checksDirectory $file), $true)
    }
    Write-TestFile (Join-Path $repository ".github\known-defect-shapes.json") @'
{
  "schemaVersion": 1,
  "patterns": [
    {
      "id": "injected-known-defect",
      "reason": "The test catalog starts at zero and rejects one injected marker.",
      "roots": ["src"],
      "filePattern": "\\.rs$",
      "regex": "KNOWN_DEFECT",
      "registeredCount": 0,
      "measuredRawCount": 0,
      "exceptions": []
    }
  ]
}
'@
    Write-TestFile (Join-Path $repository "src\lib.rs") "pub fn clean() -> bool { true }`n"
    Write-TestFile (Join-Path $repository "crates\demo\tests\fixtures\tracked.txt") "tracked`n"
    $trackedFixtureSource = @'
#[test]
fn reads_a_tracked_fixture() {
    let _path = "__TRACKED_FIXTURE__";
    assert!(true);
    assert_eq!(2 + 2, 4);
}
'@
    Write-TestFile `
        (Join-Path $repository "crates\demo\tests\contract_test.rs") `
        $trackedFixtureSource.Replace("__TRACKED_FIXTURE__", ("fix" + "tures/tracked.txt"))
    Invoke-Git $repository @("add", "-A")
    Invoke-Git $repository @("commit", "--quiet", "-m", "baseline")
    return $repository
}

function Set-KnownDefectInventoryFixture {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [switch]$InheritedDrift
    )

    $secondException = if ($InheritedDrift) {
        @'
,
        {
          "path": "src/lib.rs",
          "regex": "KNOWN_DEFECT_B",
          "allowedCount": 1,
          "line": 2,
          "macro": "KNOWN_DEFECT_B",
          "reason": "The missing second marker creates one inherited inventory drift."
        }
'@
    }
    else { "" }
    $measuredRawCount = if ($InheritedDrift) { 2 } else { 1 }
    $catalog = @"
{
  "schemaVersion": 1,
  "patterns": [
    {
      "id": "inventory-ratchet",
      "reason": "The staged ratchet compares exact inventory movement with HEAD.",
      "roots": ["src"],
      "filePattern": "\\.rs$",
      "regex": "KNOWN_DEFECT_[AB]",
      "registeredCount": 0,
      "measuredRawCount": $measuredRawCount,
      "exceptions": [
        {
          "path": "src/lib.rs",
          "regex": "KNOWN_DEFECT_A",
          "allowedCount": 1,
          "line": 1,
          "macro": "KNOWN_DEFECT_A",
          "reason": "The first marker is registered at its exact line."
        }$secondException
      ]
    }
  ]
}
"@
    Write-TestFile (Join-Path $Repository ".github\known-defect-shapes.json") $catalog
    Write-TestFile (Join-Path $Repository "src\lib.rs") "const KNOWN_DEFECT_A: usize = 1;`npub fn clean() -> bool { true }`n"
    Invoke-Git $Repository @("add", "-A")
    Invoke-Git $Repository @("commit", "--quiet", "-m", "inventory baseline")
}

function Invoke-Runner {
    param([string]$Repository, [string]$Mode = "Tree")
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        $output = @(& $PowerShellPath -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $RunnerPath -Mode $Mode -RepositoryRoot $Repository 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally { $ErrorActionPreference = $previousPreference }
    return [pscustomobject]@{ ExitCode = $exitCode; Output = (($output | ForEach-Object { [string]$_ }) -join "`n") }
}

function Assert-RejectedBy {
    param([object]$Result, [string]$CheckName)
    Assert-True ($Result.ExitCode -ne 0) "$CheckName injection must be rejected"
    Assert-True ($Result.Output.Contains("check=$CheckName")) "$CheckName rejection must identify the check"
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $SandboxRoot)) { return }
    $full = [IO.Path]::GetFullPath($SandboxRoot).TrimEnd([char[]]"\/")
    if ([IO.Path]::GetDirectoryName($full) -cne $TempRoot -or
        [IO.Path]::GetFileName($full) -notmatch '^ori3-run-hook-checks-test-[0-9a-f]{32}$') {
        throw "Refusing unsafe self-test cleanup: $full"
    }
    Remove-Item -LiteralPath $full -Recurse -Force
}

[void][IO.Directory]::CreateDirectory($SandboxRoot)
try {
    Write-Host "[1/13] a clean tree passes all five blocking checks"
    $repository = New-TestRepository "clean"
    $result = Invoke-Runner $repository
    Assert-True ($result.ExitCode -eq 0) "clean tree must pass"
    Assert-True ($result.Output.Contains("checks=5 violations=0 unavailable=0")) "clean summary must report five checks"

    Write-Host "[2/13] an added allow attribute is rejected"
    $repository = New-TestRepository "allow"
    Write-TestFile (Join-Path $repository "src\lib.rs") "#[allow(dead_code)]`npub fn hidden() {}`n"
    Assert-RejectedBy (Invoke-Runner $repository) "no-allow-attribute"

    Write-Host "[3/13] an ignore reason without a number is rejected"
    $repository = New-TestRepository "ignore"
    Write-TestFile (Join-Path $repository "crates\demo\tests\contract_test.rs") "#[test]`n#[ignore = `"not measured`"]`nfn skipped() { assert!(true); }`n"
    Assert-RejectedBy (Invoke-Runner $repository) "ignore-reason-has-number"

    Write-Host "[4/13] an untracked fixture reference is rejected"
    $repository = New-TestRepository "fixture"
    $missingFixture = "fix" + "tures/missing.txt"
    Write-TestFile (Join-Path $repository "crates\demo\tests\contract_test.rs") "#[test]`nfn missing_fixture() { let _ = `"$missingFixture`"; assert!(true); }`n"
    Assert-RejectedBy (Invoke-Runner $repository) "tracked-fixture-only"

    Write-Host "[5/13] an untracked prohibited tree path is excluded without reading its body"
    $repository = New-TestRepository "prohibited"
    Write-TestFile (Join-Path $repository "docs\competitive-review-2026-08-20.md") "body is deliberately irrelevant`n"
    $result = Invoke-Runner $repository
    Assert-True ($result.ExitCode -eq 0) "untracked prohibited path must not fail Tree mode"
    Assert-True ($result.Output.Contains("[EXCLUDED untracked-prohibited-paths=1 body-read=0]")) "Tree mode must report one excluded untracked path"

    Write-Host "[6/13] the same prohibited path is rejected when tracked"
    $repository = New-TestRepository "prohibited-tracked"
    Write-TestFile (Join-Path $repository "docs\competitive-review-2026-08-20.md") "dummy fixture body`n"
    Invoke-Git $repository @("add", "--", "docs/competitive-review-2026-08-20.md")
    Invoke-Git $repository @("commit", "--quiet", "-m", "track prohibited dummy")
    Assert-RejectedBy (Invoke-Runner $repository) "no-prohibited-doc"

    Write-Host "[7/13] one known-defect marker is rejected"
    $repository = New-TestRepository "known-defect"
    Write-TestFile (Join-Path $repository "src\lib.rs") "pub fn broken() { /* KNOWN_DEFECT */ }`nKNOWN_DEFECT`n"
    Assert-RejectedBy (Invoke-Runner $repository) "known-defect-shapes"

    Write-Host "[8/13] a missing required checker is unavailable and rejected"
    $repository = New-TestRepository "missing"
    Remove-Item -LiteralPath (Join-Path $repository "scripts\hooks\checks\no-allow-attribute.ps1") -Force
    $result = Invoke-Runner $repository
    Assert-True ($result.ExitCode -eq 2) "missing checker must use unavailable exit 2"
    Assert-True ($result.Output.Contains("check=no-allow-attribute reason=missing")) "missing checker must be identified"

    Write-Host "[9/13] a checker syntax error is unavailable and rejected"
    $repository = New-TestRepository "syntax"
    Write-TestFile (Join-Path $repository "scripts\hooks\checks\tracked-fixture-only.ps1") "param(`n"
    $result = Invoke-Runner $repository
    Assert-True ($result.ExitCode -eq 2) "syntax error must use unavailable exit 2"
    Assert-True ($result.Output.Contains("check=tracked-fixture-only reason=syntax-error")) "syntax-error checker must be identified"

    Write-Host "[10/13] the claim-scope warning remains nonblocking"
    $repository = New-TestRepository "warning"
    Write-TestFile (Join-Path $repository "crates\demo\tests\contract_test.rs") "#[test]`nfn reads_a_tracked_fixture() { assert!(true); }`n"
    Invoke-Git $repository @("add", "--", "crates/demo/tests/contract_test.rs")
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        $warningOutput = @(& $PowerShellPath -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $WarningPath -RepositoryRoot $repository 2>&1)
        $warningExit = $LASTEXITCODE
    }
    finally { $ErrorActionPreference = $previousPreference }
    $warningText = ($warningOutput | ForEach-Object { [string]$_ }) -join "`n"
    Assert-True ($warningExit -eq 0) "claim-scope finding must remain nonblocking"
    Assert-True ($warningText.Contains("assertion calls decreased")) "claim-scope warning must remain visible"

    Write-Host "[11/13] staged unrelated content inherits an existing HEAD inventory drift"
    $repository = New-TestRepository "staged-inherited"
    Set-KnownDefectInventoryFixture -Repository $repository -InheritedDrift
    Install-PreCommit -Repository $repository
    Write-TestFile (Join-Path $repository "README.md") "unrelated staged change`n"
    Invoke-Git $repository @("add", "--", "README.md")
    $result = Invoke-GitResult $repository @("commit", "--quiet", "-m", "unrelated staged change")
    Assert-True ($result.ExitCode -eq 0) "an unchanged inherited HEAD drift must not block a staged commit"
    Assert-True ($result.Output.Contains("[INHERITED head-red increase=0 decrease=0 inventory-drift=1]")) "inherited HEAD drift and counts must be reported"

    Write-Host "[12/13] staged Rust content that increases inherited drift is rejected"
    $repository = New-TestRepository "staged-worse-inherited"
    Set-KnownDefectInventoryFixture -Repository $repository -InheritedDrift
    Install-PreCommit -Repository $repository
    Write-TestFile (Join-Path $repository "src\lib.rs") "`nconst KNOWN_DEFECT_A: usize = 1;`npub fn clean() -> bool { true }`n"
    Invoke-Git $repository @("add", "--", "src/lib.rs")
    $result = Invoke-GitResult $repository @("commit", "--quiet", "-m", "worsen inherited drift")
    Assert-True ($result.ExitCode -eq 1) "an increased inherited inventory drift must be rejected"
    Assert-True ($result.Output.Contains("inventory-drift 1 -> 2")) "worsening output must report the inherited drift transition"

    Write-Host "[13/13] staged Rust content that makes a green HEAD drift is rejected"
    $repository = New-TestRepository "staged-worse-green"
    Set-KnownDefectInventoryFixture -Repository $repository
    Install-PreCommit -Repository $repository
    Write-TestFile (Join-Path $repository "src\lib.rs") "`nconst KNOWN_DEFECT_A: usize = 1;`npub fn clean() -> bool { true }`n"
    Invoke-Git $repository @("add", "--", "src/lib.rs")
    $result = Invoke-GitResult $repository @("commit", "--quiet", "-m", "create inventory drift")
    Assert-True ($result.ExitCode -eq 1) "a new inventory drift from a green HEAD must be rejected"
    Assert-True ($result.Output.Contains("inventory-drift 0 -> 1")) "worsening output must report the new drift transition"

    Write-Host "[OK] run-hook-checks self-test passed: 13/13 cases, $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
