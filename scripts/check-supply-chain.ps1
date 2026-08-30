[CmdletBinding()]
param(
    [string]$PolicyPath,
    [string]$CargoLockPath,
    [string]$PackageLockPath,
    [string]$DependabotPath,
    [string]$SecurityWorkflowPath,
    [string]$CargoAuditReceiptPath,
    [ValidateSet("FullReadiness", "PolicyAndLicenses", "ReadinessOnly")]
    [string]$Mode = "FullReadiness"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($PolicyPath)) {
    $PolicyPath = Join-Path $PSScriptRoot "..\.github\security-policy.json"
}
if ([string]::IsNullOrWhiteSpace($CargoLockPath)) {
    $CargoLockPath = Join-Path $PSScriptRoot "..\Cargo.lock"
}
if ([string]::IsNullOrWhiteSpace($PackageLockPath)) {
    $PackageLockPath = Join-Path $PSScriptRoot "..\apps\desktop\package-lock.json"
}
if ([string]::IsNullOrWhiteSpace($DependabotPath)) {
    $DependabotPath = Join-Path $PSScriptRoot "..\.github\dependabot.yml"
}
if ([string]::IsNullOrWhiteSpace($SecurityWorkflowPath)) {
    $SecurityWorkflowPath = Join-Path $PSScriptRoot "..\.github\workflows\security.yml"
}
if ([string]::IsNullOrWhiteSpace($CargoAuditReceiptPath) -and
    -not [string]::IsNullOrWhiteSpace($env:CARGO_AUDIT_PIN_RECEIPT)) {
    $CargoAuditReceiptPath = $env:CARGO_AUDIT_PIN_RECEIPT
}

# Validate the policy, exceptions, and current lockfile licenses. FullReadiness
# also preserves the 10-A blockers for immutable tool pins and the known high
# advisory assessment. CI runs PolicyAndLicenses before ecosystem-specific
# audit tools and ReadinessOnly afterwards, without scanning licenses twice.
$ScriptVersion = "1.4.0"
$Failures = New-Object System.Collections.Generic.List[string]
$ToolPinBlockers = New-Object System.Collections.Generic.List[string]
$AdvisoryAssessmentBlockers = New-Object System.Collections.Generic.List[string]
$ExpectedAutoUpdateFields = @(
    "enabled",
    "plugin",
    "endpoint",
    "publicOrSigningKey",
    "download",
    "apply",
    "rollback"
)
$ExpectedAutomaticDependencyActionFields = @(
    "merge",
    "approve",
    "applyLockfileToDefaultBranch",
    "release"
)
$ExpectedAdvisoryExceptionFields = @(
    "exceptionId",
    "ecosystem",
    "advisoryId",
    "package",
    "affectedVersions",
    "severity",
    "dependencyClass",
    "owner",
    "reason",
    "impactScope",
    "distributionImpact",
    "compensatingControls",
    "remediationPlan",
    "trackingIssue",
    "createdAt",
    "approvedAt",
    "approvedBy",
    "expiresAt"
)
$ExpectedKnownAdvisoryAssessmentFields = @(
    "source",
    "reportedCount",
    "reportedSeverity",
    "status",
    "package",
    "advisoryId",
    "dependencyPath",
    "dependencyClass",
    "distributionImpact",
    "impactAssessment",
    "remediationVersion",
    "breakingChangeAssessment",
    "resolutionEvidence",
    "exceptionId",
    "blockerReason"
)
$LicenseMetrics = [ordered]@{
    Scanned = 0
    Unknown = 0
    OutsideAllowlist = 0
    Denied = 0
    UnselectedMultiLicense = 0
    ScopeViolation = 0
}

function Add-Failure {
    param([string]$Message)
    $Failures.Add($Message)
}

function Add-ToolPinBlocker {
    param([string]$Message)
    $ToolPinBlockers.Add($Message)
}

function Add-AdvisoryAssessmentBlocker {
    param([string]$Message)
    $AdvisoryAssessmentBlockers.Add($Message)
}

function Assert-True {
    param(
        [bool]$Condition,
        [string]$Message
    )
    if (-not $Condition) {
        Add-Failure $Message
    }
}

function Test-DuplicateValues {
    param(
        [object[]]$Values,
        [string]$Label
    )
    $duplicates = @($Values | Group-Object | Where-Object { $_.Count -gt 1 })
    foreach ($duplicate in $duplicates) {
        Add-Failure "$Label has duplicate value '$($duplicate.Name)'."
    }
}

function ConvertTo-DateOnly {
    param(
        [string]$Value,
        [string]$Label
    )
    try {
        return [DateTime]::ParseExact(
            $Value,
            "yyyy-MM-dd",
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::AssumeUniversal
        ).Date
    }
    catch {
        Add-Failure "$Label must use yyyy-MM-dd: '$Value'."
        return $null
    }
}

function Get-RequiredProperty {
    param(
        [object]$Object,
        [string]$Name,
        [string]$Label
    )
    if (-not ($Object.PSObject.Properties.Name -contains $Name)) {
        Add-Failure "$Label is missing required field '$Name'."
        return $null
    }
    $value = $Object.$Name
    if ($null -eq $value -or ($value -is [string] -and [string]::IsNullOrWhiteSpace($value))) {
        Add-Failure "$Label field '$Name' must not be empty."
    }
    return $value
}

function Get-LicenseRule {
    param([string]$Id)
    if ($AllowedById.ContainsKey($Id)) {
        return $AllowedById[$Id]
    }
    return $null
}

function Get-SpdxTokens {
    param([string]$Expression)
    $matches = [regex]::Matches(
        $Expression,
        '\(|\)|\bAND\b|\bOR\b|\bWITH\b|[A-Za-z0-9][A-Za-z0-9\.\-\+]*'
    )
    return @($matches | ForEach-Object { $_.Value })
}

function Test-SpdxPrimary {
    param(
        [string[]]$Tokens,
        [ref]$Index,
        [hashtable]$Selected
    )
    if ($Index.Value -ge $Tokens.Count) {
        throw "Unexpected end of SPDX expression."
    }
    $token = $Tokens[$Index.Value]
    if ($token -eq "(") {
        $Index.Value++
        $value = Test-SpdxOr -Tokens $Tokens -Index $Index -Selected $Selected
        if ($Index.Value -ge $Tokens.Count -or $Tokens[$Index.Value] -ne ")") {
            throw "Missing closing parenthesis in SPDX expression."
        }
        $Index.Value++
        return [bool]$value
    }
    if (@("AND", "OR", "WITH", ")") -contains $token) {
        throw "Unexpected token '$token' in SPDX expression."
    }

    $atom = $token
    $Index.Value++
    if ($Index.Value -lt $Tokens.Count -and $Tokens[$Index.Value] -eq "WITH") {
        $Index.Value++
        if ($Index.Value -ge $Tokens.Count) {
            throw "Missing SPDX exception after WITH."
        }
        $atom = "$atom WITH $($Tokens[$Index.Value])"
        $Index.Value++
    }
    return $Selected.ContainsKey($atom)
}

function Test-SpdxAnd {
    param(
        [string[]]$Tokens,
        [ref]$Index,
        [hashtable]$Selected
    )
    $value = Test-SpdxPrimary -Tokens $Tokens -Index $Index -Selected $Selected
    while ($Index.Value -lt $Tokens.Count -and $Tokens[$Index.Value] -eq "AND") {
        $Index.Value++
        $right = Test-SpdxPrimary -Tokens $Tokens -Index $Index -Selected $Selected
        $value = [bool]($value -and $right)
    }
    return [bool]$value
}

function Test-SpdxOr {
    param(
        [string[]]$Tokens,
        [ref]$Index,
        [hashtable]$Selected
    )
    $value = Test-SpdxAnd -Tokens $Tokens -Index $Index -Selected $Selected
    while ($Index.Value -lt $Tokens.Count -and $Tokens[$Index.Value] -eq "OR") {
        $Index.Value++
        $right = Test-SpdxAnd -Tokens $Tokens -Index $Index -Selected $Selected
        $value = [bool]($value -or $right)
    }
    return [bool]$value
}

function Test-SpdxSelectionSatisfies {
    param(
        [string]$Expression,
        [object[]]$SelectedValues
    )
    $tokens = @(Get-SpdxTokens $Expression)
    $compactExpression = $Expression -replace '\s+', ''
    $compactTokens = $tokens -join ''
    if ($tokens.Count -eq 0 -or $compactTokens -ne $compactExpression) {
        Add-Failure "Cannot parse recorded SPDX expression '$Expression'."
        return
    }

    $selected = @{}
    foreach ($value in $SelectedValues) {
        $selected[[string]$value] = $true
        $escaped = [regex]::Escape([string]$value)
        if ($Expression -notmatch "(?<![A-Za-z0-9\.\-\+])$escaped(?![A-Za-z0-9\.\-\+])") {
            Add-Failure "Selection '$value' is not an option in SPDX expression '$Expression'."
        }
    }

    try {
        $index = 0
        $satisfied = Test-SpdxOr -Tokens $tokens -Index ([ref]$index) -Selected $selected
        if ($index -ne $tokens.Count) {
            Add-Failure "SPDX expression '$Expression' has unconsumed tokens."
            return
        }
        if (-not $satisfied) {
            Add-Failure "Recorded selections do not satisfy SPDX expression '$Expression'."
        }
    }
    catch {
        Add-Failure "Cannot evaluate SPDX expression '$Expression': $($_.Exception.Message)"
    }
}

function Test-SelectedLicense {
    param(
        [string]$Selected,
        [string]$Scope,
        [string]$PackageLabel
    )

    if ($DeniedLicenses -contains $Selected) {
        $LicenseMetrics.Denied++
        Add-Failure "$PackageLabel selects denied license '$Selected'."
        return
    }

    $withMatch = [regex]::Match($Selected, "^(.+) WITH (.+)$")
    if ($withMatch.Success) {
        $baseId = $withMatch.Groups[1].Value
        $exceptionId = $withMatch.Groups[2].Value
        $baseRule = Get-LicenseRule $baseId
        if ($null -eq $baseRule -or -not ($AllowedSpdxExceptions -contains $exceptionId)) {
            $LicenseMetrics.OutsideAllowlist++
            Add-Failure "$PackageLabel selects unapproved SPDX expression '$Selected'."
            return
        }
        if ($baseRule.scope -ne "all" -and $baseRule.scope -ne $Scope) {
            $LicenseMetrics.ScopeViolation++
            Add-Failure "$PackageLabel uses '$Selected' outside allowed scope '$($baseRule.scope)'."
        }
        return
    }

    $rule = Get-LicenseRule $Selected
    if ($null -eq $rule) {
        $LicenseMetrics.OutsideAllowlist++
        Add-Failure "$PackageLabel selects license '$Selected' outside the allowlist."
        return
    }
    if ($rule.scope -ne "all" -and $rule.scope -ne $Scope) {
        $LicenseMetrics.ScopeViolation++
        Add-Failure "$PackageLabel uses '$Selected' in scope '$Scope'; allowed scope is '$($rule.scope)'."
    }
}

function Normalize-LicenseExpression {
    param([string]$Expression)
    if ($LegacyAliases.ContainsKey($Expression)) {
        return [string]$LegacyAliases[$Expression]
    }
    return $Expression
}

function Test-LicenseExpression {
    param(
        [string]$Expression,
        [string]$Scope,
        [string]$PackageLabel
    )

    $LicenseMetrics.Scanned++
    if ([string]::IsNullOrWhiteSpace($Expression) -or
        $Expression -eq "<MISSING>" -or
        $Expression -eq "UNKNOWN" -or
        $Expression -eq "UNLICENSED" -or
        $Expression.StartsWith("SEE LICENSE IN", [StringComparison]::Ordinal)) {
        $LicenseMetrics.Unknown++
        Add-Failure "$PackageLabel has missing or unknown license '$Expression'."
        return
    }

    $normalized = Normalize-LicenseExpression $Expression
    if ($DeniedLicenses -contains $normalized) {
        $LicenseMetrics.Denied++
        Add-Failure "$PackageLabel uses denied license '$normalized'."
        return
    }

    $singleRule = Get-LicenseRule $normalized
    if ($null -ne $singleRule) {
        Test-SelectedLicense -Selected $normalized -Scope $Scope -PackageLabel $PackageLabel
        return
    }

    if ($SelectionByExpression.ContainsKey($normalized)) {
        $decision = $SelectionByExpression[$normalized]
        foreach ($selected in @($decision.selected)) {
            Test-SelectedLicense -Selected ([string]$selected) -Scope $Scope -PackageLabel $PackageLabel
        }
        return
    }

    if ($normalized -match "\s(OR|AND|WITH)\s") {
        $LicenseMetrics.UnselectedMultiLicense++
        Add-Failure "$PackageLabel has multi-license expression without a recorded selection: '$normalized'."
    }
    else {
        $LicenseMetrics.OutsideAllowlist++
        Add-Failure "$PackageLabel uses license '$normalized' outside the allowlist."
    }
}

function Test-PolicyShape {
    param([object]$Policy)

    Assert-True ($Policy.schemaVersion -eq 1) "schemaVersion must be 1."
    Assert-True ($Policy.policyId -eq "ORIGAMI3-SUPPLY-CHAIN-10A") "Unexpected policyId."
    $null = ConvertTo-DateOnly ([string]$Policy.approvedAt) "approvedAt"

    $approved = @($Policy.scope.approvedCapabilities)
    $expectedApproved = @(
        "cargo-and-npm-vulnerability-monitoring",
        "dependency-update-proposals-with-human-review",
        "code-static-analysis",
        "four-artifact-sbom-and-sha256-publication",
        "license-allowlist"
    )
    Assert-True ($approved.Count -eq 5) "Exactly five approved capabilities are required."
    foreach ($capability in $expectedApproved) {
        Assert-True ($approved -contains $capability) "Approved capability '$capability' is missing."
    }
    $excluded = @($Policy.scope.excludedCapabilities)
    $expectedExcluded = @(
        "application-auto-update",
        "updater-plugin",
        "update-endpoint",
        "update-public-or-signing-key",
        "update-download-or-apply",
        "update-rollback"
    )
    Assert-True ($excluded.Count -eq $expectedExcluded.Count) "Exactly six excluded automatic-update capabilities are required."
    foreach ($capability in $expectedExcluded) {
        Assert-True ($excluded -contains $capability) "Excluded capability '$capability' is missing."
    }

    $autoUpdateProperties = @($Policy.scope.applicationAutoUpdate.PSObject.Properties.Name)
    Assert-True ($autoUpdateProperties.Count -eq $ExpectedAutoUpdateFields.Count) "applicationAutoUpdate must contain exactly the required false fields."
    foreach ($field in $ExpectedAutoUpdateFields) {
        Assert-True ($autoUpdateProperties -contains $field) "applicationAutoUpdate.$field is required."
        if ($autoUpdateProperties -contains $field) {
            Assert-True ($Policy.scope.applicationAutoUpdate.$field -eq $false) "applicationAutoUpdate.$field must be false."
        }
    }
    $releaseArtifacts = @($Policy.scope.releaseArtifacts)
    Assert-True ($releaseArtifacts.Count -eq 4) "Exactly four release artifacts are required."
    Test-DuplicateValues $releaseArtifacts "releaseArtifacts"
    foreach ($artifact in @("setup.exe", "x64.msi", "portable.exe", "manual.pdf")) {
        Assert-True ($releaseArtifacts -contains $artifact) "Release artifact '$artifact' is missing."
    }

    $ecosystems = @($Policy.advisories.ecosystems)
    Assert-True ($ecosystems.Count -eq 2 -and $ecosystems -contains "cargo" -and $ecosystems -contains "npm") "Advisories must cover Cargo and npm."
    Assert-True ($Policy.advisories.production.failAtOrAbove -eq "high") "Production advisories must fail at high."
    Assert-True ($Policy.advisories.developmentBuild.failAtOrAbove -eq "high") "Development/build advisories must fail at high."
    Assert-True ($Policy.advisories.production.unscoredVulnerability -eq "fail-as-high") "Unscored production vulnerabilities must fail as high."
    Assert-True ($Policy.advisories.developmentBuild.unscoredVulnerability -eq "fail-as-high") "Unscored development/build vulnerabilities must fail as high."
    Assert-True ($Policy.advisories.triggers.pullRequest -eq $true) "Pull-request advisory monitoring is required."
    Assert-True ($Policy.advisories.triggers.push -eq $true) "Push advisory monitoring is required."
    Assert-True ($Policy.advisories.triggers.schedule.maximumDaysBetweenRuns -le 7) "Scheduled advisory monitoring must run at least weekly."
    Assert-True ($Policy.advisories.triggers.consecutiveScheduledWeeksAllowedToMiss -eq 0) "Scheduled monitoring may not miss a week."
    Assert-True ($Policy.advisories.triggers.failureNotificationRequired -eq $true) "Failure notification is required."

    Assert-True ($Policy.advisoryExceptionPolicy.maximumDays -eq 90) "The global advisory exception maximum must be 90 days."
    Assert-True ($Policy.advisoryExceptionPolicy.wildcardAdvisoryIdAllowed -eq $false) "Wildcard advisory IDs must be forbidden."
    Assert-True ($Policy.advisoryExceptionPolicy.expiredExceptionAllowed -eq $false) "Expired exceptions must be forbidden."
    Assert-True ($Policy.advisoryExceptionPolicy.expiryOnlyRenewalAllowed -eq $false) "Expiry-only renewal must be forbidden."
    $requiredExceptionFields = @($Policy.advisoryExceptionPolicy.requiredFields)
    Assert-True ($requiredExceptionFields.Count -eq $ExpectedAdvisoryExceptionFields.Count) "advisoryExceptionPolicy.requiredFields must contain the fixed required field set."
    foreach ($field in $ExpectedAdvisoryExceptionFields) {
        Assert-True ($requiredExceptionFields -contains $field) "advisoryExceptionPolicy.requiredFields is missing '$field'."
    }

    Assert-True ($Policy.dependencyUpdates.provider -eq "dependabot") "Dependabot must be the only update proposal provider."
    Assert-True ($Policy.dependencyUpdates.renovateEnabled -eq $false) "Renovate must be disabled."
    Assert-True ($Policy.dependencyUpdates.humanReviewRequired -eq $true) "Human review must be required."
    $automaticActionProperties = @($Policy.dependencyUpdates.automaticActions.PSObject.Properties.Name)
    Assert-True ($automaticActionProperties.Count -eq $ExpectedAutomaticDependencyActionFields.Count) "automaticActions must contain exactly the four prohibited actions."
    foreach ($field in $ExpectedAutomaticDependencyActionFields) {
        Assert-True ($automaticActionProperties -contains $field) "dependencyUpdates.automaticActions.$field is required."
        if ($automaticActionProperties -contains $field) {
            Assert-True ($Policy.dependencyUpdates.automaticActions.$field -eq $false) "dependencyUpdates.automaticActions.$field must be false."
        }
    }
    $updateEcosystems = @($Policy.dependencyUpdates.ecosystems)
    $updateNames = @($updateEcosystems | ForEach-Object { $_.name })
    Assert-True ($updateEcosystems.Count -eq 3) "Exactly three dependency update ecosystems are required."
    foreach ($name in @("cargo", "npm", "github-actions")) {
        Assert-True ($updateNames -contains $name) "Dependency update ecosystem '$name' is missing."
    }
    Test-DuplicateValues $updateNames "dependencyUpdates.ecosystems.name"
    foreach ($ecosystem in $updateEcosystems) {
        Assert-True ($ecosystem.interval -eq "weekly") "Dependency updates for '$($ecosystem.name)' must be weekly."
        Assert-True ($ecosystem.openPullRequestsLimit -eq 3) "Dependency updates for '$($ecosystem.name)' must have PR limit 3."
    }

    Assert-True ($Policy.staticAnalysis.javascriptTypescript.versionLine -eq "v3") "CodeQL must use the v3 version line."
    Assert-True ($Policy.staticAnalysis.javascriptTypescript.querySuite -eq "security-extended") "CodeQL must use security-extended."
    Assert-True ($Policy.staticAnalysis.javascriptTypescript.commitShaRequiredBeforeWorkflowEnablement -eq $true) "CodeQL must require a full commit SHA before enablement."
    Assert-True ($Policy.staticAnalysis.javascriptTypescript.enableWithoutCommitSha -eq $false) "CodeQL must not run without an immutable commit SHA."
    Assert-True ($Policy.staticAnalysis.rust.existingGateMustRemain -eq $true) "The existing Rust clippy gate must remain."

    Assert-True ([string]$Policy.toolPins.cargoAudit.version -match "^\d+\.\d+\.\d+$") "cargo-audit must have an exact semantic version."
    Assert-True ([string]$Policy.toolPins.cargoAudit.crateSha256 -match "^[0-9a-f]{64}$") "cargo-audit must have a 64-character lowercase SHA-256."
    Assert-True ([string]$Policy.toolPins.npmAudit.version -match "^\d+\.\d+\.\d+$") "npm must have an exact semantic version."
    Assert-True ([string]$Policy.toolPins.npmAudit.nodeVersionLine -match "^22\.\d+\.\d+$") "npm audit Node versionLine must be an exact Node 22 semantic version."
    Assert-True ([string]$Policy.toolPins.npmAudit.nodeVersion -match "^22\.\d+\.\d+$") "npm audit Node version must be an exact Node 22 semantic version."
    Assert-True ([string]$Policy.toolPins.npmAudit.nodeVersionLine -eq [string]$Policy.toolPins.npmAudit.nodeVersion) "npm audit Node versionLine and exact version must match."
    Assert-True ($Policy.toolPins.codeql.workflowReferenceRule -eq "full-40-character-commit-sha") "CodeQL workflow references must use a full commit SHA."
    Assert-True ($Policy.toolPins.codeql.floatingMainMasterLatestAllowed -eq $false) "Floating main/master/latest references must be forbidden."
    Assert-True ($Policy.toolPins.sbom.format -eq "CycloneDX-1.6-JSON") "SBOM format must be CycloneDX 1.6 JSON."
    Assert-True ($Policy.toolPins.sbom.enableWithoutVerifiedPin -eq $false) "SBOM generation must not run without a verified immutable pin."
    Assert-True ($Policy.toolPins.sbom.resolutionStage -eq "10-A") "SBOM tool selection and pin resolution belong to 10-A."
    Assert-True ($Policy.staticAnalysis.javascriptTypescript.commitShaResolutionStage -eq "10-A") "CodeQL pin resolution belongs to 10-A."
    Assert-True ($Policy.toolPins.codeql.resolutionStage -eq "10-A") "CodeQL tool pin resolution belongs to 10-A."
    Assert-True ($Policy.artifactPublication.sbom.requiredCount -eq 4) "Four SBOMs are required."
    Assert-True ($Policy.artifactPublication.hash.requiredSidecarCount -eq 4) "Four SHA-256 sidecars are required."
    Assert-True ($Policy.artifactPublication.hash.algorithm -eq "SHA-256") "Artifact hashes must use SHA-256."
    $identityFields = @($Policy.artifactPublication.identityFields)
    Assert-True ($identityFields.Count -eq 3) "Artifact publication must use exactly three identity fields."
    foreach ($field in @("artifactName", "version", "buildId")) {
        Assert-True ($identityFields -contains $field) "Artifact publication identity field '$field' is missing."
    }
}

function ConvertFrom-YamlScalar {
    param([string]$Value)

    $trimmed = $Value.Trim()
    if ($trimmed.Length -ge 2 -and (
        ($trimmed.StartsWith('"') -and $trimmed.EndsWith('"')) -or
        ($trimmed.StartsWith("'") -and $trimmed.EndsWith("'"))
    )) {
        return $trimmed.Substring(1, $trimmed.Length - 2)
    }
    return $trimmed
}

function Get-YamlScalarValues {
    param(
        [string]$Text,
        [string]$Key
    )

    $escapedKey = [regex]::Escape($Key)
    $matches = @([regex]::Matches(
        $Text,
        "(?m)^\s+$escapedKey\s*:\s*(?<value>[^\r\n#]+?)(?:\s+#.*)?\s*$"
    ))
    return @($matches | ForEach-Object { ConvertFrom-YamlScalar $_.Groups["value"].Value })
}

function Assert-YamlScalarEquals {
    param(
        [string]$Text,
        [string]$Key,
        [string]$Expected,
        [string]$Label
    )

    $values = @(Get-YamlScalarValues -Text $Text -Key $Key)
    Assert-True ($values.Count -eq 1) "$Label must contain exactly one '$Key' value."
    if ($values.Count -eq 1) {
        Assert-True ($values[0] -eq $Expected) "$Label '$Key' must be '$Expected', not '$($values[0])'."
    }
}

function Test-DependabotConfiguration {
    param(
        [object]$Policy,
        [string]$Path
    )

    $text = Get-Content -LiteralPath $Path -Raw -Encoding UTF8
    Assert-True ($text -match '(?m)^version:[ \t]*2[ \t]*$') "dependabot.yml must declare version 2."
    Assert-True ($text -notmatch '(?m)^\t') "dependabot.yml may not use tab indentation."
    Assert-True ($text -notmatch '(?im)^\s*(auto-merge|auto_merge|automatic-merge)\s*:') "dependabot.yml may not enable automatic merging."

    $updateBlocks = @([regex]::Matches(
        $text,
        '(?ms)^  - package-ecosystem:[ \t]*(?<ecosystem>[^\r\n#]+?)[ \t]*(?:#.*)?\r?$\r?\n(?<body>.*?)(?=^  - package-ecosystem:|\z)'
    ))
    Assert-True ($updateBlocks.Count -eq 3) "dependabot.yml must define exactly three update ecosystems."

    $expectedByEcosystem = @{}
    foreach ($ecosystem in @($Policy.dependencyUpdates.ecosystems)) {
        $expectedByEcosystem[[string]$ecosystem.name] = $ecosystem
    }
    $actualEcosystems = @()
    foreach ($block in $updateBlocks) {
        $ecosystemName = ConvertFrom-YamlScalar $block.Groups["ecosystem"].Value
        $actualEcosystems += $ecosystemName
        Assert-True ($expectedByEcosystem.ContainsKey($ecosystemName)) "dependabot.yml has unapproved ecosystem '$ecosystemName'."
        if (-not $expectedByEcosystem.ContainsKey($ecosystemName)) {
            continue
        }

        $expected = $expectedByEcosystem[$ecosystemName]
        $body = $block.Groups["body"].Value
        $label = "dependabot '$ecosystemName'"
        Assert-YamlScalarEquals -Text $body -Key "directory" -Expected ([string]$expected.directory) -Label $label
        Assert-YamlScalarEquals -Text $body -Key "interval" -Expected ([string]$expected.interval) -Label $label
        Assert-YamlScalarEquals -Text $body -Key "day" -Expected ([string]$expected.day) -Label $label
        Assert-YamlScalarEquals -Text $body -Key "time" -Expected ([string]$expected.time) -Label $label
        Assert-YamlScalarEquals -Text $body -Key "timezone" -Expected ([string]$expected.timezone) -Label $label
        Assert-YamlScalarEquals -Text $body -Key "open-pull-requests-limit" -Expected ([string]$expected.openPullRequestsLimit) -Label $label

        $groupName = "$ecosystemName-minor-and-patch"
        $escapedGroupName = [regex]::Escape($groupName)
        $groupMatch = [regex]::Match(
            $body,
            "(?ms)^      ${escapedGroupName}:[ \t]*\r?$\r?\n(?<groupBody>.*?)(?=^      [^ \t]|\z)"
        )
        Assert-True ($groupMatch.Success) "$label must group minor and patch updates as '$groupName'."
        if ($groupMatch.Success) {
            $groupBody = $groupMatch.Groups["groupBody"].Value
            Assert-YamlScalarEquals -Text $groupBody -Key "applies-to" -Expected "version-updates" -Label "$label group '$groupName'"
            $updateTypes = @([regex]::Matches(
                $groupBody,
                '(?m)^          - (?<value>[^\r\n#]+?)[ \t]*(?:#.*)?$'
            ) | ForEach-Object { ConvertFrom-YamlScalar $_.Groups["value"].Value })
            Assert-True ($updateTypes.Count -eq 2) "$label group '$groupName' must list exactly minor and patch update types."
            Assert-True ($updateTypes -contains "minor") "$label group '$groupName' must include minor updates."
            Assert-True ($updateTypes -contains "patch") "$label group '$groupName' must include patch updates."
        }
    }
    Test-DuplicateValues $actualEcosystems "dependabot.yml package-ecosystem"
    foreach ($required in @("cargo", "npm", "github-actions")) {
        Assert-True ($actualEcosystems -contains $required) "dependabot.yml is missing '$required'."
    }
}

function Test-SecurityWorkflowConfiguration {
    param(
        [object]$Policy,
        [string]$Path
    )

    $text = Get-Content -LiteralPath $Path -Raw -Encoding UTF8
    Assert-True ($text -match '(?m)^name:[ \t]*Security[ \t]*$') "security.yml must be named Security."
    Assert-True ($text -match '(?m)^on:[ \t]*$') "security.yml must declare workflow triggers."
    Assert-True ($text -match '(?m)^  pull_request:[ \t]*$') "security.yml must run for pull requests."
    Assert-True ($text -match '(?m)^  workflow_dispatch:[ \t]*$') "security.yml must allow manual execution."
    Assert-True ($text -match '(?ms)^  push:[ \t]*\r?\n[ \t]*branches:[ \t]*\r?\n[ \t]*- main[ \t]*(?:\r?\n|$)') "security.yml must run for pushes to main."

    $schedule = $Policy.advisories.triggers.schedule
    $dayToCron = @{ sunday = "0"; monday = "1"; tuesday = "2"; wednesday = "3"; thursday = "4"; friday = "5"; saturday = "6" }
    $day = ([string]$schedule.day).ToLowerInvariant()
    Assert-True ($dayToCron.ContainsKey($day)) "security-policy.json has unsupported scheduled day '$($schedule.day)'."
    $timeParts = ([string]$schedule.time).Split(":")
    Assert-True ($timeParts.Count -eq 2 -and $timeParts[0] -match '^\d{2}$' -and $timeParts[1] -match '^\d{2}$') "security-policy.json schedule time must use HH:mm."
    if ($dayToCron.ContainsKey($day) -and $timeParts.Count -eq 2 -and $timeParts[0] -match '^\d{2}$' -and $timeParts[1] -match '^\d{2}$') {
        $expectedCron = "$([int]$timeParts[1]) $([int]$timeParts[0]) * * $($dayToCron[$day])"
        $escapedCron = [regex]::Escape($expectedCron)
        $cronPattern = '(?m)^    - cron:[ \t]*"' + $escapedCron + '"[ \t]*$'
        Assert-True ($text -match $cronPattern) "security.yml weekly schedule must match security-policy.json ('$expectedCron' UTC)."
    }
    Assert-True ([string]$schedule.timezone -eq "UTC") "security.yml only accepts the policy schedule in UTC."

    foreach ($job in @("cargo_and_licenses", "npm_advisories", "codeql_blocker", "distribution_blocker", "security_status")) {
        Assert-True ($text -match "(?m)^  ${job}:[ \t]*$") "security.yml is missing job '$job'."
    }
    Assert-True ($text -match '(?ms)^permissions:[ \t]*\r?\n[ \t]*contents:[ \t]*read[ \t]*(?:\r?\n|$)') "security.yml must keep read-only contents permission."
    $supplyChainInvocationLines = @($text -split '\r?\n' | Where-Object { $_ -match 'scripts/check-supply-chain\.ps1' })
    $policyAndLicenseCallCount = @($supplyChainInvocationLines | Where-Object { $_ -match '-Mode[ \t]+PolicyAndLicenses(?:[ \t]|$)' }).Count
    $readinessOnlyCallCount = @($supplyChainInvocationLines | Where-Object { $_ -match '-Mode[ \t]+ReadinessOnly(?:[ \t]|$)' }).Count
    $readinessStepPattern = '(?ms)^      - name:[ \t]*Report unresolved supply-chain readiness blockers[ \t]*\r?\n        if:[ \t]*\$\{\{ always\(\) \}\}[ \t]*\r?\n(?:        [^\r\n]*\r?\n)*?          [^\r\n]*scripts/check-supply-chain\.ps1 -Mode ReadinessOnly[ \t]*`[ \t]*\r?\n[ \t]*-CargoAuditReceiptPath[ \t]+\$receiptPath[ \t]*$'
    Assert-True ($supplyChainInvocationLines.Count -eq 2 -and $policyAndLicenseCallCount -eq 1 -and $readinessOnlyCallCount -eq 1 -and $text -match $readinessStepPattern) "security.yml must run PolicyAndLicenses once before Cargo advisory exceptions and ReadinessOnly once in the always() readiness step."
    Assert-True ($text -match 'security-policy\.json') "security.yml must read security-policy.json instead of duplicating policy decisions."
    Assert-True ($text -match '&[ \t]+\$cargoAuditExe[ \t]+--version') "security.yml must verify the installed run-scoped Cargo advisory executable before use."
    Assert-True ($text -match 'npm audit') "security.yml must audit npm lockfiles."
    Assert-True ($text -match 'security_status:[\s\S]*if: \$\{\{ always\(\) \}\}') "security.yml must summarize failures even when a preceding job fails."
    Assert-True ($text -match 'One or more security jobs did not succeed') "security.yml must fail the summary job when an audit job is unsuccessful."
    Assert-True ($text -notmatch '(?im)^\s*uses:\s*[^\r\n@]+@(main|master|latest)\b') "security.yml may not use floating action references."

    $workflowNodeVersions = @(Get-YamlScalarValues -Text $text -Key "node-version")
    $matchingWorkflowNodeVersions = @($workflowNodeVersions | Where-Object { $_ -eq [string]$Policy.toolPins.npmAudit.nodeVersion })
    Assert-True ($workflowNodeVersions.Count -eq 2 -and $matchingWorkflowNodeVersions.Count -eq 2) "security.yml must pin the policy Node version exactly once in each Node-dependent security job."
    $cargoRegistryProtocols = @(Get-YamlScalarValues -Text $text -Key "CARGO_REGISTRIES_CRATES_IO_PROTOCOL")
    Assert-True ($cargoRegistryProtocols.Count -eq 1 -and $cargoRegistryProtocols[0] -eq "sparse") "security.yml must fetch crates.io through the sparse protocol used by cargo-audit pin evidence."
    $cargoAuditInstallIndex = $text.IndexOf("      - name: Install and verify the pinned cargo-audit", [StringComparison]::Ordinal)
    $readinessIndex = $text.IndexOf("      - name: Report unresolved supply-chain readiness blockers", [StringComparison]::Ordinal)
    Assert-True ($cargoAuditInstallIndex -ge 0 -and $readinessIndex -gt $cargoAuditInstallIndex) "security.yml must run ReadinessOnly after the pinned cargo-audit fetch/install step."
    $receiptNamePattern = 'cargo-audit-pin-evidence-\$env:GITHUB_RUN_ID-\$env:GITHUB_RUN_ATTEMPT\.json'
    Assert-True ([regex]::Matches($text, $receiptNamePattern).Count -eq 2) "security.yml must generate and consume one cargo-audit receipt path bound to the current run and attempt."
    Assert-True ($text -match 'CARGO_HOME=\$env:RUNNER_TEMP\\ori3-cargo-home-\$env:GITHUB_RUN_ID-\$env:GITHUB_RUN_ATTEMPT') "security.yml must isolate Cargo registry/cache state inside the current runner temp directory."
    Assert-True ([regex]::Matches($text, 'cargo-audit-install-\$env:GITHUB_RUN_ID-\$env:GITHUB_RUN_ATTEMPT').Count -eq 1) "security.yml must install cargo-audit into one run-scoped runner-temp root."
    Assert-True ($text -match '(?m)^[ \t]*--root[ \t]+\$cargoAuditInstallRoot[ \t]+--force[ \t]*$') "security.yml must force a same-run cargo-audit installation into the run-scoped root."
    Assert-True ($text -match '(?m)^[ \t]*cargo install cargo-audit --version \$env:CARGO_AUDIT_VERSION --locked[ \t]*`[ \t]*$') "security.yml must allow the locked cargo-audit install to fetch its exact fresh-cache dependencies after archive verification."
    Assert-True ($text -notmatch '(?m)^[ \t]*cargo install cargo-audit[^\r\n]*--offline') "security.yml must not assume bootstrap dependency resolution populated every cargo-audit lockfile dependency in a fresh cache."
    Assert-True ([regex]::Matches($text, 'CARGO_AUDIT_EXE').Count -eq 5) "security.yml must publish and consume only the run-scoped cargo-audit executable path."
    Assert-True ($text -match [regex]::Escape('https://index.crates.io/ca/rg/cargo-audit')) "security.yml must obtain same-run cargo-audit registry evidence from the crates.io sparse endpoint."
    foreach ($receiptField in @("schemaVersion", "githubRunId", "githubRunAttempt", "githubSha", "reportedVersionOutput", "registryChecksum", "registryYanked", "archiveSha256", "executableSha256")) {
        Assert-True ($text -match ("(?m)^            " + [regex]::Escape($receiptField) + "[ \t]*=")) "security.yml cargo-audit receipt must write '$receiptField'."
    }

    $workflowCargoAuditVersions = @(Get-YamlScalarValues -Text $text -Key "CARGO_AUDIT_VERSION")
    Assert-True ($workflowCargoAuditVersions.Count -eq 1 -and $workflowCargoAuditVersions[0] -eq [string]$Policy.toolPins.cargoAudit.version) "security.yml Cargo advisory version must match security-policy.json."
    $workflowCargoAuditHashes = @(Get-YamlScalarValues -Text $text -Key "CARGO_AUDIT_CRATE_SHA256")
    Assert-True ($workflowCargoAuditHashes.Count -eq 1 -and $workflowCargoAuditHashes[0] -eq [string]$Policy.toolPins.cargoAudit.crateSha256) "security.yml Cargo advisory checksum must match security-policy.json."
}

function Test-Exceptions {
    param([object]$Policy)

    $requiredFields = @($Policy.advisoryExceptionPolicy.requiredFields)
    Test-DuplicateValues $requiredFields "advisoryExceptionPolicy.requiredFields"
    $today = (Get-Date).ToUniversalTime().Date
    $exceptionIds = @()
    $exceptionKeys = @()

    foreach ($exception in @($Policy.advisoryExceptions)) {
        $label = "advisory exception"
        if ($exception.PSObject.Properties.Name -contains "exceptionId") {
            $label = "advisory exception '$($exception.exceptionId)'"
        }
        $missingRequiredField = $false
        foreach ($field in $requiredFields) {
            if (-not ($exception.PSObject.Properties.Name -contains [string]$field)) {
                $missingRequiredField = $true
            }
            $null = Get-RequiredProperty -Object $exception -Name ([string]$field) -Label $label
        }
        if ($missingRequiredField) {
            continue
        }

        $advisoryId = [string]$exception.advisoryId
        Assert-True ($advisoryId -notmatch "[\*\?]") "$label may not use a wildcard advisory ID."
        Assert-True ($advisoryId -match "^(RUSTSEC-\d{4}-\d{4}|GHSA-[0-9a-z]{4}-[0-9a-z]{4}-[0-9a-z]{4}|CVE-\d{4}-\d{4,})$") "$label has an unsupported advisory ID '$advisoryId'."

        $ecosystem = [string]$exception.ecosystem
        $dependencyClass = [string]$exception.dependencyClass
        $severity = ([string]$exception.severity).ToLowerInvariant()
        Assert-True (@("cargo", "npm") -contains $ecosystem) "$label has invalid ecosystem '$ecosystem'."
        Assert-True (@("production", "developmentBuild") -contains $dependencyClass) "$label has invalid dependencyClass '$dependencyClass'."
        Assert-True (@("critical", "high") -contains $severity) "$label may only cover critical or high severity."
        $exceptionIds += [string]$exception.exceptionId
        $exceptionKeys += "$ecosystem|$advisoryId|$([string]$exception.package)|$dependencyClass"

        $createdAt = ConvertTo-DateOnly ([string]$exception.createdAt) "$label.createdAt"
        $approvedAt = ConvertTo-DateOnly ([string]$exception.approvedAt) "$label.approvedAt"
        $expiresAt = ConvertTo-DateOnly ([string]$exception.expiresAt) "$label.expiresAt"
        if ($null -ne $createdAt -and $null -ne $approvedAt -and $null -ne $expiresAt) {
            Assert-True ($approvedAt -ge $createdAt) "$label approval may not predate creation."
            $duration = ($expiresAt - $approvedAt).Days
            Assert-True ($duration -gt 0) "$label expiry must be after approval."
            Assert-True ($duration -le 90) "$label exceeds the global 90-day maximum."
            if (@("production", "developmentBuild") -contains $dependencyClass -and @("critical", "high") -contains $severity) {
                $classPolicy = $Policy.advisoryExceptionPolicy.maximumDaysByClassAndSeverity.$dependencyClass
                $classMaximum = [int]$classPolicy.$severity
                Assert-True ($duration -le $classMaximum) "$label exceeds the $dependencyClass/$severity maximum of $classMaximum days."
            }
            Assert-True ($expiresAt -ge $today) "$label expired on $($expiresAt.ToString('yyyy-MM-dd'))."
        }
    }

    Test-DuplicateValues $exceptionIds "advisoryExceptions.exceptionId"
    Test-DuplicateValues $exceptionKeys "advisoryExceptions ecosystem/advisory/package/class key"
}

function Test-KnownAdvisoryAssessment {
    param([object]$Policy)

    $assessment = $Policy.knownAdvisoryAssessment
    $assessmentProperties = @($assessment.PSObject.Properties.Name)
    Assert-True ($assessmentProperties.Count -eq $ExpectedKnownAdvisoryAssessmentFields.Count) "knownAdvisoryAssessment must contain the fixed field set."
    foreach ($field in $ExpectedKnownAdvisoryAssessmentFields) {
        Assert-True ($assessmentProperties -contains $field) "knownAdvisoryAssessment is missing '$field'."
    }
    Assert-True ($assessment.reportedCount -eq 1) "The user-reported npm high count must remain recorded as 1 until reassessed."
    Assert-True ($assessment.reportedSeverity -eq "high") "The user-reported npm severity must remain recorded as high until reassessed."

    $status = [string]$assessment.status
    if ($status -eq "blocked-external-audit-endpoint") {
        $blockerReason = [string]$assessment.blockerReason
        if ([string]::IsNullOrWhiteSpace($blockerReason)) {
            $blockerReason = "no blocker reason was recorded"
        }
        Add-AdvisoryAssessmentBlocker "The reported npm high advisory is not assessed: $blockerReason"
        return
    }

    if ($status -eq "reassessed-no-current-high") {
        foreach ($field in @(
            "package",
            "advisoryId",
            "dependencyPath",
            "dependencyClass",
            "distributionImpact",
            "exceptionId",
            "blockerReason"
        )) {
            if ($null -ne $assessment.$field -and -not [string]::IsNullOrWhiteSpace([string]$assessment.$field)) {
                Add-AdvisoryAssessmentBlocker "A zero-current-high reassessment may not claim '$field'."
            }
        }
        foreach ($field in @(
            "source",
            "impactAssessment",
            "remediationVersion",
            "breakingChangeAssessment",
            "resolutionEvidence"
        )) {
            if ($null -eq $assessment.$field -or [string]::IsNullOrWhiteSpace([string]$assessment.$field)) {
                Add-AdvisoryAssessmentBlocker "A zero-current-high reassessment is missing '$field'."
            }
        }

        $evidence = [string]$assessment.resolutionEvidence
        $npmVersion = [regex]::Escape([string]$Policy.toolPins.npmAudit.version)
        if ($evidence -notmatch "(?:^|;\s*)npm=$npmVersion(?:;|$)") {
            Add-AdvisoryAssessmentBlocker "Zero-current-high evidence must name the pinned npm version."
        }
        $auditCommands = @{
            production = "npm.cmd audit --package-lock-only --audit-level=high --json --omit=dev"
            complete = "npm.cmd audit --package-lock-only --audit-level=high --json"
        }
        if ([regex]::Matches($evidence, '\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z\b').Count -lt 4) {
            Add-AdvisoryAssessmentBlocker "Zero-current-high evidence must record UTC start and end timestamps for both audits."
        }
        foreach ($scope in @("production", "complete")) {
            $scopeEvidence = [regex]::Match($evidence, "(?:^|;\s*)${scope} UTC=(?<value>[^;]+)")
            if (-not $scopeEvidence.Success -or
                $scopeEvidence.Groups["value"].Value.IndexOf("command=$($auditCommands[$scope])", [StringComparison]::Ordinal) -lt 0) {
                Add-AdvisoryAssessmentBlocker "Zero-current-high evidence is missing the exact $scope audit command."
            }
            if (-not $scopeEvidence.Success -or
                $scopeEvidence.Groups["value"].Value -notmatch "result: exit=0,[^;]*high=0,[^;]*critical=0,[^;]*total=0") {
                Add-AdvisoryAssessmentBlocker "Zero-current-high evidence must record exit=0 and zero high, critical, and total vulnerabilities for $scope audit."
            }
        }
        if ($evidence -notmatch 'package-lock\.json SHA-256=[0-9a-f]{64}') {
            Add-AdvisoryAssessmentBlocker "Zero-current-high evidence must identify the audited package-lock.json by SHA-256."
        }
        return
    }

    if (@("not-affected", "excepted") -notcontains $status) {
        Add-AdvisoryAssessmentBlocker "Known npm high assessment has unsupported status '$status'."
        return
    }

    $missing = $false
    foreach ($field in @(
        "package",
        "advisoryId",
        "dependencyPath",
        "dependencyClass",
        "distributionImpact",
        "impactAssessment",
        "remediationVersion",
        "breakingChangeAssessment",
        "resolutionEvidence"
    )) {
        if ($null -eq $assessment.$field -or [string]::IsNullOrWhiteSpace([string]$assessment.$field)) {
            $missing = $true
            Add-AdvisoryAssessmentBlocker "Completed npm high assessment is missing '$field'."
        }
    }
    if ($missing) {
        return
    }

    $advisoryId = [string]$assessment.advisoryId
    $dependencyClass = [string]$assessment.dependencyClass
    $distributionImpact = [string]$assessment.distributionImpact
    if ($advisoryId -notmatch "^(GHSA-[0-9a-z]{4}-[0-9a-z]{4}-[0-9a-z]{4}|CVE-\d{4}-\d{4,})$") {
        Add-AdvisoryAssessmentBlocker "Completed npm high assessment has unsupported advisory ID '$advisoryId'."
    }
    if (@("production", "developmentBuild") -notcontains $dependencyClass) {
        Add-AdvisoryAssessmentBlocker "Completed npm high assessment has invalid dependencyClass '$dependencyClass'."
    }
    if (@("included", "not-included") -notcontains $distributionImpact) {
        Add-AdvisoryAssessmentBlocker "Completed npm high assessment has invalid distributionImpact '$distributionImpact'."
    }

    if ($status -eq "not-affected") {
        if ($dependencyClass -ne "developmentBuild" -or $distributionImpact -ne "not-included") {
            Add-AdvisoryAssessmentBlocker "A not-affected high advisory must be development/build-only and absent from the distribution."
        }
        if (-not [string]::IsNullOrWhiteSpace([string]$assessment.exceptionId)) {
            Add-AdvisoryAssessmentBlocker "A not-affected assessment may not cite an advisory exception."
        }
        return
    }

    $exceptionId = [string]$assessment.exceptionId
    if ([string]::IsNullOrWhiteSpace($exceptionId)) {
        Add-AdvisoryAssessmentBlocker "An excepted high advisory must cite an active exceptionId."
        return
    }
    $matchingExceptions = @($Policy.advisoryExceptions | Where-Object {
        [string]$_.exceptionId -eq $exceptionId -and
        [string]$_.ecosystem -eq "npm" -and
        [string]$_.advisoryId -eq $advisoryId -and
        [string]$_.package -eq [string]$assessment.package -and
        [string]$_.dependencyClass -eq $dependencyClass -and
        ([string]$_.severity).ToLowerInvariant() -eq "high"
    })
    if ($matchingExceptions.Count -ne 1) {
        Add-AdvisoryAssessmentBlocker "Excepted npm high assessment must match exactly one active npm/high exception by ID, advisory, package, and class."
    }
}

function Test-ToolPinReadiness {
    param(
        [object]$Policy,
        [string]$CargoAuditReceiptPath
    )

    Test-CargoAuditPinEvidence -Policy $Policy -ReceiptPath $CargoAuditReceiptPath

    $nodeVersion = [string]$Policy.toolPins.npmAudit.nodeVersion
    $nodeVersionLine = [string]$Policy.toolPins.npmAudit.nodeVersionLine
    if ($nodeVersion -notmatch '^22\.\d+\.\d+$' -or $nodeVersionLine -ne $nodeVersion) {
        Add-ToolPinBlocker "Exact Node 22 version is not verified consistently by nodeVersionLine and nodeVersion."
    }

    $codeqlSha = [string]$Policy.toolPins.codeql.commitSha
    $codeqlEvidence = [string]$Policy.toolPins.codeql.verificationEvidence
    if ($codeqlSha -notmatch '^[0-9a-f]{40}$' -or $Policy.toolPins.codeql.commitShaVerified -ne $true -or [string]::IsNullOrWhiteSpace($codeqlEvidence)) {
        Add-ToolPinBlocker "github/codeql-action v3 full 40-character commit SHA is not verified."
    }
    else {
        Assert-True ([string]$Policy.staticAnalysis.javascriptTypescript.commitSha -eq $codeqlSha) "Static-analysis and tool-pin CodeQL SHAs must match."
        Assert-True ($Policy.staticAnalysis.javascriptTypescript.commitShaVerified -eq $true) "Static-analysis CodeQL SHA must be marked verified."
        Assert-True ([string]$Policy.staticAnalysis.javascriptTypescript.verificationEvidence -eq $codeqlEvidence) "Static-analysis and tool-pin CodeQL verification evidence must match."
    }

    $sbomTool = [string]$Policy.toolPins.sbom.tool
    $sbomVersion = [string]$Policy.toolPins.sbom.version
    $sbomSha = [string]$Policy.toolPins.sbom.sha256
    $sbomEvidence = [string]$Policy.toolPins.sbom.verificationEvidence
    if (
        [string]::IsNullOrWhiteSpace($sbomTool) -or
        $sbomVersion -notmatch '^\d+\.\d+\.\d+$' -or
        $sbomSha -notmatch '^[0-9a-f]{64}$' -or
        [string]::IsNullOrWhiteSpace($sbomEvidence) -or
        $Policy.toolPins.sbom.pinStatus -ne "verified"
    ) {
        Add-ToolPinBlocker "An existing SBOM tool exact version, SHA-256, and verification evidence are not pinned."
    }
}

function Initialize-LicensePolicy {
    param([object]$Policy)

    $script:AllowedById = @{}
    foreach ($rule in @($Policy.licensePolicy.allowed)) {
        $id = [string]$rule.id
        if ($AllowedById.ContainsKey($id)) {
            Add-Failure "licensePolicy.allowed has duplicate ID '$id'."
        }
        else {
            $AllowedById[$id] = $rule
        }
    }

    $script:DeniedLicenses = @($Policy.licensePolicy.denied | ForEach-Object { [string]$_ })
    Test-DuplicateValues $DeniedLicenses "licensePolicy.denied"
    foreach ($denied in $DeniedLicenses) {
        if ($AllowedById.ContainsKey($denied)) {
            Add-Failure "License '$denied' appears in both allow and deny lists."
        }
    }

    $script:AllowedSpdxExceptions = @($Policy.licensePolicy.allowedSpdxExceptions | ForEach-Object { [string]$_ })
    $script:LegacyAliases = @{}
    foreach ($alias in @($Policy.licensePolicy.legacyAliases)) {
        $from = [string]$alias.from
        if ($LegacyAliases.ContainsKey($from)) {
            Add-Failure "Duplicate legacy license alias '$from'."
        }
        else {
            $LegacyAliases[$from] = [string]$alias.to
        }
    }

    $script:SelectionByExpression = @{}
    foreach ($decision in @($Policy.licensePolicy.multiLicenseSelections)) {
        $expression = [string]$decision.expression
        if ($SelectionByExpression.ContainsKey($expression)) {
            Add-Failure "Duplicate multi-license selection '$expression'."
        }
        else {
            $SelectionByExpression[$expression] = $decision
        }
        Assert-True (@($decision.selected).Count -gt 0) "Multi-license selection '$expression' is empty."
        Test-SpdxSelectionSatisfies -Expression $expression -SelectedValues @($decision.selected)
        foreach ($selected in @($decision.selected)) {
            Test-SelectedLicense -Selected ([string]$selected) -Scope "production" -PackageLabel "policy decision '$expression'"
        }
    }

    foreach ($aliasTarget in $LegacyAliases.Values) {
        Assert-True ($SelectionByExpression.ContainsKey([string]$aliasTarget)) "Legacy alias target '$aliasTarget' has no selection."
    }
    Assert-True ($Policy.licensePolicy.defaultDecision -eq "deny") "Unknown licenses must be denied by default."
    Assert-True ($Policy.licensePolicy.unknownDecision -eq "deny") "UNKNOWN licenses must be denied."
    Assert-True ($Policy.licensePolicy.missingDecision -eq "deny") "Missing licenses must be denied."
    Assert-True ($Policy.licensePolicy.licenseExceptionsAllowed -eq $false) "License exceptions must be disabled."
    Assert-True (@($Policy.licensePolicy.workspaceOwnedMitOverrides).Count -eq 9) "Exactly nine workspace-owned MIT overrides are required."
    Test-DuplicateValues @($Policy.licensePolicy.workspaceOwnedMitOverrides) "workspaceOwnedMitOverrides"
}

function Test-NpmLicenses {
    param(
        [string]$LockPath,
        [string]$ExpectedNodeVersion
    )

    if (-not (Test-Path -LiteralPath $LockPath)) {
        Add-Failure "npm lockfile not found: $LockPath"
        return [pscustomobject]@{ Total = 0; Production = 0; DevelopmentBuild = 0 }
    }
    $nodeCommand = Get-Command node.exe -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($null -eq $nodeCommand) {
        Add-Failure "Node.js is required to read package-lock.json without PowerShell 5.1 empty-key loss."
        return [pscustomobject]@{ Total = 0; Production = 0; DevelopmentBuild = 0 }
    }
    $nodeExe = [string]$nodeCommand.Source
    $reportedNodeVersion = @(& $nodeExe --version 2>&1)
    $nodeVersionExitCode = $LASTEXITCODE
    if ($nodeVersionExitCode -ne 0 -or ($reportedNodeVersion -join "").Trim() -cne "v$ExpectedNodeVersion") {
        Add-Failure "Node.js used for package-lock.json must be v$ExpectedNodeVersion; '$nodeExe' reported '$($reportedNodeVersion -join ' ')'."
        return [pscustomobject]@{ Total = 0; Production = 0; DevelopmentBuild = 0 }
    }

    $nodeScript = @'
const fs = require("fs");
const lockPath = process.argv[2];
const lock = JSON.parse(fs.readFileSync(lockPath, "utf8"));
for (const [path, value] of Object.entries(lock.packages || {})) {
  if (!path) continue;
  const scope = value.dev === true || value.devOptional === true ? "developmentBuild" : "production";
  const packageName = path.replace(/^node_modules\//, "");
  const encode = (text) => Buffer.from(String(text || ""), "utf8").toString("base64");
  process.stdout.write(`${encode(packageName)}\t${scope}\t${encode(value.license || "")}\n`);
}
'@

    $encodedNodeScript = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($nodeScript))
    $nodeWrapper = "eval(Buffer.from(process.argv[1],'base64').toString('utf8'))"
    $previousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $output = @(& $nodeExe --eval $nodeWrapper $encodedNodeScript $LockPath 2>&1)
    $nodeExitCode = $LASTEXITCODE
    $ErrorActionPreference = $previousErrorActionPreference
    if ($nodeExitCode -ne 0) {
        Add-Failure "Node.js failed to read package-lock.json: $($output -join ' ')"
        return [pscustomobject]@{ Total = 0; Production = 0; DevelopmentBuild = 0 }
    }

    $total = 0
    $production = 0
    $developmentBuild = 0
    foreach ($line in $output) {
        if ([string]::IsNullOrWhiteSpace([string]$line)) {
            continue
        }
        $parts = ([string]$line).Split("`t")
        if ($parts.Count -ne 3) {
            Add-Failure "Unexpected npm license record: $line"
            continue
        }
        $name = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($parts[0]))
        $scope = $parts[1]
        $license = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($parts[2]))
        $total++
        if ($scope -eq "production") {
            $production++
        }
        else {
            $developmentBuild++
        }
        Test-LicenseExpression -Expression $license -Scope $scope -PackageLabel "npm package '$name'"
    }
    return [pscustomobject]@{ Total = $total; Production = $production; DevelopmentBuild = $developmentBuild }
}

function Get-CargoHomePath {
    if (-not [string]::IsNullOrWhiteSpace($env:CARGO_HOME)) {
        return $env:CARGO_HOME
    }
    $userProfile = $env:USERPROFILE
    if ([string]::IsNullOrWhiteSpace($userProfile)) {
        $userProfile = [Environment]::GetFolderPath("UserProfile")
    }
    return (Join-Path $userProfile ".cargo")
}

function Get-CargoRegistryRoots {
    $cargoHome = Get-CargoHomePath
    $registrySource = Join-Path $cargoHome "registry\src"
    if (-not (Test-Path -LiteralPath $registrySource)) {
        return @()
    }
    return @(Get-ChildItem -LiteralPath $registrySource -Directory | ForEach-Object { $_.FullName })
}

function Test-CargoAuditPinEvidence {
    param(
        [object]$Policy,
        [string]$ReceiptPath
    )

    if ([string]::IsNullOrWhiteSpace($ReceiptPath)) {
        Add-ToolPinBlocker "Same-run cargo-audit pin receipt path was not provided."
        return
    }
    if (-not (Test-Path -LiteralPath $ReceiptPath -PathType Leaf)) {
        Add-ToolPinBlocker "Same-run cargo-audit pin receipt is absent: $ReceiptPath"
        return
    }

    try {
        $strictUtf8 = New-Object Text.UTF8Encoding($false, $true)
        $receiptText = $strictUtf8.GetString([IO.File]::ReadAllBytes($ReceiptPath))
        $receipt = $receiptText | ConvertFrom-Json
    }
    catch {
        Add-ToolPinBlocker "Same-run cargo-audit pin receipt could not be parsed: $($_.Exception.Message)"
        return
    }

    $requiredFields = @(
        "schemaVersion",
        "githubRunId",
        "githubRunAttempt",
        "githubSha",
        "tool",
        "requestedVersion",
        "reportedVersion",
        "reportedVersionOutput",
        "registryUri",
        "registryVersion",
        "registryChecksum",
        "registryYanked",
        "archivePath",
        "archiveSha256",
        "executablePath",
        "executableSha256"
    )
    $receiptFields = @($receipt.PSObject.Properties.Name)
    $missingFields = @($requiredFields | Where-Object { $receiptFields -notcontains $_ })
    if ($missingFields.Count -gt 0) {
        Add-ToolPinBlocker "Same-run cargo-audit pin receipt is missing required fields: $($missingFields -join ', ')."
        return
    }

    $currentRunId = [string]$env:GITHUB_RUN_ID
    $currentRunAttempt = [string]$env:GITHUB_RUN_ATTEMPT
    $currentSha = [string]$env:GITHUB_SHA
    $currentRunnerTemp = [string]$env:RUNNER_TEMP
    $currentCargoHome = [string]$env:CARGO_HOME
    if ($currentRunId -notmatch '^\d+$' -or
        $currentRunAttempt -notmatch '^\d+$' -or
        $currentSha -notmatch '^[0-9a-f]{40}$' -or
        -not [IO.Path]::IsPathRooted($currentRunnerTemp) -or
        -not [IO.Path]::IsPathRooted($currentCargoHome)) {
        Add-ToolPinBlocker "Current GitHub run identity is unavailable; same-run cargo-audit evidence cannot be verified."
        return
    }
    $expectedCargoHome = [IO.Path]::GetFullPath((Join-Path $currentRunnerTemp "ori3-cargo-home-$currentRunId-$currentRunAttempt"))
    $expectedReceiptPath = [IO.Path]::GetFullPath((Join-Path $currentRunnerTemp "cargo-audit-pin-evidence-$currentRunId-$currentRunAttempt.json"))
    if (-not [string]::Equals([IO.Path]::GetFullPath($currentCargoHome), $expectedCargoHome, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals([IO.Path]::GetFullPath($ReceiptPath), $expectedReceiptPath, [StringComparison]::OrdinalIgnoreCase)) {
        Add-ToolPinBlocker "cargo-audit pin receipt and Cargo cache must use the current run-scoped runner-temp paths."
        return
    }

    $expectedVersion = [string]$Policy.toolPins.cargoAudit.version
    $expectedChecksum = [string]$Policy.toolPins.cargoAudit.crateSha256
    $registryUri = "https://index.crates.io/ca/rg/cargo-audit"
    $blockerBaseline = $ToolPinBlockers.Count
    if ([int]$receipt.schemaVersion -ne 1) {
        Add-ToolPinBlocker "Same-run cargo-audit pin receipt schemaVersion must be 1."
    }
    if ([string]$receipt.githubRunId -cne $currentRunId -or
        [string]$receipt.githubRunAttempt -cne $currentRunAttempt -or
        [string]$receipt.githubSha -cne $currentSha) {
        Add-ToolPinBlocker "cargo-audit pin receipt does not belong to the current GitHub run, attempt, and revision."
    }
    if ([string]$receipt.tool -cne "cargo-audit") {
        Add-ToolPinBlocker "cargo-audit pin receipt names an unexpected tool."
    }
    if ([string]$receipt.requestedVersion -cne $expectedVersion -or
        [string]$receipt.reportedVersion -cne $expectedVersion -or
        [string]$receipt.registryVersion -cne $expectedVersion) {
        Add-ToolPinBlocker "cargo-audit requested, reported, and registry versions must all equal $expectedVersion."
    }
    if ([string]$receipt.registryUri -cne $registryUri) {
        Add-ToolPinBlocker "cargo-audit registry evidence must come from $registryUri."
    }
    if ([string]$receipt.registryChecksum -cne $expectedChecksum -or
        [string]$receipt.archiveSha256 -cne $expectedChecksum) {
        Add-ToolPinBlocker "cargo-audit registry and archive checksums must both equal the pinned checksum."
    }
    if (-not ($receipt.registryYanked -is [bool]) -or $receipt.registryYanked -ne $false) {
        Add-ToolPinBlocker "Pinned cargo-audit version is yanked in the same-run registry evidence."
    }
    if ([string]$receipt.executableSha256 -notmatch '^[0-9a-f]{64}$') {
        Add-ToolPinBlocker "cargo-audit receipt executable SHA-256 is not a lowercase 64-character hash."
    }
    if ($ToolPinBlockers.Count -gt $blockerBaseline) {
        return
    }

    $archivePath = [string]$receipt.archivePath
    $expectedArchiveRoot = [IO.Path]::GetFullPath((Join-Path $currentCargoHome "registry\cache")).TrimEnd([char[]]"\/") + [IO.Path]::DirectorySeparatorChar
    if (-not [IO.Path]::IsPathRooted($archivePath) -or
        -not (Test-Path -LiteralPath $archivePath -PathType Leaf) -or
        [IO.Path]::GetFileName($archivePath) -cne "cargo-audit-$expectedVersion.crate") {
        Add-ToolPinBlocker "Same-run cargo-audit crate archive is absent or has an unexpected path."
        return
    }
    $archivePath = [IO.Path]::GetFullPath($archivePath)
    if (-not $archivePath.StartsWith($expectedArchiveRoot, [StringComparison]::OrdinalIgnoreCase)) {
        Add-ToolPinBlocker "Same-run cargo-audit crate archive is outside the run-scoped Cargo cache."
        return
    }
    $actualArchiveChecksum = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualArchiveChecksum -cne $expectedChecksum) {
        Add-ToolPinBlocker "Same-run cargo-audit crate archive checksum differs from the pin receipt and policy."
        return
    }

    $executablePath = [string]$receipt.executablePath
    $expectedExecutablePath = [IO.Path]::GetFullPath((Join-Path (Join-Path $currentRunnerTemp "cargo-audit-install-$currentRunId-$currentRunAttempt") "bin\cargo-audit.exe"))
    if (-not [IO.Path]::IsPathRooted($executablePath) -or
        -not (Test-Path -LiteralPath $executablePath -PathType Leaf) -or
        -not [string]::Equals([IO.Path]::GetFullPath($executablePath), $expectedExecutablePath, [StringComparison]::OrdinalIgnoreCase)) {
        Add-ToolPinBlocker "Same-run cargo-audit executable is absent or has an unexpected path."
        return
    }
    $actualExecutableChecksum = (Get-FileHash -LiteralPath $executablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualExecutableChecksum -cne [string]$receipt.executableSha256) {
        Add-ToolPinBlocker "Same-run cargo-audit executable checksum differs from the pin receipt."
        return
    }

    try {
        $reportedOutputLines = @(& $executablePath --version 2>&1)
        $reportedExitCode = $LASTEXITCODE
    }
    catch {
        Add-ToolPinBlocker "Same-run cargo-audit executable could not report its version: $($_.Exception.Message)"
        return
    }
    $reportedOutput = ($reportedOutputLines | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
    if ($reportedExitCode -ne 0 -or $reportedOutput.Trim() -cne "cargo-audit $expectedVersion") {
        Add-ToolPinBlocker "Same-run cargo-audit executable reported an unexpected version: $reportedOutput"
        return
    }
    if ([string]$receipt.reportedVersionOutput -cne $reportedOutput.Trim()) {
        Add-ToolPinBlocker "Same-run cargo-audit executable output differs from the pin receipt."
    }
}

function Test-WorkspaceLicenseEvidence {
    $rootCargo = Join-Path $PSScriptRoot "..\Cargo.toml"
    $rootLicense = Join-Path $PSScriptRoot "..\LICENSE"
    if (-not (Test-Path -LiteralPath $rootCargo)) {
        Add-Failure "Root Cargo.toml is missing for workspace license evidence."
    }
    else {
        $rootCargoText = Get-Content -LiteralPath $rootCargo -Raw -Encoding UTF8
        Assert-True ($rootCargoText -match '(?m)^license\s*=\s*"MIT"\s*$') "Root Cargo.toml must record MIT workspace license evidence."
    }
    if (-not (Test-Path -LiteralPath $rootLicense)) {
        Add-Failure "Root LICENSE is missing for workspace license evidence."
    }
    else {
        $rootLicenseText = Get-Content -LiteralPath $rootLicense -Raw -Encoding UTF8
        Assert-True ($rootLicenseText -match "MIT License") "Root LICENSE must contain the MIT License text."
    }
}

function Test-CargoLicenses {
    param(
        [string]$LockPath,
        [object]$Policy
    )

    if (-not (Test-Path -LiteralPath $LockPath)) {
        Add-Failure "Cargo lockfile not found: $LockPath"
        return [pscustomobject]@{ Total = 0; Registry = 0; Workspace = 0; UnsupportedSource = 0 }
    }
    Test-WorkspaceLicenseEvidence
    $registryRoots = @(Get-CargoRegistryRoots)
    if ($registryRoots.Count -eq 0) {
        Add-Failure "No local Cargo registry source cache is available for license inspection."
    }

    $lockText = Get-Content -LiteralPath $LockPath -Raw -Encoding UTF8
    $blocks = @([regex]::Split($lockText, '(?m)^\[\[package\]\]\s*$') | Select-Object -Skip 1)
    $total = 0
    $registry = 0
    $workspace = 0
    $unsupported = 0
    $workspaceNames = @($Policy.licensePolicy.workspaceOwnedMitOverrides)

    foreach ($block in $blocks) {
        $nameMatch = [regex]::Match($block, '(?m)^name = "([^"]+)"')
        $versionMatch = [regex]::Match($block, '(?m)^version = "([^"]+)"')
        $sourceMatch = [regex]::Match($block, '(?m)^source = "([^"]+)"')
        if (-not $nameMatch.Success -or -not $versionMatch.Success) {
            Add-Failure "Cargo.lock contains a package block without name/version."
            continue
        }
        $name = $nameMatch.Groups[1].Value
        $version = $versionMatch.Groups[1].Value
        $total++

        if (-not $sourceMatch.Success) {
            $workspace++
            if (-not ($workspaceNames -contains $name)) {
                $LicenseMetrics.Unknown++
                $LicenseMetrics.Scanned++
                Add-Failure "Cargo path package '$name@$version' is not a recorded workspace-owned MIT package."
            }
            else {
                Test-LicenseExpression -Expression "MIT" -Scope "production" -PackageLabel "Cargo workspace package '$name@$version'"
            }
            continue
        }

        $source = $sourceMatch.Groups[1].Value
        if (-not $source.StartsWith("registry+", [StringComparison]::Ordinal)) {
            $unsupported++
            $LicenseMetrics.Unknown++
            $LicenseMetrics.Scanned++
            Add-Failure "Cargo package '$name@$version' uses unsupported source '$source'."
            continue
        }

        $registry++
        $manifest = $null
        foreach ($root in $registryRoots) {
            $candidate = Join-Path (Join-Path $root "$name-$version") "Cargo.toml"
            if (Test-Path -LiteralPath $candidate) {
                $manifest = $candidate
                break
            }
        }
        if ($null -eq $manifest) {
            $LicenseMetrics.Unknown++
            $LicenseMetrics.Scanned++
            Add-Failure "Cargo registry manifest is missing from the local cache for '$name@$version'."
            continue
        }

        $manifestText = Get-Content -LiteralPath $manifest -Raw -Encoding UTF8
        $licenseMatch = [regex]::Match($manifestText, '(?m)^license = "([^"]+)"')
        if ($licenseMatch.Success) {
            Test-LicenseExpression -Expression $licenseMatch.Groups[1].Value -Scope "production" -PackageLabel "Cargo package '$name@$version'"
        }
        else {
            $licenseFileMatch = [regex]::Match($manifestText, '(?m)^license-file = "([^"]+)"')
            if ($licenseFileMatch.Success) {
                Test-LicenseExpression -Expression "SEE LICENSE IN $($licenseFileMatch.Groups[1].Value)" -Scope "production" -PackageLabel "Cargo package '$name@$version'"
            }
            else {
                Test-LicenseExpression -Expression "<MISSING>" -Scope "production" -PackageLabel "Cargo package '$name@$version'"
            }
        }
    }

    return [pscustomobject]@{ Total = $total; Registry = $registry; Workspace = $workspace; UnsupportedSource = $unsupported }
}

$requiredPaths = @($PolicyPath)
if ($Mode -ne "ReadinessOnly") {
    $requiredPaths += @($CargoLockPath, $PackageLockPath, $DependabotPath, $SecurityWorkflowPath)
}
foreach ($requiredPath in $requiredPaths) {
    if (-not (Test-Path -LiteralPath $requiredPath)) {
        throw "Required file not found: $requiredPath"
    }
}

try {
    $policyText = Get-Content -LiteralPath $PolicyPath -Raw -Encoding UTF8
    $policy = $policyText | ConvertFrom-Json
}
catch {
    Write-Host "ORIGAMI3 supply-chain 10-A policy check v$ScriptVersion"
    Write-Host "Mode: $Mode"
    Write-Host "Policy: $PolicyPath"
    Write-Host "POLICY PARSE: FAILED ($($_.Exception.Message))" -ForegroundColor Red
    Write-Host "10-A OVERALL: FAILED" -ForegroundColor Red
    exit 1
}

$configurationFailureCount = 0
$npmResult = $null
$cargoResult = $null
if ($Mode -ne "ReadinessOnly") {
    $configurationFailureBaseline = $Failures.Count
    Test-PolicyShape $policy
    Test-DependabotConfiguration -Policy $policy -Path $DependabotPath
    Test-SecurityWorkflowConfiguration -Policy $policy -Path $SecurityWorkflowPath
    $configurationFailureCount = $Failures.Count - $configurationFailureBaseline
    Test-Exceptions $policy
    Initialize-LicensePolicy $policy
    $npmResult = Test-NpmLicenses -LockPath $PackageLockPath -ExpectedNodeVersion ([string]$policy.toolPins.npmAudit.nodeVersion)
    $cargoResult = Test-CargoLicenses -LockPath $CargoLockPath -Policy $policy
}
if ($Mode -ne "PolicyAndLicenses") {
    try {
        Test-ToolPinReadiness -Policy $policy -CargoAuditReceiptPath $CargoAuditReceiptPath
        Test-KnownAdvisoryAssessment $policy
    }
    catch {
        Add-Failure "Readiness policy data could not be evaluated: $($_.Exception.Message)"
    }
}

Write-Host "ORIGAMI3 supply-chain 10-A policy check v$ScriptVersion"
Write-Host "Mode: $Mode"
Write-Host "Policy: $PolicyPath"
if ($Mode -eq "ReadinessOnly") {
    Write-Host "Offline configuration checks: NOT CHECKED IN ReadinessOnly MODE"
    Write-Host "10-A POLICY/LICENSE DATA: NOT CHECKED IN ReadinessOnly MODE"
}
else {
    Write-Host "Dependabot configuration: $DependabotPath"
    Write-Host "Security workflow configuration: $SecurityWorkflowPath"
    if ($configurationFailureCount -eq 0) {
        Write-Host "Offline configuration checks: PASSED (policy, Dependabot, security workflow)" -ForegroundColor Green
    }
    else {
        Write-Host "Offline configuration checks: FAILED ($configurationFailureCount violation(s))" -ForegroundColor Red
    }
    Write-Host "npm packages: total=$($npmResult.Total), production=$($npmResult.Production), development/build=$($npmResult.DevelopmentBuild)"
    Write-Host "Cargo packages: total=$($cargoResult.Total), registry=$($cargoResult.Registry), workspace=$($cargoResult.Workspace), unsupported-source=$($cargoResult.UnsupportedSource)"
    Write-Host "License results: scanned=$($LicenseMetrics.Scanned), unknown=$($LicenseMetrics.Unknown), outside-allowlist=$($LicenseMetrics.OutsideAllowlist), denied=$($LicenseMetrics.Denied), unselected-multi=$($LicenseMetrics.UnselectedMultiLicense), scope-violation=$($LicenseMetrics.ScopeViolation)"
    Write-Host "Advisory exceptions: $(@($policy.advisoryExceptions).Count), expired=0 (validated), maximum-days=90"
    Write-Host "Policy declarations for automatic dependency actions: merge=0, approve=0, apply-lockfile-to-default-branch=0, release=0"
}
Write-Host "10-B through 10-E external audit/static-analysis/publication execution: not run by this policy/license checker"

$policyDataFailed = $Failures.Count -gt 0
if ($Mode -eq "ReadinessOnly" -and $policyDataFailed) {
    Write-Host "10-A READINESS DATA: FAILED ($($Failures.Count) violation(s))" -ForegroundColor Red
    foreach ($failure in $Failures) {
        Write-Host " - $failure" -ForegroundColor Red
    }
}
elseif ($Mode -eq "ReadinessOnly") {
    Write-Host "10-A READINESS DATA: PASSED" -ForegroundColor Green
}
elseif ($policyDataFailed) {
    Write-Host "10-A POLICY/LICENSE DATA: FAILED ($($Failures.Count) violation(s))" -ForegroundColor Red
    foreach ($failure in $Failures) {
        Write-Host " - $failure" -ForegroundColor Red
    }
}
else {
    Write-Host "10-A POLICY/LICENSE DATA: PASSED" -ForegroundColor Green
}
if ($Mode -eq "PolicyAndLicenses") {
    Write-Host "10-A TOOL PINS: NOT CHECKED IN PolicyAndLicenses MODE"
}
elseif ($ToolPinBlockers.Count -eq 0) {
    Write-Host "10-A TOOL PINS: PASSED" -ForegroundColor Green
}
else {
    Write-Host "10-A TOOL PINS: BLOCKED ($($ToolPinBlockers.Count))" -ForegroundColor Yellow
    foreach ($blocker in $ToolPinBlockers) {
        Write-Host " - $blocker" -ForegroundColor Yellow
    }
}
if ($Mode -eq "PolicyAndLicenses") {
    Write-Host "KNOWN HIGH ADVISORY ASSESSMENT: NOT CHECKED IN PolicyAndLicenses MODE"
}
elseif ($AdvisoryAssessmentBlockers.Count -eq 0) {
    Write-Host "KNOWN HIGH ADVISORY ASSESSMENT: PASSED" -ForegroundColor Green
}
else {
    Write-Host "KNOWN HIGH ADVISORY ASSESSMENT: BLOCKED ($($AdvisoryAssessmentBlockers.Count))" -ForegroundColor Yellow
    foreach ($blocker in $AdvisoryAssessmentBlockers) {
        Write-Host " - $blocker" -ForegroundColor Yellow
    }
}

if ($policyDataFailed) {
    Write-Host "10-A OVERALL: FAILED" -ForegroundColor Red
    exit 1
}
elseif ($Mode -eq "PolicyAndLicenses") {
    Write-Host "POLICY/LICENSE CHECK: PASSED" -ForegroundColor Green
    exit 0
}
elseif ($ToolPinBlockers.Count -gt 0 -or $AdvisoryAssessmentBlockers.Count -gt 0) {
    Write-Host "10-A OVERALL: INCOMPLETE" -ForegroundColor Yellow
    exit 2
}

Write-Host "10-A OVERALL: PASSED" -ForegroundColor Green
exit 0
