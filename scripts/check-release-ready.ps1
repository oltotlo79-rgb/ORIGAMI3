[CmdletBinding()]
param(
    [string]$Tag
)

# ORIGAMI3 リリース前検査 (Windows PowerShell 5.1 / PowerShell 7 対応)
#
# build-manual.ps1 の生成経路:
#   apps/desktop/src/help + docs/manual/assets + apps/desktop/package.json
#     -> target/manual/help-content.json (生成物)
#     -> docs/manual/ORIGAMI3取扱説明書.pdf
# PDF内の文字は図形化されており安定して抽出できないため、検査3では
# export-help-content.ts がPDF表紙へ渡す package.json の version を確認する。

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$root = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot)).TrimEnd([char[]]"\/")
$pdfPath = Join-Path $root "docs\manual\ORIGAMI3取扱説明書.pdf"
$desktopPath = Join-Path $root "apps\desktop"
$srcPath = Join-Path $desktopPath "src"
$helpPath = Join-Path $srcPath "help"
$packageJsonPath = Join-Path $desktopPath "package.json"
$manualAssetsPath = Join-Path $root "docs\manual\assets"
$reportLogPath = Join-Path $root "docs\報告記録.md"
$script:failureCount = 0

function Write-Stage {
    param([int]$Number, [string]$Name)

    Write-Host ""
    Write-Host "=== ($Number/5) $Name ===" -ForegroundColor Cyan
}

function Write-Ok {
    param([string]$Message)

    Write-Host "[OK] $Message" -ForegroundColor Green
}

function Write-Ng {
    param([string]$Message)

    $script:failureCount++
    Write-Host "[NG] $Message" -ForegroundColor Red
}

function Read-Utf8Text {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "ファイルがありません: $(Get-DisplayPath $Path)"
    }
    $utf8 = [System.Text.UTF8Encoding]::new($false, $true)
    return [System.IO.File]::ReadAllText($Path, $utf8)
}

function Get-RelativePath {
    param([string]$BasePath, [string]$Path)

    $baseFull = [System.IO.Path]::GetFullPath($BasePath).TrimEnd([char[]]"\/")
    $pathFull = [System.IO.Path]::GetFullPath($Path).TrimEnd([char[]]"\/")
    if ([string]::Equals($baseFull, $pathFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        return ""
    }
    $prefix = $baseFull + [System.IO.Path]::DirectorySeparatorChar
    if (-not $pathFull.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        return $null
    }
    return $pathFull.Substring($prefix.Length).Replace("\", "/")
}

function Get-DisplayPath {
    param([string]$Path)

    $relative = Get-RelativePath $root $Path
    if ($null -eq $relative) {
        return $Path
    }
    return $relative
}

function ConvertTo-IgnoreRegexBody {
    param([string]$Pattern)

    $builder = New-Object System.Text.StringBuilder
    for ($index = 0; $index -lt $Pattern.Length; $index++) {
        $character = $Pattern[$index]
        if ($character -eq "*") {
            if (($index + 1) -lt $Pattern.Length -and $Pattern[$index + 1] -eq "*") {
                [void]$builder.Append(".*")
                $index++
            }
            else {
                [void]$builder.Append("[^/]*")
            }
        }
        elseif ($character -eq "?") {
            [void]$builder.Append("[^/]")
        }
        else {
            [void]$builder.Append([regex]::Escape([string]$character))
        }
    }
    return $builder.ToString()
}

function Read-IgnoreRules {
    param([string]$IgnoreFile)

    $rules = @()
    if (-not (Test-Path -LiteralPath $IgnoreFile -PathType Leaf)) {
        return $rules
    }

    $baseDirectory = Split-Path -Parent $IgnoreFile
    foreach ($rawLine in (Get-Content -LiteralPath $IgnoreFile -Encoding UTF8)) {
        $line = $rawLine.Trim()
        if ($line.Length -eq 0 -or $line.StartsWith("#")) {
            continue
        }

        $negated = $line.StartsWith("!")
        if ($negated) {
            $line = $line.Substring(1)
        }
        if ($line.Length -eq 0) {
            continue
        }

        $anchored = $line.StartsWith("/")
        if ($anchored) {
            $line = $line.Substring(1)
        }
        if ($line.EndsWith("/")) {
            $line = $line.Substring(0, $line.Length - 1)
        }
        if ($line.Length -eq 0) {
            continue
        }

        $body = ConvertTo-IgnoreRegexBody $line
        if ($anchored -or $line.Contains("/")) {
            $expression = "^$body(?:$|/)"
        }
        else {
            $expression = "(?:^|/)$body(?:$|/)"
        }
        $rules += [PSCustomObject]@{
            BaseDirectory = [System.IO.Path]::GetFullPath($baseDirectory)
            Expression    = $expression
            Negated       = $negated
        }
    }
    return $rules
}

function Get-AncestorIgnoreRules {
    param([string]$Directory)

    $directories = New-Object System.Collections.Generic.List[string]
    $current = [System.IO.Path]::GetFullPath($Directory).TrimEnd([char[]]"\/")
    while ($null -ne $current) {
        $relative = Get-RelativePath $root $current
        if ($null -eq $relative) {
            break
        }
        $directories.Add($current)
        if ([string]::Equals($current, $root, [System.StringComparison]::OrdinalIgnoreCase)) {
            break
        }
        $parent = Split-Path -Parent $current
        if ([string]::IsNullOrEmpty($parent) -or $parent -eq $current) {
            break
        }
        $current = $parent
    }

    $rules = @()
    for ($index = $directories.Count - 1; $index -ge 0; $index--) {
        $rules += @(Read-IgnoreRules (Join-Path $directories[$index] ".gitignore"))
    }
    return $rules
}

function Test-IsExcludedPath {
    param([string]$Path, [object[]]$Rules)

    $repositoryRelative = Get-RelativePath $root $Path
    if ($null -ne $repositoryRelative -and
        $repositoryRelative -match "(?i)(?:^|/)(?:node_modules|dist|target)(?:$|/)") {
        return $true
    }

    $ignored = $false
    foreach ($rule in $Rules) {
        $relative = Get-RelativePath $rule.BaseDirectory $Path
        if ($null -eq $relative) {
            continue
        }
        if ([regex]::IsMatch($relative, $rule.Expression, [System.Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
            $ignored = -not $rule.Negated
        }
    }
    return $ignored
}

function Get-IncludedDirectoryFiles {
    param([string]$Directory, [object[]]$ParentRules)

    $rules = @($ParentRules)
    $rules += @(Read-IgnoreRules (Join-Path $Directory ".gitignore"))
    foreach ($entry in (Get-ChildItem -LiteralPath $Directory -Force)) {
        if (Test-IsExcludedPath $entry.FullName $rules) {
            continue
        }
        if ($entry.PSIsContainer) {
            Get-IncludedDirectoryFiles $entry.FullName $rules
        }
        else {
            $entry
        }
    }
}

function Get-IncludedFiles {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return @()
    }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer) {
        $parentRules = @(Get-AncestorIgnoreRules (Split-Path -Parent $item.FullName))
        return @(Get-IncludedDirectoryFiles $item.FullName $parentRules)
    }

    $rules = @(Get-AncestorIgnoreRules (Split-Path -Parent $item.FullName))
    if (Test-IsExcludedPath $item.FullName $rules) {
        return @()
    }
    return @($item)
}

function Get-CargoWorkspaceVersion {
    param([string]$Path)

    $content = Read-Utf8Text $Path
    $section = [regex]::Match(
        $content,
        "(?ms)^[ \t]*\[workspace\.package\][ \t]*(?:\r?\n)(?<body>.*?)(?=^[ \t]*\[|\z)"
    )
    if (-not $section.Success) {
        throw "[workspace.package] が見つかりません"
    }
    $matches = [regex]::Matches(
        $section.Groups["body"].Value,
        '(?m)^[ \t]*version[ \t]*=[ \t]*"(?<version>[^"]+)"[ \t]*(?:#.*)?$'
    )
    if ($matches.Count -ne 1) {
        throw "[workspace.package] の version を1つに特定できません"
    }
    return $matches[0].Groups["version"].Value
}

function Get-JsonRootVersion {
    param([string]$Path)

    $json = Read-Utf8Text $Path | ConvertFrom-Json
    $property = $json.PSObject.Properties["version"]
    if ($null -eq $property -or [string]::IsNullOrWhiteSpace([string]$property.Value)) {
        throw "ルートの version が見つかりません"
    }
    return [string]$property.Value
}

function Get-PackageLockVersions {
    param([string]$Path)

    # Windows PowerShell 5.1のConvertFrom-Jsonは空文字のプロパティ名を扱えない。
    # packages[""]だけ一時的な名前に置換してから、2か所をJSON構造として読む。
    $content = Read-Utf8Text $Path
    $emptyKeyPattern = [regex]'(?m)^(?<indent>[ \t]*)""[ \t]*:'
    if ($emptyKeyPattern.Matches($content).Count -ne 1) {
        throw 'packages[""] を1つに特定できません'
    }
    $normalized = $emptyKeyPattern.Replace(
        $content,
        '${indent}"__release_check_workspace__":',
        1
    )
    $json = $normalized | ConvertFrom-Json
    $rootProperty = $json.PSObject.Properties["version"]
    $packagesProperty = $json.PSObject.Properties["packages"]
    if ($null -eq $rootProperty -or $null -eq $packagesProperty) {
        throw "ルートの version または packages が見つかりません"
    }
    $workspaceProperty = $packagesProperty.Value.PSObject.Properties["__release_check_workspace__"]
    if ($null -eq $workspaceProperty) {
        throw 'packages[""] が見つかりません'
    }
    $workspaceVersionProperty = $workspaceProperty.Value.PSObject.Properties["version"]
    if ($null -eq $workspaceVersionProperty) {
        throw 'packages[""].version が見つかりません'
    }
    return @([string]$rootProperty.Value, [string]$workspaceVersionProperty.Value)
}

function ConvertFrom-ReleaseTag {
    param([string]$Value)

    if ($null -eq $Value) {
        throw "タグが空です"
    }
    $normalized = $Value.Trim()
    if ($normalized.StartsWith("v", [System.StringComparison]::OrdinalIgnoreCase)) {
        $normalized = $normalized.Substring(1)
    }
    if ($normalized -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') {
        throw "タグ「$Value」は v0.4.0 の形式ではありません"
    }
    return $normalized
}

function Format-Time {
    param([datetime]$Value)

    return $Value.ToUniversalTime().ToString("yyyy-MM-dd HH:mm:ss.fffffff 'UTC'")
}

function Get-LatestReportLogDate {
    param([string]$Path)

    $content = Read-Utf8Text $Path
    $headerPattern = [regex]::new(
        '(?m)^## (?<date>\d{4}-\d{2}-\d{2}) (?<time>(?:[01]\d|2[0-3]):[0-5]\d) — (?<title>\S(?:.*\S)?)$'
    )
    $match = $headerPattern.Match($content)
    if (-not $match.Success) {
        throw "機械可読な報告記録の見出しがありません。scripts/check-report-log.ps1 を先に通してください。"
    }

    $reportDate = [datetime]::MinValue
    $dateText = $match.Groups["date"].Value
    if (-not [datetime]::TryParseExact(
        $dateText,
        "yyyy-MM-dd",
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::None,
        [ref]$reportDate
    )) {
        throw "最新の報告記録の日付が実在しません: $dateText"
    }
    return $reportDate.Date
}

# 検査1: 4ファイル・5か所の版数を読む。
Write-Stage 1 "バージョン番号の一致"
$cargoVersion = $null
$packageVersion = $null
$lockRootVersion = $null
$lockWorkspaceVersion = $null
$tauriVersion = $null
$versionEntries = @()

try {
    $cargoVersion = Get-CargoWorkspaceVersion (Join-Path $root "Cargo.toml")
    $versionEntries += [PSCustomObject]@{ Label = "Cargo.toml [workspace.package]"; Value = $cargoVersion; Error = $null }
}
catch {
    $versionEntries += [PSCustomObject]@{ Label = "Cargo.toml [workspace.package]"; Value = $null; Error = $_.Exception.Message }
}
try {
    $packageVersion = Get-JsonRootVersion $packageJsonPath
    $versionEntries += [PSCustomObject]@{ Label = "apps/desktop/package.json"; Value = $packageVersion; Error = $null }
}
catch {
    $versionEntries += [PSCustomObject]@{ Label = "apps/desktop/package.json"; Value = $null; Error = $_.Exception.Message }
}
try {
    $lockVersions = @(Get-PackageLockVersions (Join-Path $desktopPath "package-lock.json"))
    $lockRootVersion = $lockVersions[0]
    $lockWorkspaceVersion = $lockVersions[1]
    $versionEntries += [PSCustomObject]@{ Label = "apps/desktop/package-lock.json (ルート)"; Value = $lockRootVersion; Error = $null }
    $versionEntries += [PSCustomObject]@{ Label = 'apps/desktop/package-lock.json (packages[""])'; Value = $lockWorkspaceVersion; Error = $null }
}
catch {
    $versionEntries += [PSCustomObject]@{ Label = "apps/desktop/package-lock.json (ルート)"; Value = $null; Error = $_.Exception.Message }
    $versionEntries += [PSCustomObject]@{ Label = 'apps/desktop/package-lock.json (packages[""])'; Value = $null; Error = $_.Exception.Message }
}
try {
    $tauriVersion = Get-JsonRootVersion (Join-Path $desktopPath "src-tauri\tauri.conf.json")
    $versionEntries += [PSCustomObject]@{ Label = "apps/desktop/src-tauri/tauri.conf.json"; Value = $tauriVersion; Error = $null }
}
catch {
    $versionEntries += [PSCustomObject]@{ Label = "apps/desktop/src-tauri/tauri.conf.json"; Value = $null; Error = $_.Exception.Message }
}

$parsedEntries = @($versionEntries | Where-Object { $null -ne $_.Value })
$uniqueVersions = @($parsedEntries | ForEach-Object { $_.Value } | Sort-Object -Unique)
$versionsAgree = $parsedEntries.Count -eq 5 -and $uniqueVersions.Count -eq 1
if ($versionsAgree) {
    Write-Ok "4ファイル5か所の版数は $($uniqueVersions[0]) で一致しています。"
}
else {
    Write-Ng "バージョン番号が一致していません。"
    foreach ($entry in $versionEntries) {
        if ($null -ne $entry.Value) {
            Write-Host "  - $($entry.Label): $($entry.Value)"
        }
        else {
            Write-Host "  - $($entry.Label): 読み取り失敗 ($($entry.Error))"
        }
    }
    Write-Host "  修正: 上の5か所を同じ版数にそろえてください。"
}

# 検査2: PDFが画面ソース・共通ヘルプ・画面写真・版数源より厳密に新しいこと。
Write-Stage 2 "説明書が画面と生成元より新しいこと"
try {
    if (-not (Test-Path -LiteralPath $pdfPath -PathType Leaf)) {
        throw "説明書PDFがありません: $(Get-DisplayPath $pdfPath)"
    }
    if (-not (Test-Path -LiteralPath $srcPath -PathType Container)) {
        throw "画面ソースがありません: $(Get-DisplayPath $srcPath)"
    }

    $comparisonFiles = @()
    $comparisonFiles += @(Get-IncludedFiles $srcPath)
    $comparisonFiles += @(Get-IncludedFiles $packageJsonPath)
    $comparisonFiles += @(Get-IncludedFiles $manualAssetsPath)

    $deduplicatedFiles = @()
    $seenPaths = @{}
    foreach ($file in $comparisonFiles) {
        $key = $file.FullName.ToLowerInvariant()
        if (-not $seenPaths.ContainsKey($key)) {
            $seenPaths[$key] = $true
            $deduplicatedFiles += $file
        }
    }
    if ($deduplicatedFiles.Count -eq 0) {
        throw "比較対象の画面・説明書生成元ファイルがありません"
    }

    $pdf = Get-Item -LiteralPath $pdfPath
    $latestInput = $deduplicatedFiles | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
    if ($pdf.LastWriteTimeUtc -le $latestInput.LastWriteTimeUtc) {
        Write-Ng '説明書が古い。`scripts/build-manual.ps1` で作り直すこと。'
        Write-Host "  原因: $(Get-DisplayPath $latestInput.FullName) ($(Format-Time $latestInput.LastWriteTimeUtc))"
        Write-Host "  説明書: $(Get-DisplayPath $pdf.FullName) ($(Format-Time $pdf.LastWriteTimeUtc))"
    }
    else {
        Write-Ok "説明書は画面と生成元の最新ファイルより新しいです。"
        Write-Host "  最新の生成元: $(Get-DisplayPath $latestInput.FullName) ($(Format-Time $latestInput.LastWriteTimeUtc))"
        Write-Host "  説明書: $(Get-DisplayPath $pdf.FullName) ($(Format-Time $pdf.LastWriteTimeUtc))"
    }
}
catch {
    Write-Ng "説明書の新しさを検査できませんでした: $($_.Exception.Message)"
    Write-Host '  修正: 必要なファイルを確認し、`scripts/build-manual.ps1` で説明書を作り直してください。'
}

# 検査3: PDF生成時に表紙へ渡される package.json の版数をタグと比較する。
Write-Stage 3 "説明書の版数とタグの一致"
try {
    if ($null -eq $packageVersion) {
        throw "生成元 apps/desktop/package.json の version を読めません"
    }

    if ($PSBoundParameters.ContainsKey("Tag")) {
        $expectedVersion = ConvertFrom-ReleaseTag $Tag
        $expectedLabel = "タグ $Tag"
    }
    else {
        if (-not $versionsAgree) {
            throw "タグが省略され、検査1の版数も一致していないため比較対象を決められません"
        }
        $expectedVersion = $uniqueVersions[0]
        $expectedLabel = "検査1の版数 $expectedVersion"
    }

    if (-not [string]::Equals($packageVersion, $expectedVersion, [System.StringComparison]::Ordinal)) {
        Write-Ng "説明書の生成元版数が $expectedLabel と一致していません。"
        Write-Host "  生成元: apps/desktop/package.json = $packageVersion"
        Write-Host "  比較対象: $expectedVersion"
        Write-Host '  修正: 版数をそろえ、`scripts/build-manual.ps1` で説明書を作り直してください。'
    }
    else {
        Write-Ok "説明書の生成元版数 $packageVersion は $expectedLabel と一致しています。"
        Write-Host "  PDFではなく生成元 apps/desktop/package.json の version を検査しました。"
    }
}
catch {
    Write-Ng "説明書の版数を検査できませんでした: $($_.Exception.Message)"
    Write-Host '  修正: 4ファイルの版数をそろえ、必要なら -Tag v0.4.0 を指定してから説明書を作り直してください。'
}

# 検査4: ヘルプがPDFより新しければ、PDFの再生成漏れとして失敗する。
Write-Stage 4 "ヘルプが説明書より新しくないこと"
try {
    if (-not (Test-Path -LiteralPath $pdfPath -PathType Leaf)) {
        throw "説明書PDFがありません: $(Get-DisplayPath $pdfPath)"
    }
    if (-not (Test-Path -LiteralPath $helpPath -PathType Container)) {
        throw "ヘルプがありません: $(Get-DisplayPath $helpPath)"
    }
    $helpFiles = @(Get-IncludedFiles $helpPath)
    if ($helpFiles.Count -eq 0) {
        throw "比較対象のヘルプファイルがありません"
    }

    $pdf = Get-Item -LiteralPath $pdfPath
    $latestHelp = $helpFiles | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
    if ($latestHelp.LastWriteTimeUtc -gt $pdf.LastWriteTimeUtc) {
        Write-Ng "ヘルプを更新したのに説明書を作り直していません。"
        Write-Host "  原因: $(Get-DisplayPath $latestHelp.FullName) ($(Format-Time $latestHelp.LastWriteTimeUtc))"
        Write-Host "  説明書: $(Get-DisplayPath $pdf.FullName) ($(Format-Time $pdf.LastWriteTimeUtc))"
        Write-Host '  修正: `scripts/build-manual.ps1` で説明書を作り直してください。'
    }
    else {
        Write-Ok "ヘルプは説明書より新しくありません。"
        Write-Host "  最新のヘルプ: $(Get-DisplayPath $latestHelp.FullName) ($(Format-Time $latestHelp.LastWriteTimeUtc))"
        Write-Host "  説明書: $(Get-DisplayPath $pdf.FullName) ($(Format-Time $pdf.LastWriteTimeUtc))"
    }
}
catch {
    Write-Ng "ヘルプと説明書の更新順を検査できませんでした: $($_.Exception.Message)"
    Write-Host '  修正: ヘルプと説明書PDFを確認し、`scripts/build-manual.ps1` で説明書を作り直してください。'
}

# 検査5: リリース日に、利用者へ報告した記録が残っていること。
Write-Stage 5 "利用者への報告記録がリリース日と同じこと"
try {
    $latestReportDate = Get-LatestReportLogDate $reportLogPath
    $releaseDate = (Get-Date).Date
    if ($latestReportDate -ne $releaseDate) {
        Write-Ng "利用者への報告記録の最新の日付がリリース日と同じではありません。"
        Write-Host "  最新の報告: $($latestReportDate.ToString('yyyy-MM-dd'))"
        Write-Host "  リリース日: $($releaseDate.ToString('yyyy-MM-dd'))"
        Write-Host '  修正: 利用者への報告を日時とともに docs/報告記録.md へ追記してください。'
    }
    else {
        Write-Ok "利用者への報告記録の最新の日付はリリース日 $($releaseDate.ToString('yyyy-MM-dd')) と一致しています。"
    }
}
catch {
    Write-Ng "利用者への報告記録を検査できませんでした: $($_.Exception.Message)"
    Write-Host '  修正: docs/報告記録.md を確認し、scripts/check-report-log.ps1 を通してください。'
}

Write-Host ""
if ($script:failureCount -gt 0) {
    Write-Host "[NG] リリース準備の検査で $script:failureCount 件の問題が見つかりました。" -ForegroundColor Red
    exit 1
}

Write-Host "[OK] リリース準備の5検査に合格しました。" -ForegroundColor Green
exit 0
