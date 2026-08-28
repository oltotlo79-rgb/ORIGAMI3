[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$sourceScript = Join-Path $PSScriptRoot "cargo-manifest-approval.ps1"
$tempBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-cargo-manifest-approval-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [System.IO.Path]::GetFullPath((Join-Path $tempBase $sandboxName))
$sandboxScript = Join-Path $sandboxRoot "cargo-manifest-approval.ps1"
$script:AssertionCount = 0
$script:InvocationCount = 0

function Assert-ExitCode {
    param(
        [Parameter(Mandatory = $true)][int]$Actual,
        [Parameter(Mandatory = $true)][int]$Expected,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$Output = ""
    )
    $script:AssertionCount += 1
    if ($Actual -ne $Expected) {
        throw "ASSERTION FAILED: $Message (expected=$Expected, actual=$Actual)`n$Output"
    }
}

function Invoke-IsolatedCheck {
    param(
        [Parameter(Mandatory = $true)][string]$PowerShellPath,
        [string[]]$CheckArguments = @()
    )
    $script:InvocationCount += 1
    $stdoutPath = Join-Path $sandboxRoot ("stdout-{0}.txt" -f $script:InvocationCount)
    $stderrPath = Join-Path $sandboxRoot ("stderr-{0}.txt" -f $script:InvocationCount)
    $arguments = @("-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $sandboxScript) + @($CheckArguments)
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        & $PowerShellPath @arguments 1>$stdoutPath 2>$stderrPath
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    $output = @()
    if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) { $output += [System.IO.File]::ReadAllText($stdoutPath) }
    if (Test-Path -LiteralPath $stderrPath -PathType Leaf) { $output += [System.IO.File]::ReadAllText($stderrPath) }
    [pscustomobject]@{ ExitCode = $exitCode; Output = ($output -join "`n") }
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $sandboxRoot)) { return }
    $resolved = [System.IO.Path]::GetFullPath($sandboxRoot).TrimEnd([char[]]"\/")
    $parent = [System.IO.Path]::GetDirectoryName($resolved)
    $leaf = [System.IO.Path]::GetFileName($resolved)
    if (($parent -ne $tempBase) -or
        (-not [regex]::IsMatch($leaf, '^ori3-cargo-manifest-approval-test-[0-9a-f]{32}$', [System.Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
        throw "Refusing unsafe self-test cleanup: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

if (-not (Test-Path -LiteralPath $sourceScript -PathType Leaf)) {
    throw "Required implementation is missing: $sourceScript"
}
$powerShellCommand = Get-Command powershell.exe, pwsh.exe, pwsh -ErrorAction SilentlyContinue | Select-Object -First 1
if ($null -eq $powerShellCommand) {
    throw "PowerShell executable is required for the isolated self-test"
}

[void][System.IO.Directory]::CreateDirectory($sandboxRoot)
try {
    [System.IO.File]::Copy($sourceScript, $sandboxScript, $true)
    $noApprovalPath = Join-Path $sandboxRoot "no-approval.txt"
    $subjectOnlyPath = Join-Path $sandboxRoot "subject-only.txt"
    $approvedPath = Join-Path $sandboxRoot "approved.txt"
    [System.IO.File]::WriteAllText($noApprovalPath, "依存を更新`n`n理由だけを記載`n", [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText($subjectOnlyPath, "承認: 件名だけ`n`n本文には承認記録なし`n", [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText($approvedPath, "依存を更新`n`n承認: 統括が差分を確認済み`n", [System.Text.UTF8Encoding]::new($false))

    Write-Output "[1/5] an unrelated source change needs no approval"
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -CheckArguments @("src/lib.rs")
    Assert-ExitCode $result.ExitCode 0 "unrelated source must pass without a message" $result.Output

    Write-Output "[2/5] Cargo.toml without an approval line is rejected"
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -CheckArguments @("-CommitMessagePath", $noApprovalPath, "Cargo.toml")
    Assert-ExitCode $result.ExitCode 1 "Cargo.toml without approval must fail" $result.Output

    Write-Output "[3/5] approval in the subject does not satisfy the body requirement"
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -CheckArguments @("-CommitMessagePath", $subjectOnlyPath, "Cargo.lock")
    Assert-ExitCode $result.ExitCode 1 "subject-only approval must fail" $result.Output

    Write-Output "[4/5] vendor change with an approved message file passes"
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -CheckArguments @("-CommitMessagePath", $approvedPath, "vendor/package/source.rs")
    Assert-ExitCode $result.ExitCode 0 "approved vendor change must pass" $result.Output

    Write-Output "[5/5] a nested Cargo manifest accepts an explicit approved body"
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -CheckArguments @("-CommitMessageBody", "承認: 統括が差分を確認済み", "crates/example/Cargo.toml")
    Assert-ExitCode $result.ExitCode 0 "approved nested Cargo.toml must pass" $result.Output

    Write-Output "cargo-manifest-approval self-test passed: 5 cases, $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
