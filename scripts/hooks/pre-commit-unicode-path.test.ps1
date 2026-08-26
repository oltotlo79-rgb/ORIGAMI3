<#
Reproduces the report-log gate with Git's default quoted non-ASCII path output.
The test creates two disposable repositories under %TEMP%; it never writes to
the working repository or its .git directory.
#>
[CmdletBinding()]
param(
    [string]$HookPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$script:ReportFileName = (-join [char[]](0x5831, 0x544A, 0x8A18, 0x9332)) + ".md"
$script:ReportRelativePath = "docs/$script:ReportFileName"

if ([string]::IsNullOrWhiteSpace($HookPath)) {
    $HookPath = Join-Path $PSScriptRoot "pre-commit"
}

function Invoke-GitInTestRepository {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = @(& git -C $Repository @Arguments 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($exitCode -ne 0) {
        throw "PRE-COMMIT-UNICODE-TEST git $($Arguments -join ' ') failed ($exitCode): $($output -join "`n")"
    }
}

function New-IsolatedRepository {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][bool]$IncludeReport
    )

    $repositoryName = if ($IncludeReport) { "with-report" } else { "without-report" }
    $repository = Join-Path $Root $repositoryName
    $hookDirectory = Join-Path $repository "scripts/hooks"
    [System.IO.Directory]::CreateDirectory($hookDirectory) | Out-Null
    [System.IO.File]::Copy($HookPath, (Join-Path $hookDirectory "pre-commit"), $true)

    $appFile = Join-Path $repository "apps/desktop/unicode-path-fixture.ts"
    [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($appFile)) | Out-Null
    [System.IO.File]::WriteAllText($appFile, "export const fixture = true;`n", [System.Text.UTF8Encoding]::new($false))

    if ($IncludeReport) {
        $reportFile = Join-Path (Join-Path $repository "docs") $script:ReportFileName
        [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($reportFile)) | Out-Null
        [System.IO.File]::WriteAllText($reportFile, "# report fixture`n", [System.Text.UTF8Encoding]::new($false))
    }

    Invoke-GitInTestRepository -Repository $repository -Arguments @("init", "--quiet")
    # This is a repository-local setting in the disposable test repository only.
    # The hook under test must override it without depending on a user's setting.
    Invoke-GitInTestRepository -Repository $repository -Arguments @("config", "core.quotePath", "true")
    Invoke-GitInTestRepository -Repository $repository -Arguments @("add", "--", "apps/desktop/unicode-path-fixture.ts")
    if ($IncludeReport) {
        Invoke-GitInTestRepository -Repository $repository -Arguments @("add", "--", $script:ReportRelativePath)
    }
    return $repository
}

function Invoke-PreCommitWithQuotedPaths {
    param([Parameter(Mandatory = $true)][string]$Repository)

    $shellCommand = Get-Command sh.exe, sh -ErrorAction SilentlyContinue | Select-Object -First 1
    $shellPath = if ($null -ne $shellCommand) { $shellCommand.Source } else { $null }
    if ($null -eq $shellPath) {
        $gitCommand = Get-Command git -ErrorAction Stop
        $gitRoot = Split-Path (Split-Path $gitCommand.Source -Parent) -Parent
        $gitShell = Join-Path $gitRoot "bin/sh.exe"
        if (Test-Path -LiteralPath $gitShell -PathType Leaf) {
            $shellPath = $gitShell
        }
    }
    if ($null -eq $shellPath) {
        throw "PRE-COMMIT-UNICODE-TEST FAILED: sh is required to execute scripts/hooks/pre-commit"
    }

    $previousPreference = $ErrorActionPreference
    try {
        # New-IsolatedRepository sets quotePath=true only in this disposable repo.
        try {
            $ErrorActionPreference = "Continue"
            Push-Location -LiteralPath $Repository
            try {
                $output = @(& $shellPath (Join-Path $Repository "scripts/hooks/pre-commit") 2>&1)
                $exitCode = $LASTEXITCODE
            }
            finally {
                Pop-Location
            }
        }
        finally {
            $ErrorActionPreference = $previousPreference
        }
        return [pscustomobject]@{ ExitCode = $exitCode; Output = ($output -join "`n") }
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
}

function Get-StagedNames {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string]$QuotePath
    )

    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $names = @(& git -C $Repository -c "core.quotePath=$QuotePath" diff --cached --name-only --diff-filter=ACMR 2>$null)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($exitCode -ne 0) {
        throw "PRE-COMMIT-UNICODE-TEST FAILED: staged-name listing failed ($exitCode)"
    }
    return $names
}

function Get-StagedNameHexFromShell {
    param([Parameter(Mandatory = $true)][string]$Repository)

    $gitCommand = Get-Command git -ErrorAction Stop
    $gitRoot = Split-Path (Split-Path $gitCommand.Source -Parent) -Parent
    $shellPath = Join-Path $gitRoot "bin/sh.exe"
    if (-not (Test-Path -LiteralPath $shellPath -PathType Leaf)) {
        throw "PRE-COMMIT-UNICODE-TEST FAILED: Git shell is missing"
    }
    $previousPreference = $ErrorActionPreference
    Push-Location -LiteralPath $Repository
    try {
        $ErrorActionPreference = "Continue"
        return @(& $shellPath -c 'git diff --cached --name-only -z | od -An -tx1' 2>&1)
    }
    finally {
        Pop-Location
        $ErrorActionPreference = $previousPreference
    }
}

if (-not (Test-Path -LiteralPath $HookPath -PathType Leaf)) {
    throw "PRE-COMMIT-UNICODE-TEST FAILED: hook is missing: $HookPath"
}

$tempBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$tempRoot = [System.IO.Path]::GetFullPath((Join-Path $tempBase ("ori3-pre-commit-unicode-" + [guid]::NewGuid().ToString("N"))))
if (-not $tempRoot.StartsWith($tempBase, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "PRE-COMMIT-UNICODE-TEST FAILED: temporary path escaped %TEMP%: $tempRoot"
}

try {
    $withReport = New-IsolatedRepository -Root $tempRoot -IncludeReport $true
    $withoutReport = New-IsolatedRepository -Root $tempRoot -IncludeReport $false
    $quotedNames = Get-StagedNames -Repository $withReport -QuotePath "true"
    $unquotedNames = Get-StagedNames -Repository $withReport -QuotePath "false"
    Write-Output "[..] PRE-COMMIT-UNICODE quoted listing: $($quotedNames -join ' | ')"
    Write-Output "[..] PRE-COMMIT-UNICODE unquoted listing: $($unquotedNames -join ' | ')"
    Write-Output "[..] PRE-COMMIT-UNICODE NUL listing bytes: $((Get-StagedNameHexFromShell -Repository $withReport) -join ' ')"
    $includedResult = Invoke-PreCommitWithQuotedPaths -Repository $withReport
    $missingResult = Invoke-PreCommitWithQuotedPaths -Repository $withoutReport

    $failed = $false
    if ($includedResult.ExitCode -ne 0) {
        Write-Output "[NG] PRE-COMMIT-UNICODE: staged report log was rejected under quoted paths."
        Write-Output $includedResult.Output
        $failed = $true
    }
    else {
        Write-Output "[OK] PRE-COMMIT-UNICODE: staged report log was accepted under quoted paths."
    }

    if ($missingResult.ExitCode -eq 0) {
        Write-Output "[NG] PRE-COMMIT-UNICODE: missing report log was accepted."
        $failed = $true
    }
    else {
        Write-Output "[OK] PRE-COMMIT-UNICODE: missing report log was blocked."
        Write-Output $missingResult.Output
    }

    if ($failed) { exit 1 }
    Write-Output "[OK] PRE-COMMIT-UNICODE: both quoted-path cases passed."
}
finally {
    if (Test-Path -LiteralPath $tempRoot) {
        $resolved = [System.IO.Path]::GetFullPath($tempRoot)
        if (-not $resolved.StartsWith($tempBase, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "PRE-COMMIT-UNICODE-TEST FAILED: refusing to delete outside %TEMP%: $resolved"
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
