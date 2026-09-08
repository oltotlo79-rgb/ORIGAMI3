[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)

$hookSource = Join-Path $PSScriptRoot "commit-msg"
$approvalSource = Join-Path $PSScriptRoot "checks\cargo-manifest-approval.ps1"
$powerShellPath = (Get-Process -Id $PID).Path
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxRoot = Join-Path $tempBase ("ori3-c6-commit-msg-{0}" -f [Guid]::NewGuid().ToString("N"))
$script:cases = 0
$script:assertions = 0
$utf8NoBom = [Text.UTF8Encoding]::new($false)

function Assert-True {
    param([Parameter(Mandatory = $true)][bool]$Condition, [Parameter(Mandatory = $true)][string]$Message, [string]$Output = "")
    $script:assertions++
    if (-not $Condition) { throw "ASSERTION FAILED: $Message`n$Output" }
}

function Assert-Contains {
    param([Parameter(Mandatory = $true)][string]$Text, [Parameter(Mandatory = $true)][string]$Expected, [Parameter(Mandatory = $true)][string]$Message)
    $script:assertions++
    if (-not $Text.Contains($Expected)) { throw "ASSERTION FAILED: $Message (missing='$Expected')`n$Text" }
}

function ConvertTo-ArgumentString {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Values)
    $parts = foreach ($value in $Values) {
        $escaped = [regex]::Replace($value, '(\\*)"', '$1$1\"')
        $trailingBackslashes = [regex]::Match($escaped, '\\*$').Value
        '"' + $escaped + $trailingBackslashes + '"'
    }
    return $parts -join ' '
}

function Invoke-Git {
    param([Parameter(Mandatory = $true)][string]$RepoRoot, [Parameter(Mandatory = $true)][string[]]$Arguments)
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = (Get-Command git.exe -ErrorAction Stop).Source
    $startInfo.Arguments = ConvertTo-ArgumentString $Arguments
    $startInfo.WorkingDirectory = $RepoRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = $utf8NoBom
    $startInfo.StandardErrorEncoding = $utf8NoBom
    $startInfo.EnvironmentVariables["GIT_TERMINAL_PROMPT"] = "0"
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "git process did not start" }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    return [pscustomobject]@{ ExitCode = $process.ExitCode; Output = ($stdout + $stderr) }
}

function New-TestRepository {
    param([Parameter(Mandatory = $true)][string]$Name, [switch]$Manifest)
    $repoRoot = Join-Path $sandboxRoot $Name
    [void][IO.Directory]::CreateDirectory((Join-Path $repoRoot "hooks\checks"))
    Copy-Item -LiteralPath $hookSource -Destination (Join-Path $repoRoot "hooks\commit-msg") -Force
    Copy-Item -LiteralPath $approvalSource -Destination (Join-Path $repoRoot "hooks\checks\cargo-manifest-approval.ps1") -Force
    foreach ($arguments in @(
        @("init", "--quiet"),
        @("config", "user.email", "test@example.com"),
        @("config", "user.name", "Commit Message Test"),
        @("config", "commit.gpgSign", "false"),
        @("config", "core.hooksPath", "hooks")
    )) {
        $result = Invoke-Git -RepoRoot $repoRoot -Arguments $arguments
        if ($result.ExitCode -ne 0) { throw "fixture setup failed: git $($arguments -join ' ')`n$($result.Output)" }
    }
    $relativePath = if ($Manifest) { "Cargo.toml" } else { "sample.txt" }
    [IO.File]::WriteAllText((Join-Path $repoRoot $relativePath), "fixture`n", $utf8NoBom)
    $addResult = Invoke-Git -RepoRoot $repoRoot -Arguments @("add", $relativePath)
    if ($addResult.ExitCode -ne 0) { throw "fixture add failed`n$($addResult.Output)" }
    return $repoRoot
}

function Test-CommitCase {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Message,
        [Parameter(Mandatory = $true)][bool]$ShouldPass,
        [Parameter(Mandatory = $true)][string]$Diagnostic,
        [switch]$Manifest
    )
    $script:cases++
    Write-Host "[$($script:cases)/8] $Name"
    $repoRoot = New-TestRepository -Name ("case-{0}" -f $script:cases) -Manifest:$Manifest
    $messagePath = Join-Path $repoRoot "message.txt"
    [IO.File]::WriteAllText($messagePath, ($Message.Replace("`r`n", "`n")), $utf8NoBom)
    $result = Invoke-Git -RepoRoot $repoRoot -Arguments @("commit", "-F", $messagePath)
    if ($ShouldPass) {
        Assert-True ($result.ExitCode -eq 0) "$Name must create a real commit" $result.Output
        $countResult = Invoke-Git -RepoRoot $repoRoot -Arguments @("rev-list", "--count", "HEAD")
        Assert-Contains $countResult.Output "1" "$Name must leave exactly one commit"
    }
    else {
        Assert-True ($result.ExitCode -ne 0) "$Name must be rejected by commit-msg" $result.Output
        Assert-Contains $result.Output $Diagnostic "$Name must report its rejection reason"
    }
}

if (-not (Test-Path -LiteralPath $hookSource -PathType Leaf) -or -not (Test-Path -LiteralPath $approvalSource -PathType Leaf)) {
    throw "commit-msg hook sources are missing"
}

[void][IO.Directory]::CreateDirectory($sandboxRoot)
try {
    $trailer = "Co-Authored-By: Codex <noreply@openai.com>"
    $claudeTrailer = "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
    $sessionTrailer = "Claude-Session: https://claude.ai/code/session_012AtiXQ9pY9vcshY9zJnnnV"
    Test-CommitCase -Name "valid Japanese body and trailer" -Message "関門を追加する`n`nコミット文を自動で検査できるようにした。`n`n$trailer`n" -ShouldPass $true -Diagnostic "[OK]"
    Test-CommitCase -Name "co-author followed by Claude-Session trailer" -Message "関門を追加する`n`n複数のtrailerを末尾で検査できるようにした。`n`n$claudeTrailer`n$sessionTrailer`n" -ShouldPass $true -Diagnostic "[OK]"
    Test-CommitCase -Name "English-only one-line message" -Message "update checks`n" -ShouldPass $false -Diagnostic "日本語"
    Test-CommitCase -Name "forbidden conventional prefix" -Message "feat: 関門を追加`n`n日本語の本文がある。`n`n$trailer`n" -ShouldPass $false -Diagnostic "プレフィックス"
    Test-CommitCase -Name "missing co-author trailer" -Message "関門を追加する`n`n日本語の本文がある。`n" -ShouldPass $false -Diagnostic "Co-Authored-By"
    Test-CommitCase -Name "trailer block without co-author" -Message "関門を追加する`n`n日本語の本文がある。`n`n$sessionTrailer`n" -ShouldPass $false -Diagnostic "Co-Authored-By"
    Test-CommitCase -Name "manifest without approval body" -Message "依存関係を更新する`n`n日本語の本文がある。`n`n$trailer`n" -ShouldPass $false -Diagnostic "承認:" -Manifest
    Test-CommitCase -Name "manifest with approval body" -Message "依存関係を更新する`n`n承認: 利用者が依存関係の変更を承認した。`n`n$trailer`n" -ShouldPass $true -Diagnostic "[OK]" -Manifest
    Write-Host "commit-msg self-test passed: $script:cases cases, $script:assertions assertions"
    exit 0
}
finally {
    if (Test-Path -LiteralPath $sandboxRoot) {
        $resolved = [IO.Path]::GetFullPath($sandboxRoot).TrimEnd([char[]]"\/")
        if ([IO.Path]::GetDirectoryName($resolved) -ne $tempBase -or [IO.Path]::GetFileName($resolved) -notmatch '^ori3-c6-commit-msg-[0-9a-f]{32}$') {
            throw "refusing unsafe commit-msg test cleanup: $resolved"
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
