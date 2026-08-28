[CmdletBinding()]
param(
    [string]$Root,
    [string]$BaselineRef = "ebae7d8",
    [string]$BaselinePath
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
$script:FailureCount = 0
$script:Utf8NoBom = [Text.UTF8Encoding]::new($false)

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = Split-Path -Parent $PSScriptRoot
}
$Root = [IO.Path]::GetFullPath($Root).TrimEnd([char[]]"\/")

function Add-RuleSplitFailure {
    param([Parameter(Mandatory = $true)][string]$Message)

    $script:FailureCount += 1
    Write-Host "[NG] $Message" -ForegroundColor Red
}

function Get-NormalizedText {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    $Text.Replace("`r`n", "`n").Replace("`r", "`n")
}

function Read-Utf8Text {
    param([Parameter(Mandatory = $true)][string]$Path)

    Get-NormalizedText ([IO.File]::ReadAllText($Path, $script:Utf8NoBom))
}

function Get-TextLines {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    $normalized = Get-NormalizedText $Text
    $lines = @($normalized.Split([string[]]@("`n"), [StringSplitOptions]::None))
    if ($lines.Count -gt 0 -and $lines[$lines.Count - 1] -eq "") {
        if ($lines.Count -eq 1) {
            return @()
        }
        return @($lines[0..($lines.Count - 2)])
    }
    @($lines)
}

function Get-FencedCodeBlocks {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    $lines = @(Get-TextLines $Text)
    $blocks = [Collections.Generic.List[string]]::new()
    $start = -1
    for ($index = 0; $index -lt $lines.Count; $index++) {
        if ($lines[$index] -notmatch '^[ \t]*```') {
            continue
        }
        if ($start -lt 0) {
            $start = $index
            continue
        }
        $blocks.Add(($lines[$start..$index] -join "`n"))
        $start = -1
    }
    if ($start -ge 0) {
        Add-RuleSplitFailure "基準文書に閉じていないコード片があります (開始行: $($start + 1))"
    }
    @($blocks.ToArray())
}

function Get-ProtectedNumericTokens {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    # 章番号や日付のような一般的な整数は統合で頻繁に移動するため対象にしない。
    # 誤って丸めたり落としたりしやすい高精度値・科学記数法・桁区切り値だけを守る。
    $pattern = '(?<![\d_.])(?:\d+\.\d{3,}(?:e[+-]?\d+)?|\d+(?:\.\d+)?e[+-]?\d+|\d{1,3}(?:,\d{3})+(?:\.\d+)?)(?![\d_.])'
    $tokens = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($match in [regex]::Matches($Text, $pattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        [void]$tokens.Add($match.Value)
    }
    @($tokens | Sort-Object)
}

function Test-ProtectedNumericTokenPresent {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Token
    )

    $exactPattern = '(?<![\d.]){0}(?![\d.])' -f [regex]::Escape($Token)
    if ([regex]::IsMatch($Text, $exactPattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        return $true
    }

    # `7,200` を `7200` と書いても数値は失われていないため、桁区切りだけは同値とする。
    $withoutSeparators = $Token.Replace(",", "")
    if ($withoutSeparators -eq $Token) {
        return $false
    }
    $pattern = '(?<!\d){0}(?!\d)' -f [regex]::Escape($withoutSeparators)
    [regex]::IsMatch($Text, $pattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)
}

function Get-ProtectedAbsolutePaths {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    # 旧規約に実名で書かれたWindows絶対パスだけを、曖昧な自然文から安全に抽出する。
    $pattern = '(?<![A-Za-z0-9_])(?<path>[A-Za-z]:\\[A-Za-z0-9_.\\*%-]+)'
    $paths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($match in [regex]::Matches($Text, $pattern)) {
        [void]$paths.Add($match.Groups["path"].Value)
    }
    @($paths | Sort-Object)
}

function Test-ProtectedAbsolutePathPresent {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Path
    )

    if ($Text.IndexOf($Path, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
        return $true
    }
    $slashPath = $Path.Replace("\", "/")
    $Text.IndexOf($slashPath, [StringComparison]::OrdinalIgnoreCase) -ge 0
}

function Get-HeadingCorrespondenceRows {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    $rows = [Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal)
    $hasExpectedHeader = $false
    $lineNumber = 0
    foreach ($line in @(Get-TextLines $Text)) {
        $lineNumber += 1
        $trimmed = $line.Trim()
        if (-not $trimmed.StartsWith("|", [StringComparison]::Ordinal)) {
            continue
        }
        $body = $trimmed.Substring(1)
        if ($body.EndsWith("|", [StringComparison]::Ordinal)) {
            $body = $body.Substring(0, $body.Length - 1)
        }
        $cells = @($body.Split([char[]]"|") | ForEach-Object { $_.Trim() })
        if ($cells.Count -lt 4) {
            continue
        }
        if ($cells[0] -eq "旧見出し" -and $cells[1] -eq "状態" -and
            $cells[2] -eq "統合先" -and $cells[3] -eq "理由") {
            $hasExpectedHeader = $true
            continue
        }

        $oldHeadingCell = $cells[0]
        if ($oldHeadingCell -notmatch '^(?<fence>`+)(?<heading>.*)\k<fence>$') {
            continue
        }
        $oldHeading = $Matches["heading"]
        if (-not $oldHeading.StartsWith("#", [StringComparison]::Ordinal)) {
            continue
        }
        if ($rows.ContainsKey($oldHeading)) {
            Add-RuleSplitFailure "変更対応表に同じ旧見出しが複数あります (行 $lineNumber): $oldHeading"
            continue
        }
        $rows.Add($oldHeading, [pscustomobject]@{
                State = $cells[1]
                Target = $cells[2]
                LineNumber = $lineNumber
            })
    }

    [pscustomobject]@{
        HasExpectedHeader = $hasExpectedHeader
        Rows = $rows
    }
}

function Get-BaselineText {
    if (-not [string]::IsNullOrWhiteSpace($BaselinePath)) {
        $resolvedBaseline = if ([IO.Path]::IsPathRooted($BaselinePath)) {
            [IO.Path]::GetFullPath($BaselinePath)
        }
        else {
            [IO.Path]::GetFullPath((Join-Path $Root $BaselinePath))
        }
        if (-not (Test-Path -LiteralPath $resolvedBaseline -PathType Leaf)) {
            throw "分割前の基準文書が見つかりません: $resolvedBaseline"
        }
        return Read-Utf8Text $resolvedBaseline
    }

    $objectName = "{0}:CLAUDE.md" -f $BaselineRef
    $global:LASTEXITCODE = 0
    $baselineLines = @(& git -C $Root show $objectName)
    $status = $LASTEXITCODE
    if ($status -ne 0) {
        throw "git show $objectName に失敗しました (終了コード: $status)"
    }
    Get-NormalizedText (($baselineLines -join "`n") + "`n")
}

try {
    $entryPath = Join-Path $Root "CLAUDE.md"
    $rulesDirectory = Join-Path $Root "docs/rules"
    if (-not (Test-Path -LiteralPath $entryPath -PathType Leaf)) {
        throw "入口文書が見つかりません: $entryPath"
    }
    if (-not (Test-Path -LiteralPath $rulesDirectory -PathType Container)) {
        throw "規約フォルダーが見つかりません: $rulesDirectory"
    }

    $baselineText = Get-BaselineText
    $entryText = Read-Utf8Text $entryPath
    $ruleFiles = @(Get-ChildItem -LiteralPath $rulesDirectory -Filter "*.md" -File | Sort-Object Name)
    $ruleTexts = [Collections.Generic.List[string]]::new()
    foreach ($ruleFile in $ruleFiles) {
        $ruleTexts.Add((Read-Utf8Text $ruleFile.FullName))
    }
    $aggregateText = Get-NormalizedText (($entryText + "`n") + ($ruleTexts -join "`n"))

    $entryLineCount = [IO.File]::ReadAllLines($entryPath, $script:Utf8NoBom).Count
    if ($entryLineCount -gt 150) {
        Add-RuleSplitFailure "CLAUDE.md が150行を超えています (実測: $entryLineCount 行)"
    }
    else {
        Write-Host "[OK] CLAUDE.md 行数: $entryLineCount/150"
    }

    $totalRuleLineCount = $entryLineCount
    foreach ($ruleFile in $ruleFiles) {
        $totalRuleLineCount += [IO.File]::ReadAllLines($ruleFile.FullName, $script:Utf8NoBom).Count
    }
    if ($totalRuleLineCount -gt 650) {
        Add-RuleSplitFailure "CLAUDE.md と docs/rules/*.md の合計が650行を超えています (実測: $totalRuleLineCount 行)"
    }
    else {
        Write-Host "[OK] 規約総行数: $totalRuleLineCount/650 以下"
    }

    $referencedRulePaths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $localLinks = [regex]::Matches($entryText, '\[[^\]\r\n]*\]\((?<target>[^)\r\n]+)\)')
    $brokenLinkCount = 0
    foreach ($match in $localLinks) {
        $target = $match.Groups["target"].Value.Trim()
        if ($target.StartsWith("<") -and $target.EndsWith(">")) {
            $target = $target.Substring(1, $target.Length - 2)
        }
        if ($target -match '^(?:[A-Za-z][A-Za-z0-9+.-]*:|#)') {
            continue
        }
        $fragmentIndex = $target.IndexOf("#", [StringComparison]::Ordinal)
        if ($fragmentIndex -ge 0) {
            $target = $target.Substring(0, $fragmentIndex)
        }
        if ([string]::IsNullOrWhiteSpace($target)) {
            continue
        }
        $target = [Uri]::UnescapeDataString($target).Replace("\", "/")
        while ($target.StartsWith("./", [StringComparison]::Ordinal)) {
            $target = $target.Substring(2)
        }
        if ($target.StartsWith("docs/rules/", [StringComparison]::OrdinalIgnoreCase)) {
            [void]$referencedRulePaths.Add($target)
        }

        $nativeTarget = $target.Replace("/", [IO.Path]::DirectorySeparatorChar)
        $resolvedTarget = [IO.Path]::GetFullPath((Join-Path $Root $nativeTarget))
        $rootPrefix = $Root + [IO.Path]::DirectorySeparatorChar
        if (-not $resolvedTarget.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            Add-RuleSplitFailure "CLAUDE.md のリンクが作業ルート外を指しています: $target"
            $brokenLinkCount += 1
            continue
        }
        if (-not (Test-Path -LiteralPath $resolvedTarget -PathType Leaf)) {
            Add-RuleSplitFailure "CLAUDE.md のリンク先が存在しません: $target"
            $brokenLinkCount += 1
        }
    }
    if ($brokenLinkCount -eq 0) {
        Write-Host "[OK] CLAUDE.md のローカルリンク切れ: 0件"
    }

    $unreferencedRules = [Collections.Generic.List[string]]::new()
    foreach ($ruleFile in $ruleFiles) {
        $relativeRulePath = "docs/rules/{0}" -f $ruleFile.Name
        if (-not $referencedRulePaths.Contains($relativeRulePath)) {
            $unreferencedRules.Add($relativeRulePath)
        }
    }
    if ($unreferencedRules.Count -gt 0) {
        foreach ($unreferencedRule in $unreferencedRules) {
            Add-RuleSplitFailure "docs/rules の規約が CLAUDE.md から参照されていません: $unreferencedRule"
        }
    }
    else {
        Write-Host "[OK] docs/rules の参照: $($ruleFiles.Count)/$($ruleFiles.Count) ファイル"
    }

    $mappingRelativePath = "docs/rules/00-旧規約対応と施行.md"
    $mappingPath = Join-Path $Root ($mappingRelativePath.Replace("/", [IO.Path]::DirectorySeparatorChar))
    $mapping = $null
    if (-not (Test-Path -LiteralPath $mappingPath -PathType Leaf)) {
        Add-RuleSplitFailure "旧規約の変更対応表が見つかりません: $mappingRelativePath"
    }
    else {
        $mapping = Get-HeadingCorrespondenceRows (Read-Utf8Text $mappingPath)
        if (-not $mapping.HasExpectedHeader) {
            Add-RuleSplitFailure "変更対応表に列 `旧見出し | 状態 | 統合先 | 理由` がありません: $mappingRelativePath"
        }
        foreach ($pair in $mapping.Rows.GetEnumerator()) {
            $row = $pair.Value
            if (($row.State -eq "統合" -or $row.State -eq "削除") -and
                [string]::IsNullOrWhiteSpace($row.Target)) {
                Add-RuleSplitFailure "変更対応表の統合先が空です (行 $($row.LineNumber)): $($pair.Key)"
            }
        }
    }

    $baselineHeadings = @([regex]::Matches($baselineText, '(?m)^#{1,6}[ \t]+[^\r\n]+$') | ForEach-Object { $_.Value })
    $aggregateHeadings = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($heading in [regex]::Matches($aggregateText, '(?m)^#{1,6}[ \t]+[^\r\n]+$')) {
        [void]$aggregateHeadings.Add($heading.Value)
    }
    $presentHeadingCount = 0
    $mappedHeadingCount = 0
    foreach ($heading in $baselineHeadings) {
        if ($aggregateHeadings.Contains($heading)) {
            $presentHeadingCount += 1
            continue
        }
        if ($null -eq $mapping -or -not $mapping.Rows.ContainsKey($heading)) {
            Add-RuleSplitFailure "分割前の見出しが欠落し、変更対応表にもありません: $heading"
            continue
        }

        $row = $mapping.Rows[$heading]
        if ($row.State -ne "統合" -and $row.State -ne "削除") {
            Add-RuleSplitFailure "欠落した旧見出しの状態が統合または削除ではありません (行 $($row.LineNumber)、状態: $($row.State)): $heading"
            continue
        }
        if ([string]::IsNullOrWhiteSpace($row.Target)) {
            # 空欄自体の違反は対応表の検査で報告済み。ここでは救済件数に数えない。
            continue
        }
        $mappedHeadingCount += 1
    }
    if (($presentHeadingCount + $mappedHeadingCount) -eq $baselineHeadings.Count) {
        Write-Host "[OK] 分割前の見出し: 存続 $presentHeadingCount + 対応表 $mappedHeadingCount = $($baselineHeadings.Count)/$($baselineHeadings.Count)"
    }

    $baselineCodeBlocks = @(Get-FencedCodeBlocks $baselineText)
    $missingCodeBlocks = 0
    for ($index = 0; $index -lt $baselineCodeBlocks.Count; $index++) {
        if ($aggregateText.IndexOf($baselineCodeBlocks[$index], [StringComparison]::Ordinal) -lt 0) {
            Add-RuleSplitFailure "分割前のコード片が欠落しています: $($index + 1)/$($baselineCodeBlocks.Count)"
            $missingCodeBlocks += 1
        }
    }
    if ($missingCodeBlocks -eq 0) {
        Write-Host "[OK] 分割前のコード片: $($baselineCodeBlocks.Count)/$($baselineCodeBlocks.Count) (欠落0件)"
    }

    $protectedNumericTokens = @(Get-ProtectedNumericTokens $baselineText)
    $missingNumericTokens = 0
    foreach ($token in $protectedNumericTokens) {
        if (-not (Test-ProtectedNumericTokenPresent $aggregateText $token)) {
            Add-RuleSplitFailure "分割前の高精度・桁区切り数値が欠落しています: $token"
            $missingNumericTokens += 1
        }
    }
    if ($missingNumericTokens -eq 0) {
        Write-Host "[OK] 分割前の高精度・桁区切り数値: $($protectedNumericTokens.Count)/$($protectedNumericTokens.Count)"
    }

    $protectedAbsolutePaths = @(Get-ProtectedAbsolutePaths $baselineText)
    $missingAbsolutePaths = 0
    foreach ($path in $protectedAbsolutePaths) {
        if (-not (Test-ProtectedAbsolutePathPresent $aggregateText $path)) {
            Add-RuleSplitFailure "分割前のWindows絶対パスが欠落しています: $path"
            $missingAbsolutePaths += 1
        }
    }
    if ($missingAbsolutePaths -eq 0) {
        Write-Host "[OK] 分割前のWindows絶対パス: $($protectedAbsolutePaths.Count)/$($protectedAbsolutePaths.Count)"
    }

    if ($script:FailureCount -gt 0) {
        Write-Host "[NG] 規約分割検査: $script:FailureCount 件の違反" -ForegroundColor Red
        exit 1
    }

    Write-Host "[OK] 規約分割検査: 違反0件" -ForegroundColor Green
    exit 0
}
catch {
    Write-Host "[NG] 規約分割検査を完了できません: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
