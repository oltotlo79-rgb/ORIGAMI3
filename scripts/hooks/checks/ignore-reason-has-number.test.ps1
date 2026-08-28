[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$ImplementationPath = Join-Path $PSScriptRoot "ignore-reason-has-number.ps1"
$SandboxName = "ori3-ignore-reason-number-test-{0}" -f [Guid]::NewGuid().ToString("N")
$SandboxRoot = Join-Path ([IO.Path]::GetTempPath()) $SandboxName
$script:AssertionCount = 0

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

function Invoke-GitInSandbox {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $global:LASTEXITCODE = 0
    $unused = @(& git -C $Repository @Arguments)
    $status = $LASTEXITCODE
    if ($status -ne 0) {
        throw "隔離リポジトリの git $($Arguments -join ' ') が失敗しました（終了コード: $status）"
    }
}

function New-TestFile {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string]$RelativePath,
        [Parameter(Mandatory = $true)][string]$Content
    )

    $path = Join-Path $Repository ($RelativePath.Replace("/", "\"))
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $path))
    [IO.File]::WriteAllText($path, $Content, [Text.UTF8Encoding]::new($false))
}

function New-IsolatedRepository {
    param([Parameter(Mandatory = $true)][string]$Name)

    $repository = Join-Path $SandboxRoot $Name
    $checkDirectory = Join-Path $repository "scripts\hooks\checks"
    [void][IO.Directory]::CreateDirectory($checkDirectory)
    [IO.File]::Copy($ImplementationPath, (Join-Path $checkDirectory "ignore-reason-has-number.ps1"), $true)
    Invoke-GitInSandbox -Repository $repository -Arguments @("init", "--quiet")
    $excludePath = (Join-Path $repository ".git\info\exclude").Replace("\", "/")
    Invoke-GitInSandbox -Repository $repository -Arguments @("config", "core.excludesFile", $excludePath)
    Invoke-GitInSandbox -Repository $repository -Arguments @("config", "core.autocrlf", "false")
    return $repository
}

function ConvertTo-PowerShellLiteral {
    param([Parameter(Mandatory = $true)][string]$Value)

    return "'" + $Value.Replace("'", "''") + "'"
}

function Invoke-CheckProcess {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string[]]$Files
    )

    $scriptPath = Join-Path $Repository "scripts\hooks\checks\ignore-reason-has-number.ps1"
    $stdoutPath = Join-Path $Repository ("stdout-{0}.txt" -f [Guid]::NewGuid().ToString("N"))
    $stderrPath = Join-Path $Repository ("stderr-{0}.txt" -f [Guid]::NewGuid().ToString("N"))
    $powerShellPath = (Get-Process -Id $PID).Path
    $childCommand = '[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); & ' +
        (ConvertTo-PowerShellLiteral $scriptPath)
    foreach ($file in $Files) {
        $childCommand += " " + (ConvertTo-PowerShellLiteral $file)
    }
    $encodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($childCommand))

    $process = Start-Process -FilePath $powerShellPath `
        -ArgumentList "-NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand $encodedCommand" `
        -WorkingDirectory $Repository -WindowStyle Hidden -Wait -PassThru `
        -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    $stdout = if (Test-Path -LiteralPath $stdoutPath) { [IO.File]::ReadAllText($stdoutPath) } else { "" }
    $stderr = if (Test-Path -LiteralPath $stderrPath) { [IO.File]::ReadAllText($stderrPath) } else { "" }
    return [pscustomobject]@{
        ExitCode = $process.ExitCode
        Output = $stdout + $stderr
    }
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $SandboxRoot)) {
        return
    }

    $fullSandbox = [IO.Path]::GetFullPath($SandboxRoot).TrimEnd([char[]]"\/")
    $fullTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
    $leaf = [IO.Path]::GetFileName($fullSandbox)
    if ([IO.Path]::GetDirectoryName($fullSandbox) -ne $fullTemp -or
        -not [regex]::IsMatch($leaf, '^ori3-ignore-reason-number-test-[0-9a-f]{32}$', [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        throw "安全でない一時領域の削除を拒否しました: $fullSandbox"
    }
    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

if (-not (Test-Path -LiteralPath $ImplementationPath -PathType Leaf)) {
    throw "検査本体がありません: $ImplementationPath"
}

[void][IO.Directory]::CreateDirectory($SandboxRoot)
try {
    Write-Host "[1/4] 実測値の数字を含む理由は合格する"
    $repository = New-IsolatedRepository "numbered"
    New-TestFile $repository "crates/demo/tests/acceptance.rs" @'
#[ignore = "未達 16件。違反が0件になったら外す"]
#[test]
fn pending_acceptance() {}
'@
    Invoke-GitInSandbox $repository @("add", "--", "crates/demo/tests/acceptance.rs")
    $result = Invoke-CheckProcess $repository @("crates/demo/tests/acceptance.rs")
    Assert-True ($result.ExitCode -eq 0) "数字を含む理由つき #[ignore] を誤検知してはいけません"
    Assert-True ($result.Output -match '\[OK\]') "正常系は [OK] を出力する必要があります"

    Write-Host "[2/4] 数字のない理由は失敗する"
    $repository = New-IsolatedRepository "without-number"
    New-TestFile $repository "crates/demo/tests/acceptance.rs" @'
#[ignore = "未達。違反が無くなったら外す"]
#[test]
fn pending_acceptance() {}
'@
    Invoke-GitInSandbox $repository @("add", "--", "crates/demo/tests/acceptance.rs")
    $result = Invoke-CheckProcess $repository @("crates/demo/tests/acceptance.rs")
    Assert-True ($result.ExitCode -ne 0) "数字のない #[ignore] は非0で失敗する必要があります"
    Assert-True ($result.Output -match '\[NG\]') "数字のない理由には日本語理由を伴う [NG] が必要です"
    Assert-True ($result.Output -match '実測値を示す数字がありません') "数字不足を拒否した具体的な日本語理由が必要です"

    Write-Host "[3/4] 裸の #[ignore] は失敗する"
    $repository = New-IsolatedRepository "bare"
    New-TestFile $repository "crates/demo/tests/acceptance.rs" @'
#[ignore]
#[test]
fn pending_acceptance() {}
'@
    Invoke-GitInSandbox $repository @("add", "--", "crates/demo/tests/acceptance.rs")
    $result = Invoke-CheckProcess $repository @("crates/demo/tests/acceptance.rs")
    Assert-True ($result.ExitCode -ne 0) "裸の #[ignore] は非0で失敗する必要があります"
    Assert-True ($result.Output -match '\[NG\]') "裸の #[ignore] には日本語理由を伴う [NG] が必要です"
    Assert-True ($result.Output -match '理由文のない') "裸の #[ignore] を拒否した具体的な日本語理由が必要です"

    Write-Host "[4/4] コメント内の #[ignore] は属性として扱わない"
    $repository = New-IsolatedRepository "comment"
    New-TestFile $repository "crates/demo/tests/acceptance.rs" @'
// #[ignore]
#[test]
fn active_acceptance() {}
'@
    Invoke-GitInSandbox $repository @("add", "--", "crates/demo/tests/acceptance.rs")
    $result = Invoke-CheckProcess $repository @("crates/demo/tests/acceptance.rs")
    Assert-True ($result.ExitCode -eq 0) "コメント内の #[ignore] を誤検知してはいけません"
    Assert-True ($result.Output -match '\[OK\]') "コメントだけの正常系は [OK] を出力する必要があります"

    Write-Host "ignore-reason-has-number self-test passed: 4 cases, $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
