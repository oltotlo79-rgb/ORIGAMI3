[CmdletBinding()]
param(
    [string]$CheckerPath
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
$script:AssertionCount = 0
$script:Utf8NoBom = [Text.UTF8Encoding]::new($false)

if ([string]::IsNullOrWhiteSpace($CheckerPath)) {
    $CheckerPath = Join-Path $PSScriptRoot "check-rules-split.ps1"
}
$CheckerPath = [IO.Path]::GetFullPath($CheckerPath)
$RepositoryRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$SandboxName = "ori3-check-rules-split-test-{0}" -f [Guid]::NewGuid().ToString("N")
$SandboxRoot = [IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetTempPath()) $SandboxName))
$BaselinePath = Join-Path $SandboxRoot "baseline/CLAUDE.before.md"

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

function Write-Utf8Fixture {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Content
    )

    $parent = Split-Path -Parent $Path
    [void][IO.Directory]::CreateDirectory($parent)
    [IO.File]::WriteAllText($Path, $Content, $script:Utf8NoBom)
}

function New-CaseFixture {
    param([Parameter(Mandatory = $true)][string]$Name)

    $caseRoot = Join-Path $SandboxRoot $Name
    $caseRules = Join-Path $caseRoot "docs/rules"
    [void][IO.Directory]::CreateDirectory($caseRules)
    [IO.File]::Copy((Join-Path $RepositoryRoot "CLAUDE.md"), (Join-Path $caseRoot "CLAUDE.md"), $true)
    foreach ($ruleFile in @(Get-ChildItem -LiteralPath (Join-Path $RepositoryRoot "docs/rules") -Filter "*.md" -File)) {
        [IO.File]::Copy($ruleFile.FullName, (Join-Path $caseRules $ruleFile.Name), $true)
    }
    $caseRoot
}

function Get-CaseAggregateFiles {
    param([Parameter(Mandatory = $true)][string]$Root)

    @((Get-Item -LiteralPath (Join-Path $Root "CLAUDE.md"))) +
        @(Get-ChildItem -LiteralPath (Join-Path $Root "docs/rules") -Filter "*.md" -File | Sort-Object Name)
}

function Get-CaseTotalLineCount {
    param([Parameter(Mandatory = $true)][string]$Root)

    $count = 0
    foreach ($file in @(Get-CaseAggregateFiles $Root)) {
        $count += [IO.File]::ReadAllLines($file.FullName, $script:Utf8NoBom).Count
    }
    $count
}

function Test-CaseHasExactHeading {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Heading
    )

    foreach ($file in @(Get-CaseAggregateFiles $Root)) {
        foreach ($line in [IO.File]::ReadAllLines($file.FullName, $script:Utf8NoBom)) {
            if ($line -ceq $Heading) {
                return $true
            }
        }
    }
    $false
}

function Get-CorrespondenceRowIndex {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string[]]$Lines,
        [Parameter(Mandatory = $true)][string]$OldHeading
    )

    $rowIndexes = [Collections.Generic.List[int]]::new()
    for ($index = 0; $index -lt $Lines.Count; $index++) {
        if ($Lines[$index].TrimStart().StartsWith("|", [StringComparison]::Ordinal) -and
            $Lines[$index].Contains($OldHeading)) {
            $rowIndexes.Add($index)
        }
    }
    Assert-True ($rowIndexes.Count -eq 1) "変更対応表に対象の旧見出しが厳密に1行必要"
    $rowIndexes[0]
}

function Invoke-IsolatedChecker {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$PowerShellPath
    )

    $global:LASTEXITCODE = 0
    $output = @(& $PowerShellPath -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $CheckerPath -Root $Root -BaselinePath $BaselinePath)
    $exitCode = $LASTEXITCODE
    [pscustomobject]@{
        ExitCode = $exitCode
        Output = ($output -join "`n")
    }
}

function Assert-ExitCode {
    param(
        [Parameter(Mandatory = $true)]$Result,
        [Parameter(Mandatory = $true)][bool]$ShouldSucceed,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $passed = if ($ShouldSucceed) { $Result.ExitCode -eq 0 } else { $Result.ExitCode -ne 0 }
    if (-not $passed) {
        Write-Host $Result.Output
    }
    Assert-True $passed $Message
}

function Assert-OutputContains {
    param(
        [Parameter(Mandatory = $true)]$Result,
        [Parameter(Mandatory = $true)][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $contains = $Result.Output.Contains($Expected)
    if (-not $contains) {
        Write-Host $Result.Output
    }
    Assert-True $contains $Message
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $SandboxRoot)) {
        return
    }

    $resolved = [IO.Path]::GetFullPath($SandboxRoot).TrimEnd([char[]]"\/")
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
    $parent = [IO.Path]::GetDirectoryName($resolved)
    $leaf = [IO.Path]::GetFileName($resolved)
    if ($parent -ne $tempRoot -or
        -not [regex]::IsMatch($leaf, '^ori3-check-rules-split-test-[0-9a-f]{32}$', [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        throw "安全でない一時領域の削除を拒否しました: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

if (-not (Test-Path -LiteralPath $CheckerPath -PathType Leaf)) {
    throw "検査本体が見つかりません: $CheckerPath"
}

$powerShellCommand = Get-Command powershell.exe, powershell, pwsh -ErrorAction SilentlyContinue | Select-Object -First 1
if ($null -eq $powerShellCommand) {
    throw "隔離検査を起動する PowerShell が見つかりません"
}
$PowerShellPath = $powerShellCommand.Source

[void][IO.Directory]::CreateDirectory($SandboxRoot)
try {
    $global:LASTEXITCODE = 0
    $baselineLines = @(& git -C $RepositoryRoot show "ebae7d8:CLAUDE.md")
    $gitStatus = $LASTEXITCODE
    if ($gitStatus -ne 0) {
        throw "基準文書の取得に失敗しました (終了コード: $gitStatus)"
    }
    Write-Utf8Fixture $BaselinePath (($baselineLines -join "`n") + "`n")

    Write-Host "[1/9] 正しい分割は成功する"
    $caseRoot = New-CaseFixture "valid"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-ExitCode $result $true "正しい分割を拒否してはならない"

    Write-Host "[2/9] 存続見出しを1つ消すと失敗する"
    $caseRoot = New-CaseFixture "missing-heading"
    $target = Join-Path $caseRoot "docs/rules/01-役割と委譲.md"
    $text = [IO.File]::ReadAllText($target, $script:Utf8NoBom)
    Assert-True ($text.Contains("## 1. 役割分担")) "故障注入前に対象見出しが必要"
    Write-Utf8Fixture $target ($text.Replace("## 1. 役割分担", "役割分担の見出しを故障注入で削除"))
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-OutputContains $result "分割前の見出しが欠落" "見出し検査そのものが故障を検出すること"
    Assert-ExitCode $result $false "見出し欠落を成功扱いしてはならない"

    Write-Host "[3/9] リンク切れがあると失敗する"
    $caseRoot = New-CaseFixture "broken-link"
    $target = Join-Path $caseRoot "CLAUDE.md"
    $text = [IO.File]::ReadAllText($target, $script:Utf8NoBom)
    Assert-True ($text.Contains("docs/rules/02-禁止事項.md")) "故障注入前に対象リンクが必要"
    Write-Utf8Fixture $target ($text.Replace("docs/rules/02-禁止事項.md", "docs/rules/存在しない規約.md"))
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-OutputContains $result "リンク先が存在しません" "リンク検査そのものが故障を検出すること"
    Assert-ExitCode $result $false "リンク切れを成功扱いしてはならない"

    Write-Host "[4/9] CLAUDE.md が151行なら失敗する"
    $caseRoot = New-CaseFixture "too-many-lines"
    $target = Join-Path $caseRoot "CLAUDE.md"
    $lines = [Collections.Generic.List[string]]::new()
    foreach ($line in [IO.File]::ReadAllLines($target, $script:Utf8NoBom)) {
        $lines.Add($line)
    }
    while ($lines.Count -lt 151) {
        $lines.Add("<!-- 行数上限の故障注入 $($lines.Count + 1) -->")
    }
    Write-Utf8Fixture $target (($lines -join "`n") + "`n")
    Assert-True (([IO.File]::ReadAllLines($target, $script:Utf8NoBom)).Count -eq 151) "故障入力は厳密に151行であること"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-OutputContains $result "CLAUDE.md が150行を超えています" "入口行数検査そのものが故障を検出すること"
    Assert-ExitCode $result $false "151行の入口を成功扱いしてはならない"

    Write-Host "[5/9] コード片を変えると失敗する"
    $caseRoot = New-CaseFixture "missing-code-block"
    $target = Join-Path $caseRoot "docs/rules/01-役割と委譲.md"
    $text = [IO.File]::ReadAllText($target, $script:Utf8NoBom)
    $originalCode = "codex exec --model gpt-5.6-sol -c model_reasoning_effort=ultra --sandbox workspace-write --skip-git-repo-check - < 指示ファイル"
    Assert-True ($text.Contains($originalCode)) "故障注入前に対象コード片が必要"
    Write-Utf8Fixture $target ($text.Replace($originalCode, "codex exec --model 故障注入"))
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-OutputContains $result "分割前のコード片が欠落しています" "コード片検査そのものが故障を検出すること"
    Assert-ExitCode $result $false "コード片欠落を成功扱いしてはならない"

    Write-Host "[6/9] 参照されない rules ファイルがあると失敗する"
    $caseRoot = New-CaseFixture "unreferenced-rule"
    Write-Utf8Fixture (Join-Path $caseRoot "docs/rules/99-未参照.md") "# 未参照の故障注入`n"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-OutputContains $result "CLAUDE.md から参照されていません" "参照検査そのものが故障を検出すること"
    Assert-ExitCode $result $false "未参照の規約ファイルを成功扱いしてはならない"

    $integratedHeading = '### 10.7.18 「裂け0・自己交差0」だけを見て、鶴になっていない作品を正しいと判断した（2026-08-28）'

    Write-Host "[7/9] 欠落済み旧見出しの統合対応行を消すと失敗する"
    $caseRoot = New-CaseFixture "missing-correspondence-row"
    Assert-True (-not (Test-CaseHasExactHeading $caseRoot $integratedHeading)) "故障対象の旧見出しは本文から統合済みであること"
    $target = Join-Path $caseRoot "docs/rules/00-旧規約対応と施行.md"
    $lines = @([IO.File]::ReadAllLines($target, $script:Utf8NoBom))
    $rowIndex = Get-CorrespondenceRowIndex $lines $integratedHeading
    $remaining = [Collections.Generic.List[string]]::new()
    for ($index = 0; $index -lt $lines.Count; $index++) {
        if ($index -ne $rowIndex) {
            $remaining.Add($lines[$index])
        }
    }
    Write-Utf8Fixture $target (($remaining -join "`n") + "`n")
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-OutputContains $result "変更対応表にもありません" "変更対応表の行欠落検査そのものが故障を検出すること"
    Assert-ExitCode $result $false "欠落旧見出しの対応行削除を成功扱いしてはならない"

    Write-Host "[8/9] 変更対応表の統合先が空なら失敗する"
    $caseRoot = New-CaseFixture "empty-correspondence-target"
    Assert-True (-not (Test-CaseHasExactHeading $caseRoot $integratedHeading)) "故障対象の旧見出しは本文から統合済みであること"
    $target = Join-Path $caseRoot "docs/rules/00-旧規約対応と施行.md"
    $lines = @([IO.File]::ReadAllLines($target, $script:Utf8NoBom))
    $rowIndex = Get-CorrespondenceRowIndex $lines $integratedHeading
    $cells = @($lines[$rowIndex].Split([char[]]"|"))
    Assert-True ($cells.Count -ge 6) "変更対応表の対象行は4列のMarkdown表であること"
    Assert-True ($cells[2].Trim() -eq "統合" -or $cells[2].Trim() -eq "削除") "故障対象行の状態は統合または削除であること"
    Assert-True (-not [string]::IsNullOrWhiteSpace($cells[3])) "故障注入前の統合先は空でないこと"
    $cells[3] = " "
    $lines[$rowIndex] = $cells -join "|"
    Write-Utf8Fixture $target (($lines -join "`n") + "`n")
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-OutputContains $result "変更対応表の統合先が空です" "空の統合先検査そのものが故障を検出すること"
    Assert-ExitCode $result $false "空の統合先を成功扱いしてはならない"

    Write-Host "[9/9] 規約総量が651行なら失敗する"
    $caseRoot = New-CaseFixture "total-651-lines"
    $totalBefore = Get-CaseTotalLineCount $caseRoot
    Assert-True ($totalBefore -le 650) "故障注入前の規約総量は650行以下であること"
    $target = Join-Path $caseRoot "docs/rules/06-過去の失敗と対策.md"
    $lines = [Collections.Generic.List[string]]::new()
    foreach ($line in [IO.File]::ReadAllLines($target, $script:Utf8NoBom)) {
        $lines.Add($line)
    }
    $needed = 651 - $totalBefore
    for ($index = 1; $index -le $needed; $index++) {
        $lines.Add("<!-- 総量上限の故障注入 $index/$needed -->")
    }
    Write-Utf8Fixture $target (($lines -join "`n") + "`n")
    Assert-True ((Get-CaseTotalLineCount $caseRoot) -eq 651) "故障入力の規約総量は厳密に651行であること"
    $result = Invoke-IsolatedChecker $caseRoot $PowerShellPath
    Assert-OutputContains $result "合計が650行を超えています" "規約総量検査そのものが境界値を検出すること"
    Assert-ExitCode $result $false "651行の規約総量を成功扱いしてはならない"

    Write-Host "[OK] 規約分割の隔離テスト: 9/9件、$script:AssertionCount assertions"
}
finally {
    Remove-TestSandbox
}
