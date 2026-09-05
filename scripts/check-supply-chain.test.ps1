[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Isolated self-test for scripts/check-supply-chain.ps1 (10-A policy/license checker).
#
# It exists because of a real CI failure. On 2026-09-05 the security workflow reported
# "10-A OVERALL: FAILED" with 44 violations. 42 of them accused .github/dependabot.yml and
# .github/workflows/security.yml of missing declarations they actually contained. The GitHub
# Actions Windows runner checks the repository out with CRLF line endings, and many of the
# checker's patterns anchor with (?m)$ directly after [ \t]*; in .NET that position sits
# after the "`r", which [ \t] cannot consume, so every one of those patterns missed. The same
# two files with LF endings produced 0 offline configuration violations locally.
#
# The tests below therefore run the real checker as a new process against the repository's
# real configuration files, once with LF and once with CRLF, and require the same verdict
# from both. Negative cases prove the line-ending fix did not blind any check: a file with
# its declaration deleted must still be rejected in BOTH line-ending forms. Two further
# cases pin the "exactly eleven workspace-owned MIT overrides" claim as a hard equality, and
# a last one proves the number 11 is the true count of source-less packages in Cargo.lock
# rather than a number chosen to make the run green.
#
# All comments in this file are kept in plain ASCII on purpose, matching
# scripts/supply-chain-verify-release-hashes.test.ps1: when invoked as
# `powershell.exe -File <path>` (Windows PowerShell 5.1) on a machine whose system codepage
# is 932, a UTF-8 file without a BOM can fail to parse, and staying ASCII sidesteps that.

$RepositoryRoot = Split-Path -Parent $PSScriptRoot
$CheckScriptPath = Join-Path $PSScriptRoot "check-supply-chain.ps1"
$RealPolicyPath = Join-Path $RepositoryRoot ".github\security-policy.json"
$RealDependabotPath = Join-Path $RepositoryRoot ".github\dependabot.yml"
$RealSecurityWorkflowPath = Join-Path $RepositoryRoot ".github\workflows\security.yml"
$RealCargoLockPath = Join-Path $RepositoryRoot "Cargo.lock"
foreach ($requiredFile in @(
    $CheckScriptPath,
    $RealPolicyPath,
    $RealDependabotPath,
    $RealSecurityWorkflowPath,
    $RealCargoLockPath
)) {
    if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
        throw "Required input is missing: $requiredFile"
    }
}

$SandboxRoot = Join-Path ([IO.Path]::GetTempPath()) ("ori3-supply-chain-policy-test-" + [Guid]::NewGuid().ToString("N"))
$Utf8NoBom = [Text.UTF8Encoding]::new($false)
$script:AssertionCount = 0
$script:CaseIndex = 0

# The offline configuration section reads only the policy, dependabot.yml and security.yml.
# The license section additionally needs a Cargo lockfile and an npm lockfile to exist, and
# it consults the local Node.js and Cargo registry cache, both of which differ from machine
# to machine. The stand-ins below keep that section machine-independent: the npm lockfile is
# empty, and the Cargo lockfile carries only the repository's real source-less (path)
# packages, which the checker resolves against the policy alone and never looks up in a
# registry cache. Every assertion is about the offline configuration section or a named
# violation message, never about license totals or the overall exit code.
function Get-RealCargoPathPackages {
    $lockText = [IO.File]::ReadAllText($RealCargoLockPath)
    $packages = @()
    foreach ($block in @([regex]::Split($lockText, '(?m)^\[\[package\]\]\s*$') | Select-Object -Skip 1)) {
        $nameMatch = [regex]::Match($block, '(?m)^name = "([^"]+)"')
        $versionMatch = [regex]::Match($block, '(?m)^version = "([^"]+)"')
        if (-not $nameMatch.Success -or -not $versionMatch.Success) {
            throw "Cargo.lock contains a package block without a name or version"
        }
        if (-not [regex]::IsMatch($block, '(?m)^source = "')) {
            $packages += [pscustomobject]@{
                Name    = $nameMatch.Groups[1].Value
                Version = $versionMatch.Groups[1].Value
            }
        }
    }
    return @($packages)
}

$RealCargoPathPackages = Get-RealCargoPathPackages
if ($RealCargoPathPackages.Count -eq 0) {
    throw "Cargo.lock declares no workspace-owned path packages, so this self-test cannot run"
}
$cargoLockBuilder = New-Object Text.StringBuilder
[void]$cargoLockBuilder.Append("version = 4`n")
foreach ($pathPackage in $RealCargoPathPackages) {
    [void]$cargoLockBuilder.Append("`n[[package]]`nname = `"$($pathPackage.Name)`"`nversion = `"$($pathPackage.Version)`"`n")
}
$MinimalCargoLock = $cargoLockBuilder.ToString()
$MinimalPackageLock = "{`n  `"name`": `"ori3-supply-chain-selftest`",`n  `"lockfileVersion`": 3,`n  `"packages`": {}`n}`n"

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

function ConvertTo-LineEnding {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][ValidateSet("LF", "CRLF")][string]$Style
    )

    $lf = ($Text -replace "`r`n", "`n") -replace "`r", "`n"
    if ($Style -eq "LF") {
        return $lf
    }
    return ($lf -replace "`n", "`r`n")
}

function New-ConfigurationFixture {
    # Writes the repository's real configuration files into a disposable directory using the
    # requested line-ending style, optionally transforming the text of one of them first.
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][ValidateSet("LF", "CRLF")][string]$Style,
        [scriptblock]$MutateDependabot,
        [scriptblock]$MutateSecurityWorkflow,
        [scriptblock]$MutatePolicy
    )

    $root = Join-Path $SandboxRoot $Name
    [void][IO.Directory]::CreateDirectory($root)

    $dependabotText = [IO.File]::ReadAllText($RealDependabotPath)
    if ($null -ne $MutateDependabot) {
        $dependabotText = [string](& $MutateDependabot $dependabotText)
    }
    $securityWorkflowText = [IO.File]::ReadAllText($RealSecurityWorkflowPath)
    if ($null -ne $MutateSecurityWorkflow) {
        $securityWorkflowText = [string](& $MutateSecurityWorkflow $securityWorkflowText)
    }
    $policyText = [IO.File]::ReadAllText($RealPolicyPath)
    if ($null -ne $MutatePolicy) {
        $policyText = [string](& $MutatePolicy $policyText)
    }

    $dependabotPath = Join-Path $root "dependabot.yml"
    $securityWorkflowPath = Join-Path $root "security.yml"
    $policyPath = Join-Path $root "security-policy.json"
    $cargoLockPath = Join-Path $root "Cargo.lock"
    $packageLockPath = Join-Path $root "package-lock.json"
    [IO.File]::WriteAllText($dependabotPath, (ConvertTo-LineEnding -Text $dependabotText -Style $Style), $Utf8NoBom)
    [IO.File]::WriteAllText($securityWorkflowPath, (ConvertTo-LineEnding -Text $securityWorkflowText -Style $Style), $Utf8NoBom)
    [IO.File]::WriteAllText($policyPath, (ConvertTo-LineEnding -Text $policyText -Style $Style), $Utf8NoBom)
    [IO.File]::WriteAllText($cargoLockPath, (ConvertTo-LineEnding -Text $MinimalCargoLock -Style $Style), $Utf8NoBom)
    [IO.File]::WriteAllText($packageLockPath, (ConvertTo-LineEnding -Text $MinimalPackageLock -Style $Style), $Utf8NoBom)

    [pscustomobject]@{
        Root                 = $root
        Style                = $Style
        PolicyPath           = $policyPath
        DependabotPath       = $dependabotPath
        SecurityWorkflowPath = $securityWorkflowPath
        CargoLockPath        = $cargoLockPath
        PackageLockPath      = $packageLockPath
    }
}

function Invoke-CheckScript {
    # Runs the checker as a genuinely new process so the reported text and exit code come
    # from that process, not from this session's state.
    param([Parameter(Mandatory = $true)]$Fixture)

    $stdOutPath = Join-Path $Fixture.Root "stdout.txt"
    $stdErrPath = Join-Path $Fixture.Root "stderr.txt"
    $argumentList = @(
        "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ('"{0}"' -f $CheckScriptPath),
        "-Mode", "PolicyAndLicenses",
        "-PolicyPath", ('"{0}"' -f $Fixture.PolicyPath),
        "-DependabotPath", ('"{0}"' -f $Fixture.DependabotPath),
        "-SecurityWorkflowPath", ('"{0}"' -f $Fixture.SecurityWorkflowPath),
        "-CargoLockPath", ('"{0}"' -f $Fixture.CargoLockPath),
        "-PackageLockPath", ('"{0}"' -f $Fixture.PackageLockPath)
    ) -join ' '
    $process = Start-Process -FilePath "powershell.exe" -ArgumentList $argumentList `
        -NoNewWindow -PassThru -Wait `
        -RedirectStandardOutput $stdOutPath -RedirectStandardError $stdErrPath
    $stdOutText = ""
    if (Test-Path -LiteralPath $stdOutPath) {
        $raw = Get-Content -LiteralPath $stdOutPath -Raw -Encoding UTF8
        if ($null -ne $raw) {
            $stdOutText = [string]$raw
        }
    }

    [pscustomobject]@{
        ExitCode = $process.ExitCode
        StdOut   = $stdOutText
    }
}

function Get-OfflineConfigurationLine {
    param([Parameter(Mandatory = $true)][string]$StdOut)

    $line = @($StdOut -split "`r?`n" | Where-Object { $_ -like "Offline configuration checks:*" }) |
        Select-Object -First 1
    if ($null -eq $line) {
        return ""
    }
    return [string]$line
}

function Assert-OfflineConfigurationPassed {
    param(
        [Parameter(Mandatory = $true)]$Result,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $line = Get-OfflineConfigurationLine -StdOut $Result.StdOut
    Assert-True ($line -eq "Offline configuration checks: PASSED (policy, Dependabot, security workflow)") `
        "$Message (actual offline line: '$line')"
}

function Assert-StdOutContains {
    param(
        [Parameter(Mandatory = $true)]$Result,
        [Parameter(Mandatory = $true)][string]$Needle,
        [Parameter(Mandatory = $true)][string]$Message
    )

    Assert-True ($Result.StdOut.Contains($Needle)) "$Message (missing: '$Needle')"
}

function Assert-StdOutDoesNotContain {
    param(
        [Parameter(Mandatory = $true)]$Result,
        [Parameter(Mandatory = $true)][string]$Needle,
        [Parameter(Mandatory = $true)][string]$Message
    )

    Assert-True (-not $Result.StdOut.Contains($Needle)) "$Message (unexpectedly present: '$Needle')"
}

function Write-Case {
    param([Parameter(Mandatory = $true)][string]$Title)

    $script:CaseIndex += 1
    Write-Host "[$script:CaseIndex] $Title"
}

[void][IO.Directory]::CreateDirectory($SandboxRoot)
try {
    Write-Case "the repository's real configuration passes the offline checks with LF endings"
    $lfFixture = New-ConfigurationFixture -Name "pass-lf" -Style "LF"
    $lfResult = Invoke-CheckScript -Fixture $lfFixture
    Assert-OfflineConfigurationPassed -Result $lfResult `
        -Message "LF configuration must produce zero offline configuration violations"
    Assert-StdOutDoesNotContain -Result $lfResult -Needle "is not a recorded workspace-owned MIT package." `
        -Message "every workspace-owned path package in Cargo.lock must be recorded in the policy"
    Assert-StdOutDoesNotContain -Result $lfResult -Needle "Exactly eleven workspace-owned MIT overrides are required." `
        -Message "the recorded override list must satisfy the fixed count"

    Write-Case "the same bytes with CRLF endings must produce the same verdict (CI regression)"
    $crlfFixture = New-ConfigurationFixture -Name "pass-crlf" -Style "CRLF"
    $crlfResult = Invoke-CheckScript -Fixture $crlfFixture
    Assert-OfflineConfigurationPassed -Result $crlfResult `
        -Message "CRLF configuration must produce zero offline configuration violations"
    # The exact wording of the 2026-09-05 CI failures. None of them may reappear on CRLF.
    foreach ($crlfRegression in @(
        "dependabot.yml must declare version 2.",
        "dependabot 'npm' group 'npm-minor-and-patch' must list exactly minor and patch update types.",
        "security.yml must be named Security.",
        "security.yml must declare workflow triggers.",
        "security.yml is missing job 'cargo_and_licenses'.",
        "security.yml cargo-audit receipt must write 'executableSha256'."
    )) {
        Assert-StdOutDoesNotContain -Result $crlfResult -Needle $crlfRegression `
            -Message "a CRLF checkout must not be accused of a declaration the file contains"
    }

    Write-Case "deleting 'version: 2' from dependabot.yml is still rejected with LF endings"
    $lfMissingVersion = New-ConfigurationFixture -Name "fail-lf-no-version" -Style "LF" -MutateDependabot {
        param($text)
        $stripped = [regex]::Replace($text, '(?m)^version:[ \t]*2[ \t]*\r?\n', '')
        if ($stripped -eq $text) {
            throw "fixture did not remove the version declaration from dependabot.yml"
        }
        return $stripped
    }
    $lfMissingVersionResult = Invoke-CheckScript -Fixture $lfMissingVersion
    Assert-StdOutContains -Result $lfMissingVersionResult -Needle "dependabot.yml must declare version 2." `
        -Message "a dependabot.yml without 'version: 2' must fail with LF endings"

    Write-Case "deleting 'version: 2' from dependabot.yml is still rejected with CRLF endings"
    $crlfMissingVersion = New-ConfigurationFixture -Name "fail-crlf-no-version" -Style "CRLF" -MutateDependabot {
        param($text)
        $stripped = [regex]::Replace($text, '(?m)^version:[ \t]*2[ \t]*\r?\n', '')
        if ($stripped -eq $text) {
            throw "fixture did not remove the version declaration from dependabot.yml"
        }
        return $stripped
    }
    $crlfMissingVersionResult = Invoke-CheckScript -Fixture $crlfMissingVersion
    Assert-StdOutContains -Result $crlfMissingVersionResult -Needle "dependabot.yml must declare version 2." `
        -Message "a dependabot.yml without 'version: 2' must fail with CRLF endings too"

    Write-Case "renaming the security workflow is still rejected with CRLF endings"
    $crlfRenamedWorkflow = New-ConfigurationFixture -Name "fail-crlf-workflow-name" -Style "CRLF" -MutateSecurityWorkflow {
        param($text)
        # "\r?$" here, not "$": this fixture reads the file as-is and must edit it whether the
        # working copy is checked out with LF or CRLF. Anchoring with a bare "$" after [ \t]*
        # is exactly the mistake that produced the 2026-09-05 CI failure.
        $renamed = [regex]::Replace($text, '(?m)^name:[ \t]*Security[ \t]*\r?$', 'name: Something Else', 1)
        if ($renamed -eq $text) {
            throw "fixture did not rename the security workflow"
        }
        return $renamed
    }
    $crlfRenamedWorkflowResult = Invoke-CheckScript -Fixture $crlfRenamedWorkflow
    Assert-StdOutContains -Result $crlfRenamedWorkflowResult -Needle "security.yml must be named Security." `
        -Message "a renamed security workflow must fail with CRLF endings"

    Write-Case "removing one workspace-owned MIT override breaks the fixed count of eleven"
    $tooFewOverrides = New-ConfigurationFixture -Name "fail-overrides-ten" -Style "LF" -MutatePolicy {
        param($text)
        $reduced = [regex]::Replace($text, '(?m)^[ \t]*"ori3-web",[ \t]*\r?\n', '', 1)
        if ($reduced -eq $text) {
            throw "fixture did not remove an override entry from security-policy.json"
        }
        return $reduced
    }
    $tooFewOverridesResult = Invoke-CheckScript -Fixture $tooFewOverrides
    Assert-StdOutContains -Result $tooFewOverridesResult -Needle "Exactly eleven workspace-owned MIT overrides are required." `
        -Message "ten overrides must fail the fixed count"
    Assert-StdOutContains -Result $tooFewOverridesResult -Needle "Cargo path package 'ori3-web@0.5.0' is not a recorded workspace-owned MIT package." `
        -Message "an unregistered workspace package must still be reported as unknown"

    Write-Case "adding an extra override also breaks the fixed count (it is an equality, not a minimum)"
    $tooManyOverrides = New-ConfigurationFixture -Name "fail-overrides-twelve" -Style "LF" -MutatePolicy {
        param($text)
        $extended = [regex]::Replace($text, '(?m)^([ \t]*)"ori3-web",[ \t]*\r?\n', "`${1}`"ori3-web`",`n`${1}`"ori3-not-a-real-crate`",`n", 1)
        if ($extended -eq $text) {
            throw "fixture did not add an override entry to security-policy.json"
        }
        return $extended
    }
    $tooManyOverridesResult = Invoke-CheckScript -Fixture $tooManyOverrides
    Assert-StdOutContains -Result $tooManyOverridesResult -Needle "Exactly eleven workspace-owned MIT overrides are required." `
        -Message "twelve overrides must fail the fixed count"

    Write-Case "eleven is the real number of path packages in Cargo.lock, and the names match"
    # This is what makes the fixed count evidence rather than a tuned constant: every package
    # block in the committed Cargo.lock that has no 'source' line is a workspace-owned path
    # package, and that set must be exactly the recorded override set.
    $pathPackageNames = @($RealCargoPathPackages | ForEach-Object { $_.Name })
    $recordedOverrides = @(([IO.File]::ReadAllText($RealPolicyPath) | ConvertFrom-Json).licensePolicy.workspaceOwnedMitOverrides)
    Assert-True ($recordedOverrides.Count -eq 11) `
        "security-policy.json must record eleven workspace-owned MIT overrides (actual: $($recordedOverrides.Count))"
    Assert-True ($pathPackageNames.Count -eq 11) `
        "Cargo.lock must contain eleven source-less path packages (actual: $($pathPackageNames.Count))"
    $missingFromPolicy = @($pathPackageNames | Where-Object { $recordedOverrides -notcontains $_ })
    Assert-True ($missingFromPolicy.Count -eq 0) `
        "every Cargo.lock path package must be recorded as a workspace-owned MIT override (missing: $($missingFromPolicy -join ', '))"
    $missingFromLock = @($recordedOverrides | Where-Object { $pathPackageNames -notcontains $_ })
    Assert-True ($missingFromLock.Count -eq 0) `
        "every recorded override must correspond to a Cargo.lock path package (extra: $($missingFromLock -join ', '))"
    foreach ($newCrate in @("ori3-app-core", "ori3-web")) {
        Assert-True ($recordedOverrides -contains $newCrate) `
            "the workspace crate '$newCrate' added by commit 7075bd4 must be recorded"
    }

    Write-Host "check-supply-chain self-test passed: $script:CaseIndex cases, $script:AssertionCount assertions"
}
finally {
    if (Test-Path -LiteralPath $SandboxRoot) {
        $fullSandbox = [IO.Path]::GetFullPath($SandboxRoot).TrimEnd([char[]]"\/")
        $fullTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
        $leaf = [IO.Path]::GetFileName($fullSandbox)
        if (([IO.Path]::GetDirectoryName($fullSandbox) -eq $fullTemp) -and
            ([regex]::IsMatch($leaf, "^ori3-supply-chain-policy-test-[0-9a-f]{32}$", [Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
            Remove-Item -LiteralPath $fullSandbox -Recurse -Force
        }
    }
}
