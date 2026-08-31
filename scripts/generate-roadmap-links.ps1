[CmdletBinding()]
param(
    [switch]$Check,
    [switch]$Fixtures,
    [ValidateSet("M0")]
    [string]$AllowPartialScope = ""
)

# ORIGAMI3 implementation-roadmap evidence link generator (PowerShell 5.1 compatible)
#
# 7-D1 scope only:
# - read the tracked M0 section of docs/implementation-roadmap.md;
# - require all 11 M0 checkboxes to link to an actual check or unique manual ID;
# - render a deterministic partial links.json for later D2-D10 aggregation.
#
# Product counts are never used as ceilings. The exact count 11 is the delegated
# roadmap scope from improvement-roadmap section 11.3, not a product limit.

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false, $true)
$script:Root = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot)).TrimEnd([char[]]"\/")
$script:RoadmapRelativePath = "docs/implementation-roadmap.md"
$script:GeneratedRelativePath = "verification/improvement-roadmap/07-docs/links.json"
$script:ForbiddenRelativePath = "docs/competitive-review-2026-08-20.md"
$script:ForbiddenPrefixes = @("verification/", "scratchpad/", "vendor/")
$script:BeginMarker = "<!-- ORIGAMI3-ROADMAP-EVIDENCE:BEGIN scope=M0 schema=1 -->"
$script:EndMarker = "<!-- ORIGAMI3-ROADMAP-EVIDENCE:END scope=M0 -->"
$script:ExpectedTaskCounts = [ordered]@{
    "0-1" = 5
    "0-2" = 4
    "0-3" = 2
}
$script:TrackedTextCache = New-Object 'System.Collections.Generic.Dictionary[string,string]' ([System.StringComparer]::Ordinal)
$script:TrackedFileHashCache = New-Object 'System.Collections.Generic.Dictionary[string,string]' ([System.StringComparer]::Ordinal)
$script:PowerShellAstCache = New-Object 'System.Collections.Generic.Dictionary[string,object]' ([System.StringComparer]::Ordinal)
$script:TrackedPathCache = New-Object 'System.Collections.Generic.Dictionary[string,bool]' ([System.StringComparer]::Ordinal)
$script:TrackedSelectorCache = New-Object 'System.Collections.Generic.Dictionary[string,bool]' ([System.StringComparer]::Ordinal)
$script:GitSelectorCache = New-Object 'System.Collections.Generic.Dictionary[string,bool]' ([System.StringComparer]::Ordinal)

function Get-WholeRoadmapSnapshot {
    $snapshotScript = Join-Path $PSScriptRoot "get-roadmap-status.ps1"
    if (-not (Test-Path -LiteralPath $snapshotScript -PathType Leaf)) {
        throw "whole roadmap snapshot script is missing: $snapshotScript"
    }
    $powershellExe = (Get-Process -Id $PID).Path
    $global:LASTEXITCODE = 0
    $output = @(& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $snapshotScript -Format Json)
    $snapshotExitCode = $LASTEXITCODE
    if ($snapshotExitCode -ne 0) {
        throw "whole roadmap snapshot failed (exit=$snapshotExitCode)"
    }
    if ($output.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$output[0])) {
        throw "whole roadmap snapshot did not return exactly one JSON line (lines=$($output.Count))"
    }
    $snapshot = [string]$output[0] | ConvertFrom-Json
    if ([int]$snapshot.schema -ne 1 -or [string]$snapshot.scope -ne "whole" -or [bool]$snapshot.partial -or
        [int]$snapshot.audited -ne [int]$snapshot.total -or [int]$snapshot.unclassified -ne 0 -or
        [int]$snapshot.checked + [int]$snapshot.unchecked -ne [int]$snapshot.total) {
        throw "whole roadmap snapshot invariants are invalid"
    }
    return $snapshot
}
$script:D1EvidenceContract = [ordered]@{
    "M0.T0-1.C01" = [ordered]@{
        evidence = @("automated-check:CHECK.CURRENT-STATUS.WORKSPACE-MEMBERS")
        sources = @(
            "file:Cargo.toml :: section:[workspace]/field:members",
            "file:scripts/generate-current-status.ps1 :: function:Get-WorkspaceMembers"
        )
    }
    "M0.T0-1.C02" = [ordered]@{
        evidence = @("manual-acceptance:MANUAL.M0.T0-1.C02.CRATE-SCAFFOLD")
        sources = @(
            "git:19bc6ebd0009e62a15efc69f5eb17a7bdcfe6dbd :: tree:Cargo.toml+seven crate Cargo.toml/src/lib.rs",
            "file:crates/ori3-export/Cargo.toml :: section:[dependencies]/fields:resvg,svg2pdf",
            "file:docs/progress.md :: heading:## 2026-08-05 - Task 0-1 - 計算部品のフォルダ構成と空の部品一式を作成"
        )
    }
    "M0.T0-1.C03" = [ordered]@{
        evidence = @("manual-acceptance:MANUAL.M0.T0-1.C03.DEPENDENCY-BASELINE")
        sources = @(
            "file:Cargo.toml :: section:[workspace.dependencies]",
            "file:Cargo.lock :: lockfile",
            "file:docs/progress.md :: heading:## 2026-08-05 - Task 0-1 - 計算部品のフォルダ構成と空の部品一式を作成"
        )
    }
    "M0.T0-1.C04" = [ordered]@{
        evidence = @(
            "automated-check:CHECK.LOCAL.RUST-WORKSPACE-TEST",
            "automated-check:CHECK.LOCAL.RUST-WORKSPACE-CLIPPY"
        )
        sources = @(
            "file:scripts/check.ps1 :: Invoke-Check label:(1/5) cargo test --workspace",
            "file:scripts/check.ps1 :: Invoke-Check label:(2/5) cargo clippy --workspace --all-targets -- -D warnings",
            "file:.github/workflows/ci.yml :: jobs:checks+performance"
        )
    }
    "M0.T0-1.C05" = [ordered]@{
        evidence = @("manual-acceptance:MANUAL.M0.T0-1.C05.COMMIT-PUSH")
        sources = @(
            "git:19bc6ebd0009e62a15efc69f5eb17a7bdcfe6dbd :: subject:計算部品を置くためのフォルダ構成と空の部品一式を作成",
            "git:19bc6ebd0009e62a15efc69f5eb17a7bdcfe6dbd :: ancestor-of:refs/remotes/origin/main",
            "file:docs/progress.md :: heading:## 2026-08-05 - Task 0-1 - 計算部品のフォルダ構成と空の部品一式を作成"
        )
    }
    "M0.T0-2.C01" = [ordered]@{
        evidence = @("manual-acceptance:MANUAL.M0.T0-2.C01.TAURI-SCAFFOLD")
        sources = @(
            "git:e231579ea7210b9f91a9a7e4987e389f78445acc :: tree:apps/desktop Tauri+React+TypeScript+Vite scaffold",
            "file:apps/desktop/package.json :: dependencies:three,zustand+devDependencies:@types/three",
            "file:docs/progress.md :: heading:## 2026-08-05 - Task 0-2 - アプリの画面が起動する最小の土台を作成"
        )
    }
    "M0.T0-2.C02" = [ordered]@{
        evidence = @(
            "automated-check:CHECK.CURRENT-STATUS.WORKSPACE-MEMBERS",
            "automated-check:CHECK.CURRENT-STATUS.TAURI-COMMANDS"
        )
        sources = @(
            "file:Cargo.toml :: section:[workspace]/field:members",
            "file:apps/desktop/src-tauri/src/lib.rs :: run/tauri::generate_handler!",
            "file:scripts/generate-current-status.ps1 :: functions:Get-WorkspaceMembers+Get-TauriCommands"
        )
    }
    "M0.T0-2.C03" = [ordered]@{
        evidence = @("manual-acceptance:MANUAL.M0.T0-2.C03.TAURI-LAUNCH-TITLE")
        sources = @(
            "file:apps/desktop/src-tauri/tauri.conf.json :: /build/beforeDevCommand",
            "file:apps/desktop/src-tauri/tauri.conf.json :: /app/windows/0/title",
            "file:docs/progress.md :: heading:## 2026-08-05 - Task 0-2 - アプリの画面が起動する最小の土台を作成"
        )
    }
    "M0.T0-2.C04" = [ordered]@{
        evidence = @("manual-acceptance:MANUAL.M0.T0-2.C04.COMMIT-PUSH")
        sources = @(
            "git:e231579ea7210b9f91a9a7e4987e389f78445acc :: subject:アプリの画面が起動する最小の土台を作成",
            "git:e231579ea7210b9f91a9a7e4987e389f78445acc :: ancestor-of:refs/remotes/origin/main",
            "file:docs/progress.md :: heading:## 2026-08-05 - Task 0-2 - アプリの画面が起動する最小の土台を作成"
        )
    }
    "M0.T0-3.C01" = [ordered]@{
        evidence = @(
            "automated-check:CHECK.LOCAL.ALL-FIVE",
            "manual-acceptance:MANUAL.M0.T0-3.C01.ALL-FIVE-RUN"
        )
        sources = @(
            "file:scripts/check.ps1 :: function:Invoke-Check+labels:(1/5)..(5/5)",
            "file:docs/progress.md :: heading:## 2026-08-05 - M0完了 - 基盤のレビューと修正が完了",
            "file:docs/progress.md :: heading:## 2026-08-05 - Task 0-3 - 全ての自動チェックを一度に実行できる仕組みを追加",
            "file:docs/progress.md :: heading:## 2026-08-05 - Task 1-6 - 展開図を描く画面(方眼・吸着・線の描画)を追加"
        )
    }
    "M0.T0-3.C02" = [ordered]@{
        evidence = @("manual-acceptance:MANUAL.M0.T0-3.C02.COMMIT-PUSH")
        sources = @(
            "git:b89a9b6a5343715960cbc749ab0d876881438ffc :: subject:全ての自動チェックを一度に実行できる仕組みを追加",
            "git:b89a9b6a5343715960cbc749ab0d876881438ffc :: ancestor-of:refs/remotes/origin/main",
            "file:docs/progress.md :: heading:## 2026-08-05 - Task 0-3 - 全ての自動チェックを一度に実行できる仕組みを追加"
        )
    }
}

function Normalize-RepositoryRelativePath {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) { throw "empty repository-relative path" }
    $normalized = $Path.Replace("\", "/")
    while ($normalized.StartsWith("./", [System.StringComparison]::Ordinal)) {
        $normalized = $normalized.Substring(2)
    }
    if ([System.IO.Path]::IsPathRooted($Path) -or
        [string]::IsNullOrWhiteSpace($normalized) -or
        [string]::Equals($normalized, "..", [System.StringComparison]::Ordinal) -or
        $normalized.StartsWith("../", [System.StringComparison]::Ordinal) -or
        $normalized.Contains("/../") -or
        $normalized.Contains(":") -or
        $normalized.Contains([char]0)) {
        throw "unsafe repository-relative path: $Path"
    }
    return $normalized
}

function Assert-AllowedInputPath {
    param([string]$RelativePath)

    $normalized = Normalize-RepositoryRelativePath $RelativePath
    if ([string]::Equals($normalized, $script:ForbiddenRelativePath, [System.StringComparison]::Ordinal) -or
        @($script:ForbiddenPrefixes | Where-Object { $normalized.StartsWith($_, [System.StringComparison]::Ordinal) }).Count -gt 0) {
        throw "forbidden roadmap-link input path (rejected before read): $normalized"
    }
    return $normalized
}

function Test-GitTrackedPath {
    param([string]$RelativePath)

    if ($script:TrackedPathCache.ContainsKey($RelativePath)) {
        return $script:TrackedPathCache[$RelativePath]
    }
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = [int]::MinValue
        $output = @(& git -C $script:Root -c core.quotePath=false ls-files --error-unmatch -- $RelativePath 2>$null)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    $tracked = $exitCode -eq 0 -and $output.Count -eq 1 -and
        [string]::Equals([string]$output[0], $RelativePath, [System.StringComparison]::Ordinal)
    $script:TrackedPathCache.Add($RelativePath, [bool]$tracked)
    return $tracked
}

function Assert-NoReparsePointInPath {
    param([string]$FullPath)

    $rootItem = Get-Item -LiteralPath $script:Root -Force
    if (($rootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "repository root is a reparse point"
    }
    $relative = $FullPath.Substring($script:Root.Length).TrimStart([char[]]"\/")
    $current = $script:Root
    foreach ($segment in $relative.Split([char[]]"\/", [System.StringSplitOptions]::RemoveEmptyEntries)) {
        $current = Join-Path $current $segment
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "roadmap-link input/output path crosses a reparse point: $current"
            }
        }
    }
}

function Assert-TrackedInputPath {
    param([string]$RelativePath)

    $normalized = Assert-AllowedInputPath $RelativePath
    if (-not (Test-GitTrackedPath $normalized)) {
        throw "roadmap-link input is not tracked; none of its contents were read: $normalized"
    }
    $fullPath = [System.IO.Path]::GetFullPath((Join-Path $script:Root ($normalized.Replace("/", "\"))))
    if (-not $fullPath.StartsWith($script:Root + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "roadmap-link input resolves outside repository: $normalized"
    }
    Assert-NoReparsePointInPath $fullPath
    if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
        throw "tracked roadmap-link input is missing: $normalized"
    }
    return $normalized
}

function Read-TrackedUtf8File {
    param([string]$RelativePath)

    $normalized = Assert-TrackedInputPath $RelativePath
    $fullPath = Join-Path $script:Root ($normalized.Replace("/", "\"))
    $bytes = [System.IO.File]::ReadAllBytes($fullPath)
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
        throw "roadmap-link input must be UTF-8 without BOM: $normalized"
    }
    return [PSCustomObject][ordered]@{
        RelativePath = $normalized
        Bytes = $bytes
        Text = $script:Utf8NoBom.GetString($bytes)
    }
}

function Read-TrackedSourceText {
    param([string]$RelativePath)

    $normalized = Assert-TrackedInputPath $RelativePath
    if ($script:TrackedTextCache.ContainsKey($normalized)) {
        return $script:TrackedTextCache[$normalized]
    }
    $fullPath = Join-Path $script:Root ($normalized.Replace("/", "\"))
    $bytes = [System.IO.File]::ReadAllBytes($fullPath)
    $offset = 0
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
        $offset = 3
    }
    $textValue = $script:Utf8NoBom.GetString($bytes, $offset, $bytes.Length - $offset)
    $script:TrackedTextCache.Add($normalized, $textValue)
    $script:TrackedFileHashCache.Add($normalized, (Get-Sha256Hex $bytes))
    return $textValue
}

function Assert-TrackedSourceSnapshotUnchanged {
    foreach ($relativePath in $script:TrackedFileHashCache.Keys) {
        $normalized = Assert-TrackedInputPath $relativePath
        $fullPath = Join-Path $script:Root ($normalized.Replace("/", "\"))
        $actualHash = Get-Sha256Hex ([System.IO.File]::ReadAllBytes($fullPath))
        if (-not [string]::Equals($actualHash, $script:TrackedFileHashCache[$relativePath], [System.StringComparison]::Ordinal)) {
            throw "tracked roadmap evidence source changed during collection: $relativePath"
        }
    }
}

function Test-ExactTextLine {
    param([string]$Text, [string]$Expected)

    $count = 0
    foreach ($line in @($Text -split "`r?`n")) {
        if ([string]::Equals($line, $Expected, [System.StringComparison]::Ordinal)) { $count++ }
    }
    return $count -eq 1
}

function Get-ExactTomlSectionText {
    param([string]$Text, [string]$Header)

    $lines = @($Text -split "`r?`n")
    $start = -1
    for ($index = 0; $index -lt $lines.Count; $index++) {
        if ([string]::Equals($lines[$index].Trim(), $Header, [System.StringComparison]::Ordinal)) {
            if ($start -ge 0) { return $null }
            $start = $index
        }
    }
    if ($start -lt 0) { return $null }
    $end = $lines.Count
    for ($index = $start + 1; $index -lt $lines.Count; $index++) {
        if ($lines[$index].Trim() -match '^\[.+\]$') { $end = $index; break }
    }
    if ($end -le ($start + 1)) { return "" }
    return [string]::Join("`n", [string[]]$lines[($start + 1)..($end - 1)])
}

function Get-TrackedPowerShellAst {
    param([string]$RelativePath)

    $normalized = Assert-TrackedInputPath $RelativePath
    if ($script:PowerShellAstCache.ContainsKey($normalized)) {
        return $script:PowerShellAstCache[$normalized]
    }
    $sourceText = Read-TrackedSourceText $normalized
    $tokens = $null
    $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseInput($sourceText, [ref]$tokens, [ref]$errors)
    if ($errors.Count -ne 0) {
        throw "tracked PowerShell evidence source has parse errors: $normalized"
    }
    $script:PowerShellAstCache.Add($normalized, $ast)
    return $ast
}

function Test-PowerShellFunctionExistsOnce {
    param([string]$RelativePath, [string]$FunctionName)

    $ast = Get-TrackedPowerShellAst $RelativePath
    $nodes = @($ast.FindAll({
        param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
            [string]::Equals($node.Name, $FunctionName, [System.StringComparison]::OrdinalIgnoreCase)
    }, $true))
    return $nodes.Count -eq 1
}

function Test-InvokeCheckInvocationExistsOnce {
    param([string]$Label, [string]$CommandName, [string]$ArgumentsVariable)

    $ast = Get-TrackedPowerShellAst "scripts/check.ps1"
    $count = 0
    $commands = @($ast.FindAll({
        param($node)
        $node -is [System.Management.Automation.Language.CommandAst] -and
            [string]::Equals($node.GetCommandName(), "Invoke-Check", [System.StringComparison]::OrdinalIgnoreCase)
    }, $true))
    foreach ($command in $commands) {
        if ($command.CommandElements.Count -ne 4) { continue }
        $labelNode = $command.CommandElements[1]
        $commandNode = $command.CommandElements[2]
        $argumentNode = $command.CommandElements[3]
        if ($labelNode -is [System.Management.Automation.Language.StringConstantExpressionAst] -and
            $commandNode -is [System.Management.Automation.Language.StringConstantExpressionAst] -and
            $argumentNode -is [System.Management.Automation.Language.VariableExpressionAst] -and
            [string]::Equals($labelNode.Value, $Label, [System.StringComparison]::Ordinal) -and
            [string]::Equals($commandNode.Value, $CommandName, [System.StringComparison]::Ordinal) -and
            [string]::Equals($argumentNode.VariablePath.UserPath, $ArgumentsVariable, [System.StringComparison]::Ordinal)) {
            $count++
        }
    }
    return $count -eq 1
}

function Test-InvokeCheckFailureContract {
    $ast = Get-TrackedPowerShellAst "scripts/check.ps1"
    $functions = @($ast.FindAll({
        param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
            [string]::Equals($node.Name, "Invoke-Check", [System.StringComparison]::OrdinalIgnoreCase)
    }, $true))
    if ($functions.Count -ne 1) { return $false }
    $body = $functions[0].Body.Extent.Text
    foreach ($pattern in @(
        '(?m)^\s*& \$Command @CommandArgs\s*$',
        '(?m)^\s*exit 1\s*$',
        '(?m)^\s*if \(\$LASTEXITCODE -ne 0\) \{\s*$',
        '(?m)^\s*exit \$LASTEXITCODE\s*$'
    )) {
        if ([regex]::Matches($body, $pattern).Count -ne 1) { return $false }
    }
    return $true
}

function ConvertFrom-TrackedJson {
    param([string]$RelativePath)

    if (-not ("System.Web.Script.Serialization.JavaScriptSerializer" -as [type])) {
        Add-Type -AssemblyName System.Web.Extensions
    }
    $serializer = New-Object System.Web.Script.Serialization.JavaScriptSerializer
    $serializer.MaxJsonLength = [int]::MaxValue
    try {
        return $serializer.DeserializeObject((Read-TrackedSourceText $RelativePath))
    }
    catch {
        throw "tracked JSON evidence source cannot be parsed: $RelativePath"
    }
}

function Test-DictionaryKey {
    param($Dictionary, [string]$Key)
    return $null -ne $Dictionary -and $Dictionary.ContainsKey($Key)
}

function Invoke-GitReadOnlyCapture {
    param([string[]]$Arguments)

    $previousPreference = $ErrorActionPreference
    $exitCode = [int]::MinValue
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = [int]::MinValue
        $output = @(& git -C $script:Root @Arguments 2>$null)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    return [PSCustomObject][ordered]@{ ExitCode = $exitCode; Output = @($output) }
}

function Test-GitCommitExists {
    param([string]$Revision)

    $result = Invoke-GitReadOnlyCapture @("cat-file", "-t", $Revision)
    return $result.ExitCode -eq 0 -and $result.Output.Count -eq 1 -and
        [string]::Equals([string]$result.Output[0], "commit", [System.StringComparison]::Ordinal)
}

function Test-GitCommitSubject {
    param([string]$Revision, [string]$ExpectedSubject)

    $result = Invoke-GitReadOnlyCapture @("show", "-s", "--format=%s", $Revision)
    return $result.ExitCode -eq 0 -and $result.Output.Count -eq 1 -and
        [string]::Equals([string]$result.Output[0], $ExpectedSubject, [System.StringComparison]::Ordinal)
}

function Test-GitAncestorOfRef {
    param([string]$Revision, [string]$Reference)

    $result = Invoke-GitReadOnlyCapture @("merge-base", "--is-ancestor", $Revision, $Reference)
    if ($result.ExitCode -eq 0) { return $true }
    if ($result.ExitCode -eq 1) { return $false }
    throw "git ancestor evidence command failed: $Revision :: $Reference"
}

function Test-GitCommitPaths {
    param([string]$Revision, [string[]]$Paths)

    foreach ($path in $Paths) {
        $result = Invoke-GitReadOnlyCapture @("cat-file", "-e", ("{0}:{1}" -f $Revision, $path))
        if ($result.ExitCode -ne 0) { return $false }
    }
    return $true
}

function Test-GitSourceSelector {
    param([string]$Revision, [string]$Selector)

    if (-not (Test-GitCommitExists $Revision)) { return $false }
    $identity = $Revision + "`n" + $Selector
    switch -Exact -CaseSensitive ($identity) {
        "19bc6ebd0009e62a15efc69f5eb17a7bdcfe6dbd`ntree:Cargo.toml+seven crate Cargo.toml/src/lib.rs" {
            $paths = New-Object System.Collections.Generic.List[string]
            $paths.Add("Cargo.toml")
            foreach ($crate in @("model", "geometry", "cp", "rigid", "layers", "propose", "export")) {
                $paths.Add("crates/ori3-$crate/Cargo.toml")
                $paths.Add("crates/ori3-$crate/src/lib.rs")
            }
            return Test-GitCommitPaths $Revision $paths.ToArray()
        }
        "19bc6ebd0009e62a15efc69f5eb17a7bdcfe6dbd`nsubject:計算部品を置くためのフォルダ構成と空の部品一式を作成" {
            return Test-GitCommitSubject $Revision "計算部品を置くためのフォルダ構成と空の部品一式を作成"
        }
        "19bc6ebd0009e62a15efc69f5eb17a7bdcfe6dbd`nancestor-of:refs/remotes/origin/main" {
            return Test-GitAncestorOfRef $Revision "refs/remotes/origin/main"
        }
        "e231579ea7210b9f91a9a7e4987e389f78445acc`ntree:apps/desktop Tauri+React+TypeScript+Vite scaffold" {
            return Test-GitCommitPaths $Revision @(
                "apps/desktop/package.json",
                "apps/desktop/src/App.tsx",
                "apps/desktop/src/main.tsx",
                "apps/desktop/src-tauri/Cargo.toml",
                "apps/desktop/src-tauri/src/lib.rs",
                "apps/desktop/vite.config.ts"
            )
        }
        "e231579ea7210b9f91a9a7e4987e389f78445acc`nsubject:アプリの画面が起動する最小の土台を作成" {
            return Test-GitCommitSubject $Revision "アプリの画面が起動する最小の土台を作成"
        }
        "e231579ea7210b9f91a9a7e4987e389f78445acc`nancestor-of:refs/remotes/origin/main" {
            return Test-GitAncestorOfRef $Revision "refs/remotes/origin/main"
        }
        "b89a9b6a5343715960cbc749ab0d876881438ffc`nsubject:全ての自動チェックを一度に実行できる仕組みを追加" {
            return Test-GitCommitSubject $Revision "全ての自動チェックを一度に実行できる仕組みを追加"
        }
        "b89a9b6a5343715960cbc749ab0d876881438ffc`nancestor-of:refs/remotes/origin/main" {
            return Test-GitAncestorOfRef $Revision "refs/remotes/origin/main"
        }
        default { throw "unsupported 7-D1 git evidence selector: $Revision :: $Selector" }
    }
}

function Get-ExactYamlJobBlock {
    param([string]$Text, [string]$JobName)

    $lines = @($Text -split "`r?`n")
    $heading = "  ${JobName}:"
    $start = -1
    for ($index = 0; $index -lt $lines.Count; $index++) {
        if ([string]::Equals($lines[$index], $heading, [System.StringComparison]::Ordinal)) {
            if ($start -ge 0) { return $null }
            $start = $index
        }
    }
    if ($start -lt 0) { return $null }
    $end = $lines.Count
    for ($index = $start + 1; $index -lt $lines.Count; $index++) {
        if ($lines[$index] -match '^  [A-Za-z0-9_\-]+:\s*$') { $end = $index; break }
    }
    return [string]::Join("`n", [string[]]$lines[$start..($end - 1)])
}

function Test-CiRustChecksAndPerformanceComplements {
    param([string]$Text)

    $checks = Get-ExactYamlJobBlock $Text "checks"
    $performance = Get-ExactYamlJobBlock $Text "performance"
    if ($null -eq $checks -or $null -eq $performance) { return $false }
    foreach ($line in @(
        "        run: cargo test --workspace --no-fail-fast -- --skip surface_order_179_999_to_180_all_110_creases --skip surface_order_exact_endpoint_is_rank_stable_for_previous_19 --skip completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten --skip named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten --skip a_safe_coincident_partial_network_appears_after_the_first_fold --skip the_heaviest_proposal_never_hits_the_time_limit",
        "        run: cargo clippy --workspace --all-targets -- -D warnings"
    )) {
        if (-not (Test-ExactTextLine $checks $line)) { return $false }
    }
    foreach ($line in @(
        "        run: cargo test --release -p desktop --lib surface_order_179_999_to_180_all_110_creases -- --nocapture",
        "        run: cargo test --release -p desktop --lib surface_order_exact_endpoint_is_rank_stable_for_previous_19 -- --nocapture",
        "        run: cargo test --release -p ori3-propose --test acceptance -- completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten --exact --nocapture",
        "        run: cargo test --release -p ori3-propose --test end_to_end -- named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten --exact --nocapture",
        "        run: cargo test --release -p ori3-propose --test acceptance -- a_safe_coincident_partial_network_appears_after_the_first_fold --exact --nocapture",
        "        run: cargo test --release -p desktop --lib the_heaviest_proposal_never_hits_the_time_limit -- --nocapture"
    )) {
        if (-not (Test-ExactTextLine $performance $line)) { return $false }
    }
    return $true
}

function Get-BalancedJsonContainerAt {
    param([string]$Text, [int]$StartIndex)

    if ($StartIndex -lt 0 -or $StartIndex -ge $Text.Length) { return $null }
    $open = $Text[$StartIndex]
    if ($open -eq '{') { $close = '}' }
    elseif ($open -eq '[') { $close = ']' }
    else { return $null }
    $depth = 0
    $inString = $false
    $escaped = $false
    for ($index = $StartIndex; $index -lt $Text.Length; $index++) {
        $character = $Text[$index]
        if ($inString) {
            if ($escaped) { $escaped = $false }
            elseif ($character -eq '\') { $escaped = $true }
            elseif ($character -eq '"') { $inString = $false }
            continue
        }
        if ($character -eq '"') { $inString = $true; continue }
        if ($character -eq $open) { $depth++ }
        elseif ($character -eq $close) {
            $depth--
            if ($depth -eq 0) { return $Text.Substring($StartIndex, $index - $StartIndex + 1) }
            if ($depth -lt 0) { return $null }
        }
    }
    return $null
}

function Get-UniqueJsonPropertyContainer {
    param([string]$Text, [string]$PropertyName, [char]$ExpectedOpen)

    $pattern = '"' + [regex]::Escape($PropertyName) + '"\s*:'
    $matches = [regex]::Matches($Text, $pattern)
    if ($matches.Count -ne 1) { return $null }
    $index = $matches[0].Index + $matches[0].Length
    while ($index -lt $Text.Length -and [char]::IsWhiteSpace($Text[$index])) { $index++ }
    if ($index -ge $Text.Length -or $Text[$index] -ne $ExpectedOpen) { return $null }
    return Get-BalancedJsonContainerAt $Text $index
}

function Get-FirstJsonArrayObject {
    param([string]$ArrayText)

    if ([string]::IsNullOrWhiteSpace($ArrayText) -or $ArrayText[0] -ne '[') { return $null }
    $index = 1
    while ($index -lt $ArrayText.Length -and [char]::IsWhiteSpace($ArrayText[$index])) { $index++ }
    if ($index -ge $ArrayText.Length -or $ArrayText[$index] -ne '{') { return $null }
    return Get-BalancedJsonContainerAt $ArrayText $index
}

function Get-MarkdownHeadingSectionText {
    param([string]$Text, [string]$Heading)

    $lines = @($Text -split "`r?`n")
    $start = -1
    for ($index = 0; $index -lt $lines.Count; $index++) {
        if ([string]::Equals($lines[$index], $Heading, [System.StringComparison]::Ordinal)) {
            if ($start -ge 0) { return $null }
            $start = $index
        }
    }
    if ($start -lt 0) { return $null }
    $end = $lines.Count
    for ($index = $start + 1; $index -lt $lines.Count; $index++) {
        if ($lines[$index].StartsWith("## ", [System.StringComparison]::Ordinal)) { $end = $index; break }
    }
    return [string]::Join("`n", [string[]]$lines[$start..($end - 1)])
}

function Test-D1ProgressSectionEvidence {
    param([string]$Text, [string]$Heading)

    $section = Get-MarkdownHeadingSectionText $Text $Heading
    if ($null -eq $section) { return $false }
    switch -Exact -CaseSensitive ($Heading) {
        "## 2026-08-05 - Task 0-1 - 計算部品のフォルダ構成と空の部品一式を作成" {
            $required = @(
                "計算コアを置く7つの部品",
                "共通で使う外部部品の版数を1か所で管理",
                "自動テストと静的検査(警告ゼロ扱い)が通ることを確認",
                "画像書き出し用の外部部品は使う段階(M4)まで追加しない方針"
            )
        }
        "## 2026-08-05 - Task 0-2 - アプリの画面が起動する最小の土台を作成" {
            $required = @(
                "ウィンドウ題名もORIGAMI3",
                "アプリ側の計算部分を全体のビルド管理に組み込んだ",
                "できあがったアプリ本体が起動することを確認済み"
            )
        }
        "## 2026-08-05 - M0完了 - 基盤のレビューと修正が完了" {
            $required = @(
                "検査道具が起動できなかった場合に合格扱いになる抜け道",
                "M0完了時チェック: 全自動検査合格"
            )
        }
        "## 2026-08-05 - Task 0-3 - 全ての自動チェックを一度に実行できる仕組みを追加" {
            $required = @(
                "4つの検査(計算部分のテスト・静的検査、画面部分のビルド・文法検査)",
                "失敗があれば非0で終了するスクリプト",
                "手動実行で全検査の合格を確認済み"
            )
        }
        "## 2026-08-05 - Task 1-6 - 展開図を描く画面(方眼・吸着・線の描画)を追加" {
            $required = @(
                "検査の仕組みにテスト実行(vitest)を5番目として追加",
                "全検査合格(Rustテスト・静的検査・画面ビルド・文法検査・画面テスト)"
            )
        }
        default { throw "unsupported 7-D1 progress evidence heading: $Heading" }
    }
    foreach ($literal in $required) {
        if ($section.IndexOf($literal, [System.StringComparison]::Ordinal) -lt 0) { return $false }
    }
    return $true
}

function Test-TrackedSourceSelector {
    param([string]$RelativePath, [string]$Selector)

    $normalized = Assert-TrackedInputPath $RelativePath
    $identity = $normalized + "`n" + $Selector
    switch -Exact -CaseSensitive ($identity) {
        "Cargo.toml`nsection:[workspace]/field:members" {
            $sourceText = Read-TrackedSourceText $normalized
            $section = Get-ExactTomlSectionText $sourceText "[workspace]"
            if ($null -eq $section) { return $false }
            $members = [regex]::Match($section, '(?ms)^members\s*=\s*\[(?<body>.*?)\]')
            return $members.Success -and
                [regex]::Matches($section, '(?m)^members\s*=\s*\[').Count -eq 1 -and
                [regex]::Matches($members.Groups['body'].Value, '"apps/desktop/src-tauri"').Count -eq 1
        }
        "Cargo.toml`nsection:[workspace.dependencies]" {
            $sourceText = Read-TrackedSourceText $normalized
            $section = Get-ExactTomlSectionText $sourceText "[workspace.dependencies]"
            if ($null -eq $section) { return $false }
            foreach ($name in @("glam", "serde", "serde_json", "thiserror")) {
                if ([regex]::Matches($section, ("(?m)^{0}\s*=\s*\S+" -f [regex]::Escape($name))).Count -ne 1) { return $false }
            }
            return $true
        }
        "Cargo.lock`nlockfile" {
            $sourceText = Read-TrackedSourceText $normalized
            return [regex]::Matches($sourceText, '(?m)^version = 4\r?$').Count -eq 1 -and
                [regex]::Matches($sourceText, '(?m)^\[\[package\]\]\r?$').Count -gt 0
        }
        "crates/ori3-export/Cargo.toml`nsection:[dependencies]/fields:resvg,svg2pdf" {
            $sourceText = Read-TrackedSourceText $normalized
            $section = Get-ExactTomlSectionText $sourceText "[dependencies]"
            if ($null -eq $section) { return $false }
            return [regex]::Matches($section, '(?m)^resvg\s*=\s*\S+').Count -eq 1 -and
                [regex]::Matches($section, '(?m)^svg2pdf\s*=\s*\S+').Count -eq 1
        }
        "scripts/generate-current-status.ps1`nfunction:Get-WorkspaceMembers" {
            return Test-PowerShellFunctionExistsOnce $normalized "Get-WorkspaceMembers"
        }
        "scripts/generate-current-status.ps1`nfunctions:Get-WorkspaceMembers+Get-TauriCommands" {
            return (Test-PowerShellFunctionExistsOnce $normalized "Get-WorkspaceMembers") -and
                (Test-PowerShellFunctionExistsOnce $normalized "Get-TauriCommands")
        }
        "scripts/check.ps1`nInvoke-Check label:(1/5) cargo test --workspace" {
            return Test-InvokeCheckInvocationExistsOnce "(1/5) cargo test --workspace" "cargo" "rustW4Arguments"
        }
        "scripts/check.ps1`nInvoke-Check label:(2/5) cargo clippy --workspace --all-targets -- -D warnings" {
            return Test-InvokeCheckInvocationExistsOnce "(2/5) cargo clippy --workspace --all-targets -- -D warnings" "cargo" "clippyArguments"
        }
        "scripts/check.ps1`nfunction:Invoke-Check+labels:(1/5)..(5/5)" {
            if (-not (Test-PowerShellFunctionExistsOnce $normalized "Invoke-Check") -or
                -not (Test-InvokeCheckFailureContract)) { return $false }
            return (Test-InvokeCheckInvocationExistsOnce "(1/5) cargo test --workspace" "cargo" "rustW4Arguments") -and
                (Test-InvokeCheckInvocationExistsOnce "(2/5) cargo clippy --workspace --all-targets -- -D warnings" "cargo" "clippyArguments") -and
                (Test-InvokeCheckInvocationExistsOnce "(3/5) npm run build (apps/desktop)" "npm" "npmBuildArguments") -and
                (Test-InvokeCheckInvocationExistsOnce "(4/5) npm run lint (apps/desktop)" "npm" "npmLintArguments") -and
                (Test-InvokeCheckInvocationExistsOnce "(5/5) npm run test (apps/desktop)" "npm" "npmTestArguments")
        }
        ".github/workflows/ci.yml`njobs:checks+performance" {
            $sourceText = Read-TrackedSourceText $normalized
            return Test-CiRustChecksAndPerformanceComplements $sourceText
        }
        "apps/desktop/package.json`ndependencies:three,zustand+devDependencies:@types/three" {
            $json = ConvertFrom-TrackedJson $normalized
            $sourceText = Read-TrackedSourceText $normalized
            $dependenciesObject = Get-UniqueJsonPropertyContainer $sourceText "dependencies" '{'
            $devDependenciesObject = Get-UniqueJsonPropertyContainer $sourceText "devDependencies" '{'
            return (Test-DictionaryKey $json "dependencies") -and
                (Test-DictionaryKey $json["dependencies"] "three") -and
                (Test-DictionaryKey $json["dependencies"] "zustand") -and
                (Test-DictionaryKey $json "devDependencies") -and
                (Test-DictionaryKey $json["devDependencies"] "@types/three") -and
                $null -ne $dependenciesObject -and
                $null -ne $devDependenciesObject -and
                [regex]::Matches($dependenciesObject, '"three"\s*:').Count -eq 1 -and
                [regex]::Matches($dependenciesObject, '"zustand"\s*:').Count -eq 1 -and
                [regex]::Matches($devDependenciesObject, '"@types/three"\s*:').Count -eq 1
        }
        "apps/desktop/src-tauri/src/lib.rs`nrun/tauri::generate_handler!" {
            $sourceText = Read-TrackedSourceText $normalized
            $handlers = [regex]::Matches($sourceText, '(?s)\.invoke_handler\s*\(\s*tauri::generate_handler!\s*\[(?<body>.*?)\]\s*\)')
            return [regex]::Matches($sourceText, '(?m)^pub\s+fn\s+run\s*\(\s*\)\s*\{').Count -eq 1 -and
                $handlers.Count -eq 1 -and
                [regex]::Matches($handlers[0].Groups['body'].Value, '\bgreet\b').Count -eq 0
        }
        "apps/desktop/src-tauri/tauri.conf.json`n/build/beforeDevCommand" {
            $json = ConvertFrom-TrackedJson $normalized
            $sourceText = Read-TrackedSourceText $normalized
            $buildObject = Get-UniqueJsonPropertyContainer $sourceText "build" '{'
            return (Test-DictionaryKey $json "build") -and
                (Test-DictionaryKey $json["build"] "beforeDevCommand") -and
                $null -ne $buildObject -and
                [regex]::Matches($buildObject, '"beforeDevCommand"\s*:').Count -eq 1 -and
                [string]::Equals([string]$json["build"]["beforeDevCommand"], "npm run dev", [System.StringComparison]::Ordinal)
        }
        "apps/desktop/src-tauri/tauri.conf.json`n/app/windows/0/title" {
            $json = ConvertFrom-TrackedJson $normalized
            $sourceText = Read-TrackedSourceText $normalized
            $appObject = Get-UniqueJsonPropertyContainer $sourceText "app" '{'
            $windowsArray = if ($null -ne $appObject) { Get-UniqueJsonPropertyContainer $appObject "windows" '[' } else { $null }
            $firstWindowObject = if ($null -ne $windowsArray) { Get-FirstJsonArrayObject $windowsArray } else { $null }
            return (Test-DictionaryKey $json "app") -and
                (Test-DictionaryKey $json["app"] "windows") -and
                @($json["app"]["windows"]).Count -gt 0 -and
                (Test-DictionaryKey $json["app"]["windows"][0] "title") -and
                $null -ne $firstWindowObject -and
                [regex]::Matches($firstWindowObject, '"title"\s*:').Count -eq 1 -and
                [string]::Equals([string]$json["app"]["windows"][0]["title"], "ORIGAMI3", [System.StringComparison]::Ordinal)
        }
        default {
            if ([string]::Equals($normalized, "docs/progress.md", [System.StringComparison]::Ordinal) -and
                $Selector.StartsWith("heading:", [System.StringComparison]::Ordinal)) {
                $heading = $Selector.Substring(8)
                $allowedHeadings = @(
                    "## 2026-08-05 - Task 0-1 - 計算部品のフォルダ構成と空の部品一式を作成",
                    "## 2026-08-05 - Task 0-2 - アプリの画面が起動する最小の土台を作成",
                    "## 2026-08-05 - M0完了 - 基盤のレビューと修正が完了",
                    "## 2026-08-05 - Task 0-3 - 全ての自動チェックを一度に実行できる仕組みを追加",
                    "## 2026-08-05 - Task 1-6 - 展開図を描く画面(方眼・吸着・線の描画)を追加"
                )
                if (-not @($allowedHeadings | Where-Object { [string]::Equals($_, $heading, [System.StringComparison]::Ordinal) }).Count) {
                    throw "unsupported 7-D1 progress selector: $Selector"
                }
                $sourceText = Read-TrackedSourceText $normalized
                return Test-D1ProgressSectionEvidence $sourceText $heading
            }
            throw "unsupported 7-D1 tracked evidence selector: $normalized :: $Selector"
        }
    }
}

function Test-CachedTrackedSourceSelector {
    param([string]$RelativePath, [string]$Selector)

    $identity = $RelativePath + "`n" + $Selector
    if ($script:TrackedSelectorCache.ContainsKey($identity)) {
        return $script:TrackedSelectorCache[$identity]
    }
    $valid = Test-TrackedSourceSelector $RelativePath $Selector
    $script:TrackedSelectorCache.Add($identity, [bool]$valid)
    return $valid
}

function Test-CachedGitSourceSelector {
    param([string]$Revision, [string]$Selector)

    $identity = $Revision + "`n" + $Selector
    if ($script:GitSelectorCache.ContainsKey($identity)) {
        return $script:GitSelectorCache[$identity]
    }
    $valid = Test-GitSourceSelector $Revision $Selector
    $script:GitSelectorCache.Add($identity, [bool]$valid)
    return $valid
}

function Get-Sha256Hex {
    param([byte[]]$Bytes)

    $sha = [System.Security.Cryptography.SHA256]::Create()
    try { $hash = $sha.ComputeHash($Bytes) }
    finally { $sha.Dispose() }
    return ([System.BitConverter]::ToString($hash)).Replace("-", "").ToLowerInvariant()
}

function Get-TextSha256 {
    param([string]$Text)
    return Get-Sha256Hex ($script:Utf8NoBom.GetBytes($Text))
}

function Get-ExactLineIndex {
    param([string[]]$Lines, [string]$Expected)

    $indices = New-Object System.Collections.Generic.List[int]
    for ($index = 0; $index -lt $Lines.Count; $index++) {
        if ([string]::Equals($Lines[$index], $Expected, [System.StringComparison]::Ordinal)) {
            $indices.Add($index)
        }
    }
    if ($indices.Count -ne 1) {
        throw "roadmap line must occur exactly once(count=$($indices.Count)): $Expected"
    }
    return $indices[0]
}

function Get-ExactMarkerBlock {
    param([string]$Text, [string]$Begin, [string]$End)

    $beginCount = [regex]::Matches($Text, [regex]::Escape($Begin)).Count
    $endCount = [regex]::Matches($Text, [regex]::Escape($End)).Count
    if ($beginCount -ne 1 -or $endCount -ne 1) {
        throw "roadmap evidence marker count differs(begin=$beginCount,end=$endCount)"
    }
    $beginIndex = $Text.IndexOf($Begin, [System.StringComparison]::Ordinal)
    $contentStart = $beginIndex + $Begin.Length
    $endIndex = $Text.IndexOf($End, $contentStart, [System.StringComparison]::Ordinal)
    if ($endIndex -lt $contentStart) { throw "roadmap evidence markers are reversed or nested" }
    return $Text.Substring($contentStart, $endIndex - $contentStart).Trim("`r", "`n")
}

function ConvertFrom-EvidenceCell {
    param([string]$Cell, [string]$LinkId)

    $result = New-Object System.Collections.Generic.List[object]
    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::Ordinal)
    foreach ($entry in @($Cell -split '<br>')) {
        $match = [regex]::Match($entry.Trim(), '^(?<kind>自動|手動) `(?<id>[A-Z0-9][A-Z0-9.\-]+)`$')
        if (-not $match.Success) { throw "invalid evidence cell for ${LinkId}: $entry" }
        $id = $match.Groups['id'].Value
        if (-not $seen.Add($id)) { throw "duplicate evidence id for ${LinkId}: $id" }
        $kind = if ($match.Groups['kind'].Value -eq "自動") { "automated-check" } else { "manual-acceptance" }
        if ($kind -eq "automated-check" -and -not $id.StartsWith("CHECK.", [System.StringComparison]::Ordinal)) {
            throw "automated evidence id must start with CHECK. for ${LinkId}: $id"
        }
        if ($kind -eq "manual-acceptance" -and -not $id.StartsWith("MANUAL.", [System.StringComparison]::Ordinal)) {
            throw "manual acceptance id must start with MANUAL. for ${LinkId}: $id"
        }
        $result.Add([ordered]@{ kind = $kind; id = $id })
    }
    if ($result.Count -eq 0) { throw "zero evidence entries for $LinkId" }
    return $result.ToArray()
}

function ConvertFrom-SourceCell {
    param(
        [string]$Cell,
        [string]$LinkId,
        [System.Collections.Generic.List[string]]$Issues
    )

    $result = New-Object System.Collections.Generic.List[object]
    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::Ordinal)
    foreach ($entry in @($Cell -split '<br>')) {
        $match = [regex]::Match($entry.Trim(), '^`(?<locator>(?:file:[^`]+|git:[0-9a-f]{40}))` :: `(?<selector>[^`]+)`$')
        if (-not $match.Success) { throw "invalid authoritative source for ${LinkId}: $entry" }
        $locator = $match.Groups['locator'].Value
        $selector = $match.Groups['selector'].Value
        $identity = $locator + "`n" + $selector
        if (-not $seen.Add($identity)) { throw "duplicate authoritative source for ${LinkId}: $locator :: $selector" }
        if ($locator.StartsWith("file:", [System.StringComparison]::Ordinal)) {
            $path = $locator.Substring(5)
            if (-not (Test-CachedTrackedSourceSelector $path $selector)) {
                $Issues.Add("authoritative selector is not resolved for ${LinkId}: file:$path :: $selector")
            }
            $result.Add([ordered]@{ kind = "tracked-file"; path = $path; selector = $selector })
        }
        else {
            $revision = $locator.Substring(4)
            if (-not (Test-CachedGitSourceSelector $revision $selector)) {
                $Issues.Add("authoritative selector is not resolved for ${LinkId}: git:$revision :: $selector")
            }
            $result.Add([ordered]@{ kind = "git-commit-record"; revision = $revision; selector = $selector })
        }
    }
    if ($result.Count -eq 0) { throw "zero authoritative sources for $LinkId" }
    return $result.ToArray()
}

function ConvertFrom-RegistryBlock {
    param(
        [string]$Block,
        [System.Collections.Generic.List[string]]$Issues
    )

    $rows = New-Object 'System.Collections.Generic.Dictionary[string,object]' ([System.StringComparer]::Ordinal)
    $lines = @($Block -split "`r?`n")
    $headerSeen = $false
    $separatorSeen = $false
    foreach ($line in $lines) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        if ([string]::Equals($line, '| link ID | evidence | authoritative source | acceptance | progress | unresolved |', [System.StringComparison]::Ordinal)) {
            if ($headerSeen) { throw "duplicate roadmap evidence table header" }
            $headerSeen = $true
            continue
        }
        if ([string]::Equals($line, '|---|---|---|---|---|---|', [System.StringComparison]::Ordinal)) {
            if ($separatorSeen) { throw "duplicate roadmap evidence table separator" }
            $separatorSeen = $true
            continue
        }
        if (-not $line.StartsWith("|", [System.StringComparison]::Ordinal) -or -not $line.EndsWith("|", [System.StringComparison]::Ordinal)) {
            throw "unexpected line inside roadmap evidence marker: $line"
        }
        $cells = @($line.Substring(1, $line.Length - 2).Split('|') | ForEach-Object { $_.Trim() })
        if ($cells.Count -ne 6) { throw "roadmap evidence row must have 6 cells: $line" }
        $idMatch = [regex]::Match($cells[0], '^<a id="(?<anchor>roadmap-evidence-m0-t0-[123]-c\d{2})"></a>`(?<id>M0\.T0-[123]\.C\d{2})`$')
        if (-not $idMatch.Success) { throw "invalid roadmap evidence row id: $($cells[0])" }
        $id = $idMatch.Groups['id'].Value
        $expectedAnchor = "roadmap-evidence-" + $id.ToLowerInvariant().Replace(".", "-")
        if (-not [string]::Equals($idMatch.Groups['anchor'].Value, $expectedAnchor, [System.StringComparison]::Ordinal)) {
            throw "roadmap evidence anchor differs for $id"
        }
        if ($rows.ContainsKey($id)) { throw "duplicate roadmap evidence row id: $id" }
        $evidence = @(ConvertFrom-EvidenceCell $cells[1] $id)
        $sources = @(ConvertFrom-SourceCell $cells[2] $id $Issues)
        if ([string]::IsNullOrWhiteSpace($cells[3])) { throw "empty acceptance condition for $id" }
        $progressMatch = [regex]::Match($cells[4], '^(?<state>consistent|historical-evolution|contradiction|not-applicable) — (?<note>.+)$')
        if (-not $progressMatch.Success) { throw "invalid progress assessment for ${id}: $($cells[4])" }
        $unresolved = if ([string]::Equals($cells[5], "none", [System.StringComparison]::Ordinal)) {
            @()
        }
        else {
            @($cells[5] -split '<br>' | ForEach-Object { $_.Trim() } | Where-Object { $_.Length -gt 0 })
        }
        $rows.Add($id, [ordered]@{
            evidence = $evidence
            authoritative_sources = $sources
            acceptance = $cells[3]
            progress = [ordered]@{
                consistency = $progressMatch.Groups['state'].Value
                note = $progressMatch.Groups['note'].Value
            }
            unresolved = @($unresolved)
        })
    }
    if (-not $headerSeen -or -not $separatorSeen) { throw "roadmap evidence table header is incomplete" }
    return $rows
}

function Get-ExpectedLinkIds {
    $result = New-Object System.Collections.Generic.List[string]
    foreach ($task in $script:ExpectedTaskCounts.Keys) {
        for ($ordinal = 1; $ordinal -le [int]$script:ExpectedTaskCounts[$task]; $ordinal++) {
            $result.Add(("M0.T{0}.C{1:D2}" -f $task, $ordinal))
        }
    }
    return $result.ToArray()
}

function Assert-ExactOrdinalSequence {
    param([object[]]$Actual, [object[]]$Expected, [string]$Label)

    if ($Actual.Count -ne $Expected.Count) {
        throw "$Label count differs(actual=$($Actual.Count),expected=$($Expected.Count))"
    }
    for ($index = 0; $index -lt $Expected.Count; $index++) {
        if (-not [string]::Equals([string]$Actual[$index], [string]$Expected[$index], [System.StringComparison]::Ordinal)) {
            throw "$Label differs at index $index"
        }
    }
}

function Get-AuthoritativeSourceIdentity {
    param($Source)

    if ([string]::Equals([string]$Source.kind, "tracked-file", [System.StringComparison]::Ordinal)) {
        return "file:$($Source.path) :: $($Source.selector)"
    }
    if ([string]::Equals([string]$Source.kind, "git-commit-record", [System.StringComparison]::Ordinal)) {
        return "git:$($Source.revision) :: $($Source.selector)"
    }
    throw "unknown authoritative source kind in D1 contract"
}

function Assert-D1EvidenceContract {
    param($Registry)

    if ($script:D1EvidenceContract.Count -ne 11) { throw "D1 evidence contract must contain the delegated 11 links" }
    foreach ($id in $script:D1EvidenceContract.Keys) {
        if (-not $Registry.ContainsKey($id)) { throw "D1 evidence contract link is missing from registry: $id" }
        $row = $Registry[$id]
        $actualEvidence = @($row.evidence | ForEach-Object { "$($_.kind):$($_.id)" })
        $actualSources = @($row.authoritative_sources | ForEach-Object { Get-AuthoritativeSourceIdentity $_ })
        Assert-ExactOrdinalSequence $actualEvidence @($script:D1EvidenceContract[$id].evidence) "D1 evidence sequence for $id"
        Assert-ExactOrdinalSequence $actualSources @($script:D1EvidenceContract[$id].sources) "D1 source sequence for $id"
    }
}

function ConvertFrom-RoadmapText {
    param([string]$Text, [byte[]]$SourceBytes)

    $issues = New-Object System.Collections.Generic.List[string]
    $lines = @($Text -split "`r?`n")
    $m0Start = Get-ExactLineIndex $lines "## M0: プロジェクト基盤"
    $m1Start = Get-ExactLineIndex $lines "## M1: 展開図エディタ + 剛体折り(受け入れ: やっこさん)"
    if ($m1Start -le $m0Start) { throw "M0/M1 roadmap sections are reversed" }
    $registryBeginLine = Get-ExactLineIndex $lines $script:BeginMarker
    $registryEndLine = Get-ExactLineIndex $lines $script:EndMarker
    if ($registryBeginLine -le $m0Start -or $registryEndLine -le $registryBeginLine -or $registryEndLine -ge $m1Start) {
        throw "M0 evidence registry is outside the M0 section"
    }
    $m0ScopeText = [string]::Join("`n", [string[]]$lines[$m0Start..($m1Start - 1)])

    $registryBlock = Get-ExactMarkerBlock $Text $script:BeginMarker $script:EndMarker
    $registry = ConvertFrom-RegistryBlock $registryBlock $issues
    $expectedIds = @(Get-ExpectedLinkIds)
    $expectedSet = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::Ordinal)
    foreach ($id in $expectedIds) { [void]$expectedSet.Add($id) }
    foreach ($registryId in $registry.Keys) {
        if (-not $expectedSet.Contains($registryId)) { throw "orphan/unknown M0 evidence row: $registryId" }
    }
    Assert-D1EvidenceContract $registry

    $taskOrdinals = @{}
    $currentTask = ""
    $checkboxCount = 0
    $items = New-Object System.Collections.Generic.List[object]
    $linkedIds = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::Ordinal)
    $linkPattern = '^- \[(?<checked>[ xX])\] (?<text>.+?) — \[証拠:(?<id>M0\.T0-[123]\.C\d{2})\]\(#roadmap-evidence-(?<anchor>m0-t0-[123]-c\d{2})\) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=(?<markerId>M0\.T0-[123]\.C\d{2}) evidence=(?<evidence>[A-Z0-9.\-,]+) -->$'

    for ($lineIndex = $m0Start + 1; $lineIndex -lt $m1Start; $lineIndex++) {
        $line = $lines[$lineIndex]
        $taskMatch = [regex]::Match($line, '^### Task (?<task>0-[123]):')
        if ($taskMatch.Success) {
            $currentTask = $taskMatch.Groups['task'].Value
            if ($taskOrdinals.ContainsKey($currentTask)) { throw "duplicate M0 task heading: $currentTask" }
            $taskOrdinals[$currentTask] = 0
            continue
        }
        if (-not $line.StartsWith("- [", [System.StringComparison]::Ordinal)) { continue }
        if ([string]::IsNullOrWhiteSpace($currentTask) -or -not $script:ExpectedTaskCounts.Contains($currentTask)) {
            throw "M0 checkbox is outside Task 0-1..0-3 at line $($lineIndex + 1)"
        }
        $checkboxCount++
        $taskOrdinals[$currentTask] = [int]$taskOrdinals[$currentTask] + 1
        $expectedId = "M0.T$currentTask.C$(([int]$taskOrdinals[$currentTask]).ToString('D2'))"
        $match = [regex]::Match($line, $linkPattern)
        if (-not $match.Success) {
            $issues.Add("unlinked or malformed checkbox at line $($lineIndex + 1), expected id=$expectedId")
            continue
        }
        $id = $match.Groups['id'].Value
        if (-not [string]::Equals($id, $expectedId, [System.StringComparison]::Ordinal)) {
            $issues.Add("position-derived id mismatch at line $($lineIndex + 1): actual=$id expected=$expectedId")
        }
        if (-not [string]::Equals($match.Groups['markerId'].Value, $id, [System.StringComparison]::Ordinal)) {
            throw "visible/marker link id differs at line $($lineIndex + 1)"
        }
        $expectedAnchor = $id.ToLowerInvariant().Replace(".", "-")
        if (-not [string]::Equals($match.Groups['anchor'].Value, $expectedAnchor, [System.StringComparison]::Ordinal)) {
            throw "visible roadmap evidence anchor differs at line $($lineIndex + 1)"
        }
        if (-not $linkedIds.Add($id)) { throw "duplicate roadmap checkbox link id: $id" }
        if (-not $registry.ContainsKey($id)) {
            $issues.Add("checkbox link has no registry row: $id")
            continue
        }
        $row = $registry[$id]
        $markerEvidence = @($match.Groups['evidence'].Value.Split(','))
        $registryEvidence = @($row.evidence | ForEach-Object { $_.id })
        if ($markerEvidence.Count -ne $registryEvidence.Count) {
            $issues.Add("marker/registry evidence count differs for $id")
        }
        else {
            for ($evidenceIndex = 0; $evidenceIndex -lt $markerEvidence.Count; $evidenceIndex++) {
                if (-not [string]::Equals($markerEvidence[$evidenceIndex], $registryEvidence[$evidenceIndex], [System.StringComparison]::Ordinal)) {
                    $issues.Add("marker/registry evidence differs for $id")
                    break
                }
            }
        }
        $checked = $match.Groups['checked'].Value.ToLowerInvariant() -eq "x"
        if (-not $checked) { $issues.Add("implemented M0 evidence is linked to an unchecked roadmap item: $id") }
        if ($row.unresolved.Count -gt 0) { $issues.Add("unresolved evidence remains for ${id}: $($row.unresolved -join ', ')") }
        if ([string]::Equals($row.progress.consistency, "contradiction", [System.StringComparison]::Ordinal)) {
            $issues.Add("progress contradiction remains for $id")
        }
        $textValue = $match.Groups['text'].Value
        $items.Add([ordered]@{
            id = $id
            ordinal = [int]$checkboxCount
            task = $currentTask
            roadmap = [ordered]@{
                path = $script:RoadmapRelativePath
                selector = "milestone:M0/task:$currentTask/checkbox:$(([int]$taskOrdinals[$currentTask]).ToString('D2'))"
                checked = [bool]$checked
                text = $textValue
                text_sha256 = Get-TextSha256 $textValue
            }
            evidence = @($row.evidence)
            authoritative_sources = @($row.authoritative_sources)
            acceptance = $row.acceptance
            progress = $row.progress
            assessment = [ordered]@{
                roadmap_state = if ($checked) { "complete" } else { "incomplete" }
                implementation_state = "implemented"
                progress_consistency = $row.progress.consistency
                unresolved = @($row.unresolved)
            }
        })
    }

    foreach ($task in $script:ExpectedTaskCounts.Keys) {
        $actual = if ($taskOrdinals.ContainsKey($task)) { [int]$taskOrdinals[$task] } else { 0 }
        $expected = [int]$script:ExpectedTaskCounts[$task]
        if ($actual -ne $expected) { $issues.Add("M0 Task $task checkbox count differs(actual=$actual,expected=$expected)") }
    }
    if ($checkboxCount -ne 11) { $issues.Add("M0 delegated checkbox count differs(actual=$checkboxCount,expected=11)") }
    foreach ($expectedId in $expectedIds) {
        if (-not $registry.ContainsKey($expectedId)) { $issues.Add("missing M0 evidence registry row: $expectedId") }
        if (-not $linkedIds.Contains($expectedId)) { $issues.Add("missing M0 checkbox link: $expectedId") }
    }

    $automaticCount = 0
    $manualCount = 0
    $historicalEvolutionCount = 0
    $progressContradictionCount = 0
    $unresolvedCount = 0
    $manualEvidenceIds = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::Ordinal)
    foreach ($item in $items) {
        foreach ($evidence in @($item.evidence)) {
            if ($evidence.kind -eq "automated-check") { $automaticCount++ }
            elseif ($evidence.kind -eq "manual-acceptance") {
                if (-not $manualEvidenceIds.Add([string]$evidence.id)) {
                    throw "manual acceptance id is not globally unique: $($evidence.id)"
                }
                $manualCount++
            }
        }
        if ($item.progress.consistency -eq "historical-evolution") { $historicalEvolutionCount++ }
        if ($item.progress.consistency -eq "contradiction") { $progressContradictionCount++ }
        $unresolvedCount += @($item.assessment.unresolved).Count
    }
    $linkedCount = $items.Count
    $unlinkedCount = [Math]::Max(0, $checkboxCount - $linkedCount)
    $status = [ordered]@{
        schema_version = 1
        profile = "implementation-roadmap-evidence-links"
        source_snapshot = [ordered]@{
            source_set = "git-tracked"
            write_mode = "generated-artifact-only"
            untracked_candidate_policy = "error"
            path_format = "repository-relative-forward-slash"
        }
        contracts = @(
            [ordered]@{
                path = $script:RoadmapRelativePath
                selector = "heading:## M0: プロジェクト基盤+ORIGAMI3-ROADMAP-EVIDENCE scope=M0"
                hash_profile = "canonical-lf-selected-m0-section"
                sha256 = Get-TextSha256 $m0ScopeText
            }
        )
        scopes = @(
            [ordered]@{
                stage = "7-D1"
                milestone = "M0"
                expected_checkbox_count = 11
                links = @($items.ToArray())
            }
        )
        summary = [ordered]@{
            checkbox_count = [int]$checkboxCount
            linked_count = [int]$linkedCount
            unlinked_count = [int]$unlinkedCount
            automated_evidence_count = [int]$automaticCount
            manual_acceptance_count = [int]$manualCount
            unresolved_count = [int]$unresolvedCount
            historical_evolution_count = [int]$historicalEvolutionCount
            progress_contradiction_count = [int]$progressContradictionCount
        }
    }
    return [PSCustomObject][ordered]@{
        Status = $status
        Issues = @($issues.ToArray())
    }
}

function Format-JsonTwoSpace {
    param([string]$CompressedJson)

    $builder = New-Object System.Text.StringBuilder
    $indent = 0
    $inString = $false
    $escaped = $false
    for ($index = 0; $index -lt $CompressedJson.Length; $index++) {
        $character = $CompressedJson[$index]
        if ($inString) {
            [void]$builder.Append($character)
            if ($escaped) { $escaped = $false }
            elseif ($character -eq '\') { $escaped = $true }
            elseif ($character -eq '"') { $inString = $false }
            continue
        }
        if ($character -eq '"') {
            $inString = $true
            [void]$builder.Append($character)
            continue
        }
        switch ($character) {
            '{' {
                [void]$builder.Append($character)
                if (($index + 1) -lt $CompressedJson.Length -and $CompressedJson[$index + 1] -ne '}') {
                    $indent++
                    [void]$builder.Append("`n" + ("  " * $indent))
                }
            }
            '[' {
                [void]$builder.Append($character)
                if (($index + 1) -lt $CompressedJson.Length -and $CompressedJson[$index + 1] -ne ']') {
                    $indent++
                    [void]$builder.Append("`n" + ("  " * $indent))
                }
            }
            '}' {
                if ($index -gt 0 -and $CompressedJson[$index - 1] -ne '{') {
                    $indent--
                    [void]$builder.Append("`n" + ("  " * $indent))
                }
                [void]$builder.Append($character)
            }
            ']' {
                if ($index -gt 0 -and $CompressedJson[$index - 1] -ne '[') {
                    $indent--
                    [void]$builder.Append("`n" + ("  " * $indent))
                }
                [void]$builder.Append($character)
            }
            ',' { [void]$builder.Append(",`n" + ("  " * $indent)) }
            ':' { [void]$builder.Append(": ") }
            default { [void]$builder.Append($character) }
        }
    }
    if ($inString -or $indent -ne 0) { throw "canonical links JSON formatter ended in an invalid state" }
    return $builder.ToString().TrimEnd("`r", "`n") + "`n"
}

function ConvertTo-CanonicalJson {
    param([System.Collections.IDictionary]$Status)
    $compressed = ConvertTo-Json -InputObject $Status -Compress -Depth 100
    return Format-JsonTwoSpace $compressed
}

function Write-GeneratedLinksAtomically {
    param([string]$Json)

    $relative = $script:GeneratedRelativePath
    if (Test-GitTrackedPath $relative) { throw "refusing to overwrite tracked links.json: $relative" }
    $fullPath = [System.IO.Path]::GetFullPath((Join-Path $script:Root ($relative.Replace("/", "\"))))
    $expectedPrefix = [System.IO.Path]::GetFullPath((Join-Path $script:Root "verification\improvement-roadmap\07-docs")).TrimEnd([char[]]"\/")
    if (-not [string]::Equals((Split-Path -Parent $fullPath).TrimEnd([char[]]"\/"), $expectedPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "generated links output escaped exact directory"
    }
    $directory = Split-Path -Parent $fullPath
    if (-not (Test-Path -LiteralPath $directory)) {
        [void](New-Item -ItemType Directory -Path $directory)
    }
    Assert-NoReparsePointInPath $directory
    if (Test-Path -LiteralPath $fullPath) {
        $destinationItem = Get-Item -LiteralPath $fullPath -Force
        if (($destinationItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "refusing to replace reparse-point links.json"
        }
        if ($destinationItem.PSIsContainer) { throw "generated links destination is not a file" }
    }
    $temporary = Join-Path $directory (".links-{0}.tmp" -f [Guid]::NewGuid().ToString("N"))
    $backup = Join-Path $directory (".links-{0}.bak" -f [Guid]::NewGuid().ToString("N"))
    try {
        [System.IO.File]::WriteAllText($temporary, $Json, $script:Utf8NoBom)
        if (Test-Path -LiteralPath $fullPath) {
            [System.IO.File]::Replace($temporary, $fullPath, $backup, $true)
            if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Force }
        }
        else {
            Move-Item -LiteralPath $temporary -Destination $fullPath
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
        if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Force }
    }
}

function Invoke-RoadmapLinkFixtures {
    param([string]$RoadmapText, [byte[]]$RoadmapBytes)

    $passed = 0
    $clean = ConvertFrom-RoadmapText $RoadmapText $RoadmapBytes
    if ($clean.Issues.Count -ne 0 -or $clean.Status.summary.linked_count -ne 11) { throw "clean D1 fixture failed" }
    $json1 = ConvertTo-CanonicalJson $clean.Status
    $json2 = ConvertTo-CanonicalJson $clean.Status
    if (-not [string]::Equals($json1, $json2, [System.StringComparison]::Ordinal)) { throw "D1 JSON fixture is not deterministic" }
    $lfText = $RoadmapText.Replace("`r`n", "`n").Replace("`r", "`n")
    $crlfText = $lfText.Replace("`n", "`r`n")
    $lfStatus = ConvertFrom-RoadmapText $lfText ($script:Utf8NoBom.GetBytes($lfText))
    $crlfStatus = ConvertFrom-RoadmapText $crlfText ($script:Utf8NoBom.GetBytes($crlfText))
    if (-not [string]::Equals($json1, (ConvertTo-CanonicalJson $lfStatus.Status), [System.StringComparison]::Ordinal) -or
        -not [string]::Equals($json1, (ConvertTo-CanonicalJson $crlfStatus.Status), [System.StringComparison]::Ordinal)) {
        throw "D1 JSON fixture depends on checkout line endings"
    }
    $postM0Text = $RoadmapText + "`nD2-AND-LATER-FIXTURE-CONTENT`n"
    $postM0Status = ConvertFrom-RoadmapText $postM0Text ($script:Utf8NoBom.GetBytes($postM0Text))
    if (-not [string]::Equals($json1, (ConvertTo-CanonicalJson $postM0Status.Status), [System.StringComparison]::Ordinal)) {
        throw "D1 JSON fixture depends on content after M0"
    }
    $passed++

    $selfConsistentBogusText = $RoadmapText.Replace(
        'CHECK.CURRENT-STATUS.WORKSPACE-MEMBERS',
        'CHECK.CURRENT-STATUS.BOGUS'
    )
    $bogusEvidenceRejected = $false
    try { [void](ConvertFrom-RoadmapText $selfConsistentBogusText ($script:Utf8NoBom.GetBytes($selfConsistentBogusText))) }
    catch { $bogusEvidenceRejected = $_.Exception.Message -like '*D1 evidence sequence*' }
    if (-not $bogusEvidenceRejected) { throw "self-consistent bogus evidence fixture bypassed the D1 contract" }
    $passed++

    $missingSourceText = $RoadmapText.Replace(
        '<br>`file:.github/workflows/ci.yml` :: `jobs:checks+performance`',
        ''
    )
    $missingSourceRejected = $false
    try { [void](ConvertFrom-RoadmapText $missingSourceText ($script:Utf8NoBom.GetBytes($missingSourceText))) }
    catch { $missingSourceRejected = $_.Exception.Message -like '*D1 source sequence*' }
    if (-not $missingSourceRejected) { throw "missing-source fixture bypassed the D1 contract" }
    $passed++

    $firstLinePattern = '(?m)^(?<prefix>- \[x\] .*?) — \[証拠:M0\.T0-1\.C01\].*?$'
    $missingLinkText = [regex]::Replace($RoadmapText, $firstLinePattern, '${prefix}', 1)
    $missing = ConvertFrom-RoadmapText $missingLinkText ($script:Utf8NoBom.GetBytes($missingLinkText))
    if ($missing.Issues.Count -eq 0 -or $missing.Status.summary.unlinked_count -ne 1) { throw "missing-link fixture did not fail semantically" }
    $passed++

    $evidenceDriftText = $RoadmapText.Replace(
        'evidence=CHECK.CURRENT-STATUS.WORKSPACE-MEMBERS -->',
        'evidence=CHECK.CURRENT-STATUS.UNKNOWN -->'
    )
    $evidenceDrift = ConvertFrom-RoadmapText $evidenceDriftText ($script:Utf8NoBom.GetBytes($evidenceDriftText))
    if (-not @($evidenceDrift.Issues | Where-Object { $_ -like 'marker/registry evidence differs*' }).Count) {
        throw "evidence-drift fixture did not fail semantically"
    }
    $passed++

    $duplicateIdText = $RoadmapText.Replace(
        'id=M0.T0-1.C02 evidence=MANUAL.M0.T0-1.C02.CRATE-SCAFFOLD',
        'id=M0.T0-1.C01 evidence=MANUAL.M0.T0-1.C02.CRATE-SCAFFOLD'
    ).Replace(
        '[証拠:M0.T0-1.C02](#roadmap-evidence-m0-t0-1-c02)',
        '[証拠:M0.T0-1.C01](#roadmap-evidence-m0-t0-1-c01)'
    )
    $duplicateRejected = $false
    try { [void](ConvertFrom-RoadmapText $duplicateIdText ($script:Utf8NoBom.GetBytes($duplicateIdText))) }
    catch { $duplicateRejected = $_.Exception.Message -like '*duplicate roadmap checkbox link id*' }
    if (-not $duplicateRejected) { throw "duplicate-id fixture was not rejected as schema error" }
    $passed++

    $duplicateManualText = $RoadmapText.Replace(
        'MANUAL.M0.T0-1.C03.DEPENDENCY-BASELINE',
        'MANUAL.M0.T0-1.C02.CRATE-SCAFFOLD'
    )
    $duplicateManualRejected = $false
    try { [void](ConvertFrom-RoadmapText $duplicateManualText ($script:Utf8NoBom.GetBytes($duplicateManualText))) }
    catch {
        $duplicateManualRejected = $_.Exception.Message -like '*manual acceptance id is not globally unique*' -or
            $_.Exception.Message -like '*D1 evidence sequence*'
    }
    if (-not $duplicateManualRejected) { throw "duplicate-manual-id fixture was not rejected as schema error" }
    $passed++

    if (Test-PowerShellFunctionExistsOnce "scripts/generate-current-status.ps1" "D1_FUNCTION_THAT_DOES_NOT_EXIST") {
        throw "missing-selector helper fixture unexpectedly resolved"
    }
    $passed++

    $unknownSelectorText = $RoadmapText.Replace(
        '`file:scripts/generate-current-status.ps1` :: `function:Get-WorkspaceMembers`',
        '`file:scripts/generate-current-status.ps1` :: `function:D1_UNKNOWN`'
    )
    $unknownSelectorRejected = $false
    try { [void](ConvertFrom-RoadmapText $unknownSelectorText ($script:Utf8NoBom.GetBytes($unknownSelectorText))) }
    catch { $unknownSelectorRejected = $_.Exception.Message -like '*unsupported 7-D1 tracked evidence selector*' }
    if (-not $unknownSelectorRejected) { throw "unknown-selector fixture was not rejected as schema error" }
    $passed++

    if (Test-InvokeCheckInvocationExistsOnce "(1/5) cargo test --workspace" "cargo" "D1WrongArguments") {
        throw "Invoke-Check argument mutation helper fixture unexpectedly resolved"
    }
    $ciText = Read-TrackedSourceText ".github/workflows/ci.yml"
    $ciMutation = $ciText.Replace(
        "        run: cargo test --release -p desktop --lib the_heaviest_proposal_never_hits_the_time_limit -- --nocapture",
        "        run: D1_MUTATED_COMMAND"
    )
    if (Test-CiRustChecksAndPerformanceComplements $ciMutation) {
        throw "CI complement mutation helper fixture unexpectedly resolved"
    }
    $progressText = Read-TrackedSourceText "docs/progress.md"
    $progressMutation = $progressText.Replace(
        "検査の仕組みにテスト実行(vitest)を5番目として追加",
        "D1_PROGRESS_CLAIM_REMOVED"
    )
    if (Test-D1ProgressSectionEvidence $progressMutation "## 2026-08-05 - Task 1-6 - 展開図を描く画面(方眼・吸着・線の描画)を追加") {
        throw "progress claim mutation helper fixture unexpectedly resolved"
    }
    $duplicateJsonText = '{"build":{"beforeDevCommand":"wrong","beforeDevCommand":"npm run dev"}}'
    $duplicateBuildObject = Get-UniqueJsonPropertyContainer $duplicateJsonText "build" '{'
    if ($null -eq $duplicateBuildObject -or [regex]::Matches($duplicateBuildObject, '"beforeDevCommand"\s*:').Count -ne 2) {
        throw "duplicate JSON key helper fixture did not expose both keys"
    }
    $passed++

    $forbiddenText = $RoadmapText.Replace('`file:Cargo.toml` :: `section:[workspace]/field:members`', '`file:verification/forbidden.json` :: `fixture`')
    $forbiddenRejected = $false
    try { [void](ConvertFrom-RoadmapText $forbiddenText ($script:Utf8NoBom.GetBytes($forbiddenText))) }
    catch { $forbiddenRejected = $_.Exception.Message -like '*forbidden roadmap-link input path*' }
    if (-not $forbiddenRejected) { throw "forbidden-input fixture was not rejected before read" }
    $passed++

    return [PSCustomObject][ordered]@{ Passed = $passed; Total = 11; JsonSha256 = Get-TextSha256 $json1 }
}

$exitCode = 2
try {
    if ($Check -and $Fixtures) { throw "-Check and -Fixtures cannot be combined" }
    if ($Fixtures -and -not [string]::IsNullOrWhiteSpace($AllowPartialScope)) {
        throw "-AllowPartialScope is not used with -Fixtures"
    }
    $wholeSnapshot = Get-WholeRoadmapSnapshot
    $roadmap = Read-TrackedUtf8File $script:RoadmapRelativePath
    if ($Fixtures) {
        $fixture = Invoke-RoadmapLinkFixtures $roadmap.Text $roadmap.Bytes
        $freshRoadmap = Read-TrackedUtf8File $script:RoadmapRelativePath
        if (-not [string]::Equals((Get-Sha256Hex $roadmap.Bytes), (Get-Sha256Hex $freshRoadmap.Bytes), [System.StringComparison]::Ordinal)) {
            throw "implementation roadmap changed during D1 fixtures"
        }
        Assert-TrackedSourceSnapshotUnchanged
        Write-Host ("[FIXTURE] cases={0} passed={1}; json_sha256={2}" -f $fixture.Total, $fixture.Passed, $fixture.JsonSha256)
        Write-Host ("[PARTIAL] scope=M0 audited=11/{0} partial=true full_coverage=false" -f $wholeSnapshot.total)
        Write-Host "generated output: bypassed in Fixtures"
        $exitCode = 0
    }
    else {
        $first = ConvertFrom-RoadmapText $roadmap.Text $roadmap.Bytes
        $second = ConvertFrom-RoadmapText $roadmap.Text $roadmap.Bytes
        $wholeMetadata = [ordered]@{
            schema = [int]$wholeSnapshot.schema
            roadmap_sha256 = [string]$wholeSnapshot.roadmap_sha256
            policy_sha256 = [string]$wholeSnapshot.policy_sha256
            total = [int]$wholeSnapshot.total
            audited = [int]$wholeSnapshot.audited
            checked = [int]$wholeSnapshot.checked
            unchecked = [int]$wholeSnapshot.unchecked
            evidence_linked = [int]$wholeSnapshot.evidence_linked
            explicit_outside = [int]$wholeSnapshot.explicit_outside
            unclassified = [int]$wholeSnapshot.unclassified
        }
        $first.Status["whole_roadmap_snapshot"] = $wholeMetadata
        $second.Status["whole_roadmap_snapshot"] = $wholeMetadata
        $firstJson = ConvertTo-CanonicalJson $first.Status
        $secondJson = ConvertTo-CanonicalJson $second.Status
        $jsonHash = Get-TextSha256 $firstJson
        if (-not [string]::Equals($firstJson, $secondJson, [System.StringComparison]::Ordinal)) {
            throw "roadmap links JSON differs across two renders"
        }
        $freshRoadmap = Read-TrackedUtf8File $script:RoadmapRelativePath
        if (-not [string]::Equals((Get-Sha256Hex $roadmap.Bytes), (Get-Sha256Hex $freshRoadmap.Bytes), [System.StringComparison]::Ordinal)) {
            throw "implementation roadmap changed during D1 collection"
        }
        Assert-TrackedSourceSnapshotUnchanged
        $m0Audited = [int]$first.Status.summary.checkbox_count
        if ($m0Audited -ne [int]$wholeSnapshot.scopes.M0) {
            throw "M0 scope count differs from whole snapshot: generator=$m0Audited snapshot=$($wholeSnapshot.scopes.M0)"
        }
        Write-Host ("[PARTIAL] scope=M0 audited={0}/{1} partial=true full_coverage=false" -f $m0Audited, $wholeSnapshot.total)
        foreach ($issue in @($first.Issues)) { Write-Warning $issue }
        if ($first.Issues.Count -gt 0) {
            Write-Host ("roadmap links: {0}/{1}; unlinked={2}; unresolved={3}; progress contradictions={4}" -f
                $first.Status.summary.linked_count,
                $first.Status.summary.checkbox_count,
                $first.Status.summary.unlinked_count,
                $first.Status.summary.unresolved_count,
                $first.Status.summary.progress_contradiction_count)
            $exitCode = 1
        }
        else {
            $partialScopeAccepted = [string]::Equals($AllowPartialScope, "M0", [StringComparison]::Ordinal)
            if (-not $Check -and $partialScopeAccepted) { Write-GeneratedLinksAtomically $firstJson }
            Write-Host ("roadmap links: {0}/{1}; automated evidence={2}; manual acceptance={3}; unresolved={4}; progress contradictions={5}" -f
                $first.Status.summary.linked_count,
                $first.Status.summary.checkbox_count,
                $first.Status.summary.automated_evidence_count,
                $first.Status.summary.manual_acceptance_count,
                $first.Status.summary.unresolved_count,
                $first.Status.summary.progress_contradiction_count)
            Write-Host ("historical evolution notes: {0}; deterministic json_sha256={1}; output={2}" -f
                $first.Status.summary.historical_evolution_count,
                $jsonHash,
                $(if ($Check) { "none (-Check)" } else { $script:GeneratedRelativePath }))
            if ($partialScopeAccepted) {
                Write-Host "[OK] partial scope M0 was explicitly accepted with -AllowPartialScope M0"
                $exitCode = 0
            }
            else {
                Write-Warning "partial audit 11/$($wholeSnapshot.total) cannot be treated as whole-roadmap verification; rerun with -AllowPartialScope M0 only when M0-only evidence is intended"
                $exitCode = 1
            }
        }
    }
}
catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    $exitCode = 2
}
exit $exitCode
