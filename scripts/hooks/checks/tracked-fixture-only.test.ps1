[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$ImplementationPath = Join-Path $PSScriptRoot "tracked-fixture-only.ps1"
$SandboxName = "ori3-tracked-fixture-only-test-{0}" -f [Guid]::NewGuid().ToString("N")
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
    [IO.File]::Copy($ImplementationPath, (Join-Path $checkDirectory "tracked-fixture-only.ps1"), $true)
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

    $scriptPath = Join-Path $Repository "scripts\hooks\checks\tracked-fixture-only.ps1"
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
        -not [regex]::IsMatch($leaf, '^ori3-tracked-fixture-only-test-[0-9a-f]{32}$', [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        throw "安全でない一時領域の削除を拒否しました: $fullSandbox"
    }
    Remove-Item -LiteralPath $fullSandbox -Recurse -Force
}

if (-not (Test-Path -LiteralPath $ImplementationPath -PathType Leaf)) {
    throw "検査本体がありません: $ImplementationPath"
}

[void][IO.Directory]::CreateDirectory($SandboxRoot)
try {
    Write-Host "[1/3] 追跡済みfixtureの参照は合格する"
    $repository = New-IsolatedRepository "tracked"
    New-TestFile $repository "crates/demo/tests/load.rs" 'const DATA: &str = include_str!("fixtures/input.txt");'
    New-TestFile $repository "crates/demo/tests/fixtures/input.txt" "tracked fixture"
    Invoke-GitInSandbox $repository @("add", "--", "crates/demo/tests/load.rs", "crates/demo/tests/fixtures/input.txt")
    $result = Invoke-CheckProcess $repository @("crates/demo/tests/load.rs")
    Assert-True ($result.ExitCode -eq 0) "追跡済みfixtureを誤検知してはいけません（終了コード: $($result.ExitCode)）"
    Assert-True ($result.Output -match '\[OK\]') "正常系は [OK] を出力する必要があります"

    Write-Host "[2/3] 存在しても未追跡のfixture参照は失敗する"
    $repository = New-IsolatedRepository "untracked"
    New-TestFile $repository "crates/demo/tests/load.rs" 'const DATA: &str = include_str!("fixtures/input.txt");'
    New-TestFile $repository "crates/demo/tests/fixtures/input.txt" "untracked fixture"
    Invoke-GitInSandbox $repository @("add", "--", "crates/demo/tests/load.rs")
    $result = Invoke-CheckProcess $repository @("crates/demo/tests/load.rs")
    Assert-True ($result.ExitCode -ne 0) "未追跡fixture参照は非0で失敗する必要があります"
    Assert-True ($result.Output -match '\[NG\]') "未追跡fixture参照は日本語理由を伴う [NG] を出力する必要があります"
    Assert-True ($result.Output -match '追跡対象ではありません') "未追跡fixtureを拒否した具体的な日本語理由が必要です"

    Write-Host "[3/3] 存在しないfixture参照は失敗する"
    $repository = New-IsolatedRepository "missing"
    New-TestFile $repository "crates/demo/tests/load.rs" 'const DATA: &str = include_str!("fixtures/missing.txt");'
    Invoke-GitInSandbox $repository @("add", "--", "crates/demo/tests/load.rs")
    $result = Invoke-CheckProcess $repository @("crates/demo/tests/load.rs")
    Assert-True ($result.ExitCode -ne 0) "存在しないfixture参照は非0で失敗する必要があります"
    Assert-True ($result.Output -match '\[NG\]') "存在しないfixture参照は日本語理由を伴う [NG] を出力する必要があります"
    Assert-True ($result.Output -match '追跡対象ではありません') "存在しないfixtureを拒否した具体的な日本語理由が必要です"

    Write-Host "tracked-fixture-only self-test passed: 3 cases, $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
