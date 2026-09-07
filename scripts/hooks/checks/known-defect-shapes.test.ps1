[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$sourceScript = Join-Path $PSScriptRoot "known-defect-shapes.ps1"
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-known-defect-shapes-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [IO.Path]::GetFullPath((Join-Path $tempBase $sandboxName))
$sandboxScript = Join-Path $sandboxRoot "known-defect-shapes.ps1"
$repository = Join-Path $sandboxRoot "repo"
$definitionPath = Join-Path $repository ".github\known-defect-shapes.json"
$sourcePath = Join-Path $repository "src\lib.rs"
$script:AssertionCount = 0
$script:InvocationCount = 0

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)]$Actual,
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$Output = ""
    )

    $script:AssertionCount += 1
    if ($Actual -ne $Expected) {
        throw "ASSERTION FAILED: $Message (expected=$Expected, actual=$Actual)`n$Output"
    }
}

function Assert-Contains {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $script:AssertionCount += 1
    if (-not $Text.Contains($Expected)) {
        throw "ASSERTION FAILED: $Message (missing='$Expected')`n$Text"
    }
}

function Write-TestSource {
    param([Parameter(Mandatory = $true)][string]$Content)

    [IO.File]::WriteAllText($sourcePath, $Content, [Text.UTF8Encoding]::new($false))
}

function Write-TestDefinition {
    param([Parameter(Mandatory = $true)]$Definition)

    $definitionJson = $Definition | ConvertTo-Json -Depth 10
    [IO.File]::WriteAllText($definitionPath, $definitionJson, [Text.UTF8Encoding]::new($false))
}

function Invoke-IsolatedCheck {
    param(
        [Parameter(Mandatory = $true)][string]$PowerShellPath,
        [switch]$StrictDecrease
    )

    $script:InvocationCount += 1
    $stdoutPath = Join-Path $sandboxRoot ("stdout-{0}.txt" -f $script:InvocationCount)
    $stderrPath = Join-Path $sandboxRoot ("stderr-{0}.txt" -f $script:InvocationCount)
    $arguments = @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $sandboxScript,
        "-RepositoryRoot", $repository,
        "-DefinitionPath", $definitionPath
    )
    if ($StrictDecrease) {
        $arguments += "-FailOnDecrease"
    }

    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        & $PowerShellPath @arguments 1> $stdoutPath 2> $stderrPath
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }

    $output = New-Object System.Collections.Generic.List[string]
    if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) {
        $output.Add([IO.File]::ReadAllText($stdoutPath))
    }
    if (Test-Path -LiteralPath $stderrPath -PathType Leaf) {
        $output.Add([IO.File]::ReadAllText($stderrPath))
    }
    return [pscustomobject]@{
        ExitCode = $exitCode
        Output = ($output -join "`n")
    }
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $sandboxRoot)) {
        return
    }
    $resolved = [IO.Path]::GetFullPath($sandboxRoot).TrimEnd([char[]]"\/")
    $parent = [IO.Path]::GetDirectoryName($resolved)
    $leaf = [IO.Path]::GetFileName($resolved)
    if (($parent -ne $tempBase) -or
        (-not [regex]::IsMatch($leaf, '^ori3-known-defect-shapes-test-[0-9a-f]{32}$', [Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
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

[void][IO.Directory]::CreateDirectory($sandboxRoot)
try {
    [IO.File]::Copy($sourceScript, $sandboxScript, $true)
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $definitionPath))
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $sourcePath))

    $definition = [ordered]@{
        schemaVersion = 1
        patterns = @(
            [ordered]@{
                id = "isolated-negated-comparison"
                reason = "隔離試験用の否定比較"
                roots = @("src")
                filePattern = "\.rs$"
                regex = "(?<![A-Za-z0-9_])!\s*\((?=[^()\r\n]{0,128}(?:<=|>=|<|>))[^()\r\n]{1,128}\)"
                registeredCount = 1
                measuredRawCount = 2
                exceptions = @(
                    [ordered]@{
                        path = "src/lib.rs"
                        line = 3
                        macro = "!("
                        regex = "!\s*\(\s*integer\s*<\s*2\s*\)"
                        allowedCount = 1
                        reason = "integer は usize の全順序比較"
                    }
                )
            }
        )
    }
    Write-TestDefinition -Definition $definition

    $baseline = @'
pub fn check(value: f64, integer: usize) {
    if !(value < 1.0) {}
    if !(integer < 2) {}
    // if !(commented < 3.0) {}
    let _normal = "if !(string_value < 4.0) {}";
    let _raw = r#"if !(raw_string_value < 5.0) {}"#;
    /* if !(block_commented < 6.0) {} */
}
'@

    Write-Output "[1/10] registered count, comment/string masking, and one exception pass"
    Write-TestSource -Content $baseline
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 0 "unchanged registered count must pass" $result.Output
    Assert-Contains $result.Output "1 件（登録 1、raw=2、許可例外=1）" "masking and exception counts must be reported"
    Assert-Contains $result.Output "台帳移動 0 件" "the classified false-positive inventory must match its registered line"

    Write-Output "[2/10] adding only comments and strings does not change the count"
    Write-TestSource -Content ($baseline + @'
// if !(another_comment < 7.0) {}
const TEXT: &str = r##"if !(another_raw_string < 8.0) {}"##;
/* outer /* if !(nested_block_comment < 9.0) {} */ still comment */
'@)
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 0 "comment and string shapes must not be counted" $result.Output
    Assert-Contains $result.Output "raw=2" "masked additions must leave raw count unchanged"

    Write-Output "[3/10] increasing a known defect shape fails"
    Write-TestSource -Content ($baseline + "`nfn added(other: f64) { if !(other < 3.0) {} }`n")
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 1 "an increased shape count must fail" $result.Output
    Assert-Contains $result.Output "増加しました: 1 -> 2" "increase output must include old and current counts"

    $decreased = @'
pub fn check(integer: usize) {
    if !(integer < 2) {}
    // if !(commented < 3.0) {}
    let _normal = "if !(string_value < 4.0) {}";
}
'@
    Write-Output "[4/10] decreasing a known defect shape reports the reduction and passes by default"
    Write-TestSource -Content $decreased
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source
    Assert-Equal $result.ExitCode 0 "a decrease must pass by default" $result.Output
    Assert-Contains $result.Output "[減少]" "decrease output must use an explicit reduction marker"
    Assert-Contains $result.Output "1 -> 0" "decrease output must include old and current counts"
    Assert-Contains $result.Output "known-defect-shapes.json" "decrease output must prompt a data update"
    Assert-Contains $result.Output "誤検知例外台帳が移動または消失しました" "a moved classified false-positive entry must be visible"

    Write-Output "[5/10] -FailOnDecrease makes the same reduction fail"
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -StrictDecrease
    Assert-Equal $result.ExitCode 1 "-FailOnDecrease must reject a reduction until data is updated" $result.Output
    Assert-Contains $result.Output "-FailOnDecrease" "strict decrease output must name the option"

    $candidateDefinition = [ordered]@{
        schemaVersion = 1
        patterns = @(
            [ordered]@{
                id = "isolated-strict-equality"
                reason = "隔離試験用の厳密比較"
                roots = @("src")
                filePattern = "\.rs$"
                regex = "\bassert_(?:eq|ne)!\s*\("
                registeredCount = 1
                measuredRawCount = 1
                candidateInventory = @(
                    [ordered]@{
                        path = "src/lib.rs"
                        line = 2
                        macro = "assert_eq!"
                        regex = "assert_eq!\s*\(\s*computed\s*,\s*1\.0\s*\)"
                        shape = "computedと1.0"
                    }
                )
                registeredBreakdown = [ordered]@{ "assert_eq!" = 1 }
                exceptions = @()
            }
        )
    }
    $candidateBaseline = @'
pub fn check(computed: f64) {
    assert_eq!(computed, 1.0);
}
'@

    Write-Output "[6/10] regex fingerprint accepts the unchanged candidate"
    Write-TestDefinition -Definition $candidateDefinition
    Write-TestSource -Content $candidateBaseline
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -StrictDecrease
    Assert-Equal $result.ExitCode 0 "an unchanged regex fingerprint must pass" $result.Output
    Assert-Contains $result.Output "台帳移動 0 件" "the unchanged fingerprint must not drift"

    Write-Output "[7/10] same-count replacement on the registered line is rejected"
    Write-TestSource -Content ($candidateBaseline.Replace("computed, 1.0", "replacement, 2.0").Replace("computed: f64", "replacement: f64"))
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -StrictDecrease
    Assert-Equal $result.ExitCode 1 "a different strict comparison on the same line must be rejected" $result.Output
    Assert-Contains $result.Output "候補台帳が移動または消失しました" "same-count replacement must report inventory drift"

    Write-Output "[8/10] regex fingerprint detects a line move"
    Write-TestSource -Content ($candidateBaseline.Replace("    assert_eq!", "`n    assert_eq!"))
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -StrictDecrease
    Assert-Equal $result.ExitCode 1 "a fingerprint moved away from its registered line must be rejected" $result.Output
    Assert-Contains $result.Output "候補台帳が移動または消失しました" "line drift must be visible"

    Write-Output "[9/10] candidate without regex keeps the legacy macro check"
    [void]$candidateDefinition.patterns[0].candidateInventory[0].Remove("regex")
    Write-TestDefinition -Definition $candidateDefinition
    Write-TestSource -Content $candidateBaseline
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -StrictDecrease
    Assert-Equal $result.ExitCode 0 "a legacy candidate without regex must still pass" $result.Output
    Assert-Contains $result.Output "台帳移動 0 件" "legacy macro identity must remain compatible"

    Write-Output "[10/10] exception regex rejects a different expression on the same macro line"
    Write-TestDefinition -Definition $definition
    Write-TestSource -Content ($baseline.Replace("integer < 2", "integer > 2"))
    $result = Invoke-IsolatedCheck -PowerShellPath $powerShellCommand.Source -StrictDecrease
    Assert-Equal $result.ExitCode 1 "an exception fingerprint replacement must be rejected" $result.Output
    Assert-Contains $result.Output "誤検知例外台帳が移動または消失しました" "exception replacement must report inventory drift"
    Assert-Contains $result.Output "増加しました" "the replaced exception must not consume the new expression"

    Write-Output "known-defect-shapes self-test passed: 10 cases, $script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
