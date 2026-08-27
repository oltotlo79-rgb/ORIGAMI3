[CmdletBinding()]
param(
    [string]$OutputDirectory,
    [switch]$Check,
    [switch]$MarkerFixtures,
    [string]$CargoTargetDir = $env:CARGO_TARGET_DIR
)

# ORIGAMI3 現在値generator (Windows PowerShell 5.1 / PowerShell 7対応)
#
# 段階7-Bの責務:
# - gitで追跡されているsourceだけを排他制御した隔離cacheへ同期する。
# - 6指標を実装・runner・PDF成果物から収集する。
# - 同じsnapshotで2回収集し、JSON/Markdownのbyte一致を確かめる。
# - 追跡文書は書かず、TEMPのcacheとverification配下の生成物だけを書き換える。
#
# 段階7-Cでは-Check時にdocs/progress.mdのmarkerを照合する。markerの更新と
# CI配置はこのscript自身では行わず、MarkerFixturesは生成物・実文書をgateしない。

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false, $true)
$script:Latin1 = [System.Text.Encoding]::GetEncoding(28591)
$script:Ordinal = [System.StringComparer]::Ordinal
$script:MirrorSetDiagnostics = New-Object 'System.Collections.Generic.Dictionary[string,string]' ($script:Ordinal)
$script:Root = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot)).TrimEnd([char[]]"\/")
$script:CollectorPath = [System.IO.Path]::GetFullPath($MyInvocation.MyCommand.Path)
$script:GeneratedRelativeRoot = "verification/improvement-roadmap/07-docs"
$script:ForbiddenRelativePath = "docs/competitive-review-2026-08-20.md"
$script:ForbiddenPrefixes = @("verification/", "scratchpad/", "vendor/")
$script:TrackedEntries = $null
$script:TrackedSet = $null
$script:SelectedSourcePaths = $null
$script:SnapshotRoot = $null
$script:SnapshotCacheLock = $null
$script:SnapshotCacheLeaf = "ori3-current-status-source-cache"
$script:SnapshotCacheMetadataLeaf = "ori3-current-status-source-cache.metadata.json"
$script:SnapshotCacheLockLeaf = "ori3-current-status-source-cache.lock"
$script:SnapshotCacheSchemaVersion = 1
$script:FrontendPrepared = $false
$script:LastTestInventoryTimings = $null

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $script:Root ($script:GeneratedRelativeRoot.Replace("/", "\"))
}
else {
    $OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
}

$cargoTargetLeaf = "ori3-target-docs7b"
$allowedCargoTargetParents = New-Object System.Collections.Generic.List[string]
[void]$allowedCargoTargetParents.Add([System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd([char[]]"\/"))
if ([string]::Equals($env:GITHUB_ACTIONS, "true", [System.StringComparison]::OrdinalIgnoreCase) -and
    -not [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    $runnerTemp = [System.IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd([char[]]"\/")
    if (-not @($allowedCargoTargetParents | Where-Object { [string]::Equals($_, $runnerTemp, [System.StringComparison]::OrdinalIgnoreCase) }).Count) {
        [void]$allowedCargoTargetParents.Add($runnerTemp)
    }
}
$defaultCargoTargetDir = Join-Path $allowedCargoTargetParents[0] $cargoTargetLeaf
$script:CargoTargetError = $null
if ([string]::IsNullOrWhiteSpace($CargoTargetDir)) {
    $CargoTargetDir = $defaultCargoTargetDir
}
$CargoTargetDir = [System.IO.Path]::GetFullPath($CargoTargetDir)
$cargoTargetParent = [System.IO.Path]::GetFullPath((Split-Path -Parent $CargoTargetDir)).TrimEnd([char[]]"\/")
$cargoTargetName = Split-Path -Leaf $CargoTargetDir
$cargoTargetParentAllowed = @($allowedCargoTargetParents | Where-Object { [string]::Equals($_, $cargoTargetParent, [System.StringComparison]::OrdinalIgnoreCase) }).Count -eq 1
$cargoTargetInsideRepository = [string]::Equals($CargoTargetDir.TrimEnd([char[]]"\/"), $script:Root, [System.StringComparison]::OrdinalIgnoreCase) -or
    $CargoTargetDir.StartsWith($script:Root + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)
if (-not $cargoTargetParentAllowed -or
    -not [string]::Equals($cargoTargetName, $cargoTargetLeaf, [System.StringComparison]::Ordinal) -or
    $cargoTargetInsideRepository) {
    $allowedTargets = @($allowedCargoTargetParents | ForEach-Object { Join-Path $_ $cargoTargetLeaf }) -join ", "
    $script:CargoTargetError = "current-status collector requires CARGO_TARGET_DIR to be one of: $allowedTargets"
}

function ConvertTo-NativeArgument {
    param([AllowEmptyString()][string]$Value)

    if ($Value.Length -eq 0) {
        return '""'
    }
    if ($Value -notmatch '[\s"]') {
        return $Value
    }

    $builder = New-Object System.Text.StringBuilder
    [void]$builder.Append('"')
    $backslashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq '\') {
            $backslashes++
            continue
        }
        if ($character -eq '"') {
            if ($backslashes -gt 0) {
                [void]$builder.Append(('\' * ($backslashes * 2)))
            }
            [void]$builder.Append('\"')
            $backslashes = 0
            continue
        }
        if ($backslashes -gt 0) {
            [void]$builder.Append(('\' * $backslashes))
            $backslashes = 0
        }
        [void]$builder.Append($character)
    }
    if ($backslashes -gt 0) {
        [void]$builder.Append(('\' * ($backslashes * 2)))
    }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Invoke-NativeCapture {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [hashtable]$Environment = @{}
    )

    $resolvedCommand = Get-Command $FilePath -ErrorAction Stop
    $executable = $resolvedCommand.Source
    $argumentText = (($Arguments | ForEach-Object { ConvertTo-NativeArgument ([string]$_) }) -join " ")

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $executable
    $startInfo.Arguments = $argumentText
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    try {
        $startInfo.StandardOutputEncoding = $script:Utf8NoBom
        $startInfo.StandardErrorEncoding = $script:Utf8NoBom
    }
    catch {
        # 古い.NETではencoding指定が無い。ASCIIのtool protocolだけをparseする。
    }
    foreach ($name in $Environment.Keys) {
        $startInfo.EnvironmentVariables[[string]$name] = [string]$Environment[$name]
    }

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        if (-not $process.Start()) {
            throw "processを開始できません: $FilePath"
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        $exitCode = $process.ExitCode
    }
    finally {
        $watch.Stop()
        $process.Dispose()
    }

    return [PSCustomObject][ordered]@{
        ExitCode   = [int]$exitCode
        StdOut     = [string]$stdout
        StdErr     = [string]$stderr
        ElapsedMs  = [double]$watch.Elapsed.TotalMilliseconds
        Command    = "$executable $argumentText"
    }
}

function Invoke-GitCapture {
    param([string[]]$Arguments)

    $allArguments = @("-c", "core.quotepath=false", "-C", $script:Root) + $Arguments
    $result = Invoke-NativeCapture "git.exe" $allArguments $script:Root
    if ($result.ExitCode -ne 0) {
        throw "gitのread-only照会に失敗しました(exit $($result.ExitCode)): $($result.StdErr.Trim())"
    }
    return $result
}

function ConvertTo-RepositoryPath {
    param([string]$Path)

    $normalized = $Path.Replace("\", "/")
    while ($normalized.StartsWith("./", [System.StringComparison]::Ordinal)) {
        $normalized = $normalized.Substring(2)
    }
    if ([string]::IsNullOrWhiteSpace($normalized) -or
        [System.IO.Path]::IsPathRooted($normalized) -or
        $normalized.Contains(":") -or
        $normalized.Contains([char]0)) {
        throw "repository相対pathではありません: $Path"
    }
    $segments = $normalized.Split('/')
    if ($segments -contains "" -or $segments -contains "." -or $segments -contains "..") {
        throw "正規化できないrepository pathです: $Path"
    }
    return $normalized
}

function Get-AbsoluteRepositoryPath {
    param([string]$RelativePath, [string]$BaseRoot = $script:Root)

    $relative = ConvertTo-RepositoryPath $RelativePath
    $full = [System.IO.Path]::GetFullPath((Join-Path $BaseRoot ($relative.Replace("/", "\"))))
    $base = [System.IO.Path]::GetFullPath($BaseRoot).TrimEnd([char[]]"\/")
    $prefix = $base + [System.IO.Path]::DirectorySeparatorChar
    if (-not $full.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "repository外へ解決されるpathです: $RelativePath"
    }
    return $full
}

function Test-ForbiddenSourcePath {
    param([string]$RelativePath)

    if ([string]::Equals($RelativePath, $script:ForbiddenRelativePath, [System.StringComparison]::Ordinal)) {
        return $true
    }
    foreach ($prefix in $script:ForbiddenPrefixes) {
        if ($RelativePath.StartsWith($prefix, [System.StringComparison]::Ordinal)) {
            return $true
        }
    }
    return $false
}

function Get-TrackedEntries {
    $pathSpecs = @(
        "Cargo.toml",
        "Cargo.lock",
        "CLAUDE.md",
        "crates",
        "apps/desktop",
        "scripts/build-manual.ps1",
        "docs/requirements-definition.md",
        "docs/implementation-roadmap.md",
        "docs/improvement-roadmap-2026-08-24.md",
        "docs/progress.md",
        "docs/manual/ORIGAMI3取扱説明書.pdf"
    )
    $result = Invoke-GitCapture (@("ls-files", "-z", "--stage", "--") + $pathSpecs)
    $records = $result.StdOut.Split([char]0, [System.StringSplitOptions]::RemoveEmptyEntries)
    $entries = New-Object System.Collections.Generic.List[object]
    $set = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    foreach ($record in $records) {
        if ([string]::IsNullOrEmpty($record)) {
            continue
        }
        $match = [regex]::Match($record, '^(?<mode>[0-9]{6}) (?<hash>[0-9a-f]{40,64}) (?<stage>[0-3])\t(?<path>.+)$')
        if (-not $match.Success) {
            throw "git ls-filesの行を解釈できません: $record"
        }
        if ($match.Groups["stage"].Value -ne "0") {
            throw "未解決index stageがあります: $($match.Groups['path'].Value)"
        }
        $mode = $match.Groups["mode"].Value
        if ($mode -ne "100644" -and $mode -ne "100755") {
            throw "通常fileでない追跡sourceは読めません(mode $mode): $($match.Groups['path'].Value)"
        }
        $relative = ConvertTo-RepositoryPath $match.Groups["path"].Value
        if (Test-ForbiddenSourcePath $relative) {
            throw "禁止pathがsource allowlistへ入りました: $relative"
        }
        if (-not $set.Add($relative)) {
            throw "追跡pathが重複しています: $relative"
        }
        [void]$entries.Add([PSCustomObject][ordered]@{
            Path = $relative
            Mode = $mode
            Hash = $match.Groups["hash"].Value
        })
    }
    if ($entries.Count -eq 0) {
        throw "追跡source集合が空です"
    }
    $script:TrackedSet = $set
    return $entries.ToArray()
}

function Assert-TrackedPath {
    param([string]$RelativePath)

    $relative = ConvertTo-RepositoryPath $RelativePath
    if (Test-ForbiddenSourcePath $relative) {
        throw "禁止pathを読もうとしました: $relative"
    }
    if (-not $script:TrackedSet.Contains($relative)) {
        throw "追跡されていないsourceは読めません: $relative"
    }
    return $relative
}

function Assert-NoReparsePathChain {
    param([string]$BaseRoot, [string]$AbsolutePath, [string]$SourceLabel)

    $base = [System.IO.Path]::GetFullPath($BaseRoot).TrimEnd([char[]]"\/")
    $full = [System.IO.Path]::GetFullPath($AbsolutePath)
    $prefix = $base + [System.IO.Path]::DirectorySeparatorChar
    if (-not $full.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$SourceLabel is outside its declared root"
    }
    $cursor = $base
    $segments = $full.Substring($prefix.Length).Split([System.IO.Path]::DirectorySeparatorChar)
    $paths = @($base)
    foreach ($segment in $segments) {
        $cursor = [System.IO.Path]::Combine($cursor, $segment)
        $paths += $cursor
    }
    foreach ($path in $paths) {
        if ([System.IO.File]::Exists($path) -or [System.IO.Directory]::Exists($path)) {
            $item = Get-Item -LiteralPath $path -Force
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$SourceLabel contains a reparse point: $path"
            }
        }
    }
}

function Read-TrackedBytes {
    param([string]$RelativePath, [string]$BaseRoot = $script:Root)

    $relative = Assert-TrackedPath $RelativePath
    $path = Get-AbsoluteRepositoryPath $relative $BaseRoot
    Assert-NoReparsePathChain $BaseRoot $path $relative
    if (-not [System.IO.File]::Exists($path)) {
        throw "追跡sourceが作業treeにありません: $relative"
    }
    $bytes = [System.IO.File]::ReadAllBytes($path)
    return ,$bytes
}

function Read-TrackedText {
    param([string]$RelativePath, [string]$BaseRoot = $script:Root)

    $bytes = Read-TrackedBytes $RelativePath $BaseRoot
    try {
        return $script:Utf8NoBom.GetString($bytes)
    }
    catch {
        throw "UTF-8として読めません: $RelativePath ($($_.Exception.Message))"
    }
}

function Get-OrdinalSortedStrings {
    param([string[]]$Values)

    $copy = [string[]]@($Values)
    [System.Array]::Sort($copy, $script:Ordinal)
    return $copy
}

function Get-SelectedSourcePaths {
    $selected = New-Object System.Collections.Generic.List[string]
    foreach ($entry in $script:TrackedEntries) {
        $path = [string]$entry.Path
        $include = $path -eq "Cargo.toml" -or
            $path -eq "Cargo.lock" -or
            $path -eq "CLAUDE.md" -or
            $path.StartsWith("crates/", [System.StringComparison]::Ordinal) -or
            $path.StartsWith("apps/desktop/", [System.StringComparison]::Ordinal) -or
            $path -eq "scripts/build-manual.ps1" -or
            $path -eq "docs/requirements-definition.md" -or
            $path -eq "docs/implementation-roadmap.md" -or
            $path -eq "docs/improvement-roadmap-2026-08-24.md" -or
            $path -eq "docs/progress.md" -or
            $path -eq "docs/manual/ORIGAMI3取扱説明書.pdf"
        if ($include) {
            if (Test-ForbiddenSourcePath $path) {
                throw "禁止pathが選択されました: $path"
            }
            [void]$selected.Add($path)
        }
    }
    return Get-OrdinalSortedStrings $selected.ToArray()
}

function Get-UntrackedMetricCandidatePaths {
    # Only ask git about metric source roots.  In particular, never enumerate
    # docs/ broadly: the forbidden competitive review must not be opened or
    # even selected by an accidental repository-wide scan.
    $pathSpecs = @(
        "Cargo.toml",
        "Cargo.lock",
        "crates",
        "apps/desktop/src",
        "apps/desktop/src-tauri",
        "apps/desktop/package.json",
        "apps/desktop/package-lock.json",
        "apps/desktop/vite.config.ts",
        "apps/desktop/vitest.config.ts",
        "apps/desktop/tsconfig.json",
        "apps/desktop/tsconfig.app.json",
        "apps/desktop/tsconfig.node.json"
    )
    $results = @(
        Invoke-GitCapture (@("ls-files", "-z", "--others", "--exclude-standard", "--") + $pathSpecs)
        Invoke-GitCapture (@("ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--") + $pathSpecs)
    )
    $candidates = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    foreach ($result in $results) {
        foreach ($raw in $result.StdOut.Split([char]0, [System.StringSplitOptions]::RemoveEmptyEntries)) {
            if ([string]::IsNullOrEmpty($raw)) {
                continue
            }
            $relative = ConvertTo-RepositoryPath $raw
            $isCandidate =
                ($relative -match '^crates/.+\.rs$') -or
                ($relative -match '^crates/[^/]+/Cargo\.toml$') -or
                ($relative -match '^apps/desktop/src/.+\.(?:ts|tsx)$') -or
                ($relative -match '^apps/desktop/src-tauri/.+\.rs$') -or
                ($relative -eq 'apps/desktop/src-tauri/Cargo.toml') -or
                ($relative -eq 'apps/desktop/package.json') -or
                ($relative -eq 'apps/desktop/package-lock.json') -or
                ($relative -match '^apps/desktop/(?:vite|vitest)\.config\.(?:ts|js|mts|mjs)$') -or
                ($relative -match '^apps/desktop/tsconfig(?:\.[A-Za-z0-9_-]+)?\.json$')
            if ($isCandidate) {
                [void]$candidates.Add($relative)
            }
        }
    }
    return Get-OrdinalSortedStrings @($candidates)
}

function Assert-NoUntrackedMetricCandidates {
    $candidates = @(Get-UntrackedMetricCandidatePaths)
    if ($candidates.Count -gt 0) {
        $display = ($candidates -join ", ")
        throw "untracked metric source candidates are present; none were read: $display"
    }
}

function Assert-CleanCommittedCheckTree {
    if (-not $Check) {
        return
    }
    $status = Invoke-GitCapture @("status", "--porcelain=v1", "--untracked-files=all")
    if (-not [string]::IsNullOrWhiteSpace($status.StdOut)) {
        throw "-Check requires a clean committed working tree"
    }
}

function Get-Sha256Bytes {
    param([byte[]]$Bytes)

    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $hash = $sha.ComputeHash($Bytes)
        return ,$hash
    }
    finally {
        $sha.Dispose()
    }
}

function ConvertTo-Hex {
    param([byte[]]$Bytes)

    return ([System.BitConverter]::ToString($Bytes)).Replace("-", "").ToLowerInvariant()
}

function Get-TextSha256 {
    param([string]$Text)

    return ConvertTo-Hex (Get-Sha256Bytes ($script:Utf8NoBom.GetBytes($Text)))
}

function Get-SourceManifestHash {
    param([string]$BaseRoot = $script:Root)

    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        foreach ($relative in $script:SelectedSourcePaths) {
            $pathBytes = $script:Utf8NoBom.GetBytes($relative)
            [void]$sha.TransformBlock($pathBytes, 0, $pathBytes.Length, $pathBytes, 0)
            $separator = [byte[]](0)
            [void]$sha.TransformBlock($separator, 0, 1, $separator, 0)
            $bytes = Read-TrackedBytes $relative $BaseRoot
            if ($bytes.Length -gt 0) {
                [void]$sha.TransformBlock($bytes, 0, $bytes.Length, $bytes, 0)
            }
            [void]$sha.TransformBlock($separator, 0, 1, $separator, 0)
        }
        [void]$sha.TransformFinalBlock((New-Object byte[] 0), 0, 0)
        return ConvertTo-Hex $sha.Hash
    }
    finally {
        $sha.Dispose()
    }
}

function Get-SnapshotCachePaths {
    $temp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
    return [PSCustomObject][ordered]@{
        Root = [System.IO.Path]::Combine($temp, $script:SnapshotCacheLeaf)
        Metadata = [System.IO.Path]::Combine($temp, $script:SnapshotCacheMetadataLeaf)
        Lock = [System.IO.Path]::Combine($temp, $script:SnapshotCacheLockLeaf)
    }
}

function Assert-SafeSnapshotCachePath {
    param([string]$Path, [string]$ExpectedLeaf)

    $full = [System.IO.Path]::GetFullPath($Path).TrimEnd([char[]]"\/")
    $temp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
    $parent = [System.IO.Path]::GetDirectoryName($full).TrimEnd([char[]]"\/")
    $leaf = [System.IO.Path]::GetFileName($full)
    if (-not [string]::Equals($parent, $temp, [System.StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals($leaf, $ExpectedLeaf, [System.StringComparison]::Ordinal)) {
        throw "安全なcurrent-status cache pathではありません: $Path"
    }
    if ([System.IO.File]::Exists($full) -or [System.IO.Directory]::Exists($full)) {
        $item = Get-Item -LiteralPath $full -Force
        if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "current-status cache pathがreparse pointです: $full"
        }
    }
    return $full
}

function Assert-SnapshotCacheTreeHasNoReparsePoints {
    param([string]$Root)

    if (-not [System.IO.Directory]::Exists($Root)) {
        return
    }
    $pending = New-Object System.Collections.Generic.Stack[string]
    $pending.Push($Root)
    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        foreach ($entry in (Get-ChildItem -LiteralPath $directory -Force)) {
            if (($entry.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "current-status cache内にreparse pointがあります: $($entry.FullName)"
            }
            if ($entry.PSIsContainer) {
                $pending.Push($entry.FullName)
            }
        }
    }
}

function Remove-SnapshotCacheTree {
    param([string]$Root, [string]$MetadataPath)

    $safeRoot = Assert-SafeSnapshotCachePath $Root $script:SnapshotCacheLeaf
    $safeMetadata = Assert-SafeSnapshotCachePath $MetadataPath $script:SnapshotCacheMetadataLeaf
    Assert-SnapshotCacheTreeHasNoReparsePoints $safeRoot
    if ([System.IO.Directory]::Exists($safeRoot)) {
        [System.IO.Directory]::Delete($safeRoot, $true)
    }
    if ([System.IO.File]::Exists($safeMetadata)) {
        [System.IO.File]::Delete($safeMetadata)
    }
}

function Enter-SnapshotCacheLock {
    param([string]$LockPath)

    $safeLock = Assert-SafeSnapshotCachePath $LockPath $script:SnapshotCacheLockLeaf
    $stream = $null
    try {
        $stream = [System.IO.File]::Open(
            $safeLock,
            [System.IO.FileMode]::OpenOrCreate,
            [System.IO.FileAccess]::ReadWrite,
            [System.IO.FileShare]::None
        )
        $stream.SetLength(0)
        $owner = $script:Utf8NoBom.GetBytes("pid=$PID`nstarted_utc=$([datetime]::UtcNow.ToString('o'))`n")
        $stream.Write($owner, 0, $owner.Length)
        $stream.Flush($true)
        return $stream
    }
    catch {
        if ($null -ne $stream) {
            $stream.Dispose()
        }
        throw "current-status snapshot cacheの排他lockを取得できません。別の収集が実行中か確認してください: $($_.Exception.Message)"
    }
}

function Invoke-CacheToolIdentity {
    param([string]$FilePath, [string[]]$Arguments, [string]$Label)

    $result = Invoke-NativeCapture $FilePath $Arguments $script:Root
    if ($result.ExitCode -ne 0) {
        throw "snapshot cache key用の$Label取得に失敗しました(exit $($result.ExitCode)): $($result.StdErr.Trim())"
    }
    return "$Label`0$($result.StdOut.Trim())`0$($result.StdErr.Trim())"
}

function Get-SnapshotCacheIdentity {
    $nodeCommand = Get-Command "node.exe" -ErrorAction Stop
    $npmCommand = Get-Command "npm.cmd" -ErrorAction Stop
    $nodeDirectory = Split-Path -Parent $npmCommand.Source
    $npmCli = Join-Path $nodeDirectory "node_modules\npm\bin\npm-cli.js"
    if (-not [System.IO.File]::Exists($npmCli)) {
        throw "snapshot cache key用のnpm-cli.jsがありません: $npmCli"
    }

    $toolchainParts = @(
        (Invoke-CacheToolIdentity "rustc.exe" @("-vV") "rustc"),
        (Invoke-CacheToolIdentity "cargo.exe" @("-vV") "cargo"),
        (Invoke-CacheToolIdentity $nodeCommand.Source @("--version") "node"),
        (Invoke-CacheToolIdentity $nodeCommand.Source @($npmCli, "--version") "npm")
    )
    $toolchainHash = Get-TextSha256 ($toolchainParts -join "`0")

    $profileLines = @(
        "cache_contract=snapshot-cache-v1",
        "cargo_registered=cargo test --workspace --locked -- --list --format terse",
        "cargo_ignored=cargo test --workspace --locked -- --list --ignored --format terse",
        "cargo_target_dir=$CargoTargetDir",
        "cargo_incremental=0",
        "cargo_term_color=never",
        "vitest_default=vitest list --json --includeTaskLocation --configLoader runner",
        "vitest_production=vitest list --json --includeTaskLocation --configLoader runner --mode=production src/lib/symmetry.test.ts"
    )
    foreach ($name in @(
        "CARGO_BUILD_TARGET",
        "CARGO_ENCODED_RUSTFLAGS",
        "NODE_OPTIONS",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTFLAGS",
        "RUSTUP_TOOLCHAIN"
    )) {
        $profileLines += "$name=$([Environment]::GetEnvironmentVariable($name))"
    }
    $profileHash = Get-TextSha256 ($profileLines -join "`n")
    $cargoLockHash = ConvertTo-Hex (Get-Sha256Bytes (Read-TrackedBytes "Cargo.lock"))
    $npmLockHash = ConvertTo-Hex (Get-Sha256Bytes (Read-TrackedBytes "apps/desktop/package-lock.json"))
    $collectorHash = ConvertTo-Hex (Get-Sha256Bytes ([System.IO.File]::ReadAllBytes($script:CollectorPath)))
    $cacheKey = Get-TextSha256 ("$toolchainHash`n$profileHash`n$cargoLockHash`n$npmLockHash`n$collectorHash")

    return [PSCustomObject][ordered]@{
        CacheKey = $cacheKey
        ToolchainHash = $toolchainHash
        ProfileHash = $profileHash
        CargoLockHash = $cargoLockHash
        NpmLockHash = $npmLockHash
        CollectorHash = $collectorHash
    }
}

function Test-ByteSequenceEqual {
    param([byte[]]$Left, [byte[]]$Right)

    if ($Left.Length -ne $Right.Length) {
        return $false
    }
    for ($index = 0; $index -lt $Left.Length; $index++) {
        if ($Left[$index] -ne $Right[$index]) {
            return $false
        }
    }
    return $true
}

function Get-SnapshotCacheSourcePaths {
    param([string]$Root)

    if (-not [System.IO.Directory]::Exists($Root)) {
        return @()
    }
    $paths = New-Object System.Collections.Generic.List[string]
    $pending = New-Object System.Collections.Generic.Stack[string]
    $pending.Push($Root)
    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        foreach ($entry in (Get-ChildItem -LiteralPath $directory -Force)) {
            if (($entry.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "current-status cache内にreparse pointがあります: $($entry.FullName)"
            }
            $relative = Get-RelativePathUnder $Root $entry.FullName "snapshot cache entry"
            if ($entry.PSIsContainer) {
                if ([string]::Equals($relative, "apps/desktop/node_modules", [System.StringComparison]::Ordinal)) {
                    continue
                }
                $pending.Push($entry.FullName)
            }
            else {
                [void]$paths.Add((ConvertTo-RepositoryPath $relative))
            }
        }
    }
    return Get-OrdinalSortedStrings $paths.ToArray()
}

function Get-CachedSourceManifestHash {
    param([string]$Root, [string[]]$Paths)

    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        foreach ($relative in $Paths) {
            $safeRelative = ConvertTo-RepositoryPath $relative
            if (Test-ForbiddenSourcePath $safeRelative) {
                throw "禁止pathがsnapshot cache metadataへ入りました: $safeRelative"
            }
            $path = Get-AbsoluteRepositoryPath $safeRelative $Root
            Assert-NoReparsePathChain $Root $path "snapshot cache $safeRelative"
            if (-not [System.IO.File]::Exists($path)) {
                throw "snapshot cache sourceがありません: $safeRelative"
            }
            $pathBytes = $script:Utf8NoBom.GetBytes($safeRelative)
            [void]$sha.TransformBlock($pathBytes, 0, $pathBytes.Length, $pathBytes, 0)
            $separator = [byte[]](0)
            [void]$sha.TransformBlock($separator, 0, 1, $separator, 0)
            $bytes = [System.IO.File]::ReadAllBytes($path)
            if ($bytes.Length -gt 0) {
                [void]$sha.TransformBlock($bytes, 0, $bytes.Length, $bytes, 0)
            }
            [void]$sha.TransformBlock($separator, 0, 1, $separator, 0)
        }
        [void]$sha.TransformFinalBlock((New-Object byte[] 0), 0, 0)
        return ConvertTo-Hex $sha.Hash
    }
    finally {
        $sha.Dispose()
    }
}

function Read-SnapshotCacheMetadata {
    param([string]$MetadataPath)

    $safeMetadata = Assert-SafeSnapshotCachePath $MetadataPath $script:SnapshotCacheMetadataLeaf
    if (-not [System.IO.File]::Exists($safeMetadata)) {
        return $null
    }
    $text = [System.IO.File]::ReadAllText($safeMetadata, $script:Utf8NoBom)
    $metadata = ConvertFrom-JsonStrict $text "current-status snapshot cache metadata"
    if ($metadata -isnot [System.Collections.IDictionary]) {
        throw "current-status snapshot cache metadataはobjectではありません"
    }
    Assert-ExactKeys $metadata @(
        "schema_version",
        "cache_key",
        "toolchain_sha256",
        "profile_sha256",
        "cargo_lock_sha256",
        "npm_lock_sha256",
        "collector_sha256",
        "source_manifest_sha256",
        "source_paths"
    ) "snapshot cache metadata"
    if ([int]$metadata["schema_version"] -ne $script:SnapshotCacheSchemaVersion) {
        throw "current-status snapshot cache metadataのschemaが一致しません"
    }
    foreach ($key in @(
        "cache_key",
        "toolchain_sha256",
        "profile_sha256",
        "cargo_lock_sha256",
        "npm_lock_sha256",
        "collector_sha256",
        "source_manifest_sha256"
    )) {
        $value = Get-RequiredString $metadata[$key] "snapshot cache metadata.$key"
        if ($value -notmatch '^[0-9a-f]{64}$') {
            throw "current-status snapshot cache metadataのhashが不正です: $key"
        }
    }
    $paths = @($metadata["source_paths"] | ForEach-Object { ConvertTo-RepositoryPath ([string]$_) })
    $sorted = @(Get-OrdinalSortedStrings $paths)
    if (-not (Test-OrdinalSequenceEqual $paths $sorted) -or @($paths | Select-Object -Unique).Count -ne $paths.Count) {
        throw "current-status snapshot cache metadataのsource_pathsがsort済み一意集合ではありません"
    }
    $metadata["source_paths"] = $paths
    return $metadata
}

function Write-SnapshotCacheMetadata {
    param(
        [string]$MetadataPath,
        [object]$Identity,
        [string]$SourceManifestHash,
        [string[]]$SourcePaths
    )

    $safeMetadata = Assert-SafeSnapshotCachePath $MetadataPath $script:SnapshotCacheMetadataLeaf
    $metadata = [ordered]@{
        schema_version = $script:SnapshotCacheSchemaVersion
        cache_key = $Identity.CacheKey
        toolchain_sha256 = $Identity.ToolchainHash
        profile_sha256 = $Identity.ProfileHash
        cargo_lock_sha256 = $Identity.CargoLockHash
        npm_lock_sha256 = $Identity.NpmLockHash
        collector_sha256 = $Identity.CollectorHash
        source_manifest_sha256 = $SourceManifestHash
        source_paths = @($SourcePaths)
    }
    $json = (ConvertTo-Json -InputObject $metadata -Compress -Depth 10) + "`n"
    [void](ConvertFrom-JsonStrict $json "new current-status snapshot cache metadata")
    $temporary = "$safeMetadata.tmp.$PID.$([Guid]::NewGuid().ToString('N'))"
    $backup = "$safeMetadata.bak.$PID.$([Guid]::NewGuid().ToString('N'))"
    try {
        [System.IO.File]::WriteAllBytes($temporary, $script:Utf8NoBom.GetBytes($json))
        if ([System.IO.File]::Exists($safeMetadata)) {
            [System.IO.File]::Replace($temporary, $safeMetadata, $backup)
            [System.IO.File]::Delete($backup)
        }
        else {
            [System.IO.File]::Move($temporary, $safeMetadata)
        }
    }
    finally {
        if ([System.IO.File]::Exists($temporary)) {
            [System.IO.File]::Delete($temporary)
        }
        if ([System.IO.File]::Exists($backup)) {
            [System.IO.File]::Delete($backup)
        }
    }
}

function Sync-TrackedSourceSnapshot {
    param([object]$Identity, [string]$ExpectedSourceManifestHash)

    $paths = Get-SnapshotCachePaths
    $root = Assert-SafeSnapshotCachePath $paths.Root $script:SnapshotCacheLeaf
    $metadataPath = Assert-SafeSnapshotCachePath $paths.Metadata $script:SnapshotCacheMetadataLeaf
    $metadata = $null
    $rebuildReason = $null
    try {
        $metadata = Read-SnapshotCacheMetadata $metadataPath
        if ($null -eq $metadata) {
            if ([System.IO.Directory]::Exists($root)) {
                $rebuildReason = "metadata missing"
            }
        }
        elseif (-not [System.IO.Directory]::Exists($root)) {
            $rebuildReason = "cache root missing"
        }
        elseif (-not [string]::Equals([string]$metadata["cache_key"], [string]$Identity.CacheKey, [System.StringComparison]::Ordinal)) {
            $rebuildReason = "toolchain/profile/lockfile/collector key changed"
        }
        elseif (-not [string]::Equals([string]$metadata["toolchain_sha256"], [string]$Identity.ToolchainHash, [System.StringComparison]::Ordinal) -or
            -not [string]::Equals([string]$metadata["profile_sha256"], [string]$Identity.ProfileHash, [System.StringComparison]::Ordinal) -or
            -not [string]::Equals([string]$metadata["cargo_lock_sha256"], [string]$Identity.CargoLockHash, [System.StringComparison]::Ordinal) -or
            -not [string]::Equals([string]$metadata["npm_lock_sha256"], [string]$Identity.NpmLockHash, [System.StringComparison]::Ordinal) -or
            -not [string]::Equals([string]$metadata["collector_sha256"], [string]$Identity.CollectorHash, [System.StringComparison]::Ordinal)) {
            $rebuildReason = "cache identity fields differ from the current toolchain/profile/lockfiles/collector"
        }
        else {
            $metadataPaths = [string[]]@($metadata["source_paths"])
            $actualCachePaths = [string[]]@(Get-SnapshotCacheSourcePaths $root)
            if (-not (Test-OrdinalSequenceEqual $metadataPaths $actualCachePaths)) {
                $rebuildReason = "cached source path set differs from metadata"
            }
            else {
                $cachedHash = Get-CachedSourceManifestHash $root $metadataPaths
                if (-not [string]::Equals($cachedHash, [string]$metadata["source_manifest_sha256"], [System.StringComparison]::Ordinal)) {
                    $rebuildReason = "cached source bytes differ from metadata"
                }
            }
        }
    }
    catch {
        $rebuildReason = "metadata/cache validation failed: $($_.Exception.Message)"
    }

    if ($null -ne $rebuildReason) {
        Write-Host "snapshot cache rebuild: $rebuildReason"
        Remove-SnapshotCacheTree $root $metadataPath
        $metadata = $null
    }
    if (-not [System.IO.Directory]::Exists($root)) {
        [void][System.IO.Directory]::CreateDirectory($root)
    }

    $previousPaths = if ($null -eq $metadata) { @() } else { [string[]]@($metadata["source_paths"]) }
    $currentPaths = [string[]]@($script:SelectedSourcePaths)
    $currentSet = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    foreach ($relative in $currentPaths) { [void]$currentSet.Add($relative) }

    $removed = 0
    foreach ($relative in $previousPaths) {
        if (-not $currentSet.Contains($relative)) {
            $obsolete = Get-AbsoluteRepositoryPath $relative $root
            Assert-NoReparsePathChain $root $obsolete "obsolete snapshot source $relative"
            if ([System.IO.File]::Exists($obsolete)) {
                [System.IO.File]::Delete($obsolete)
                $removed++
            }
        }
    }

    $added = 0
    $rewritten = 0
    $unchanged = 0
    foreach ($relative in $currentPaths) {
        $destination = Get-AbsoluteRepositoryPath $relative $root
        Assert-NoReparsePathChain $root $destination "snapshot source $relative"
        $sourceBytes = [byte[]](Read-TrackedBytes $relative)
        if ([System.IO.File]::Exists($destination)) {
            $cachedBytes = [System.IO.File]::ReadAllBytes($destination)
            if (Test-ByteSequenceEqual $sourceBytes $cachedBytes) {
                $unchanged++
                continue
            }
            [System.IO.File]::WriteAllBytes($destination, $sourceBytes)
            $rewritten++
        }
        else {
            $parent = [System.IO.Path]::GetDirectoryName($destination)
            if (-not [System.IO.Directory]::Exists($parent)) {
                [void][System.IO.Directory]::CreateDirectory($parent)
            }
            [System.IO.File]::WriteAllBytes($destination, $sourceBytes)
            $added++
        }
    }

    $actualPaths = [string[]]@(Get-SnapshotCacheSourcePaths $root)
    if (-not (Test-OrdinalSequenceEqual $currentPaths $actualPaths)) {
        throw "snapshot cache同期後のsource path集合がtracked manifestと一致しません"
    }
    $actualHash = Get-SourceManifestHash $root
    if (-not [string]::Equals($ExpectedSourceManifestHash, $actualHash, [System.StringComparison]::Ordinal)) {
        throw "snapshot cache同期後の全byte manifest hashが作業treeと一致しません"
    }
    Write-SnapshotCacheMetadata $metadataPath $Identity $actualHash $currentPaths

    return [PSCustomObject][ordered]@{
        Root = $root
        Key = $Identity.CacheKey
        Rebuilt = ($null -ne $rebuildReason)
        Added = [int]$added
        Rewritten = [int]$rewritten
        Removed = [int]$removed
        Unchanged = [int]$unchanged
        SourceManifestHash = $actualHash
    }
}

function Get-JavaScriptSerializer {
    Add-Type -AssemblyName System.Web.Extensions
    $serializer = New-Object System.Web.Script.Serialization.JavaScriptSerializer
    $serializer.MaxJsonLength = [int]::MaxValue
    $serializer.RecursionLimit = 512
    return $serializer
}

function ConvertFrom-JsonStrict {
    param([string]$Text, [string]$SourceLabel)

    try {
        return (Get-JavaScriptSerializer).DeserializeObject($Text)
    }
    catch {
        throw "JSONを解釈できません($SourceLabel): $($_.Exception.Message)"
    }
}

function Test-DictionaryKey {
    param([System.Collections.IDictionary]$Dictionary, [string]$Key)
    if ($null -eq $Dictionary) { return $false }
    foreach ($candidate in $Dictionary.Keys) {
        if ([string]::Equals([string]$candidate, $Key, [System.StringComparison]::Ordinal)) {
            return $true
        }
    }
    return $false
}

function Get-DictionaryValue {
    param(
        [System.Collections.IDictionary]$Dictionary,
        [string]$Key,
        [string]$SourceLabel
    )

    if ($null -eq $Dictionary -or -not (Test-DictionaryKey $Dictionary $Key)) {
        throw "必須keyがありません($SourceLabel): $Key"
    }
    return $Dictionary[$Key]
}

function Get-RequiredString {
    param([object]$Value, [string]$SourceLabel)

    if ($null -eq $Value -or $Value -isnot [string] -or [string]::IsNullOrWhiteSpace([string]$Value)) {
        throw "空でないstringが必要です: $SourceLabel"
    }
    return [string]$Value
}

function Convert-RustIntegerLiteral {
    param([string]$Literal, [string]$SourceLabel)

    $normalized = $Literal.Replace("_", "")
    [long]$value = 0
    if (-not [long]::TryParse($normalized, [Globalization.NumberStyles]::None, [Globalization.CultureInfo]::InvariantCulture, [ref]$value) -or $value -lt 0) {
        throw "Rust整数literalを解釈できません($SourceLabel): $Literal"
    }
    return $value
}

function Get-UniqueRegexMatch {
    param(
        [string]$Text,
        [string]$Pattern,
        [string]$SourceLabel,
        [System.Text.RegularExpressions.RegexOptions]$Options = [System.Text.RegularExpressions.RegexOptions]::Multiline
    )

    $matches = [regex]::Matches($Text, $Pattern, $Options)
    if ($matches.Count -ne 1) {
        throw "$SourceLabel を1つに特定できません(matches=$($matches.Count))"
    }
    return $matches[0]
}

function Get-TomlSectionBody {
    param([string]$Text, [string]$SectionName, [string]$SourceLabel)

    $escaped = [regex]::Escape($SectionName)
    $match = Get-UniqueRegexMatch $Text "(?ms)^[ \t]*\[$escaped\][ \t]*(?:\r?\n)(?<body>.*?)(?=^[ \t]*\[|\z)" "$SourceLabel [$SectionName]"
    return $match.Groups["body"].Value
}

function Get-CargoWorkspaceVersionValue {
    param([string]$CargoText)

    $body = Get-TomlSectionBody $CargoText "workspace.package" "Cargo.toml"
    $match = Get-UniqueRegexMatch $body '(?m)^[ \t]*version[ \t]*=[ \t]*"(?<value>[^"]+)"[ \t]*(?:#.*)?$' "[workspace.package].version"
    return Get-RequiredString $match.Groups["value"].Value "Cargo.toml [workspace.package].version"
}

function Get-WorkspaceMemberValues {
    param([string]$CargoText)

    $body = Get-TomlSectionBody $CargoText "workspace" "Cargo.toml"
    $match = Get-UniqueRegexMatch $body '(?ms)^[ \t]*members[ \t]*=[ \t]*\[(?<value>.*?)\][ \t]*(?:#.*)?$' "[workspace].members"
    $arrayBody = $match.Groups["value"].Value
    if ($arrayBody -match '\\' -or $arrayBody -match '[*?]') {
        throw "workspace memberのescape/globはschema version 1で扱えません"
    }
    $stringMatches = [regex]::Matches($arrayBody, '"(?<value>[^"]+)"')
    if ($stringMatches.Count -eq 0) {
        throw "workspace memberが空です"
    }
    $remainder = [regex]::Replace($arrayBody, '"[^"]+"', '')
    $remainder = [regex]::Replace($remainder, '(?m)#.*$', '')
    $remainder = $remainder.Replace(",", "")
    if (-not [string]::IsNullOrWhiteSpace($remainder)) {
        throw "workspace membersに未対応の構文があります: $($remainder.Trim())"
    }

    $members = New-Object System.Collections.Generic.List[string]
    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    foreach ($item in $stringMatches) {
        $value = ConvertTo-RepositoryPath $item.Groups["value"].Value
        if (-not $seen.Add($value)) {
            throw "workspace memberが重複しています: $value"
        }
        $manifest = "$value/Cargo.toml"
        [void](Assert-TrackedPath $manifest)
        if (-not [System.IO.File]::Exists((Get-AbsoluteRepositoryPath $manifest $script:SnapshotRoot))) {
            throw "workspace memberのCargo.tomlがsnapshotにありません: $manifest"
        }
        [void]$members.Add($value)
    }
    return $members.ToArray()
}

function Get-MarkdownSection {
    param([string]$Text, [string]$HeadingPattern, [string]$SourceLabel)

    $match = Get-UniqueRegexMatch $Text "(?ms)^(?<heading>$HeadingPattern)[ \t]*\r?\n(?<body>.*?)(?=^#{1,3}[ \t]|\z)" $SourceLabel
    return [PSCustomObject][ordered]@{
        Heading = $match.Groups["heading"].Value
        Body    = $match.Groups["body"].Value
    }
}

function Get-WorkspaceVersion {
    $cargoText = Read-TrackedText "Cargo.toml" $script:SnapshotRoot
    $value = Get-CargoWorkspaceVersionValue $cargoText

    $packageJson = ConvertFrom-JsonStrict (Read-TrackedText "apps/desktop/package.json" $script:SnapshotRoot) "apps/desktop/package.json"
    $packageValue = Get-RequiredString (Get-DictionaryValue $packageJson "version" "apps/desktop/package.json") "apps/desktop/package.json /version"

    $lockJson = ConvertFrom-JsonStrict (Read-TrackedText "apps/desktop/package-lock.json" $script:SnapshotRoot) "apps/desktop/package-lock.json"
    $lockRootValue = Get-RequiredString (Get-DictionaryValue $lockJson "version" "apps/desktop/package-lock.json") "package-lock /version"
    $packages = Get-DictionaryValue $lockJson "packages" "apps/desktop/package-lock.json"
    if ($packages -isnot [System.Collections.IDictionary]) {
        throw "package-lock /packagesがobjectではありません"
    }
    $workspacePackage = Get-DictionaryValue $packages "" "apps/desktop/package-lock.json /packages"
    if ($workspacePackage -isnot [System.Collections.IDictionary]) {
        throw 'package-lock /packages[""]がobjectではありません'
    }
    $lockPackageValue = Get-RequiredString (Get-DictionaryValue $workspacePackage "version" 'package-lock /packages[""]') 'package-lock /packages[""]/version'

    $tauriJson = ConvertFrom-JsonStrict (Read-TrackedText "apps/desktop/src-tauri/tauri.conf.json" $script:SnapshotRoot) "apps/desktop/src-tauri/tauri.conf.json"
    $tauriValue = Get-RequiredString (Get-DictionaryValue $tauriJson "version" "tauri.conf.json") "tauri.conf.json /version"

    return [ordered]@{
        profile = "workspace-manifest-current"
        value = $value
        source = [ordered]@{
            path = "Cargo.toml"
            selector = "[workspace.package].version"
        }
        mirrors = @(
            [ordered]@{
                id = "desktop-package-version"
                path = "apps/desktop/package.json"
                selector = "/version"
                observed_value = $packageValue
                matches_source = [string]::Equals($packageValue, $value, [System.StringComparison]::Ordinal)
            },
            [ordered]@{
                id = "desktop-lock-root-version"
                path = "apps/desktop/package-lock.json"
                selector = "/version"
                observed_value = $lockRootValue
                matches_source = [string]::Equals($lockRootValue, $value, [System.StringComparison]::Ordinal)
            },
            [ordered]@{
                id = "desktop-lock-package-version"
                path = "apps/desktop/package-lock.json"
                selector = '/packages[""]/version'
                observed_value = $lockPackageValue
                matches_source = [string]::Equals($lockPackageValue, $value, [System.StringComparison]::Ordinal)
            },
            [ordered]@{
                id = "tauri-config-version"
                path = "apps/desktop/src-tauri/tauri.conf.json"
                selector = "/version"
                observed_value = $tauriValue
                matches_source = [string]::Equals($tauriValue, $value, [System.StringComparison]::Ordinal)
            }
        )
    }
}

function Get-WorkspaceMembers {
    $members = @(Get-WorkspaceMemberValues (Read-TrackedText "Cargo.toml" $script:SnapshotRoot))
    $requirements = Read-TrackedText "docs/requirements-definition.md" $script:SnapshotRoot
    $section = Get-MarkdownSection $requirements '### 9\.2 Rustクレート構成' "requirements §9.2"
    $rows = [regex]::Matches($section.Body, '(?m)^\| `(?<name>[^`]+)` \|')
    if ($rows.Count -eq 0) {
        throw "requirements §9.2のcrate表が空です"
    }
    $observedCount = [int]$rows.Count
    $observedMembers = New-Object System.Collections.Generic.List[string]
    foreach ($row in $rows) {
        $name = [string]$row.Groups["name"].Value
        $member = if ($name.StartsWith("apps/", [System.StringComparison]::Ordinal)) { $name } else { "crates/$name" }
        [void]$observedMembers.Add((ConvertTo-RepositoryPath $member))
    }
    $memberSetMatches = Test-OrdinalSetEqual $members $observedMembers.ToArray()
        $script:MirrorSetDiagnostics["requirements-crate-table"] = "source_value_set=[$($members -join ',')]; observed_value_set=[$($observedMembers -join ',')]"

    return [ordered]@{
        profile = "cargo-workspace"
        count = [int]$members.Count
        members = @($members)
        source = [ordered]@{
            path = "Cargo.toml"
            selector = "[workspace].members"
        }
        mirrors = @(
            [ordered]@{
                id = "requirements-crate-table"
                path = "docs/requirements-definition.md"
                selector = "section:9.2/table:crate-responsibilities"
                observed_count = $observedCount
                matches_source = ($observedCount -eq $members.Count -and $memberSetMatches)
            }
        )
    }
}

function Find-MatchingRustDelimiter {
    param(
        [string]$Text,
        [int]$OpenIndex,
        [char]$Open,
        [char]$Close,
        [string]$SourceLabel
    )

    if ($OpenIndex -lt 0 -or $OpenIndex -ge $Text.Length -or $Text[$OpenIndex] -ne $Open) {
        throw "$SourceLabel opening delimiter is invalid"
    }
    $depth = 0
    $state = "code"
    $blockDepth = 0
    for ($index = $OpenIndex; $index -lt $Text.Length; $index++) {
        $character = $Text[$index]
        $next = if (($index + 1) -lt $Text.Length) { $Text[$index + 1] } else { [char]0 }
        if ($state -eq "line-comment") {
            if ($character -eq "`n") { $state = "code" }
            continue
        }
        if ($state -eq "block-comment") {
            if ($character -eq '/' -and $next -eq '*') { $blockDepth++; $index++; continue }
            if ($character -eq '*' -and $next -eq '/') {
                $blockDepth--; $index++
                if ($blockDepth -eq 0) { $state = "code" }
            }
            continue
        }
        if ($state -eq "string") {
            if ($character -eq '\') { $index++; continue }
            if ($character -eq '"') { $state = "code" }
            continue
        }
        if ($character -eq '/' -and $next -eq '/') { $state = "line-comment"; $index++; continue }
        if ($character -eq '/' -and $next -eq '*') { $state = "block-comment"; $blockDepth = 1; $index++; continue }
        if ($character -eq '"') { $state = "string"; continue }
        if (($character -eq 'r' -or $character -eq 'b') -and ($index + 1) -lt $Text.Length) {
            $rawMatch = [regex]::Match($Text.Substring($index), '^(?:br|r)(?<hash>#{0,16})"')
            if ($rawMatch.Success) {
                $terminator = '"' + $rawMatch.Groups["hash"].Value
                $rawEnd = $Text.IndexOf($terminator, $index + $rawMatch.Length, [System.StringComparison]::Ordinal)
                if ($rawEnd -lt 0) { throw "$SourceLabel has an unterminated raw string" }
                $index = $rawEnd + $terminator.Length - 1
                continue
            }
        }
        if ($character -eq "'") {
            if (($index + 2) -lt $Text.Length -and $Text[$index + 2] -eq "'") { $index += 2; continue }
            if (($index + 3) -lt $Text.Length -and $Text[$index + 1] -eq '\' -and $Text[$index + 3] -eq "'") { $index += 3; continue }
        }
        if ($character -eq $Open) { $depth++; continue }
        if ($character -eq $Close) {
            $depth--
            if ($depth -eq 0) { return $index }
            if ($depth -lt 0) { break }
        }
    }
    throw "$SourceLabel closing delimiter was not found"
}

function Get-RustHandlerPaths {
    param([string]$Text)

    $runMatch = Get-UniqueRegexMatch $Text '(?m)^[ \t]*pub[ \t]+fn[ \t]+run[ \t]*\(' "lib.rs::run"
    $runOpen = $Text.IndexOf('{', $runMatch.Index + $runMatch.Length)
    if ($runOpen -lt 0) { throw "lib.rs::run body was not found" }
    $runClose = Find-MatchingRustDelimiter $Text $runOpen '{' '}' "lib.rs::run"
    $runBody = $Text.Substring($runOpen + 1, $runClose - $runOpen - 1)
    $startMatch = Get-UniqueRegexMatch $runBody 'tauri::generate_handler!\s*\[' "lib.rs run/tauri::generate_handler!" ([Text.RegularExpressions.RegexOptions]::Multiline)
    $open = $startMatch.Index + $startMatch.Length - 1
    $end = Find-MatchingRustDelimiter $runBody $open '[' ']' "run/tauri::generate_handler!"
    $body = $runBody.Substring($open + 1, $end - $open - 1)
    $body = [regex]::Replace($body, '(?m)//.*$', '')
    $paths = New-Object System.Collections.Generic.List[string]
    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    foreach ($raw in $body.Split(',')) {
        $path = $raw.Trim()
        if ($path.Length -eq 0) {
            continue
        }
        if ($path -notmatch '^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+$') {
            throw "generate_handler! の未対応entryです: $path"
        }
        if (-not $seen.Add($path)) {
            throw "generate_handler! entryが重複しています: $path"
        }
        [void]$paths.Add($path)
    }
    if ($paths.Count -eq 0) {
        throw "generate_handler! が空です"
    }
    return $paths.ToArray()
}

function Get-TauriCommandAttributes {
    param([string]$Text)

    $pattern = '(?ms)^[ \t]*#\[[ \t]*tauri::command(?<args>[^\]]*)\][ \t]*\r?\n(?:(?:^[ \t]*#\[[^\]]+\][ \t]*\r?\n)*)^[ \t]*(?:pub(?:\([^\)]*\))?[ \t]+)?(?:async[ \t]+)?fn[ \t]+(?<name>[A-Za-z_][A-Za-z0-9_]*)[ \t]*\('
    $matches = [regex]::Matches($Text, $pattern)
    if ($matches.Count -eq 0) {
        throw "#[tauri::command] functionがありません"
    }
    $commands = New-Object System.Collections.Generic.List[object]
    $seenRust = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    $seenIpc = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    foreach ($match in $matches) {
        $rustName = $match.Groups["name"].Value
        $args = $match.Groups["args"].Value.Trim()
        if ($args.Length -gt 0) {
            if (-not $args.StartsWith("(", [System.StringComparison]::Ordinal) -or -not $args.EndsWith(")", [System.StringComparison]::Ordinal)) {
                throw "tauri command arguments are not a parenthesized list: $rustName"
            }
            $args = $args.Substring(1, $args.Length - 2).Trim()
        }
        $rename = $null
        $asyncSeen = $false
        if ($args.Length -gt 0) {
            foreach ($rawArgument in $args.Split(',')) {
                $argument = $rawArgument.Trim()
                if ([string]::Equals($argument, "async", [System.StringComparison]::Ordinal)) {
                    if ($asyncSeen) { throw "tauri command async argument is duplicated: $rustName" }
                    $asyncSeen = $true
                    continue
                }
                $renameMatch = [regex]::Match($argument, '^rename[ \t]*=[ \t]*"(?<name>[a-z][a-z0-9_]*)"$')
                if ($renameMatch.Success) {
                    if ($null -ne $rename) { throw "tauri command rename is duplicated: $rustName" }
                    $rename = $renameMatch.Groups["name"].Value
                    continue
                }
                throw "unsupported tauri command argument for $rustName`: $argument"
            }
        }
        $ipcName = if ($null -ne $rename) { $rename } else { $rustName }
        if (-not $seenRust.Add($rustName) -or -not $seenIpc.Add($ipcName)) {
            throw "tauri command名が重複しています: $rustName / $ipcName"
        }
        [void]$commands.Add([PSCustomObject][ordered]@{
            RustName = $rustName
            IpcName = $ipcName
        })
    }
    return $commands.ToArray()
}

function Get-FrontendInvokeWrappers {
    param([string]$Text)

    $functions = [regex]::Matches($Text, '(?m)^[ \t]*export[ \t]+function[ \t]+(?<name>[A-Za-z_$][A-Za-z0-9_$]*)[ \t]*\(')
    if ($functions.Count -eq 0) {
        throw "frontend export functionがありません"
    }
    $allInvoke = [regex]::Matches($Text, '\binvoke[ \t]*\(')
    $wrappers = New-Object System.Collections.Generic.List[object]
    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    for ($index = 0; $index -lt $functions.Count; $index++) {
        $start = $functions[$index].Index
        $end = if (($index + 1) -lt $functions.Count) { $functions[$index + 1].Index } else { $Text.Length }
        $segment = $Text.Substring($start, $end - $start)
        $invoke = [regex]::Matches($segment, '(?m)^[ \t]*return[ \t]+invoke[ \t]*\([ \t]*"(?<name>[^"]+)"')
        if ($invoke.Count -ne 1) {
            throw "export functionのdirect return invokeを1つに特定できません: $($functions[$index].Groups['name'].Value)"
        }
        $ipcName = $invoke[0].Groups["name"].Value
        if (-not $seen.Add($ipcName)) {
            throw "frontend invoke名が重複しています: $ipcName"
        }
        [void]$wrappers.Add([PSCustomObject][ordered]@{
            FunctionName = $functions[$index].Groups["name"].Value
            IpcName = $ipcName
        })
    }
    if ($allInvoke.Count -ne $wrappers.Count) {
        throw "wrapper外または複数のinvokeがあります(total=$($allInvoke.Count), wrappers=$($wrappers.Count))"
    }
    return $wrappers.ToArray()
}

function Get-MarkdownBacktickNames {
    param([string]$Text)

    $names = New-Object System.Collections.Generic.List[string]
    foreach ($match in [regex]::Matches($Text, '`(?<name>[a-z][a-z0-9_]*)`')) {
        [void]$names.Add($match.Groups["name"].Value)
    }
    return $names.ToArray()
}

function Test-OrdinalSequenceEqual {
    param([string[]]$Left, [string[]]$Right)

    if ($Left.Count -ne $Right.Count) {
        return $false
    }
    for ($index = 0; $index -lt $Left.Count; $index++) {
        if (-not [string]::Equals($Left[$index], $Right[$index], [System.StringComparison]::Ordinal)) {
            return $false
        }
    }
    return $true
}

function Test-OrdinalSetEqual {
    param([string[]]$Left, [string[]]$Right)

    if ($Left.Count -ne $Right.Count) {
        return $false
    }
    $leftSorted = @(Get-OrdinalSortedStrings $Left)
    $rightSorted = @(Get-OrdinalSortedStrings $Right)
    return Test-OrdinalSequenceEqual $leftSorted $rightSorted
}

function Get-TauriCommands {
    $handlerPaths = @(Get-RustHandlerPaths (Read-TrackedText "apps/desktop/src-tauri/src/lib.rs" $script:SnapshotRoot))
    $attributes = @(Get-TauriCommandAttributes (Read-TrackedText "apps/desktop/src-tauri/src/commands.rs" $script:SnapshotRoot))
    $attributeByRust = New-Object 'System.Collections.Generic.Dictionary[string,object]' ($script:Ordinal)
    foreach ($attribute in $attributes) {
        $attributeByRust.Add([string]$attribute.RustName, $attribute)
    }

    $commands = New-Object System.Collections.Generic.List[object]
    $handlerIpcNames = New-Object System.Collections.Generic.List[string]
    foreach ($path in $handlerPaths) {
        $rustName = $path.Substring($path.LastIndexOf("::", [System.StringComparison]::Ordinal) + 2)
        if (-not $attributeByRust.ContainsKey($rustName)) {
            throw "handler entryに対応する#[tauri::command]がありません: $path"
        }
        $attribute = $attributeByRust[$rustName]
        [void]$handlerIpcNames.Add([string]$attribute.IpcName)
        [void]$commands.Add([ordered]@{
            registration_path = $path
            rust_function = $rustName
            ipc_name = [string]$attribute.IpcName
        })
    }
    if ($attributes.Count -ne $commands.Count) {
        throw "handlerと#[tauri::command]の件数が違います(handler=$($commands.Count), attributes=$($attributes.Count))"
    }

    $wrappers = @(Get-FrontendInvokeWrappers (Read-TrackedText "apps/desktop/src/ipc/client.ts" $script:SnapshotRoot))
    $wrapperNames = @($wrappers | ForEach-Object { [string]$_.IpcName })
    if ($wrappers.Count -ne $commands.Count -or -not (Test-OrdinalSetEqual $wrapperNames $handlerIpcNames.ToArray())) {
        throw "frontend invoke wrappers and Tauri handler IPC names differ(handler=[$($handlerIpcNames -join ',')], wrappers=[$($wrapperNames -join ',')])"
    }

    $requirementsText = Read-TrackedText "docs/requirements-definition.md" $script:SnapshotRoot
    $requirementsSection = Get-MarkdownSection $requirementsText '### 9\.3 IPCコマンド[^\r\n]*' "requirements §9.3"
    $requirementsCountMatch = Get-UniqueRegexMatch $requirementsSection.Heading '現在(?<count>[0-9]+)個' "requirements §9.3 current count" ([Text.RegularExpressions.RegexOptions]::None)
    $requirementsCount = [int]$requirementsCountMatch.Groups["count"].Value
    $requirementsRows = [regex]::Matches($requirementsSection.Body, '(?m)^\| `(?<name>[a-z][a-z0-9_]*)` \|')
    if ($requirementsRows.Count -eq 0) {
        throw "requirements §9.3 command table is empty"
    }
    $requirementsNames = @($requirementsRows | ForEach-Object { $_.Groups["name"].Value })
    $requirementsLastRow = $requirementsRows[$requirementsRows.Count - 1]
    $requirementsAfterTable = $requirementsSection.Body.Substring($requirementsLastRow.Index + $requirementsLastRow.Length)
    $requirementsParagraphMatch = Get-UniqueRegexMatch $requirementsAfterTable '(?m)^[ \t]*(?<count>[0-9]+)個であること自体は[^\r\n]*$' "requirements §9.3 following current-count paragraph"
    $requirementsParagraphCount = [int]$requirementsParagraphMatch.Groups["count"].Value
        $script:MirrorSetDiagnostics["requirements-command-table"] = "heading_count=$requirementsCount; paragraph_count=$requirementsParagraphCount; source_value_set=[$($handlerIpcNames -join ',')]; observed_value_set=[$($requirementsNames -join ',')]"

    $implementation = Read-TrackedText "docs/implementation-roadmap.md" $script:SnapshotRoot
    $fileMapSection = Get-MarkdownSection $implementation '## 1\. 最終ファイル構成マップ' "implementation §1 file map"
    $fileTreeFence = Get-UniqueRegexMatch $fileMapSection.Body '(?ms)^```[A-Za-z0-9_-]*[ \t]*\r?\n(?<body>.*?)^```[ \t]*\r?$' "implementation §1 unique fenced file tree"
    $fileTree = $fileTreeFence.Groups["body"].Value
    $treeCommandCount = [int](Get-UniqueRegexMatch $fileTree 'commands\.rs[ \t]+#[^\r\n]*Tauriコマンド(?<count>[0-9]+)個' "implementation tree command count").Groups["count"].Value
    $treeWrapperCount = [int](Get-UniqueRegexMatch $fileTree 'ipc/client\.ts[ \t]+#[^\r\n]*invokeラッパー(?<count>[0-9]+)関数' "implementation tree wrapper count").Groups["count"].Value
    $ipcSection = Get-MarkdownSection $implementation '### IPCコマンド一覧\([^\r\n]*' "implementation IPC list"
    $ipcCount = [int](Get-UniqueRegexMatch $ipcSection.Heading '現在(?<count>[0-9]+)個' "implementation IPC current count" ([Text.RegularExpressions.RegexOptions]::None)).Groups["count"].Value
    $ipcListMatch = Get-UniqueRegexMatch $ipcSection.Body '(?m)^`(?<list>[a-z0-9_ /]+)`[ \t]*$' "implementation IPC slash list"
    $ipcListLine = $ipcListMatch.Groups["list"].Value
    $ipcNames = @($ipcListLine.Split('/') | ForEach-Object { $_.Trim() } | Where-Object { $_.Length -gt 0 })
    $ipcAfterList = $ipcSection.Body.Substring($ipcListMatch.Index + $ipcListMatch.Length)
    $ipcParagraphMatch = Get-UniqueRegexMatch $ipcAfterList '(?m)^[ \t]*(?<count>[0-9]+)個であること自体は[^\r\n]*$' "implementation IPC following current-count paragraph"
    $ipcParagraphCount = [int]$ipcParagraphMatch.Groups["count"].Value
        $script:MirrorSetDiagnostics["implementation-ipc-list"] = "heading_count=$ipcCount; paragraph_count=$ipcParagraphCount; source_value_set=[$($handlerIpcNames -join ',')]; observed_value_set=[$($ipcNames -join ',')]"
    $commonSection = Get-MarkdownSection $implementation '## 3\. マイルストーン完了時の共通チェック' "implementation §3"
    $commonCount = [int](Get-UniqueRegexMatch $commonSection.Body '(?m)^[ \t]*3\.[^\r\n]*IPC[^\r\n]*現在の(?<count>[0-9]+)個' "implementation §3 item 3").Groups["count"].Value

    $improvement = Read-TrackedText "docs/improvement-roadmap-2026-08-24.md" $script:SnapshotRoot
    $improvementSection = Get-MarkdownSection $improvement '### 11\.1 目的と方針' "improvement §11.1"
    $improvementCount = [int](Get-UniqueRegexMatch $improvementSection.Body 'handler[^\r\n]*?(?<count>[0-9]+)個' "improvement §11.1 handler count").Groups["count"].Value
    $acceptanceSection = Get-MarkdownSection $improvement '### 11\.4 数値の合格条件' "improvement §11.4"
    $acceptanceCount = [int](Get-UniqueRegexMatch $acceptanceSection.Body '(?m)^[ \t]*2\.[^\r\n]*Tauri command[ \t]+(?<count>[0-9]+)個' "improvement §11.4 item2").Groups["count"].Value

    $sourceCount = $commands.Count
    return [ordered]@{
        profile = "desktop.invoke_handler"
        count = [int]$sourceCount
        commands = @($commands.ToArray())
        cross_checks = [ordered]@{
            tauri_command_attribute_count = [int]$attributes.Count
            frontend_invoke_wrapper_count = [int]$wrappers.Count
            frontend_invoke_names = @($wrapperNames)
        }
        source = [ordered]@{
            path = "apps/desktop/src-tauri/src/lib.rs"
            selector = "run/tauri::generate_handler!"
        }
        cross_check_sources = @(
            [ordered]@{
                id = "tauri-command-attributes"
                path = "apps/desktop/src-tauri/src/commands.rs"
                selector = "top-level-function/attribute:tauri::command"
            },
            [ordered]@{
                id = "frontend-invoke-wrappers"
                path = "apps/desktop/src/ipc/client.ts"
                selector = "top-level-export-function/body:single-direct-return-invoke-string-literal"
            }
        )
        mirrors = @(
            [ordered]@{
                id = "requirements-command-table"
                path = "docs/requirements-definition.md"
                selector = "section:9.3/header+command-table+following-current-count-paragraph"
                comparison_fields = @("handler.count", "handler.ipc_names")
                source_count = [int]$sourceCount
                observed_count = $requirementsCount
                observed_names = @($requirementsNames)
                matches_source = ($requirementsCount -eq $sourceCount -and $requirementsParagraphCount -eq $sourceCount -and (Test-OrdinalSetEqual $requirementsNames $handlerIpcNames.ToArray()))
            },
            [ordered]@{
                id = "implementation-tree-command-count"
                path = "docs/implementation-roadmap.md"
                selector = "architecture-tree/apps/desktop/src-tauri/src/commands.rs"
                comparison_fields = @("handler.count")
                source_count = [int]$sourceCount
                observed_count = $treeCommandCount
                observed_names = @()
                matches_source = ($treeCommandCount -eq $sourceCount)
            },
            [ordered]@{
                id = "implementation-tree-wrapper-count"
                path = "docs/implementation-roadmap.md"
                selector = "architecture-tree/apps/desktop/src/ipc/client.ts"
                comparison_fields = @("frontend_wrapper.count")
                source_count = [int]$wrappers.Count
                observed_count = $treeWrapperCount
                observed_names = @()
                matches_source = ($treeWrapperCount -eq $wrappers.Count)
            },
            [ordered]@{
                id = "implementation-ipc-list"
                path = "docs/implementation-roadmap.md"
                selector = "heading:IPCコマンド一覧/header+slash-list+following-current-count-paragraph"
                comparison_fields = @("handler.count", "handler.ipc_names")
                source_count = [int]$sourceCount
                observed_count = $ipcCount
                observed_names = @($ipcNames)
                matches_source = ($ipcCount -eq $sourceCount -and $ipcParagraphCount -eq $sourceCount -and (Test-OrdinalSetEqual $ipcNames $handlerIpcNames.ToArray()))
            },
            [ordered]@{
                id = "implementation-common-check-count"
                path = "docs/implementation-roadmap.md"
                selector = "section:3/list-item:3"
                comparison_fields = @("handler.count")
                source_count = [int]$sourceCount
                observed_count = $commonCount
                observed_names = @()
                matches_source = ($commonCount -eq $sourceCount)
            },
            [ordered]@{
                id = "improvement-section-11-current-count"
                path = "docs/improvement-roadmap-2026-08-24.md"
                selector = "section:11.1/paragraph:2"
                comparison_fields = @("handler.count")
                source_count = [int]$sourceCount
                observed_count = $improvementCount
                observed_names = @()
                matches_source = ($improvementCount -eq $sourceCount)
            },
            [ordered]@{
                id = "improvement-section-11-acceptance-count"
                path = "docs/improvement-roadmap-2026-08-24.md"
                selector = "section:11.4/list-item:2"
                comparison_fields = @("handler.count")
                source_count = [int]$sourceCount
                observed_count = $acceptanceCount
                observed_names = @()
                matches_source = ($acceptanceCount -eq $sourceCount)
            }
        )
    }
}

function Get-RustFieldInteger {
    param([string]$Block, [string]$Field, [string]$SourceLabel)

    $escaped = [regex]::Escape($Field)
    $match = Get-UniqueRegexMatch $Block "(?m)^[ \t]*$escaped[ \t]*:[ \t]*(?<value>[0-9][0-9_]*)[ \t]*," "$SourceLabel.$Field"
    return Convert-RustIntegerLiteral $match.Groups["value"].Value "$SourceLabel.$Field"
}

function Get-ProposalBudgets {
    $search = Read-TrackedText "crates/ori3-propose/src/search.rs" $script:SnapshotRoot
    $enumerate = Read-TrackedText "crates/ori3-propose/src/enumerate.rs" $script:SnapshotRoot
    $commands = Read-TrackedText "apps/desktop/src-tauri/src/commands.rs" $script:SnapshotRoot
    $endToEnd = Read-TrackedText "crates/ori3-propose/tests/end_to_end.rs" $script:SnapshotRoot

    $libraryMatch = Get-UniqueRegexMatch $search '(?ms)pub const DEFAULT:[ \t]*Self[ \t]*=[ \t]*Self[ \t]*\{(?<body>[ \t\r\n]*max_states:.*?)[ \t\r\n]*\};' "SearchBudget::DEFAULT"
    $libraryBody = $libraryMatch.Groups["body"].Value
    $libraryMaxStates = Get-RustFieldInteger $libraryBody "max_states" "SearchBudget::DEFAULT"
    $libraryMaxDepth = Get-RustFieldInteger $libraryBody "max_depth" "SearchBudget::DEFAULT"
    $libraryBranch = Get-RustFieldInteger $libraryBody "branch" "SearchBudget::DEFAULT"
    $rankMatch = Get-UniqueRegexMatch $libraryBody '(?m)^[ \t]*rank_scan[ \t]*:[ \t]*PoseScan[ \t]*\{[ \t]*steps[ \t]*:[ \t]*(?<value>[0-9][0-9_]*)[ \t]*\}[ \t]*,' "SearchBudget::DEFAULT.rank_scan"
    $libraryRankSteps = Convert-RustIntegerLiteral $rankMatch.Groups["value"].Value "SearchBudget::DEFAULT.rank_scan.steps"
    [void](Get-UniqueRegexMatch $libraryBody '(?m)^[ \t]*scan[ \t]*:[ \t]*PoseScan::DEFAULT[ \t]*,' "SearchBudget::DEFAULT.scan")

    $poseMatch = Get-UniqueRegexMatch $enumerate '(?m)^[ \t]*pub const DEFAULT:[ \t]*PoseScan[ \t]*=[ \t]*PoseScan[ \t]*\{[ \t]*steps[ \t]*:[ \t]*(?<value>[0-9][0-9_]*)[ \t]*\}[ \t]*;' "PoseScan::DEFAULT"
    $scanSteps = Convert-RustIntegerLiteral $poseMatch.Groups["value"].Value "PoseScan::DEFAULT.steps"
    $watchdogMatch = Get-UniqueRegexMatch $search '(?m)^[ \t]*pub const MAX_MILLIS:[ \t]*u64[ \t]*=[ \t]*(?<value>[0-9][0-9_]*)[ \t]*;' "SearchWatchdog::MAX_MILLIS"
    $libraryWatchdog = Convert-RustIntegerLiteral $watchdogMatch.Groups["value"].Value "SearchWatchdog::MAX_MILLIS"
    [void](Get-UniqueRegexMatch $search '(?ms)pub const DEFAULT:[ \t]*Self[ \t]*=[ \t]*Self[ \t]*\{[ \t\r\n]*max_millis:[ \t]*Self::MAX_MILLIS[ \t]*,[ \t\r\n]*\};' "SearchWatchdog::DEFAULT")

    $productMatch = Get-UniqueRegexMatch $commands '(?ms)^[ \t]*const PLAN_BUDGET:[ \t]*PlanBudget[ \t]*=[ \t]*PlanBudget[ \t]*\{(?<body>.*?)[ \t\r\n]*\};' "commands.rs::PLAN_BUDGET"
    $productBody = $productMatch.Groups["body"].Value
    $deterministicMatch = Get-UniqueRegexMatch $productBody '(?ms)deterministic:[ \t]*SearchBudget[ \t]*\{(?<body>.*?)\}[ \t]*,' "PLAN_BUDGET.deterministic"
    $deterministicBody = $deterministicMatch.Groups["body"].Value
    $productMaxStates = Get-RustFieldInteger $deterministicBody "max_states" "PLAN_BUDGET.deterministic"
    $productBranch = Get-RustFieldInteger $deterministicBody "branch" "PLAN_BUDGET.deterministic"
    [void](Get-UniqueRegexMatch $deterministicBody '(?m)^[ \t]*max_depth[ \t]*:[ \t]*SearchBudget::DEFAULT\.max_depth[ \t]*,' "PLAN_BUDGET.max_depth")
    [void](Get-UniqueRegexMatch $deterministicBody '(?m)^[ \t]*rank_scan[ \t]*:[ \t]*SearchBudget::DEFAULT\.rank_scan[ \t]*,' "PLAN_BUDGET.rank_scan")
    [void](Get-UniqueRegexMatch $deterministicBody '(?m)^[ \t]*scan[ \t]*:[ \t]*SearchBudget::DEFAULT\.scan[ \t]*,' "PLAN_BUDGET.scan")
    $productWatchdogMatch = Get-UniqueRegexMatch $productBody '(?m)^[ \t]*watchdog[ \t]*:[ \t]*SearchWatchdog[ \t]*\{[ \t]*max_millis[ \t]*:[ \t]*(?<value>[0-9][0-9_]*)[ \t]*\}[ \t]*,' "PLAN_BUDGET.watchdog"
    $productWatchdog = Convert-RustIntegerLiteral $productWatchdogMatch.Groups["value"].Value "PLAN_BUDGET.watchdog.max_millis"

    $testMatch = Get-UniqueRegexMatch $commands '(?ms)^[ \t]*const TIME_FREE_PLAN_BUDGET:[ \t]*PlanBudget[ \t]*=[ \t]*PlanBudget[ \t]*\{(?<body>.*?)[ \t\r\n]*\};' "TIME_FREE_PLAN_BUDGET"
    $testBody = $testMatch.Groups["body"].Value
    $testWatchdogMatch = Get-UniqueRegexMatch $testBody '(?m)^[ \t]*max_millis[ \t]*:[ \t]*(?<value>[0-9][0-9_]*)[ \t]*,' "TIME_FREE_PLAN_BUDGET.watchdog.max_millis"
    $testWatchdog = Convert-RustIntegerLiteral $testWatchdogMatch.Groups["value"].Value "TIME_FREE_PLAN_BUDGET.watchdog.max_millis"
    [void](Get-UniqueRegexMatch $testBody '(?m)^[ \t]*\.\.PLAN_BUDGET[ \t]*$' "TIME_FREE_PLAN_BUDGET update")

    $endMatch = Get-UniqueRegexMatch $endToEnd '(?ms)^[ \t]*const PRODUCT_PLAN_BUDGET:[ \t]*SearchBudget[ \t]*=[ \t]*SearchBudget[ \t]*\{(?<body>.*?)[ \t\r\n]*\};' "end_to_end::PRODUCT_PLAN_BUDGET"
    $endBody = $endMatch.Groups["body"].Value
    $endMaxStates = Get-RustFieldInteger $endBody "max_states" "PRODUCT_PLAN_BUDGET"
    $endBranch = Get-RustFieldInteger $endBody "branch" "PRODUCT_PLAN_BUDGET"
    [void](Get-UniqueRegexMatch $endBody '(?m)^[ \t]*max_depth[ \t]*:[ \t]*SearchBudget::DEFAULT\.max_depth[ \t]*,' "PRODUCT_PLAN_BUDGET.max_depth")

    $claude = Read-TrackedText "CLAUDE.md" $script:SnapshotRoot
    $claudeLine = (Get-UniqueRegexMatch $claude '(?m)^- \*\*#21について\*\*:[^\r\n]*$' "CLAUDE §10.6 #21").Value
    $claudeLiteralMatches = [regex]::Matches($claudeLine, 'max_millis=(?<value>[0-9][0-9_]*)')
    if ($claudeLiteralMatches.Count -gt 1) { throw "CLAUDE #21 max_millis is duplicated" }
    if ($claudeLiteralMatches.Count -eq 1) {
        $claudeWatchdogRaw = $claudeLiteralMatches[0].Groups["value"].Value
    }
    else {
        $claudeDisplay = Get-UniqueRegexMatch $claudeLine '(?<value>[0-9][0-9,]*)msのwatchdog' "CLAUDE #21 watchdog display" ([Text.RegularExpressions.RegexOptions]::None)
        $claudeWatchdogRaw = $claudeDisplay.Groups["value"].Value.Replace(",", "")
    }
    $claudeWatchdog = Convert-RustIntegerLiteral $claudeWatchdogRaw "CLAUDE #21 max_millis"
    $claudeMaxStates = Convert-RustIntegerLiteral (Get-UniqueRegexMatch $claudeLine 'max_states=(?<value>[0-9][0-9_]*)' "CLAUDE #21 max_states" ([Text.RegularExpressions.RegexOptions]::None)).Groups["value"].Value "CLAUDE #21 max_states"
    $claudeBranch = Convert-RustIntegerLiteral (Get-UniqueRegexMatch $claudeLine 'branch=(?<value>[0-9][0-9_]*)' "CLAUDE #21 branch" ([Text.RegularExpressions.RegexOptions]::None)).Groups["value"].Value "CLAUDE #21 branch"

    $improvement = Read-TrackedText "docs/improvement-roadmap-2026-08-24.md" $script:SnapshotRoot
    $introSection = Get-MarkdownSection $improvement '### 0\.2 まだ残っている項目' "improvement §0.2 budget"
    $introValue = Convert-RustIntegerLiteral ((Get-UniqueRegexMatch $introSection.Body '(?m)^追加の同期不良として[^\r\n]*?(?<value>[0-9][0-9,]*)[ \t]*ms' "improvement §0.2 current budget claim").Groups["value"].Value.Replace(",", "")) "improvement intro stale budget"
    $prioritySection = Get-MarkdownSection $improvement '### 3\.2 実装・検証の順序' "improvement §3.2 budget"
    $priorityValue = Convert-RustIntegerLiteral ((Get-UniqueRegexMatch $prioritySection.Body '(?m)^\|[ \t]*[0-9]+[ \t]*\|[ \t]*施策7[ \t]+機械検証付き文書[ \t]*\|[^\r\n]*?(?<value>[0-9][0-9,]*)/30,000[ \t]*ms' "improvement §3.2 row:施策7").Groups["value"].Value.Replace(",", "")) "improvement priority stale budget"
    $purposeSection = Get-MarkdownSection $improvement '### 11\.1 目的と方針' "improvement §11.1 budget"
    $purposeValue = Convert-RustIntegerLiteral ((Get-UniqueRegexMatch $purposeSection.Body '(?m)^version[^\r\n]*?search\.rs[^\r\n]*?(?<value>[0-9][0-9,]*)[ \t]*ms' "improvement §11.1 stale budget").Groups["value"].Value.Replace(",", "")) "improvement §11.1 stale budget"
    $failureSection = Get-MarkdownSection $improvement '### 11\.6 過去の失敗と原因' "improvement §11.6 budget"
    $failureValue = Convert-RustIntegerLiteral ((Get-UniqueRegexMatch $failureSection.Body '(?m)^-[ \t]*現在の[^\r\n]*?search\.rs[^\r\n]*?(?<value>[0-9][0-9,]*)[ \t]*ms' "improvement §11.6 stale budget").Groups["value"].Value.Replace(",", "")) "improvement §11.6 stale budget"

    $libraryProfile = [ordered]@{
        id = "library_default"
        max_states = [int]$libraryMaxStates
        max_depth = [int]$libraryMaxDepth
        branch = [int]$libraryBranch
        rank_scan_steps = [int]$libraryRankSteps
        rank_scan_points = [int]($libraryRankSteps + 1)
        scan_steps = [int]$scanSteps
        scan_points = [int]($scanSteps + 1)
        watchdog_max_millis = [long]$libraryWatchdog
        sources = @(
            [ordered]@{
                path = "crates/ori3-propose/src/search.rs"
                selector = "SearchBudget::DEFAULT+SearchWatchdog::{MAX_MILLIS,DEFAULT}"
            },
            [ordered]@{
                path = "crates/ori3-propose/src/enumerate.rs"
                selector = "PoseScan::DEFAULT"
            }
        )
    }
    $productProfile = [ordered]@{
        id = "desktop_product"
        max_states = [int]$productMaxStates
        max_depth = [int]$libraryMaxDepth
        branch = [int]$productBranch
        rank_scan_steps = [int]$libraryRankSteps
        rank_scan_points = [int]($libraryRankSteps + 1)
        scan_steps = [int]$scanSteps
        scan_points = [int]($scanSteps + 1)
        watchdog_max_millis = [long]$productWatchdog
        sources = @(
            [ordered]@{
                path = "apps/desktop/src-tauri/src/commands.rs"
                selector = "PLAN_BUDGET"
            },
            [ordered]@{
                path = "crates/ori3-propose/src/enumerate.rs"
                selector = "PoseScan::DEFAULT"
            }
        )
    }
    $testProfile = [ordered]@{
        id = "desktop_test_time_free"
        max_states = [int]$productMaxStates
        max_depth = [int]$libraryMaxDepth
        branch = [int]$productBranch
        rank_scan_steps = [int]$libraryRankSteps
        rank_scan_points = [int]($libraryRankSteps + 1)
        scan_steps = [int]$scanSteps
        scan_points = [int]($scanSteps + 1)
        watchdog_max_millis = [long]$testWatchdog
        sources = @(
            [ordered]@{
                path = "apps/desktop/src-tauri/src/commands.rs"
                selector = "tests::TIME_FREE_PLAN_BUDGET"
            },
            [ordered]@{
                path = "apps/desktop/src-tauri/src/commands.rs"
                selector = "PLAN_BUDGET inherited fields"
            }
        )
    }

    return [ordered]@{
        profile = "resolved-operational-budgets"
        profiles = @($libraryProfile, $productProfile, $testProfile)
        mirrors = @(
            [ordered]@{ id = "end-to-end-product-max-states"; path = "crates/ori3-propose/tests/end_to_end.rs"; selector = "PRODUCT_PLAN_BUDGET.max_states"; field = "desktop_product.max_states"; source_value = [int]$productMaxStates; observed_value = [int]$endMaxStates; matches_source = ($endMaxStates -eq $productMaxStates) },
            [ordered]@{ id = "end-to-end-product-max-depth"; path = "crates/ori3-propose/tests/end_to_end.rs"; selector = "PRODUCT_PLAN_BUDGET.max_depth"; field = "desktop_product.max_depth"; source_value = [int]$libraryMaxDepth; observed_value = [int]$libraryMaxDepth; matches_source = $true },
            [ordered]@{ id = "end-to-end-product-branch"; path = "crates/ori3-propose/tests/end_to_end.rs"; selector = "PRODUCT_PLAN_BUDGET.branch"; field = "desktop_product.branch"; source_value = [int]$productBranch; observed_value = [int]$endBranch; matches_source = ($endBranch -eq $productBranch) },
            [ordered]@{ id = "claude-product-watchdog"; path = "CLAUDE.md"; selector = "section:10.6/list-item:#21"; field = "desktop_product.watchdog_max_millis"; source_value = [long]$productWatchdog; observed_value = [long]$claudeWatchdog; matches_source = ($claudeWatchdog -eq $productWatchdog) },
            [ordered]@{ id = "claude-product-max-states"; path = "CLAUDE.md"; selector = "section:10.6/list-item:#21"; field = "desktop_product.max_states"; source_value = [int]$productMaxStates; observed_value = [int]$claudeMaxStates; matches_source = ($claudeMaxStates -eq $productMaxStates) },
            [ordered]@{ id = "claude-product-branch"; path = "CLAUDE.md"; selector = "section:10.6/list-item:#21"; field = "desktop_product.branch"; source_value = [int]$productBranch; observed_value = [int]$claudeBranch; matches_source = ($claudeBranch -eq $productBranch) },
            [ordered]@{ id = "improvement-intro-stale-source-budget"; path = "docs/improvement-roadmap-2026-08-24.md"; selector = "section:0.2/paragraph:追加の同期不良"; field = "search_source_comment.desktop_product.watchdog_max_millis"; source_value = [long]$productWatchdog; observed_value = [long]$introValue; matches_source = ($introValue -eq $productWatchdog) },
            [ordered]@{ id = "improvement-priority-stale-source-budget"; path = "docs/improvement-roadmap-2026-08-24.md"; selector = "section:3.2/table:実装・検証の順序/row:施策7"; field = "search_source_comment.desktop_product.watchdog_max_millis"; source_value = [long]$productWatchdog; observed_value = [long]$priorityValue; matches_source = ($priorityValue -eq $productWatchdog) },
            [ordered]@{ id = "improvement-section-11-purpose-stale-source-budget"; path = "docs/improvement-roadmap-2026-08-24.md"; selector = "section:11.1/paragraph:1"; field = "search_source_comment.desktop_product.watchdog_max_millis"; source_value = [long]$productWatchdog; observed_value = [long]$purposeValue; matches_source = ($purposeValue -eq $productWatchdog) },
            [ordered]@{ id = "improvement-section-11-failure-stale-source-budget"; path = "docs/improvement-roadmap-2026-08-24.md"; selector = "section:11.6/list-item:4"; field = "search_source_comment.desktop_product.watchdog_max_millis"; source_value = [long]$productWatchdog; observed_value = [long]$failureValue; matches_source = ($failureValue -eq $productWatchdog) }
        )
    }
}

function Get-ManualPageCount {
    $relative = "docs/manual/ORIGAMI3取扱説明書.pdf"
    $bytes = Read-TrackedBytes $relative $script:SnapshotRoot
    if ($bytes.Length -lt 5 -or $script:Latin1.GetString($bytes, 0, 5) -ne "%PDF-") {
        throw "manual PDFのsignatureがありません"
    }
    $text = $script:Latin1.GetString($bytes)
    $mediaBoxCount = [regex]::Matches($text, '/MediaBox\b').Count

    $objects = New-Object 'System.Collections.Generic.Dictionary[string,string]' ($script:Ordinal)
    foreach ($match in [regex]::Matches($text, '(?ms)(?<number>[0-9]+)[ \t]+(?<generation>[0-9]+)[ \t]+obj\b(?<body>.*?)\bendobj\b')) {
        $key = "$($match.Groups['number'].Value) $($match.Groups['generation'].Value)"
        if ($objects.ContainsKey($key)) {
            throw "PDF objectが重複しています: $key"
        }
        $objects.Add($key, $match.Groups["body"].Value)
    }
    if ($objects.Count -eq 0) {
        throw "manual PDFのobjectを読めません"
    }

    $pageObjectCount = 0
    $catalogBodies = New-Object System.Collections.Generic.List[string]
    foreach ($body in $objects.Values) {
        if ([regex]::IsMatch($body, '/Type[ \t\r\n]*/Page\b(?!s)')) {
            $pageObjectCount++
        }
        if ([regex]::IsMatch($body, '/Type[ \t\r\n]*/Catalog\b')) {
            [void]$catalogBodies.Add($body)
        }
    }
    if ($catalogBodies.Count -ne 1) {
        throw "PDF Catalogを1つに特定できません(count=$($catalogBodies.Count))"
    }
    $pagesRef = Get-UniqueRegexMatch $catalogBodies[0] '/Pages[ \t\r\n]+(?<number>[0-9]+)[ \t\r\n]+(?<generation>[0-9]+)[ \t\r\n]+R\b' "PDF Catalog /Pages" ([Text.RegularExpressions.RegexOptions]::None)
    $pagesKey = "$($pagesRef.Groups['number'].Value) $($pagesRef.Groups['generation'].Value)"
    if (-not $objects.ContainsKey($pagesKey)) {
        throw "Catalogが参照するPages objectがありません: $pagesKey"
    }
    $pagesBody = $objects[$pagesKey]
    if (-not [regex]::IsMatch($pagesBody, '/Type[ \t\r\n]*/Pages\b')) {
        throw "Catalog /Pages参照先がPages objectではありません: $pagesKey"
    }
    $countMatch = Get-UniqueRegexMatch $pagesBody '/Count[ \t\r\n]+(?<value>[0-9]+)\b' "root Pages /Count" ([Text.RegularExpressions.RegexOptions]::None)
    $pageTreeCount = [int]$countMatch.Groups["value"].Value

    if ($mediaBoxCount -le 0 -or $pageObjectCount -le 0 -or $pageTreeCount -le 0) {
        throw "manual page countが正数ではありません(MediaBox=$mediaBoxCount, Page=$pageObjectCount, Count=$pageTreeCount)"
    }
    if ($mediaBoxCount -ne $pageObjectCount -or $mediaBoxCount -ne $pageTreeCount) {
        throw "manual PDFの3方式が一致しません(MediaBox=$mediaBoxCount, Page=$pageObjectCount, Count=$pageTreeCount)"
    }

    return [ordered]@{
        profile = "published-pdf"
        page_count = [int]$pageTreeCount
        evidence = [ordered]@{
            pdf_signature_valid = $true
            media_box_count = [int]$mediaBoxCount
            page_object_count = [int]$pageObjectCount
            page_tree_count = [int]$pageTreeCount
        }
        source = [ordered]@{
            path = $relative
            selector = "%PDF- signature+/MediaBox+/Type /Page+page-tree /Count"
        }
        generator_sources = @(
            [ordered]@{
                path = "crates/ori3-export/src/manual.rs"
                selector = "manual_pdf_with_stats+manual_svg_pages+ManualPdfStats.page_count"
            },
            [ordered]@{
                path = "scripts/build-manual.ps1"
                selector = "PDF signature and /MediaBox count"
            }
        )
        mirrors = @()
    }
}

function Get-RelativePathUnder {
    param([string]$BasePath, [string]$Path, [string]$SourceLabel)

    $base = [System.IO.Path]::GetFullPath($BasePath).TrimEnd([char[]]"\/")
    $full = [System.IO.Path]::GetFullPath($Path)
    $prefix = $base + [System.IO.Path]::DirectorySeparatorChar
    if (-not $full.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$SourceLabel がbase path外です: $Path"
    }
    return $full.Substring($prefix.Length).Replace("\", "/")
}

function Get-TrackedRustSourcePaths {
    return Get-OrdinalSortedStrings @($script:SelectedSourcePaths | Where-Object { $_.EndsWith(".rs", [System.StringComparison]::Ordinal) })
}

function Get-TrackedFrontendTestPaths {
    $paths = @($script:SelectedSourcePaths | Where-Object {
        $_ -match '^apps/desktop/src/.+\.(?:test|spec)\.tsx?$'
    })
    if ($paths.Count -eq 0) {
        throw "追跡済みfrontend test sourceがありません"
    }
    return Get-OrdinalSortedStrings $paths
}

function ConvertTo-RustCodeMask {
    param([string]$Text, [string]$SourceLabel)

    $characters = $Text.ToCharArray()
    $state = "code"
    $blockDepth = 0
    for ($index = 0; $index -lt $characters.Length; $index++) {
        $character = $characters[$index]
        $next = if (($index + 1) -lt $characters.Length) { $characters[$index + 1] } else { [char]0 }
        if ($state -eq "line-comment") {
            if ($character -eq "`n") { $state = "code" } else { $characters[$index] = ' ' }
            continue
        }
        if ($state -eq "block-comment") {
            if ($character -eq '/' -and $next -eq '*') {
                $characters[$index] = ' '; $characters[$index + 1] = ' '; $blockDepth++; $index++; continue
            }
            if ($character -eq '*' -and $next -eq '/') {
                $characters[$index] = ' '; $characters[$index + 1] = ' '; $blockDepth--; $index++
                if ($blockDepth -eq 0) { $state = "code" }
                continue
            }
            if ($character -ne "`r" -and $character -ne "`n") { $characters[$index] = ' ' }
            continue
        }
        if ($state -eq "string") {
            if ($character -eq '\') {
                $characters[$index] = ' '
                if (($index + 1) -lt $characters.Length) { $index++; $characters[$index] = ' ' }
                continue
            }
            if ($character -eq '"') { $state = "code" }
            if ($character -ne "`r" -and $character -ne "`n") { $characters[$index] = ' ' }
            continue
        }
        if ($character -eq '/' -and $next -eq '/') {
            $characters[$index] = ' '; $characters[$index + 1] = ' '; $state = "line-comment"; $index++; continue
        }
        if ($character -eq '/' -and $next -eq '*') {
            $characters[$index] = ' '; $characters[$index + 1] = ' '; $state = "block-comment"; $blockDepth = 1; $index++; continue
        }
        if (($character -eq 'r' -or $character -eq 'b') -and ($index + 1) -lt $characters.Length) {
            $rawMatch = [regex]::Match($Text.Substring($index), '^(?:br|r)(?<hash>#{0,16})"')
            if ($rawMatch.Success) {
                $terminator = '"' + $rawMatch.Groups["hash"].Value
                $rawEnd = $Text.IndexOf($terminator, $index + $rawMatch.Length, [System.StringComparison]::Ordinal)
                if ($rawEnd -lt 0) { throw "$SourceLabel has an unterminated raw string" }
                $last = $rawEnd + $terminator.Length - 1
                for ($maskIndex = $index; $maskIndex -le $last; $maskIndex++) {
                    if ($characters[$maskIndex] -ne "`r" -and $characters[$maskIndex] -ne "`n") { $characters[$maskIndex] = ' ' }
                }
                $index = $last
                continue
            }
        }
        if ($character -eq '"') { $characters[$index] = ' '; $state = "string"; continue }
        if ($character -eq "'") {
            $charEnd = -1
            if (($index + 2) -lt $characters.Length -and $characters[$index + 2] -eq "'") { $charEnd = $index + 2 }
            elseif (($index + 3) -lt $characters.Length -and $characters[$index + 1] -eq '\' -and $characters[$index + 3] -eq "'") { $charEnd = $index + 3 }
            if ($charEnd -gt $index) {
                for ($maskIndex = $index; $maskIndex -le $charEnd; $maskIndex++) { $characters[$maskIndex] = ' ' }
                $index = $charEnd
            }
        }
    }
    if ($state -eq "block-comment" -or $state -eq "string") {
        throw "$SourceLabel has an unterminated comment or string"
    }
    return -join $characters
}

function Get-RustAttributeRecords {
    param([string]$CodeMask, [string]$SourceLabel, [string]$OriginalText = $CodeMask)

    if ($OriginalText.Length -ne $CodeMask.Length) {
        throw "$SourceLabel original text and code mask lengths differ"
    }
    $records = New-Object System.Collections.Generic.List[object]
    $search = 0
    while ($search -lt $CodeMask.Length) {
        $outerStart = $CodeMask.IndexOf('#[', $search, [System.StringComparison]::Ordinal)
        $innerStart = $CodeMask.IndexOf('#![', $search, [System.StringComparison]::Ordinal)
        if ($outerStart -lt 0 -and $innerStart -lt 0) { break }
        $isInner = $innerStart -ge 0 -and ($outerStart -lt 0 -or $innerStart -lt $outerStart)
        $start = if ($isInner) { $innerStart } else { $outerStart }
        $bracket = if ($isInner) { $start + 2 } else { $start + 1 }
        $end = Find-MatchingRustDelimiter $CodeMask $bracket '[' ']' "$SourceLabel attribute"
        [void]$records.Add([PSCustomObject][ordered]@{
            Start = [int]$start
            End = [int]$end
            Inner = [bool]$isInner
            Text = $CodeMask.Substring($start, $end - $start + 1)
            RawText = $OriginalText.Substring($start, $end - $start + 1)
        })
        $search = $end + 1
    }
    return $records.ToArray()
}

function Test-RustTargetAttributeAffectsInventory {
    param([string]$AttributeText)

    $targetPattern = '\b(?:target_arch|target_feature|target_os|target_env|target_family|target_pointer_width|target_endian|windows|unix)\b'
    if ($AttributeText -notmatch $targetPattern) { return $false }
    if ($AttributeText -match '^#!?\[[ \t\r\n]*cfg[ \t\r\n]*\(') { return $true }
    if ($AttributeText -match '^#!?\[[ \t\r\n]*cfg_attr[ \t\r\n]*\(' -and
        $AttributeText -match ',[\s\S]*(?:cfg[ \t\r\n]*\(|cfg_attr[ \t\r\n]*\(|test\b|ignore\b|path\b)') {
        return $true
    }
    return $false
}

function Get-RustEnclosingBraceRange {
    param([string]$CodeMask, [int]$Position, [string]$SourceLabel)

    $stack = New-Object System.Collections.Generic.List[int]
    for ($index = 0; $index -lt $Position; $index++) {
        if ($CodeMask[$index] -eq '{') {
            [void]$stack.Add($index)
        }
        elseif ($CodeMask[$index] -eq '}') {
            if ($stack.Count -eq 0) { throw "$SourceLabel has an unmatched closing brace" }
            $stack.RemoveAt($stack.Count - 1)
        }
    }
    if ($stack.Count -eq 0) {
        return [PSCustomObject][ordered]@{ Start = 0; End = [int]$CodeMask.Length }
    }
    $open = $stack[$stack.Count - 1]
    $close = Find-MatchingRustDelimiter $CodeMask $open '{' '}' "$SourceLabel enclosing module"
    return [PSCustomObject][ordered]@{ Start = [int]($open + 1); End = [int]$close }
}

function Get-RustExternalModuleDeclarations {
    param([string]$CodeMask, [object[]]$Attributes)

    $declarations = New-Object System.Collections.Generic.List[object]
    foreach ($match in [regex]::Matches($CodeMask, '\bmod[ \t\r\n]+(?<name>[A-Za-z_][A-Za-z0-9_]*)[ \t\r\n]*;')) {
        $insideAttribute = $false
        foreach ($attribute in $Attributes) {
            if ($match.Index -ge $attribute.Start -and $match.Index -le $attribute.End) {
                $insideAttribute = $true
                break
            }
        }
        if (-not $insideAttribute) {
            [void]$declarations.Add([PSCustomObject][ordered]@{
                Start = [int]$match.Index
                Name = [string]$match.Groups["name"].Value
            })
        }
    }
    return $declarations.ToArray()
}

function Get-RustInlineModuleAncestors {
    param([string]$CodeMask, [object[]]$Attributes, [int]$Position, [string]$SourceLabel)

    $names = New-Object System.Collections.Generic.List[string]
    foreach ($match in [regex]::Matches($CodeMask, '\bmod[ \t\r\n]+(?<name>[A-Za-z_][A-Za-z0-9_]*)[ \t\r\n]*\{')) {
        $insideAttribute = $false
        foreach ($attribute in $Attributes) {
            if ($match.Index -ge $attribute.Start -and $match.Index -le $attribute.End) {
                $insideAttribute = $true
                break
            }
        }
        if ($insideAttribute) { continue }
        $open = $match.Index + $match.Length - 1
        $close = Find-MatchingRustDelimiter $CodeMask $open '{' '}' "$SourceLabel inline module"
        if ($Position -gt $open -and $Position -lt $close) {
            [void]$names.Add([string]$match.Groups["name"].Value)
        }
    }
    return $names.ToArray()
}

function Join-RepositoryPathParts {
    param([string[]]$Parts)

    $nonempty = @($Parts | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    return ($nonempty -join '/').Trim('/')
}

function Resolve-RustExternalModulePath {
    param(
        [string]$SourceRelative,
        [string]$CodeMask,
        [object[]]$Attributes,
        [int]$DeclarationPosition,
        [string]$ModuleName,
        [string]$KnownModuleBase = ""
    )

    $precedingGroup = New-Object System.Collections.Generic.List[object]
    $candidateIndex = $Attributes.Count - 1
    while ($candidateIndex -ge 0 -and $Attributes[$candidateIndex].Start -gt $DeclarationPosition) { $candidateIndex-- }
    if ($candidateIndex -ge 0 -and -not $Attributes[$candidateIndex].Inner) {
        $between = $CodeMask.Substring($Attributes[$candidateIndex].End + 1, $DeclarationPosition - $Attributes[$candidateIndex].End - 1)
        if ([string]::IsNullOrWhiteSpace($between)) {
            [void]$precedingGroup.Add($Attributes[$candidateIndex])
            while ($candidateIndex -gt 0 -and -not $Attributes[$candidateIndex - 1].Inner) {
                $between = $CodeMask.Substring($Attributes[$candidateIndex - 1].End + 1, $Attributes[$candidateIndex].Start - $Attributes[$candidateIndex - 1].End - 1)
                if (-not [string]::IsNullOrWhiteSpace($between)) { break }
                $candidateIndex--
                [void]$precedingGroup.Add($Attributes[$candidateIndex])
            }
        }
    }
    $precedingPathAttributes = @($precedingGroup | Where-Object {
        $_.RawText -match '^#\[[ \t\r\n]*path[ \t\r\n]*='
    })
    if ($precedingPathAttributes.Count -gt 0) {
        throw "$SourceRelative module $ModuleName uses #[path]; schema 1 requires an explicit module resolver"
    }

    $sourceDirectory = [System.IO.Path]::GetDirectoryName($SourceRelative).Replace('\', '/')
    $stem = [System.IO.Path]::GetFileNameWithoutExtension($SourceRelative)
    $baseCandidates = New-Object System.Collections.Generic.List[string]
    if (-not [string]::IsNullOrWhiteSpace($KnownModuleBase)) {
        [void]$baseCandidates.Add($KnownModuleBase.Trim('/'))
    }
    elseif ($stem -eq 'lib' -or $stem -eq 'main' -or $stem -eq 'mod' -or $stem -eq 'build') {
        [void]$baseCandidates.Add($sourceDirectory.Trim('/'))
    }
    else {
        [void]$baseCandidates.Add((Join-RepositoryPathParts @($sourceDirectory, $stem)))
        [void]$baseCandidates.Add($sourceDirectory.Trim('/'))
    }

    $inlineAncestors = @(Get-RustInlineModuleAncestors $CodeMask $Attributes $DeclarationPosition $SourceRelative)
    $matches = New-Object System.Collections.Generic.List[object]
    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    foreach ($base in $baseCandidates) {
        $moduleParts = @($base) + @($inlineAncestors) + @($ModuleName)
        $moduleBase = Join-RepositoryPathParts $moduleParts
        foreach ($candidate in @("$moduleBase.rs", "$moduleBase/mod.rs")) {
            $normalized = ConvertTo-RepositoryPath $candidate
            if ($script:TrackedSet.Contains($normalized) -and $seen.Add($normalized)) {
                [void]$matches.Add([PSCustomObject][ordered]@{
                    Path = $normalized
                    ModuleBase = $moduleBase
                })
            }
        }
    }
    if ($matches.Count -ne 1) {
        throw "$SourceRelative module $ModuleName did not resolve to exactly one tracked Rust file(count=$($matches.Count))"
    }
    return $matches[0]
}

function Test-RustModuleTreeContainsTest {
    param([string]$SourceRelative, [string]$KnownModuleBase, [System.Collections.Generic.HashSet[string]]$Visiting)

    if (-not $Visiting.Add($SourceRelative)) {
        throw "Rust module graph has a cycle at $SourceRelative"
    }
    try {
        $text = Read-TrackedText $SourceRelative $script:SnapshotRoot
        $mask = ConvertTo-RustCodeMask $text $SourceRelative
        $attributes = @(Get-RustAttributeRecords $mask $SourceRelative $text)
        if (@($attributes | Where-Object { -not $_.Inner -and $_.Text -match '^#\[[ \t\r\n]*test[ \t\r\n]*\]$' }).Count -gt 0) {
            return $true
        }
        foreach ($declaration in @(Get-RustExternalModuleDeclarations $mask $attributes)) {
            $resolved = Resolve-RustExternalModulePath $SourceRelative $mask $attributes $declaration.Start $declaration.Name $KnownModuleBase
            if (Test-RustModuleTreeContainsTest $resolved.Path $resolved.ModuleBase $Visiting) {
                return $true
            }
        }
        return $false
    }
    finally {
        [void]$Visiting.Remove($SourceRelative)
    }
}

function Test-RustRangeContainsTestRegistration {
    param(
        [string]$SourceRelative,
        [string]$CodeMask,
        [object[]]$Attributes,
        [int]$RangeStart,
        [int]$RangeEnd
    )

    if (@($Attributes | Where-Object {
        -not $_.Inner -and $_.Start -ge $RangeStart -and $_.Start -lt $RangeEnd -and
        $_.Text -match '^#\[[ \t\r\n]*test[ \t\r\n]*\]$'
    }).Count -gt 0) {
        return $true
    }
    foreach ($declaration in @(Get-RustExternalModuleDeclarations $CodeMask $Attributes)) {
        if ($declaration.Start -lt $RangeStart -or $declaration.Start -ge $RangeEnd) { continue }
        $resolved = Resolve-RustExternalModulePath $SourceRelative $CodeMask $Attributes $declaration.Start $declaration.Name
        $visiting = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
        if (Test-RustModuleTreeContainsTest $resolved.Path $resolved.ModuleBase $visiting) {
            return $true
        }
    }
    return $false
}

function Get-TargetConditionalTestRegistrationCount {
    param([string]$CodeMask, [object[]]$Attributes, [string]$SourceLabel)

    $seenGroups = New-Object 'System.Collections.Generic.HashSet[int]'
    $count = 0
    for ($attributeIndex = 0; $attributeIndex -lt $Attributes.Count; $attributeIndex++) {
        $attribute = $Attributes[$attributeIndex]
        if (-not (Test-RustTargetAttributeAffectsInventory $attribute.Text)) { continue }
        if ($attribute.Text -match '^#!?\[[ \t\r\n]*cfg_attr[ \t\r\n]*\(' -and
            $attribute.Text -match ',[\s\S]*(?:path\b|cfg_attr[ \t\r\n]*\()') {
            throw "$SourceLabel has target-dependent path or nested cfg_attr; schema 1 cannot prove a host-independent test inventory"
        }

        if ($attribute.Inner) {
            $range = Get-RustEnclosingBraceRange $CodeMask $attribute.Start $SourceLabel
            if (Test-RustRangeContainsTestRegistration $SourceLabel $CodeMask $Attributes $range.Start $range.End) {
                $count++
            }
            continue
        }

        $groupStart = $attributeIndex
        while ($groupStart -gt 0 -and -not $Attributes[$groupStart - 1].Inner) {
            $between = $CodeMask.Substring($Attributes[$groupStart - 1].End + 1, $Attributes[$groupStart].Start - $Attributes[$groupStart - 1].End - 1)
            if (-not [string]::IsNullOrWhiteSpace($between)) { break }
            $groupStart--
        }
        $groupEnd = $attributeIndex
        while (($groupEnd + 1) -lt $Attributes.Count -and -not $Attributes[$groupEnd + 1].Inner) {
            $between = $CodeMask.Substring($Attributes[$groupEnd].End + 1, $Attributes[$groupEnd + 1].Start - $Attributes[$groupEnd].End - 1)
            if (-not [string]::IsNullOrWhiteSpace($between)) { break }
            $groupEnd++
        }
        if (-not $seenGroups.Add($groupStart)) { continue }

        $groupHasTestAttribute = $false
        for ($groupIndex = $groupStart; $groupIndex -le $groupEnd; $groupIndex++) {
            $text = $Attributes[$groupIndex].Text
            if ($text -match '^#\[[ \t\r\n]*test[ \t\r\n]*\]$' -or
                ($text -match '^#\[[ \t\r\n]*cfg_attr[ \t\r\n]*\(' -and $text -match ',[ \t\r\n]*test\b')) {
                $groupHasTestAttribute = $true
            }
        }

        $cursor = $Attributes[$groupEnd].End + 1
        while ($cursor -lt $CodeMask.Length -and [char]::IsWhiteSpace($CodeMask[$cursor])) { $cursor++ }
        $itemMatch = [regex]::Match(
            $CodeMask.Substring($cursor),
            '^(?:(?:pub(?:[ \t\r\n]*\([^\)]*\))?|unsafe|async|const|extern)[ \t\r\n]+)*(?<kind>fn|mod)[ \t\r\n]+(?<name>[A-Za-z_][A-Za-z0-9_]*)'
        )
        if (-not $itemMatch.Success) { continue }
        $kind = $itemMatch.Groups["kind"].Value
        $name = $itemMatch.Groups["name"].Value
        if ($kind -eq "fn" -and $groupHasTestAttribute) {
            $count++
            continue
        }
        if ($kind -ne "mod") { continue }

        $afterName = $cursor + $itemMatch.Length
        while ($afterName -lt $CodeMask.Length -and [char]::IsWhiteSpace($CodeMask[$afterName])) { $afterName++ }
        if ($afterName -lt $CodeMask.Length -and $CodeMask[$afterName] -eq '{') {
            $bodyEnd = Find-MatchingRustDelimiter $CodeMask $afterName '{' '}' "$SourceLabel module $name"
            if (Test-RustRangeContainsTestRegistration $SourceLabel $CodeMask $Attributes ($afterName + 1) $bodyEnd) {
                $count++
            }
        }
        elseif ($afterName -lt $CodeMask.Length -and $CodeMask[$afterName] -eq ';') {
            $resolved = Resolve-RustExternalModulePath $SourceLabel $CodeMask $Attributes $cursor $name
            $visiting = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
            if (Test-RustModuleTreeContainsTest $resolved.Path $resolved.ModuleBase $visiting) {
                $count++
            }
        }
        else {
            throw "$SourceLabel module $name has an unsupported declaration"
        }
    }
    return [int]$count
}

function Get-RustStaticInventory {
    $sites = 0
    $targetConditional = 0
    foreach ($relative in (Get-TrackedRustSourcePaths)) {
        $text = Read-TrackedText $relative $script:SnapshotRoot
        $mask = ConvertTo-RustCodeMask $text $relative
        $attributes = @(Get-RustAttributeRecords $mask $relative $text)
        $sites += @($attributes | Where-Object { -not $_.Inner -and $_.Text -match '^#\[[ \t\r\n]*test[ \t\r\n]*\]$' }).Count
        $targetConditional += Get-TargetConditionalTestRegistrationCount $mask $attributes $relative
    }
    foreach ($relative in @($script:SelectedSourcePaths | Where-Object { $_.EndsWith("Cargo.toml", [System.StringComparison]::Ordinal) })) {
        $text = Read-TrackedText $relative $script:SnapshotRoot
        if ($text -match '(?ms)^\[\[test\]\].*?^[ \t]*required-features[ \t]*=') {
            $targetConditional++
        }
    }
    if ($targetConditional -ne 0) {
        throw "target/feature依存のtest登録があります(sites=$targetConditional)。schemaへreference targetを追加してください"
    }
    return [PSCustomObject][ordered]@{
        Sites = [int]$sites
        TargetConditional = [int]$targetConditional
    }
}

function Get-CargoRunnerInventory {
    param([switch]$IgnoredOnly)

    $arguments = @("test", "--workspace", "--locked", "--", "--list")
    if ($IgnoredOnly) {
        $arguments += "--ignored"
    }
    $arguments += @("--format", "terse")
    $environment = @{
        CARGO_TARGET_DIR = $CargoTargetDir
        CARGO_TERM_COLOR = "never"
        CARGO_INCREMENTAL = "0"
    }
    $result = Invoke-NativeCapture "cargo.exe" $arguments $script:SnapshotRoot $environment
    if ($result.ExitCode -ne 0) {
        $kind = if ($IgnoredOnly) { "ignored inventory" } else { "registered inventory" }
        $stderr = $result.StdErr.Trim()
        if ($stderr.Length -gt 4000) {
            $stderr = $stderr.Substring($stderr.Length - 4000)
        }
        throw "cargo $kind が失敗しました(exit $($result.ExitCode))。追跡source snapshotだけで組み立てられるか確認してください: $stderr"
    }
    $testCount = 0
    $benchmarkCount = 0
    foreach ($line in ($result.StdOut -split "\r?\n")) {
        if ($line -match ': test$') {
            $testCount++
        }
        elseif ($line -match ': benchmark$') {
            $benchmarkCount++
        }
    }
    return [PSCustomObject][ordered]@{
        Tests = [int]$testCount
        Benchmarks = [int]$benchmarkCount
        ElapsedMs = [double]$result.ElapsedMs
    }
}

function Get-LockVitestVersion {
    $lock = ConvertFrom-JsonStrict (Read-TrackedText "apps/desktop/package-lock.json" $script:SnapshotRoot) "package-lock vitest"
    $packages = Get-DictionaryValue $lock "packages" "package-lock vitest"
    if ($packages -isnot [System.Collections.IDictionary]) {
        throw "package-lock packagesがobjectではありません"
    }
    $vitest = Get-DictionaryValue $packages "node_modules/vitest" "package-lock packages"
    if ($vitest -isnot [System.Collections.IDictionary]) {
        throw "package-lock node_modules/vitestがobjectではありません"
    }
    return Get-RequiredString (Get-DictionaryValue $vitest "version" "package-lock vitest") "package-lock vitest version"
}

function Assert-FrontendTestScriptContract {
    $package = ConvertFrom-JsonStrict (Read-TrackedText "apps/desktop/package.json" $script:SnapshotRoot) "frontend package.json"
    $scripts = Get-DictionaryValue $package "scripts" "frontend package.json"
    if ($scripts -isnot [System.Collections.IDictionary]) {
        throw "apps/desktop/package.json /scripts must be an object"
    }
    $testScript = Get-RequiredString (Get-DictionaryValue $scripts "test" "frontend package.json /scripts") "frontend test script"
    if (-not [string]::Equals($testScript, "vitest run --configLoader runner", [System.StringComparison]::Ordinal)) {
        throw "unsupported frontend test script; update the list collector contract: $testScript"
    }
}

function Initialize-FrontendTooling {
    if ($script:FrontendPrepared) {
        return
    }
    Assert-FrontendTestScriptContract
    $desktop = Get-AbsoluteRepositoryPath "apps/desktop" $script:SnapshotRoot
    $nodeCommand = Get-Command "node.exe" -ErrorAction Stop
    $npmCommand = Get-Command "npm.cmd" -ErrorAction Stop
    $nodeDirectory = Split-Path -Parent $npmCommand.Source
    $npmCli = Join-Path $nodeDirectory "node_modules\npm\bin\npm-cli.js"
    if (-not [System.IO.File]::Exists($npmCli)) {
        throw "npm-cli.jsがありません: $npmCli"
    }
    $result = Invoke-NativeCapture $nodeCommand.Source @($npmCli, "ci", "--ignore-scripts", "--no-audit", "--no-fund") $desktop @{
        CARGO_TARGET_DIR = $CargoTargetDir
    }
    if ($result.ExitCode -ne 0) {
        $stderr = $result.StdErr.Trim()
        if ($stderr.Length -gt 4000) {
            $stderr = $stderr.Substring($stderr.Length - 4000)
        }
        throw "隔離snapshotのnpm ciが失敗しました(exit $($result.ExitCode)): $stderr"
    }
    $installedPath = Join-Path $desktop "node_modules\vitest\package.json"
    if (-not [System.IO.File]::Exists($installedPath)) {
        throw "npm ci後にVitestがありません"
    }
    $installed = ConvertFrom-JsonStrict ([System.IO.File]::ReadAllText($installedPath, $script:Utf8NoBom)) "installed vitest package.json"
    $installedVersion = Get-RequiredString (Get-DictionaryValue $installed "version" "installed vitest") "installed vitest version"
    $lockVersion = Get-LockVitestVersion
    if (-not [string]::Equals($installedVersion, $lockVersion, [System.StringComparison]::Ordinal)) {
        throw "installed Vitestとlockが一致しません(installed=$installedVersion, lock=$lockVersion)"
    }
    $script:FrontendPrepared = $true
}

function Convert-VitestList {
    param([string]$Json, [string[]]$AllowedTestPaths, [string]$Profile)

    $parsed = ConvertFrom-JsonStrict $Json "Vitest $Profile list"
    if ($parsed -is [System.Collections.IDictionary]) {
        if (Test-DictionaryKey $parsed "tests") {
            $items = @($parsed["tests"])
        }
        elseif (Test-DictionaryKey $parsed "tasks") {
            $items = @($parsed["tasks"])
        }
        else {
            throw "Vitest $Profile JSONのtest配列がありません"
        }
    }
    elseif ($parsed -is [System.Collections.IEnumerable] -and $parsed -isnot [string]) {
        $items = @($parsed)
    }
    else {
        throw "Vitest $Profile JSONのtop-levelが配列/objectではありません"
    }

    $allowed = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    foreach ($path in $AllowedTestPaths) {
        [void]$allowed.Add($path)
    }
    $files = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    $counts = New-Object 'System.Collections.Generic.Dictionary[string,int]' ($script:Ordinal)
    $cases = New-Object System.Collections.Generic.List[object]
    $desktopRoot = Get-AbsoluteRepositoryPath "apps/desktop" $script:SnapshotRoot
    foreach ($item in $items) {
        if ($item -isnot [System.Collections.IDictionary]) {
            throw "Vitest $Profile itemがobjectではありません"
        }
        $rawFile = Get-RequiredString (Get-DictionaryValue $item "file" "Vitest $Profile item") "Vitest $Profile file"
        $nameValue = if (Test-DictionaryKey $item "fullName") { $item["fullName"] } else { Get-DictionaryValue $item "name" "Vitest $Profile item" }
        $name = Get-RequiredString $nameValue "Vitest $Profile name"
        $location = Get-DictionaryValue $item "location" "Vitest $Profile item"
        if ($location -isnot [System.Collections.IDictionary]) {
            throw "Vitest $Profile locationがobjectではありません"
        }
        $line = [int](Get-DictionaryValue $location "line" "Vitest $Profile location")
        $column = [int](Get-DictionaryValue $location "column" "Vitest $Profile location")
        if ($line -le 0 -or $column -le 0) {
            throw "Vitest $Profile locationが正数ではありません: $rawFile ${line}:$column"
        }
        $fileFull = if ([System.IO.Path]::IsPathRooted($rawFile)) { $rawFile } else { Join-Path $desktopRoot $rawFile }
        $desktopRelative = Get-RelativePathUnder $desktopRoot $fileFull "Vitest $Profile file"
        $repositoryFile = ConvertTo-RepositoryPath ("apps/desktop/" + $desktopRelative)
        if (-not $allowed.Contains($repositoryFile)) {
            throw "Vitest $Profile が追跡済みtest集合外を返しました: $repositoryFile"
        }
        [void]$files.Add($repositoryFile)
        $identity = "$repositoryFile`0$name`0$line`0$column"
        if ($counts.ContainsKey($identity)) {
            $counts[$identity] = $counts[$identity] + 1
        }
        else {
            $counts.Add($identity, 1)
        }
        [void]$cases.Add([PSCustomObject][ordered]@{
            File = $repositoryFile
            Name = $name
            Line = $line
            Column = $column
            Identity = $identity
        })
    }
    return [PSCustomObject][ordered]@{
        Cases = $cases.ToArray()
        Files = $files
        Counts = $counts
    }
}

function Get-VitestInventory {
    param([switch]$ProductionSymmetry)

    Initialize-FrontendTooling
    $allowedPaths = @(Get-TrackedFrontendTestPaths)
    $desktop = Get-AbsoluteRepositoryPath "apps/desktop" $script:SnapshotRoot
    $arguments = @("./node_modules/vitest/vitest.mjs", "list", "--json", "--includeTaskLocation", "--configLoader", "runner")
    if ($ProductionSymmetry) {
        $arguments += @("--mode=production", "src/lib/symmetry.test.ts")
        $profile = "production-symmetry"
    }
    else {
        $arguments += @($allowedPaths | ForEach-Object { $_.Substring("apps/desktop/".Length) })
        $profile = "default"
    }
    $result = Invoke-NativeCapture "node.exe" $arguments $desktop
    if ($result.ExitCode -ne 0) {
        $stderr = $result.StdErr.Trim()
        if ($stderr.Length -gt 4000) {
            $stderr = $stderr.Substring($stderr.Length - 4000)
        }
        throw "Vitest $profile listが失敗しました(exit $($result.ExitCode)): $stderr"
    }
    $inventory = Convert-VitestList $result.StdOut $allowedPaths $profile
    return [PSCustomObject][ordered]@{
        Inventory = $inventory
        ElapsedMs = [double]$result.ElapsedMs
    }
}

function Get-FrontendStaticSites {
    Initialize-FrontendTooling
    $desktop = Get-AbsoluteRepositoryPath "apps/desktop" $script:SnapshotRoot
    $relativePaths = @(Get-TrackedFrontendTestPaths | ForEach-Object { $_.Substring("apps/desktop/".Length) })
    $program = @'
const fs = require("fs");
const ts = require("typescript");
const paths = process.argv.slice(1);
const counts = { sites: 0, skip: 0, todo: 0, only: 0, files: paths.length };
function classify(expression) {
  if (ts.isIdentifier(expression) && expression.text === "it") return "sites";
  if (ts.isPropertyAccessExpression(expression) &&
      ts.isIdentifier(expression.expression) && expression.expression.text === "it") {
    if (expression.name.text === "each") return "sites";
    if (["skip", "todo", "only"].includes(expression.name.text)) return expression.name.text;
    throw new Error(`unsupported it.${expression.name.text} registration`);
  }
  return null;
}
for (const path of paths) {
  const source = ts.createSourceFile(path, fs.readFileSync(path, "utf8"), ts.ScriptTarget.Latest, true,
    path.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  if (source.parseDiagnostics.length !== 0) {
    throw new Error(`TypeScript parse error in ${path}: ${source.parseDiagnostics[0].messageText}`);
  }
  function visit(node) {
    if (ts.isCallExpression(node)) {
      const kind = classify(node.expression);
      if (kind) counts[kind] += 1;
    } else if (ts.isTaggedTemplateExpression(node)) {
      const kind = classify(node.tag);
      if (kind) counts[kind] += 1;
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
}
process.stdout.write(JSON.stringify(counts));
'@
    $result = Invoke-NativeCapture "node.exe" (@("-e", $program, "--") + $relativePaths) $desktop @{
        CARGO_TARGET_DIR = $CargoTargetDir
    }
    if ($result.ExitCode -ne 0) {
        throw "TypeScript AST registration collector failed(exit $($result.ExitCode)): $($result.StdErr.Trim())"
    }
    $parsed = ConvertFrom-JsonStrict $result.StdOut "frontend static registration sites"
    foreach ($key in @("sites", "skip", "todo", "only", "files")) {
        if (-not (Test-DictionaryKey $parsed $key)) { throw "frontend AST collector omitted $key" }
        Assert-IntegerValue $parsed[$key] "frontend AST $key"
    }
    if ([int]$parsed["files"] -ne $relativePaths.Count) {
        throw "frontend AST collector file count differs from tracked input"
    }
    return [PSCustomObject][ordered]@{
        Sites = [int]$parsed["sites"]
        Skip = [int]$parsed["skip"]
        Todo = [int]$parsed["todo"]
        Only = [int]$parsed["only"]
    }
}

function Get-TestInventory {
    $script:LastTestInventoryTimings = $null
    $inventoryWatch = [System.Diagnostics.Stopwatch]::StartNew()

    $stepWatch = [System.Diagnostics.Stopwatch]::StartNew()
    $rustStatic = Get-RustStaticInventory
    $stepWatch.Stop()
    $rustStaticMs = [double]$stepWatch.Elapsed.TotalMilliseconds

    $stepWatch.Restart()
    $frontendStatic = Get-FrontendStaticSites
    $stepWatch.Stop()
    $frontendStaticMs = [double]$stepWatch.Elapsed.TotalMilliseconds

    $stepWatch.Restart()
    $defaultVitest = Get-VitestInventory
    $stepWatch.Stop()
    $vitestDefaultMs = [double]$stepWatch.Elapsed.TotalMilliseconds

    $stepWatch.Restart()
    $productionVitest = Get-VitestInventory -ProductionSymmetry
    $stepWatch.Stop()
    $vitestProductionMs = [double]$stepWatch.Elapsed.TotalMilliseconds
    $default = $defaultVitest.Inventory
    $production = $productionVitest.Inventory

    $stepWatch.Restart()
    $symmetryPath = "apps/desktop/src/lib/symmetry.test.ts"
    if (-not $script:TrackedSet.Contains($symmetryPath)) {
        throw "production symmetry testが追跡されていません: $symmetryPath"
    }
    $defaultSymmetry = @($default.Cases | Where-Object { $_.File -eq $symmetryPath }).Count

    $allIdentities = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    foreach ($identity in $default.Counts.Keys) { [void]$allIdentities.Add($identity) }
    foreach ($identity in $production.Counts.Keys) { [void]$allIdentities.Add($identity) }
    $unionCases = 0
    $productionOnly = 0
    foreach ($identity in $allIdentities) {
        $defaultCount = if ($default.Counts.ContainsKey($identity)) { $default.Counts[$identity] } else { 0 }
        $productionCount = if ($production.Counts.ContainsKey($identity)) { $production.Counts[$identity] } else { 0 }
        $unionCases += [Math]::Max($defaultCount, $productionCount)
        $productionOnly += [Math]::Max(0, $productionCount - $defaultCount)
    }
    $unionFiles = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    foreach ($file in $default.Files) { [void]$unionFiles.Add($file) }
    foreach ($file in $production.Files) { [void]$unionFiles.Add($file) }

    Write-Host ("frontend runner inventory: default={0}/{1} files; production_symmetry={2}; union={3}; static_sites={4}" -f $default.Cases.Count, $default.Files.Count, $production.Cases.Count, $unionCases, $frontendStatic.Sites)
    Write-Host ("Rust tracked static test attributes: {0}" -f $rustStatic.Sites)
    $stepWatch.Stop()
    $profileMergeMs = [double]$stepWatch.Elapsed.TotalMilliseconds

    $stepWatch.Restart()
    $registeredCargo = Get-CargoRunnerInventory
    $stepWatch.Stop()
    $cargoRegisteredMs = [double]$stepWatch.Elapsed.TotalMilliseconds

    $stepWatch.Restart()
    $ignoredCargo = Get-CargoRunnerInventory -IgnoredOnly
    $stepWatch.Stop()
    $cargoIgnoredMs = [double]$stepWatch.Elapsed.TotalMilliseconds

    $stepWatch.Restart()
    if ($ignoredCargo.Tests -gt $registeredCargo.Tests) {
        throw "ignored Rust test数がregisteredを超えました($($ignoredCargo.Tests) > $($registeredCargo.Tests))"
    }
    if ($registeredCargo.Tests -ne $rustStatic.Sites) {
        throw "Rust runner inventory and static #[test] sites differ(runner=$($registeredCargo.Tests), static=$($rustStatic.Sites))"
    }
    $trackedFrontendFiles = @(Get-TrackedFrontendTestPaths).Count
    if ($default.Files.Count -ne $trackedFrontendFiles) {
        throw "Vitest default file inventory and tracked test files differ(runner=$($default.Files.Count), tracked=$trackedFrontendFiles)"
    }

    $result = [ordered]@{
        profile = "runner-discovery"
        rust = [ordered]@{
            selection = "windows-test-debug-default-features-no-name-filter"
            collector_command = "cargo test --workspace --locked -- --list --format terse"
            ignored_collector_command = "cargo test --workspace --locked -- --list --ignored --format terse"
            registered_cases = [int]$registeredCargo.Tests
            default_runnable_cases = [int]($registeredCargo.Tests - $ignoredCargo.Tests)
            ignored_cases = [int]$ignoredCargo.Tests
            benchmark_cases = [int]$registeredCargo.Benchmarks
            static_test_attribute_sites = [int]$rustStatic.Sites
            target_conditional_registration_sites = [int]$rustStatic.TargetConditional
        }
        frontend = [ordered]@{
            default = [ordered]@{
                selection = "vitest-default"
                collector_command = "node ./node_modules/vitest/vitest.mjs list --json --includeTaskLocation --configLoader runner"
                runnable_cases = [int]$default.Cases.Count
                files = [int]$default.Files.Count
                symmetry_cases = [int]$defaultSymmetry
            }
            production_symmetry = [ordered]@{
                selection = "vitest-mode-production-src/lib/symmetry.test.ts"
                collector_command = "node ./node_modules/vitest/vitest.mjs list --json --includeTaskLocation --configLoader runner --mode=production src/lib/symmetry.test.ts"
                runnable_cases = [int]$production.Cases.Count
                files = [int]$production.Files.Count
                production_only_cases = [int]$productionOnly
            }
            cross_profile = [ordered]@{
                union_cases = [int]$unionCases
                union_files = [int]$unionFiles.Count
                identity = "repository-relative-file+full-name+line+column+same-location-ordinal"
            }
            static_registration_sites = [int]$frontendStatic.Sites
            skip_sites = [int]$frontendStatic.Skip
            todo_sites = [int]$frontendStatic.Todo
            only_sites = [int]$frontendStatic.Only
        }
        sources = @(
            [ordered]@{ path = "Cargo.toml"; selector = "[workspace].members" },
            [ordered]@{ path = "apps/desktop/package.json"; selector = "/scripts/test" },
            [ordered]@{ path = "apps/desktop/package-lock.json"; selector = '/packages["node_modules/vitest"]/version' },
            [ordered]@{ path = "apps/desktop/src"; selector = "tracked test sources discovered by Vitest" }
        )
        mirrors = @()
    }
    $stepWatch.Stop()
    $validationAndShapeMs = [double]$stepWatch.Elapsed.TotalMilliseconds
    $inventoryWatch.Stop()
    $script:LastTestInventoryTimings = [ordered]@{
        rust_static = $rustStaticMs
        frontend_static_ast = $frontendStaticMs
        vitest_default = $vitestDefaultMs
        vitest_production_symmetry = $vitestProductionMs
        frontend_profile_merge = $profileMergeMs
        cargo_registered = $cargoRegisteredMs
        cargo_ignored = $cargoIgnoredMs
        validation_and_shape = $validationAndShapeMs
        total = [double]$inventoryWatch.Elapsed.TotalMilliseconds
    }
    return $result
}

function Assert-ExactKeys {
    param(
        [Parameter(Mandatory = $true)][System.Collections.IDictionary]$Object,
        [Parameter(Mandatory = $true)][string[]]$Keys,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $actual = @($Object.Keys | ForEach-Object { [string]$_ })
    if (-not (Test-OrdinalSequenceEqual $actual $Keys)) {
        throw "$Label keys differ (actual=$($actual -join ','), expected=$($Keys -join ','))"
    }
}

function Assert-StringValue {
    param([object]$Value, [string]$Label)
    if ($Value -isnot [string] -or [string]::IsNullOrWhiteSpace([string]$Value)) {
        throw "$Label must be a non-empty string"
    }
}

function Assert-IntegerValue {
    param([object]$Value, [string]$Label, [switch]$Positive)
    $isInteger = $Value -is [byte] -or $Value -is [sbyte] -or
        $Value -is [int16] -or $Value -is [uint16] -or
        $Value -is [int32] -or $Value -is [uint32] -or
        $Value -is [int64] -or $Value -is [uint64]
    if (-not $isInteger) {
        throw "$Label must be an integer"
    }
    if (($Positive -and [decimal]$Value -le 0) -or (-not $Positive -and [decimal]$Value -lt 0)) {
        throw "$Label is outside the schema range: $Value"
    }
}

function Assert-BooleanValue {
    param([object]$Value, [string]$Label)
    if ($Value -isnot [bool]) {
        throw "$Label must be a boolean"
    }
}

function Assert-PathSelector {
    param([System.Collections.IDictionary]$Object, [string]$Label)
    Assert-ExactKeys $Object @("path", "selector") $Label
    Assert-StringValue $Object.path "$Label.path"
    Assert-StringValue $Object.selector "$Label.selector"
    if ([System.IO.Path]::IsPathRooted([string]$Object.path) -or ([string]$Object.path).Contains("\")) {
        throw "$Label.path must be repository-relative with forward slashes"
    }
    $normalized = ConvertTo-RepositoryPath ([string]$Object.path)
    if (Test-ForbiddenSourcePath $normalized) {
        throw "$Label.path is forbidden as a collector source: $normalized"
    }
}

function Assert-MirrorBase {
    param([System.Collections.IDictionary]$Mirror, [string[]]$Keys, [string]$Label)
    Assert-ExactKeys $Mirror $Keys $Label
    Assert-StringValue $Mirror.id "$Label.id"
    Assert-StringValue $Mirror.path "$Label.path"
    Assert-StringValue $Mirror.selector "$Label.selector"
    Assert-BooleanValue $Mirror.matches_source "$Label.matches_source"
}

function Assert-ConstantString {
    param([object]$Value, [string]$Expected, [string]$Label)
    Assert-StringValue $Value $Label
    if (-not [string]::Equals([string]$Value, $Expected, [System.StringComparison]::Ordinal)) {
        throw "$Label differs (actual=$Value, expected=$Expected)"
    }
}

function Assert-PathSelectorIdentity {
    param(
        [System.Collections.IDictionary]$Source,
        [string]$ExpectedPath,
        [string]$ExpectedSelector,
        [string]$Label
    )

    Assert-PathSelector $Source $Label
    Assert-ConstantString $Source.path $ExpectedPath "$Label.path"
    Assert-ConstantString $Source.selector $ExpectedSelector "$Label.selector"
}

function Assert-PathSelectorIdentitySequence {
    param([object[]]$Sources, [string[]]$Expected, [string]$Label)

    if ($Sources.Count -ne $Expected.Count) {
        throw "$Label source count differs (actual=$($Sources.Count), expected=$($Expected.Count))"
    }
    for ($index = 0; $index -lt $Expected.Count; $index++) {
        $parts = $Expected[$index].Split([char]0)
        if ($parts.Count -ne 2 -or $Sources[$index] -isnot [System.Collections.IDictionary]) {
            throw "$Label source identity contract is invalid at index $index"
        }
        Assert-PathSelectorIdentity $Sources[$index] $parts[0] $parts[1] "$Label[$index]"
    }
}

function Assert-IdentifiedPathSelectorIdentitySequence {
    param([object[]]$Sources, [string[]]$Expected, [string]$Label)

    if ($Sources.Count -ne $Expected.Count) {
        throw "$Label source count differs (actual=$($Sources.Count), expected=$($Expected.Count))"
    }
    for ($index = 0; $index -lt $Expected.Count; $index++) {
        $parts = $Expected[$index].Split([char]0)
        $source = $Sources[$index]
        if ($parts.Count -ne 3 -or $source -isnot [System.Collections.IDictionary]) {
            throw "$Label source identity contract is invalid at index $index"
        }
        Assert-ExactKeys $source @("id", "path", "selector") "$Label[$index]"
        Assert-ConstantString $source.id $parts[0] "$Label[$index].id"
        Assert-ConstantString $source.path $parts[1] "$Label[$index].path"
        Assert-ConstantString $source.selector $parts[2] "$Label[$index].selector"
    }
}

function Assert-MirrorIdentitySequence {
    param([object[]]$Mirrors, [string[]]$Expected, [string]$Label)
    if ($Mirrors.Count -ne $Expected.Count) {
        throw "$Label mirror count differs (actual=$($Mirrors.Count), expected=$($Expected.Count))"
    }
    for ($index = 0; $index -lt $Expected.Count; $index++) {
        $parts = $Expected[$index].Split([char]0)
        $mirror = $Mirrors[$index]
        if ($parts.Count -ne 3 -or
            -not [string]::Equals([string]$mirror.id, $parts[0], [System.StringComparison]::Ordinal) -or
            -not [string]::Equals([string]$mirror.path, $parts[1], [System.StringComparison]::Ordinal) -or
            -not [string]::Equals([string]$mirror.selector, $parts[2], [System.StringComparison]::Ordinal)) {
            throw "$Label mirror identity differs at index $index"
        }
    }
}

function Assert-CurrentStatusSchema {
    param([System.Collections.IDictionary]$Status)

    Assert-ExactKeys $Status @("schema_version", "source_snapshot", "metrics") "root"
    if ($Status.schema_version -ne 1) { throw "schema_version must be 1" }

    $snapshot = $Status.source_snapshot
    Assert-ExactKeys $snapshot @("source_set", "write_mode", "check_mode", "untracked_candidate_policy", "path_format") "source_snapshot"
    Assert-ConstantString $snapshot.source_set "git-tracked" "source_snapshot.source_set"
    Assert-ConstantString $snapshot.write_mode "tracked-working-tree" "source_snapshot.write_mode"
    Assert-ConstantString $snapshot.check_mode "clean-committed-tree" "source_snapshot.check_mode"
    Assert-ConstantString $snapshot.untracked_candidate_policy "error" "source_snapshot.untracked_candidate_policy"
    Assert-ConstantString $snapshot.path_format "repository-relative-forward-slash" "source_snapshot.path_format"

    $metrics = $Status.metrics
    Assert-ExactKeys $metrics @("workspace_version", "workspace_members", "tauri_commands", "test_inventory", "proposal_budgets", "manual_pages") "metrics"

    $version = $metrics.workspace_version
    Assert-ExactKeys $version @("profile", "value", "source", "mirrors") "workspace_version"
    Assert-ConstantString $version.profile "workspace-manifest-current" "workspace_version.profile"
    Assert-StringValue $version.value "workspace_version.value"
    Assert-PathSelectorIdentity $version.source "Cargo.toml" "[workspace.package].version" "workspace_version.source"
    foreach ($mirror in @($version.mirrors)) {
        Assert-MirrorBase $mirror @("id", "path", "selector", "observed_value", "matches_source") "workspace_version.mirror"
        Assert-StringValue $mirror.observed_value "workspace_version.mirror.observed_value"
    }
    Assert-MirrorIdentitySequence @($version.mirrors) @(
        "desktop-package-version$([char]0)apps/desktop/package.json$([char]0)/version",
        "desktop-lock-root-version$([char]0)apps/desktop/package-lock.json$([char]0)/version",
        ('desktop-lock-package-version' + [char]0 + 'apps/desktop/package-lock.json' + [char]0 + '/packages[""]/version'),
        "tauri-config-version$([char]0)apps/desktop/src-tauri/tauri.conf.json$([char]0)/version"
    ) "workspace_version"

    $workspace = $metrics.workspace_members
    Assert-ExactKeys $workspace @("profile", "count", "members", "source", "mirrors") "workspace_members"
    Assert-ConstantString $workspace.profile "cargo-workspace" "workspace_members.profile"
    Assert-IntegerValue $workspace.count "workspace_members.count" -Positive
    if (@($workspace.members).Count -ne $workspace.count) { throw "workspace member count is inconsistent" }
    foreach ($member in @($workspace.members)) { Assert-StringValue $member "workspace_members.member" }
    Assert-PathSelectorIdentity $workspace.source "Cargo.toml" "[workspace].members" "workspace_members.source"
    foreach ($mirror in @($workspace.mirrors)) {
        Assert-MirrorBase $mirror @("id", "path", "selector", "observed_count", "matches_source") "workspace_members.mirror"
        Assert-IntegerValue $mirror.observed_count "workspace_members.mirror.observed_count" -Positive
    }
    Assert-MirrorIdentitySequence @($workspace.mirrors) @(
        "requirements-crate-table$([char]0)docs/requirements-definition.md$([char]0)section:9.2/table:crate-responsibilities"
    ) "workspace_members"

    $tauri = $metrics.tauri_commands
    Assert-ExactKeys $tauri @("profile", "count", "commands", "cross_checks", "source", "cross_check_sources", "mirrors") "tauri_commands"
    Assert-ConstantString $tauri.profile "desktop.invoke_handler" "tauri_commands.profile"
    Assert-IntegerValue $tauri.count "tauri_commands.count" -Positive
    if (@($tauri.commands).Count -ne $tauri.count) { throw "Tauri command count is inconsistent" }
    foreach ($command in @($tauri.commands)) {
        Assert-ExactKeys $command @("registration_path", "rust_function", "ipc_name") "tauri_commands.command"
        foreach ($key in @("registration_path", "rust_function", "ipc_name")) { Assert-StringValue $command[$key] "tauri_commands.command.$key" }
    }
    Assert-ExactKeys $tauri.cross_checks @("tauri_command_attribute_count", "frontend_invoke_wrapper_count", "frontend_invoke_names") "tauri_commands.cross_checks"
    Assert-IntegerValue $tauri.cross_checks.tauri_command_attribute_count "tauri command attribute count" -Positive
    Assert-IntegerValue $tauri.cross_checks.frontend_invoke_wrapper_count "frontend wrapper count" -Positive
    foreach ($name in @($tauri.cross_checks.frontend_invoke_names)) { Assert-StringValue $name "frontend invoke name" }
    Assert-PathSelectorIdentity $tauri.source "apps/desktop/src-tauri/src/lib.rs" "run/tauri::generate_handler!" "tauri_commands.source"
    foreach ($source in @($tauri.cross_check_sources)) {
        Assert-ExactKeys $source @("id", "path", "selector") "tauri_commands.cross_check_source"
        Assert-StringValue $source.id "tauri_commands.cross_check_source.id"
        Assert-StringValue $source.path "tauri_commands.cross_check_source.path"
        Assert-StringValue $source.selector "tauri_commands.cross_check_source.selector"
    }
    Assert-IdentifiedPathSelectorIdentitySequence @($tauri.cross_check_sources) @(
        "tauri-command-attributes$([char]0)apps/desktop/src-tauri/src/commands.rs$([char]0)top-level-function/attribute:tauri::command",
        "frontend-invoke-wrappers$([char]0)apps/desktop/src/ipc/client.ts$([char]0)top-level-export-function/body:single-direct-return-invoke-string-literal"
    ) "tauri_commands.cross_check_sources"
    foreach ($mirror in @($tauri.mirrors)) {
        Assert-MirrorBase $mirror @("id", "path", "selector", "comparison_fields", "source_count", "observed_count", "observed_names", "matches_source") "tauri_commands.mirror"
        foreach ($field in @($mirror.comparison_fields)) { Assert-StringValue $field "tauri_commands.mirror.comparison_field" }
        Assert-IntegerValue $mirror.source_count "tauri_commands.mirror.source_count" -Positive
        Assert-IntegerValue $mirror.observed_count "tauri_commands.mirror.observed_count" -Positive
        foreach ($name in @($mirror.observed_names)) { Assert-StringValue $name "tauri_commands.mirror.observed_name" }
    }
    Assert-MirrorIdentitySequence @($tauri.mirrors) @(
        "requirements-command-table$([char]0)docs/requirements-definition.md$([char]0)section:9.3/header+command-table+following-current-count-paragraph",
        "implementation-tree-command-count$([char]0)docs/implementation-roadmap.md$([char]0)architecture-tree/apps/desktop/src-tauri/src/commands.rs",
        "implementation-tree-wrapper-count$([char]0)docs/implementation-roadmap.md$([char]0)architecture-tree/apps/desktop/src/ipc/client.ts",
        "implementation-ipc-list$([char]0)docs/implementation-roadmap.md$([char]0)heading:IPCコマンド一覧/header+slash-list+following-current-count-paragraph",
        "implementation-common-check-count$([char]0)docs/implementation-roadmap.md$([char]0)section:3/list-item:3",
        "improvement-section-11-current-count$([char]0)docs/improvement-roadmap-2026-08-24.md$([char]0)section:11.1/paragraph:2",
        "improvement-section-11-acceptance-count$([char]0)docs/improvement-roadmap-2026-08-24.md$([char]0)section:11.4/list-item:2"
    ) "tauri_commands"

    $tests = $metrics.test_inventory
    Assert-ExactKeys $tests @("profile", "rust", "frontend", "sources", "mirrors") "test_inventory"
    Assert-ConstantString $tests.profile "runner-discovery" "test_inventory.profile"
    Assert-ExactKeys $tests.rust @("selection", "collector_command", "ignored_collector_command", "registered_cases", "default_runnable_cases", "ignored_cases", "benchmark_cases", "static_test_attribute_sites", "target_conditional_registration_sites") "test_inventory.rust"
    foreach ($key in @("selection", "collector_command", "ignored_collector_command")) { Assert-StringValue $tests.rust[$key] "test_inventory.rust.$key" }
    foreach ($key in @("registered_cases", "default_runnable_cases", "static_test_attribute_sites")) { Assert-IntegerValue $tests.rust[$key] "test_inventory.rust.$key" -Positive }
    foreach ($key in @("ignored_cases", "benchmark_cases", "target_conditional_registration_sites")) { Assert-IntegerValue $tests.rust[$key] "test_inventory.rust.$key" }
    Assert-ExactKeys $tests.frontend @("default", "production_symmetry", "cross_profile", "static_registration_sites", "skip_sites", "todo_sites", "only_sites") "test_inventory.frontend"
    Assert-ExactKeys $tests.frontend.default @("selection", "collector_command", "runnable_cases", "files", "symmetry_cases") "test_inventory.frontend.default"
    Assert-ExactKeys $tests.frontend.production_symmetry @("selection", "collector_command", "runnable_cases", "files", "production_only_cases") "test_inventory.frontend.production_symmetry"
    Assert-ExactKeys $tests.frontend.cross_profile @("union_cases", "union_files", "identity") "test_inventory.frontend.cross_profile"
    foreach ($profile in @($tests.frontend.default, $tests.frontend.production_symmetry)) {
        Assert-StringValue $profile.selection "frontend profile selection"
        Assert-StringValue $profile.collector_command "frontend collector command"
        foreach ($key in @("runnable_cases", "files")) { Assert-IntegerValue $profile[$key] "frontend profile $key" -Positive }
    }
    Assert-IntegerValue $tests.frontend.default.symmetry_cases "frontend default symmetry cases" -Positive
    Assert-IntegerValue $tests.frontend.production_symmetry.production_only_cases "frontend production-only cases"
    Assert-IntegerValue $tests.frontend.cross_profile.union_cases "frontend union cases" -Positive
    Assert-IntegerValue $tests.frontend.cross_profile.union_files "frontend union files" -Positive
    Assert-StringValue $tests.frontend.cross_profile.identity "frontend union identity"
    Assert-IntegerValue $tests.frontend.static_registration_sites "test_inventory.frontend.static_registration_sites" -Positive
    foreach ($key in @("skip_sites", "todo_sites", "only_sites")) { Assert-IntegerValue $tests.frontend[$key] "test_inventory.frontend.$key" }
    Assert-PathSelectorIdentitySequence @($tests.sources) @(
        "Cargo.toml$([char]0)[workspace].members",
        "apps/desktop/package.json$([char]0)/scripts/test",
        ('apps/desktop/package-lock.json' + [char]0 + '/packages["node_modules/vitest"]/version'),
        "apps/desktop/src$([char]0)tracked test sources discovered by Vitest"
    ) "test_inventory.sources"
    if (@($tests.mirrors).Count -ne 0) { throw "test_inventory.mirrors must be empty in schema 1" }

    $budgets = $metrics.proposal_budgets
    Assert-ExactKeys $budgets @("profile", "profiles", "mirrors") "proposal_budgets"
    Assert-ConstantString $budgets.profile "resolved-operational-budgets" "proposal_budgets.profile"
    if (@($budgets.profiles).Count -ne 3) { throw "proposal_budgets must contain three profiles" }
    $expectedBudgetIds = @("library_default", "desktop_product", "desktop_test_time_free")
    $expectedBudgetSources = [ordered]@{
        library_default = @(
            "crates/ori3-propose/src/search.rs$([char]0)SearchBudget::DEFAULT+SearchWatchdog::{MAX_MILLIS,DEFAULT}",
            "crates/ori3-propose/src/enumerate.rs$([char]0)PoseScan::DEFAULT"
        )
        desktop_product = @(
            "apps/desktop/src-tauri/src/commands.rs$([char]0)PLAN_BUDGET",
            "crates/ori3-propose/src/enumerate.rs$([char]0)PoseScan::DEFAULT"
        )
        desktop_test_time_free = @(
            "apps/desktop/src-tauri/src/commands.rs$([char]0)tests::TIME_FREE_PLAN_BUDGET",
            "apps/desktop/src-tauri/src/commands.rs$([char]0)PLAN_BUDGET inherited fields"
        )
    }
    for ($profileIndex = 0; $profileIndex -lt @($budgets.profiles).Count; $profileIndex++) {
        $profile = @($budgets.profiles)[$profileIndex]
        Assert-ExactKeys $profile @("id", "max_states", "max_depth", "branch", "rank_scan_steps", "rank_scan_points", "scan_steps", "scan_points", "watchdog_max_millis", "sources") "proposal_budgets.profile"
        Assert-ConstantString $profile.id $expectedBudgetIds[$profileIndex] "proposal_budgets.profile.id"
        foreach ($key in @("max_states", "max_depth", "branch", "rank_scan_steps", "rank_scan_points", "scan_steps", "scan_points", "watchdog_max_millis")) { Assert-IntegerValue $profile[$key] "proposal_budgets.profile.$key" -Positive }
        Assert-PathSelectorIdentitySequence @($profile.sources) @($expectedBudgetSources[[string]$profile.id]) "proposal_budgets.$($profile.id).sources"
    }
    foreach ($mirror in @($budgets.mirrors)) {
        Assert-MirrorBase $mirror @("id", "path", "selector", "field", "source_value", "observed_value", "matches_source") "proposal_budgets.mirror"
        Assert-StringValue $mirror.field "proposal_budgets.mirror.field"
        Assert-IntegerValue $mirror.source_value "proposal_budgets.mirror.source_value"
        Assert-IntegerValue $mirror.observed_value "proposal_budgets.mirror.observed_value"
    }
    Assert-MirrorIdentitySequence @($budgets.mirrors) @(
        "end-to-end-product-max-states$([char]0)crates/ori3-propose/tests/end_to_end.rs$([char]0)PRODUCT_PLAN_BUDGET.max_states",
        "end-to-end-product-max-depth$([char]0)crates/ori3-propose/tests/end_to_end.rs$([char]0)PRODUCT_PLAN_BUDGET.max_depth",
        "end-to-end-product-branch$([char]0)crates/ori3-propose/tests/end_to_end.rs$([char]0)PRODUCT_PLAN_BUDGET.branch",
        "claude-product-watchdog$([char]0)CLAUDE.md$([char]0)section:10.6/list-item:#21",
        "claude-product-max-states$([char]0)CLAUDE.md$([char]0)section:10.6/list-item:#21",
        "claude-product-branch$([char]0)CLAUDE.md$([char]0)section:10.6/list-item:#21",
        "improvement-intro-stale-source-budget$([char]0)docs/improvement-roadmap-2026-08-24.md$([char]0)section:0.2/paragraph:追加の同期不良",
        "improvement-priority-stale-source-budget$([char]0)docs/improvement-roadmap-2026-08-24.md$([char]0)section:3.2/table:実装・検証の順序/row:施策7",
        "improvement-section-11-purpose-stale-source-budget$([char]0)docs/improvement-roadmap-2026-08-24.md$([char]0)section:11.1/paragraph:1",
        "improvement-section-11-failure-stale-source-budget$([char]0)docs/improvement-roadmap-2026-08-24.md$([char]0)section:11.6/list-item:4"
    ) "proposal_budgets"

    $manual = $metrics.manual_pages
    Assert-ExactKeys $manual @("profile", "page_count", "evidence", "source", "generator_sources", "mirrors") "manual_pages"
    Assert-ConstantString $manual.profile "published-pdf" "manual_pages.profile"
    Assert-IntegerValue $manual.page_count "manual_pages.page_count" -Positive
    Assert-ExactKeys $manual.evidence @("pdf_signature_valid", "media_box_count", "page_object_count", "page_tree_count") "manual_pages.evidence"
    Assert-BooleanValue $manual.evidence.pdf_signature_valid "manual_pages.evidence.pdf_signature_valid"
    foreach ($key in @("media_box_count", "page_object_count", "page_tree_count")) { Assert-IntegerValue $manual.evidence[$key] "manual_pages.evidence.$key" -Positive }
    Assert-PathSelectorIdentity $manual.source "docs/manual/ORIGAMI3取扱説明書.pdf" "%PDF- signature+/MediaBox+/Type /Page+page-tree /Count" "manual_pages.source"
    Assert-PathSelectorIdentitySequence @($manual.generator_sources) @(
        "crates/ori3-export/src/manual.rs$([char]0)manual_pdf_with_stats+manual_svg_pages+ManualPdfStats.page_count",
        "scripts/build-manual.ps1$([char]0)PDF signature and /MediaBox count"
    ) "manual_pages.generator_sources"
    if (@($manual.mirrors).Count -ne 0) { throw "manual_pages.mirrors must be empty in schema 1" }
}

function Invoke-MetricCollector {
    param([string]$Id, [scriptblock]$Collector)

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        $value = & $Collector
        if ($null -eq $value) { throw "collector returned null" }
        return [PSCustomObject][ordered]@{ Value = $value; ElapsedMs = [double]$watch.Elapsed.TotalMilliseconds }
    }
    catch {
        throw "metric=$Id collector error: $($_.Exception.Message)"
    }
    finally {
        $watch.Stop()
    }
}

function New-CurrentStatusCollection {
    param([int]$Pass)

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $metricTimings = [ordered]@{}
    try {
        Write-Host "collection pass ${Pass}: workspace_version"
        $version = Invoke-MetricCollector "workspace_version" { Get-WorkspaceVersion }
        $metricTimings.workspace_version = $version.ElapsedMs
        Write-Host "collection pass ${Pass}: workspace_members"
        $members = Invoke-MetricCollector "workspace_members" { Get-WorkspaceMembers }
        $metricTimings.workspace_members = $members.ElapsedMs
        Write-Host "collection pass ${Pass}: tauri_commands"
        $tauri = Invoke-MetricCollector "tauri_commands" { Get-TauriCommands }
        $metricTimings.tauri_commands = $tauri.ElapsedMs
        Write-Host "collection pass ${Pass}: proposal_budgets"
        $budgets = Invoke-MetricCollector "proposal_budgets" { Get-ProposalBudgets }
        $metricTimings.proposal_budgets = $budgets.ElapsedMs
        Write-Host "collection pass ${Pass}: manual_pages"
        $manual = Invoke-MetricCollector "manual_pages" { Get-ManualPageCount }
        $metricTimings.manual_pages = $manual.ElapsedMs
        Write-Host "collection pass ${Pass}: test_inventory"
        $tests = Invoke-MetricCollector "test_inventory" { Get-TestInventory }
        $metricTimings.test_inventory = $tests.ElapsedMs
        if ($null -eq $script:LastTestInventoryTimings) {
            throw "test inventory timing diagnostics were not recorded"
        }

        $status = [ordered]@{
            schema_version = 1
            source_snapshot = [ordered]@{
                source_set = "git-tracked"
                write_mode = "tracked-working-tree"
                check_mode = "clean-committed-tree"
                untracked_candidate_policy = "error"
                path_format = "repository-relative-forward-slash"
            }
            metrics = [ordered]@{
                workspace_version = $version.Value
                workspace_members = $members.Value
                tauri_commands = $tauri.Value
                test_inventory = $tests.Value
                proposal_budgets = $budgets.Value
                manual_pages = $manual.Value
            }
        }
        Assert-CurrentStatusSchema $status
        return [PSCustomObject][ordered]@{
            Status = $status
            ElapsedMs = [double]$watch.Elapsed.TotalMilliseconds
            MetricTimings = $metricTimings
            TestInventoryTimings = $script:LastTestInventoryTimings
        }
    }
    finally {
        $watch.Stop()
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
    if ($inString -or $indent -ne 0) { throw "canonical JSON formatter ended in an invalid state" }
    return $builder.ToString().TrimEnd("`r", "`n") + "`n"
}

function ConvertTo-CanonicalJson {
    param([System.Collections.IDictionary]$Status)

    Assert-CurrentStatusSchema $Status
    $compressed = ConvertTo-Json -InputObject $Status -Compress -Depth 100
    $json = Format-JsonTwoSpace $compressed
    [void](ConvertFrom-JsonStrict $json "generated current-status.json")
    return $json
}

function Get-BudgetProfile {
    param([System.Collections.IDictionary]$Budgets, [string]$Id)
    $matches = @($Budgets.profiles | Where-Object { $_.id -eq $Id })
    if ($matches.Count -ne 1) { throw "budget profile is not unique: $Id" }
    return $matches[0]
}

function Format-Integer {
    param([long]$Value)
    return [string]::Format([System.Globalization.CultureInfo]::InvariantCulture, "{0:N0}", $Value)
}

function ConvertTo-GeneratedMarkdown {
    param([System.Collections.IDictionary]$Status)

    Assert-CurrentStatusSchema $Status
    $m = $Status.metrics
    $rust = $m.test_inventory.rust
    $frontend = $m.test_inventory.frontend
    $library = Get-BudgetProfile $m.proposal_budgets "library_default"
    $product = Get-BudgetProfile $m.proposal_budgets "desktop_product"
    $testOnly = Get-BudgetProfile $m.proposal_budgets "desktop_test_time_free"
    $crateCount = @($m.workspace_members.members | Where-Object { $_.StartsWith("crates/", [System.StringComparison]::Ordinal) }).Count
    $hostCount = @($m.workspace_members.members | Where-Object { [string]::Equals($_, "apps/desktop/src-tauri", [System.StringComparison]::Ordinal) }).Count
    $otherMemberCount = $m.workspace_members.count - $crateCount - $hostCount
    $workspaceComposition = if ($otherMemberCount -eq 0) {
        "計算crate $crateCount + desktop Tauri host $hostCount"
    }
    else {
        "計算crate $crateCount + desktop Tauri host $hostCount + other $otherMemberCount"
    }

    $lines = New-Object System.Collections.Generic.List[string]
    foreach ($line in @(
        '<!-- ORIGAMI3-CURRENT-STATUS:BEGIN schema=1 -->',
        '## 現在値（機械生成・手編集禁止）',
        'この現在値表は、HTMLコメント形式の「ORIGAMI3-CURRENT-STATUS」開始・終了印で囲み、実装から再生成した値との一致を自動検査します。',
        '',
        '| 指標 | 現在値 | profile | 正本 |',
        '|---|---:|---|---|',
        ('| version | `{0}` | `{1}` | `Cargo.toml [workspace.package].version` |' -f $m.workspace_version.value, $m.workspace_version.profile),
        ('| workspace | {0} member（{1}） | `{2}` | `Cargo.toml [workspace].members` |' -f $m.workspace_members.count, $workspaceComposition, $m.workspace_members.profile),
        ('| Tauri commands | handler {0}（command属性{1}、frontend wrapper {2}） | `{3}` | `apps/desktop/src-tauri/src/lib.rs::run/tauri::generate_handler!` |' -f $m.tauri_commands.count, $m.tauri_commands.cross_checks.tauri_command_attribute_count, $m.tauri_commands.cross_checks.frontend_invoke_wrapper_count, $m.tauri_commands.profile),
        ('| tests | Rust {0}（runnable {1}、ignored {2}）／frontend default {3} case・{4} file | `{5}` | Cargo test inventory／Vitest list inventory |' -f (Format-Integer $rust.registered_cases), (Format-Integer $rust.default_runnable_cases), (Format-Integer $rust.ignored_cases), (Format-Integer $frontend.default.runnable_cases), (Format-Integer $frontend.default.files), $m.test_inventory.profile),
        ('| proposal budgets | library `{0}/{1}/{2}/{3}/{4}/{5}ms`、product `{6}/{7}/{8}/{9}/{10}/{11}ms`、test-only `{12}/{13}/{14}/{15}/{16}/{17}ms` | `{18}` | `SearchBudget::DEFAULT`／`PLAN_BUDGET`／`TIME_FREE_PLAN_BUDGET` |' -f $library.max_states, $library.max_depth, $library.branch, $library.rank_scan_steps, $library.scan_steps, $library.watchdog_max_millis, $product.max_states, $product.max_depth, $product.branch, $product.rank_scan_steps, $product.scan_steps, $product.watchdog_max_millis, $testOnly.max_states, $testOnly.max_depth, $testOnly.branch, $testOnly.rank_scan_steps, $testOnly.scan_steps, $testOnly.watchdog_max_millis, $m.proposal_budgets.profile),
        ('| manual | {0}ページ（3方式一致） | `{1}` | `docs/manual/ORIGAMI3取扱説明書.pdf` |' -f $m.manual_pages.page_count, $m.manual_pages.profile),
        '',
        '### test inventory内訳',
        '',
        '| profile | case | file | 補足 |',
        '|---|---:|---:|---|',
        ('| Rust registered | {0} | - | default runnable {1}、ignored {2}、benchmark {3} |' -f (Format-Integer $rust.registered_cases), (Format-Integer $rust.default_runnable_cases), (Format-Integer $rust.ignored_cases), (Format-Integer $rust.benchmark_cases)),
        ('| frontend default | {0} | {1} | `symmetry.test.ts`は{2} case |' -f (Format-Integer $frontend.default.runnable_cases), (Format-Integer $frontend.default.files), (Format-Integer $frontend.default.symmetry_cases)),
        ('| frontend production symmetry | {0} | {1} | production-only {2} case |' -f (Format-Integer $frontend.production_symmetry.runnable_cases), (Format-Integer $frontend.production_symmetry.files), (Format-Integer $frontend.production_symmetry.production_only_cases)),
        ('| frontend cross-profile union | {0} | {1} | 同一location内ordinalを含めcase multiplicityを保持 |' -f (Format-Integer $frontend.cross_profile.union_cases), (Format-Integer $frontend.cross_profile.union_files)),
        '',
        '### proposal budget内訳',
        '',
        '| profile | max states | max depth | branch | rank scan | final scan | watchdog |',
        '|---|---:|---:|---:|---:|---:|---:|',
        ('| library default | {0} | {1} | {2} | steps {3}（{4}点） | steps {5}（{6}点） | {7}ms |' -f $library.max_states, $library.max_depth, $library.branch, $library.rank_scan_steps, $library.rank_scan_points, $library.scan_steps, $library.scan_points, (Format-Integer $library.watchdog_max_millis)),
        ('| desktop product | {0} | {1} | {2} | steps {3}（{4}点） | steps {5}（{6}点） | {7}ms |' -f $product.max_states, $product.max_depth, $product.branch, $product.rank_scan_steps, $product.rank_scan_points, $product.scan_steps, $product.scan_points, (Format-Integer $product.watchdog_max_millis)),
        ('| desktop test-only | {0} | {1} | {2} | steps {3}（{4}点） | steps {5}（{6}点） | {7}ms |' -f $testOnly.max_states, $testOnly.max_depth, $testOnly.branch, $testOnly.rank_scan_steps, $testOnly.rank_scan_points, $testOnly.scan_steps, $testOnly.scan_points, (Format-Integer $testOnly.watchdog_max_millis)),
        '',
        '<!-- ORIGAMI3-CURRENT-STATUS:END -->'
    )) { [void]$lines.Add([string]$line) }
    return ($lines -join "`n") + "`n"
}

function Get-OrdinalOccurrenceCount {
    param([string]$Text, [string]$Needle)

    if ([string]::IsNullOrEmpty($Needle)) {
        throw "occurrence needle must not be empty"
    }
    $count = 0
    $search = 0
    while ($search -lt $Text.Length) {
        $found = $Text.IndexOf($Needle, $search, [System.StringComparison]::Ordinal)
        if ($found -lt 0) { break }
        $count++
        $search = $found + $Needle.Length
    }
    return [int]$count
}

function Get-ProgressCurrentStatusMarkerBlock {
    param([string]$ProgressText)

    $beginToken = '<!-- ORIGAMI3-CURRENT-STATUS:BEGIN'
    $beginLine = '<!-- ORIGAMI3-CURRENT-STATUS:BEGIN schema=1 -->'
    $endToken = '<!-- ORIGAMI3-CURRENT-STATUS:END'
    $endLine = '<!-- ORIGAMI3-CURRENT-STATUS:END -->'
    $beginCount = Get-OrdinalOccurrenceCount $ProgressText $beginToken
    $endCount = Get-OrdinalOccurrenceCount $ProgressText $endToken
    if ($beginCount -ne 1 -or $endCount -ne 1) {
        throw "docs/progress.md current-status marker must have exactly one BEGIN and one END (BEGIN=$beginCount, END=$endCount)"
    }

    $beginIndex = $ProgressText.IndexOf($beginLine, [System.StringComparison]::Ordinal)
    $endIndex = $ProgressText.IndexOf($endLine, [System.StringComparison]::Ordinal)
    if ($beginIndex -lt 0 -or $endIndex -lt 0) {
        throw "docs/progress.md current-status marker delimiter differs from schema 1"
    }
    if ($endIndex -le $beginIndex) {
        throw "docs/progress.md current-status marker END precedes BEGIN"
    }
    if ($beginIndex -gt 0 -and $ProgressText[$beginIndex - 1] -ne "`n") {
        throw "docs/progress.md current-status BEGIN is not at the start of a line"
    }
    $beginLineEnd = $beginIndex + $beginLine.Length
    if ($beginLineEnd -ge $ProgressText.Length -or $ProgressText[$beginLineEnd] -ne "`n") {
        throw "docs/progress.md current-status BEGIN must be followed immediately by LF"
    }
    if ($endIndex -gt 0 -and $ProgressText[$endIndex - 1] -ne "`n") {
        throw "docs/progress.md current-status END is not at the start of a line"
    }
    $endLineEnd = $endIndex + $endLine.Length
    if ($endLineEnd -ge $ProgressText.Length -or $ProgressText[$endLineEnd] -ne "`n") {
        throw "docs/progress.md current-status END must be followed immediately by LF"
    }

    $h1Matches = [regex]::Matches($ProgressText, '(?m)^# [^\r\n]+\r?$')
    if ($h1Matches.Count -ne 1 -or $h1Matches[0].Index -ne 0) {
        throw "docs/progress.md must have exactly one H1 at byte zero"
    }
    $h1LineFeed = $ProgressText.IndexOf("`n", $h1Matches[0].Index)
    if ($h1LineFeed -lt 0 -or $beginIndex -le $h1LineFeed) {
        throw "docs/progress.md current-status marker is not after the H1"
    }
    $betweenH1AndMarker = $ProgressText.Substring($h1LineFeed + 1, $beginIndex - $h1LineFeed - 1)
    if ($betweenH1AndMarker -ne "`n" -and $betweenH1AndMarker -ne "`r`n") {
        throw "docs/progress.md current-status marker must be the first block immediately after the H1"
    }

    return $ProgressText.Substring($beginIndex, $endLineEnd - $beginIndex + 1)
}

function Get-FirstDifferingMarkdownLine {
    param([string]$Expected, [string]$Observed)

    $expectedLines = @($Expected.Split([char]"`n"))
    $observedLines = @($Observed.Split([char]"`n"))
    $common = [Math]::Min($expectedLines.Count, $observedLines.Count)
    for ($index = 0; $index -lt $common; $index++) {
        if (-not [string]::Equals($expectedLines[$index], $observedLines[$index], [System.StringComparison]::Ordinal)) {
            return [PSCustomObject][ordered]@{
                Line = [int]($index + 1)
                Expected = [string]$expectedLines[$index]
                Observed = [string]$observedLines[$index]
            }
        }
    }
    if ($expectedLines.Count -ne $observedLines.Count) {
        $expectedLine = if ($common -lt $expectedLines.Count) { [string]$expectedLines[$common] } else { "<end-of-marker>" }
        $observedLine = if ($common -lt $observedLines.Count) { [string]$observedLines[$common] } else { "<end-of-marker>" }
        return [PSCustomObject][ordered]@{
            Line = [int]($common + 1)
            Expected = $expectedLine
            Observed = $observedLine
        }
    }
    throw "first differing line was requested for identical marker blocks"
}

function Get-MetricMarkdownEvidence {
    param([string]$Markdown, [string]$MetricId)

    $rowPrefix = switch ($MetricId) {
        "workspace_version" { "| version |" }
        "workspace_members" { "| workspace |" }
        "tauri_commands" { "| Tauri commands |" }
        "test_inventory" { "| tests |" }
        "proposal_budgets" { "| proposal budgets |" }
        "manual_pages" { "| manual |" }
        default { throw "unknown metric for Markdown evidence: $MetricId" }
    }
    $lines = @($Markdown.Split([char]"`n"))
    $rowMatches = @($lines | Where-Object { $_.StartsWith($rowPrefix, [System.StringComparison]::Ordinal) })
    $row = if ($rowMatches.Count -eq 1) { [string]$rowMatches[0] } else { "<metric-row-count:$($rowMatches.Count)>" }

    if ($MetricId -eq "test_inventory") {
        $start = [Array]::IndexOf($lines, "### test inventory内訳")
        $end = [Array]::IndexOf($lines, "### proposal budget内訳")
        if ($start -ge 0 -and $end -gt $start) {
            return $row + "`n" + (($lines[$start..($end - 1)]) -join "`n")
        }
        return $row + "`n<test-inventory-section-missing>"
    }
    if ($MetricId -eq "proposal_budgets") {
        $start = [Array]::IndexOf($lines, "### proposal budget内訳")
        $end = [Array]::IndexOf($lines, '<!-- ORIGAMI3-CURRENT-STATUS:END -->')
        if ($start -ge 0 -and $end -gt $start) {
            return $row + "`n" + (($lines[$start..($end - 1)]) -join "`n")
        }
        return $row + "`n<proposal-budget-section-missing>"
    }
    return $row
}

function Get-MetricSourceSet {
    param([System.Collections.IDictionary]$Status, [string]$MetricId)

    $metric = $Status.metrics[$MetricId]
    $candidates = New-Object System.Collections.Generic.List[object]
    if (Test-DictionaryKey $metric "source") {
        [void]$candidates.Add($metric.source)
    }
    if (Test-DictionaryKey $metric "sources") {
        foreach ($source in @($metric.sources)) { [void]$candidates.Add($source) }
    }
    if (Test-DictionaryKey $metric "cross_check_sources") {
        foreach ($source in @($metric.cross_check_sources)) { [void]$candidates.Add($source) }
    }
    if (Test-DictionaryKey $metric "generator_sources") {
        foreach ($source in @($metric.generator_sources)) { [void]$candidates.Add($source) }
    }
    if (Test-DictionaryKey $metric "profiles") {
        foreach ($profile in @($metric.profiles)) {
            if (Test-DictionaryKey $profile "sources") {
                foreach ($source in @($profile.sources)) { [void]$candidates.Add($source) }
            }
        }
    }

    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ($script:Ordinal)
    $result = New-Object System.Collections.Generic.List[object]
    foreach ($source in $candidates) {
        if ($source -isnot [System.Collections.IDictionary] -or
            -not (Test-DictionaryKey $source "path") -or
            -not (Test-DictionaryKey $source "selector")) {
            throw "metric source lacks path/selector: $MetricId"
        }
        $identity = [string]$source.path + [char]0 + [string]$source.selector
        if ($seen.Add($identity)) { [void]$result.Add($source) }
    }
    if ($result.Count -eq 0) { throw "metric has no source set: $MetricId" }
    return $result.ToArray()
}

function Get-MetricPrimarySource {
    param([System.Collections.IDictionary]$Status, [string]$MetricId)

    return @(Get-MetricSourceSet $Status $MetricId)[0]
}

function ConvertTo-MetricSourceSetDiagnostic {
    param([System.Collections.IDictionary]$Status, [string]$MetricId)

    $parts = New-Object System.Collections.Generic.List[string]
    foreach ($source in @(Get-MetricSourceSet $Status $MetricId)) {
        [void]$parts.Add(('path="{0}",selector="{1}"' -f
            (ConvertTo-MarkerDiagnosticText ([string]$source.path)),
            (ConvertTo-MarkerDiagnosticText ([string]$source.selector))))
    }
    return "[" + ($parts -join ";") + "]"
}

function ConvertTo-MarkerDiagnosticText {
    param([AllowEmptyString()][string]$Value)

    return $Value.Replace("\", "\\").Replace('"', '\"').Replace("`r", "\r").Replace("`n", "\n")
}

function Invoke-CurrentStatusMarkerGate {
    param(
        [string]$ExpectedMarkdown,
        [System.Collections.IDictionary]$Status,
        [string]$ProgressText
    )

    try {
        $observedMarker = Get-ProgressCurrentStatusMarkerBlock $ProgressText
    }
    catch {
        return [PSCustomObject][ordered]@{
            ExitCode = 2
            MetricIds = @()
            Diagnostics = @("marker structure error: $($_.Exception.Message)")
        }
    }

    if ([string]::Equals($ExpectedMarkdown, $observedMarker, [System.StringComparison]::Ordinal)) {
        return [PSCustomObject][ordered]@{
            ExitCode = 0
            MetricIds = @()
            Diagnostics = @()
        }
    }

    $expectedHash = Get-TextSha256 $ExpectedMarkdown
    $observedHash = Get-TextSha256 $observedMarker
    $firstDifference = Get-FirstDifferingMarkdownLine $ExpectedMarkdown $observedMarker
    $metricIds = New-Object System.Collections.Generic.List[string]
    $diagnostics = New-Object System.Collections.Generic.List[string]
    foreach ($metricId in @("workspace_version", "workspace_members", "tauri_commands", "test_inventory", "proposal_budgets", "manual_pages")) {
        $generatedEvidence = Get-MetricMarkdownEvidence $ExpectedMarkdown $metricId
        $markerEvidence = Get-MetricMarkdownEvidence $observedMarker $metricId
        if ([string]::Equals($generatedEvidence, $markerEvidence, [System.StringComparison]::Ordinal)) { continue }
        [void]$metricIds.Add($metricId)
        $metric = $Status.metrics[$metricId]
        $primary = Get-MetricPrimarySource $Status $metricId
        $sourceSet = ConvertTo-MetricSourceSetDiagnostic $Status $metricId
        [void]$diagnostics.Add((
            'marker drift metric={0} profile={1} primary_path={2} primary_selector={3} source_set={4} generated="{5}" marker="{6}" expected_sha256={7} observed_sha256={8} first_differing_line={9} expected_line="{10}" observed_line="{11}"' -f
                $metricId,
                $metric.profile,
                $primary.path,
                $primary.selector,
                $sourceSet,
                (ConvertTo-MarkerDiagnosticText $generatedEvidence),
                (ConvertTo-MarkerDiagnosticText $markerEvidence),
                $expectedHash,
                $observedHash,
                $firstDifference.Line,
                (ConvertTo-MarkerDiagnosticText $firstDifference.Expected),
                (ConvertTo-MarkerDiagnosticText $firstDifference.Observed)
        ))
    }
    if ($metricIds.Count -eq 0) {
        [void]$diagnostics.Add((
            'marker drift metric=marker_document profile=generated-current-status primary_path=docs/progress.md primary_selector=H1/current-status-marker generated="{0}" marker="{1}" expected_sha256={2} observed_sha256={3} first_differing_line={4} expected_line="{5}" observed_line="{6}"' -f
                (ConvertTo-MarkerDiagnosticText $ExpectedMarkdown),
                (ConvertTo-MarkerDiagnosticText $observedMarker),
                $expectedHash,
                $observedHash,
                $firstDifference.Line,
                (ConvertTo-MarkerDiagnosticText $firstDifference.Expected),
                (ConvertTo-MarkerDiagnosticText $firstDifference.Observed)
        ))
    }
    return [PSCustomObject][ordered]@{
        ExitCode = 1
        MetricIds = @($metricIds.ToArray())
        Diagnostics = @($diagnostics.ToArray())
    }
}

function New-MarkerFixtureStatus {
    return [ordered]@{
        metrics = [ordered]@{
            workspace_version = [ordered]@{
                profile = "workspace-manifest-current"
                source = [ordered]@{ path = "Cargo.toml"; selector = "[workspace.package].version" }
            }
            workspace_members = [ordered]@{
                profile = "cargo-workspace"
                source = [ordered]@{ path = "Cargo.toml"; selector = "[workspace].members" }
            }
            tauri_commands = [ordered]@{
                profile = "desktop.invoke_handler"
                source = [ordered]@{ path = "apps/desktop/src-tauri/src/lib.rs"; selector = "run/tauri::generate_handler!" }
                cross_check_sources = @(
                    [ordered]@{ path = "apps/desktop/src-tauri/src/commands.rs"; selector = "top-level-function/attribute:tauri::command" },
                    [ordered]@{ path = "apps/desktop/src/ipc/client.ts"; selector = "top-level-export-function/body:single-direct-return-invoke-string-literal" }
                )
            }
            test_inventory = [ordered]@{
                profile = "runner-discovery"
                sources = @(
                    [ordered]@{ path = "Cargo.toml"; selector = "[workspace].members" },
                    [ordered]@{ path = "apps/desktop/package.json"; selector = "/scripts/test" },
                    [ordered]@{ path = "apps/desktop/package-lock.json"; selector = '/packages["node_modules/vitest"]/version' },
                    [ordered]@{ path = "apps/desktop/src"; selector = "tracked test sources discovered by Vitest" }
                )
            }
            proposal_budgets = [ordered]@{
                profile = "resolved-operational-budgets"
                profiles = @(
                    [ordered]@{
                        id = "library_default"
                        sources = @(
                            [ordered]@{ path = "crates/ori3-propose/src/search.rs"; selector = "SearchBudget::DEFAULT+SearchWatchdog::{MAX_MILLIS,DEFAULT}" },
                            [ordered]@{ path = "crates/ori3-propose/src/enumerate.rs"; selector = "PoseScan::DEFAULT" }
                        )
                    },
                    [ordered]@{
                        id = "desktop_product"
                        sources = @(
                            [ordered]@{ path = "apps/desktop/src-tauri/src/commands.rs"; selector = "PLAN_BUDGET" },
                            [ordered]@{ path = "crates/ori3-propose/src/enumerate.rs"; selector = "PoseScan::DEFAULT" }
                        )
                    },
                    [ordered]@{
                        id = "desktop_test_time_free"
                        sources = @(
                            [ordered]@{ path = "apps/desktop/src-tauri/src/commands.rs"; selector = "tests::TIME_FREE_PLAN_BUDGET" },
                            [ordered]@{ path = "apps/desktop/src-tauri/src/commands.rs"; selector = "PLAN_BUDGET inherited fields" }
                        )
                    }
                )
            }
            manual_pages = [ordered]@{
                profile = "published-pdf"
                source = [ordered]@{ path = "docs/manual/ORIGAMI3取扱説明書.pdf"; selector = "%PDF- signature+/MediaBox+/Type /Page+page-tree /Count" }
                generator_sources = @(
                    [ordered]@{ path = "crates/ori3-export/src/manual.rs"; selector = "manual_pdf_with_stats+manual_svg_pages+ManualPdfStats.page_count" },
                    [ordered]@{ path = "scripts/build-manual.ps1"; selector = "PDF signature and /MediaBox count" }
                )
            }
        }
    }
}

function New-MarkerFixtureMarkdown {
    param([System.Collections.IDictionary]$Tokens)

    $lines = @(
        '<!-- ORIGAMI3-CURRENT-STATUS:BEGIN schema=1 -->',
        '## 現在値（機械生成・手編集禁止）',
        '',
        '| 指標 | fixture value |',
        '|---|---|',
        ('| version | {0} |' -f $Tokens.workspace_version),
        ('| workspace | {0} |' -f $Tokens.workspace_members),
        ('| Tauri commands | {0} |' -f $Tokens.tauri_commands),
        ('| tests | {0} |' -f $Tokens.test_inventory),
        ('| proposal budgets | {0} |' -f $Tokens.proposal_budgets),
        ('| manual | {0} |' -f $Tokens.manual_pages),
        '',
        '### test inventory内訳',
        '',
        ('fixture-test-detail={0}' -f $Tokens.test_inventory),
        '',
        '### proposal budget内訳',
        '',
        ('fixture-budget-detail={0}' -f $Tokens.proposal_budgets),
        '',
        '<!-- ORIGAMI3-CURRENT-STATUS:END -->'
    )
    return ($lines -join "`n") + "`n"
}

function Assert-MarkerFixtureResult {
    param(
        [string]$Name,
        [object]$Result,
        [int]$ExpectedExitCode,
        [string]$ExpectedMetricId = ""
    )

    if ([int]$Result.ExitCode -ne $ExpectedExitCode) {
        throw "marker fixture $Name exit differs(actual=$($Result.ExitCode), expected=$ExpectedExitCode): $($Result.Diagnostics -join '; ')"
    }
    if ([string]::IsNullOrEmpty($ExpectedMetricId)) {
        if (@($Result.MetricIds).Count -ne 0) {
            throw "marker fixture $Name unexpectedly reported metrics: $($Result.MetricIds -join ',')"
        }
        return
    }
    if (@($Result.MetricIds).Count -ne 1 -or -not [string]::Equals([string]$Result.MetricIds[0], $ExpectedMetricId, [System.StringComparison]::Ordinal)) {
        throw "marker fixture $Name metric diagnostic differs: $($Result.MetricIds -join ',')"
    }
    $diagnostic = @($Result.Diagnostics) -join "`n"
    if ($diagnostic.IndexOf("System.Object[]", [System.StringComparison]::Ordinal) -ge 0) {
        throw "marker fixture $Name diagnostic flattened a source set to System.Object[]"
    }
    foreach ($required in @(
        "metric=$ExpectedMetricId",
        "profile=",
        "primary_path=",
        "primary_selector=",
        "source_set=",
        "generated=",
        "marker=",
        "expected_sha256=",
        "observed_sha256=",
        "first_differing_line="
    )) {
        if ($diagnostic.IndexOf($required, [System.StringComparison]::Ordinal) -lt 0) {
            throw "marker fixture $Name diagnostic omitted: $required"
        }
    }
}

function Invoke-MarkerFixtureSuite {
    $baselineStatus = New-MarkerFixtureStatus
    $nonce = [Guid]::NewGuid().ToString("N")
    $tokens = [ordered]@{}
    foreach ($metricId in @("workspace_version", "workspace_members", "tauri_commands", "test_inventory", "proposal_budgets", "manual_pages")) {
        $tokens[$metricId] = "marker-fixture-$metricId-$nonce"
    }
    $baselineMarkdown = New-MarkerFixtureMarkdown $tokens
    $fixtureH1 = "# marker fixture progress"
    $cleanProgress = $fixtureH1 + "`n`n" + $BaselineMarkdown + "`nfixture body`n"
    $clean = Invoke-CurrentStatusMarkerGate $BaselineMarkdown $BaselineStatus $cleanProgress
    Assert-MarkerFixtureResult "clean" $clean 0
    $passed = 1

    $metricIds = @("workspace_version", "workspace_members", "tauri_commands", "test_inventory", "proposal_budgets", "manual_pages")
    $expectedSourceCounts = [ordered]@{
        workspace_version = 1
        workspace_members = 1
        tauri_commands = 3
        test_inventory = 4
        proposal_budgets = 5
        manual_pages = 3
    }
    foreach ($metricId in $metricIds) {
        $fixtureSources = @(Get-MetricSourceSet $BaselineStatus $metricId)
        if ($fixtureSources.Count -ne [int]$expectedSourceCounts[$metricId]) {
            throw "marker fixture source set differs(metric=$metricId, actual=$($fixtureSources.Count), expected=$($expectedSourceCounts[$metricId]))"
        }
        $baselineToken = [string]$tokens[$metricId]
        $changedToken = $baselineToken + "-relative-delta"
        $mutatedMarkdown = $BaselineMarkdown.Replace($baselineToken, $changedToken)
        if ([string]::Equals($mutatedMarkdown, $BaselineMarkdown, [System.StringComparison]::Ordinal)) {
            throw "marker fixture could not mutate runtime token: $metricId"
        }
        $mutatedProgress = $fixtureH1 + "`n`n" + $mutatedMarkdown + "`nfixture body`n"
        $result = Invoke-CurrentStatusMarkerGate $BaselineMarkdown $BaselineStatus $mutatedProgress
        Assert-MarkerFixtureResult $metricId $result 1 $metricId
        $diagnostic = @($result.Diagnostics) -join "`n"
        foreach ($source in $fixtureSources) {
            foreach ($field in @("path", "selector")) {
                $expectedSourcePart = ConvertTo-MarkerDiagnosticText ([string]$source[$field])
                if ($diagnostic.IndexOf($expectedSourcePart, [System.StringComparison]::Ordinal) -lt 0) {
                    throw "marker fixture $metricId diagnostic omitted source $field=$expectedSourcePart"
                }
            }
        }
        $passed++
    }

    $endWithLf = '<!-- ORIGAMI3-CURRENT-STATUS:END -->' + "`n"
    $malformedProgress = $cleanProgress.Replace($endWithLf, "")
    if ([string]::Equals($malformedProgress, $cleanProgress, [System.StringComparison]::Ordinal)) {
        throw "marker fixture could not remove END"
    }
    $malformed = Invoke-CurrentStatusMarkerGate $BaselineMarkdown $BaselineStatus $malformedProgress
    Assert-MarkerFixtureResult "missing-END" $malformed 2
    $passed++

    return [PSCustomObject][ordered]@{
        Passed = [int]$passed
        Total = 8
        MetricDrifts = [int]$metricIds.Count
    }
}

function Get-AllMirrorDrifts {
    param([System.Collections.IDictionary]$Status)
    $drifts = New-Object System.Collections.Generic.List[object]
    foreach ($metricId in @("workspace_version", "workspace_members", "tauri_commands", "test_inventory", "proposal_budgets", "manual_pages")) {
        $metric = $Status.metrics[$metricId]
        foreach ($mirror in @($metric.mirrors)) {
            if (-not [bool]$mirror.matches_source) {
                [void]$drifts.Add([PSCustomObject][ordered]@{ Metric = $metricId; Profile = $metric.profile; Mirror = $mirror })
            }
        }
    }
    return $drifts.ToArray()
}

function Assert-GeneratedOutputLocation {
    $expected = [System.IO.Path]::GetFullPath((Join-Path $script:Root ($script:GeneratedRelativeRoot.Replace("/", "\"))))
    $actual = [System.IO.Path]::GetFullPath($OutputDirectory)
    if (-not [string]::Equals($actual.TrimEnd([char[]]"\/"), $expected.TrimEnd([char[]]"\/"), [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "output directory must be the generated artifact directory: $expected"
    }
    foreach ($relative in @("$($script:GeneratedRelativeRoot)/current-status.json", "$($script:GeneratedRelativeRoot)/current-status.md")) {
        $tracked = Invoke-GitCapture @("ls-files", "-z", "--", $relative)
        if ($tracked.StdOut.Length -ne 0) { throw "collector refuses to overwrite a tracked path: $relative" }
    }
    $cursor = $actual
    while ($cursor.StartsWith($script:Root, [System.StringComparison]::OrdinalIgnoreCase) -and -not [string]::Equals($cursor, $script:Root, [System.StringComparison]::OrdinalIgnoreCase)) {
        if ([System.IO.Directory]::Exists($cursor)) {
            $item = Get-Item -LiteralPath $cursor -Force
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { throw "generated output ancestor is a reparse point: $cursor" }
        }
        $cursor = [System.IO.Path]::GetDirectoryName($cursor)
    }
}

function Write-GeneratedFilesAtomically {
    param([string]$Json, [string]$Markdown)

    Assert-GeneratedOutputLocation
    [void][System.IO.Directory]::CreateDirectory($OutputDirectory)
    $jsonPath = Join-Path $OutputDirectory "current-status.json"
    $markdownPath = Join-Path $OutputDirectory "current-status.md"
    $jsonTemp = "$jsonPath.$([Guid]::NewGuid().ToString('N')).tmp"
    $markdownTemp = "$markdownPath.$([Guid]::NewGuid().ToString('N')).tmp"
    $entries = @(
        [PSCustomObject][ordered]@{ Temp = $jsonTemp; Destination = $jsonPath; Backup = "$jsonPath.$([Guid]::NewGuid().ToString('N')).bak"; HadOriginal = [System.IO.File]::Exists($jsonPath); Applied = $false },
        [PSCustomObject][ordered]@{ Temp = $markdownTemp; Destination = $markdownPath; Backup = "$markdownPath.$([Guid]::NewGuid().ToString('N')).bak"; HadOriginal = [System.IO.File]::Exists($markdownPath); Applied = $false }
    )
    foreach ($entry in $entries) {
        if ($entry.HadOriginal) {
            $item = Get-Item -LiteralPath $entry.Destination -Force
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "generated output file is a reparse point: $($entry.Destination)"
            }
        }
    }
    $updatesComplete = $false
    try {
        [System.IO.File]::WriteAllText($jsonTemp, $Json, $script:Utf8NoBom)
        [System.IO.File]::WriteAllText($markdownTemp, $Markdown, $script:Utf8NoBom)
        foreach ($entry in $entries) {
            if ($entry.HadOriginal) { [System.IO.File]::Replace($entry.Temp, $entry.Destination, $entry.Backup) }
            else { [System.IO.File]::Move($entry.Temp, $entry.Destination) }
            $entry.Applied = $true
        }
        $updatesComplete = $true
    }
    catch {
        $originalError = $_.Exception.Message
        $rollbackErrors = New-Object System.Collections.Generic.List[string]
        for ($index = $entries.Count - 1; $index -ge 0; $index--) {
            $entry = $entries[$index]
            if (-not $entry.Applied) { continue }
            try {
                if ($entry.HadOriginal) {
                    if ([System.IO.File]::Exists($entry.Destination)) { [System.IO.File]::Delete($entry.Destination) }
                    if (-not [System.IO.File]::Exists($entry.Backup)) { throw "backup is missing" }
                    [System.IO.File]::Move($entry.Backup, $entry.Destination)
                }
                elseif ([System.IO.File]::Exists($entry.Destination)) {
                    [System.IO.File]::Delete($entry.Destination)
                }
            }
            catch {
                [void]$rollbackErrors.Add("$($entry.Destination): $($_.Exception.Message)")
            }
        }
        if ($rollbackErrors.Count -gt 0) {
            throw "generated output update failed: $originalError; rollback failed: $($rollbackErrors -join '; ')"
        }
        throw "generated output update failed and was rolled back: $originalError"
    }
    finally {
        foreach ($temporary in @($jsonTemp, $markdownTemp)) {
            if ([System.IO.File]::Exists($temporary)) { [System.IO.File]::Delete($temporary) }
        }
        if ($updatesComplete) {
            foreach ($entry in $entries) {
                if ([System.IO.File]::Exists($entry.Backup)) {
                    try { [System.IO.File]::Delete($entry.Backup) }
                    catch { Write-Warning "generated output backup cleanup failed: $($entry.Backup)" }
                }
            }
        }
    }
}

# Metric counts are observations, never upper/lower limits.  No current count
# (including 9, 18, the test inventory, or 82 pages) is a pass boundary.
# Proposal time budgets are existing product/library limits and are only
# reported here; this collector neither relaxes nor derives new thresholds
# from measured timings (CLAUDE.md sections 9 and 10.7.9).
$scriptExitCode = 2
$jobWatch = [System.Diagnostics.Stopwatch]::StartNew()
try {
    if ($Check -and $MarkerFixtures) {
        throw "-Check and -MarkerFixtures cannot be used together"
    }
    if ($MarkerFixtures) {
        $fixtureResult = Invoke-MarkerFixtureSuite
        Write-Host ("marker fixtures: {0}/{1} passed; metric drift diagnostics: {2}/6" -f $fixtureResult.Passed, $fixtureResult.Total, $fixtureResult.MetricDrifts)
        Write-Host "tracked sources, real progress marker, mirrors, Cargo, and generated outputs: bypassed in MarkerFixtures"
        $scriptExitCode = 0
    }
    else {
        if ($null -ne $script:CargoTargetError) {
            throw $script:CargoTargetError
        }
        $rootResult = Invoke-GitCapture @("rev-parse", "--show-toplevel")
    $gitRoot = [System.IO.Path]::GetFullPath($rootResult.StdOut.Trim()).TrimEnd([char[]]"\/")
    if (-not [string]::Equals($gitRoot, $script:Root, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "script root and git root differ"
    }

    Assert-CleanCommittedCheckTree
    $script:TrackedEntries = @(Get-TrackedEntries)
    $script:SelectedSourcePaths = @(Get-SelectedSourcePaths)
    Assert-NoUntrackedMetricCandidates
    $sourceHashBefore = Get-SourceManifestHash
    $cacheIdentity = Get-SnapshotCacheIdentity
    $cachePaths = Get-SnapshotCachePaths
    $script:SnapshotCacheLock = Enter-SnapshotCacheLock $cachePaths.Lock
    $cacheState = Sync-TrackedSourceSnapshot $cacheIdentity $sourceHashBefore
    $script:SnapshotRoot = $cacheState.Root
    Write-Host ("snapshot cache: rebuilt={0}; key={1}; added={2}; rewritten={3}; removed={4}; unchanged={5}; source_sha256={6}" -f
        $cacheState.Rebuilt,
        $cacheState.Key,
        $cacheState.Added,
        $cacheState.Rewritten,
        $cacheState.Removed,
        $cacheState.Unchanged,
        $cacheState.SourceManifestHash)
    $sourceHashAfterCopy = Get-SourceManifestHash
    $snapshotHashBefore = Get-SourceManifestHash $script:SnapshotRoot
    if (-not [string]::Equals($sourceHashBefore, $sourceHashAfterCopy, [System.StringComparison]::Ordinal) -or
        -not [string]::Equals($sourceHashBefore, $snapshotHashBefore, [System.StringComparison]::Ordinal)) {
        throw "tracked sources changed while the isolated snapshot was copied"
    }

    $toolingWatch = [System.Diagnostics.Stopwatch]::StartNew()
    Initialize-FrontendTooling
    $toolingWatch.Stop()

    $first = New-CurrentStatusCollection 1
    $firstJson = ConvertTo-CanonicalJson $first.Status
    $firstMarkdown = ConvertTo-GeneratedMarkdown $first.Status
    $second = New-CurrentStatusCollection 2
    $secondJson = ConvertTo-CanonicalJson $second.Status
    $secondMarkdown = ConvertTo-GeneratedMarkdown $second.Status

    $snapshotPathsAfter = [string[]]@(Get-SnapshotCacheSourcePaths $script:SnapshotRoot)
    if (-not (Test-OrdinalSequenceEqual ([string[]]@($script:SelectedSourcePaths)) $snapshotPathsAfter)) {
        throw "isolated tracked source snapshot path set changed during collection"
    }
    $snapshotHashAfter = Get-SourceManifestHash $script:SnapshotRoot
    if (-not [string]::Equals($snapshotHashBefore, $snapshotHashAfter, [System.StringComparison]::Ordinal)) {
        throw "isolated tracked source snapshot changed during collection"
    }

    $jsonEqual = [string]::Equals($firstJson, $secondJson, [System.StringComparison]::Ordinal)
    $markdownEqual = [string]::Equals($firstMarkdown, $secondMarkdown, [System.StringComparison]::Ordinal)
    if (-not $jsonEqual -or -not $markdownEqual) {
        [Console]::Error.WriteLine("two consecutive collections were not byte-identical")
        $scriptExitCode = 1
    }
    else {
        $jsonHash = Get-TextSha256 $firstJson
        $markdownHash = Get-TextSha256 $firstMarkdown
        $elapsed = @([double]$first.ElapsedMs, [double]$second.ElapsedMs)
        $averageMs = ($elapsed[0] + $elapsed[1]) / 2.0
        $maximumMs = [Math]::Max($elapsed[0], $elapsed[1])

        $markerGate = [PSCustomObject][ordered]@{
            ExitCode = 0
            MetricIds = @()
            Diagnostics = @()
        }
        if ($Check) {
            $progressText = Read-TrackedText "docs/progress.md" $script:SnapshotRoot
            $markerGate = Invoke-CurrentStatusMarkerGate $firstMarkdown $first.Status $progressText
            foreach ($diagnostic in @($markerGate.Diagnostics)) {
                if ([int]$markerGate.ExitCode -eq 2) {
                    [Console]::Error.WriteLine([string]$diagnostic)
                }
                else {
                    Write-Warning ([string]$diagnostic)
                }
            }
        }

        if (-not $Check) {
            Write-GeneratedFilesAtomically $firstJson $firstMarkdown
        }

        $mirrorDrifts = @(Get-AllMirrorDrifts $first.Status)
        foreach ($drift in $mirrorDrifts) {
            $mirror = $drift.Mirror
            $metricObject = $first.Status.metrics[$drift.Metric]
            $metricSourceSet = ConvertTo-MetricSourceSetDiagnostic $first.Status $drift.Metric
            $sourceValue = if (Test-DictionaryKey $mirror "source_value") { $mirror.source_value } elseif (Test-DictionaryKey $mirror "source_count") { $mirror.source_count } elseif (Test-DictionaryKey $metricObject "count") { $metricObject.count } else { $metricObject.value }
            $observedValue = if (Test-DictionaryKey $mirror "observed_value") { $mirror.observed_value } elseif (Test-DictionaryKey $mirror "observed_count") { $mirror.observed_count } else { "unknown" }
            $setDetail = if ($script:MirrorSetDiagnostics.ContainsKey([string]$mirror.id)) {
                " " + $script:MirrorSetDiagnostics[[string]$mirror.id]
            }
            elseif ((Test-DictionaryKey $mirror "observed_names") -and @($mirror.observed_names).Count -gt 0) {
                $sourceNames = @($metricObject.commands | ForEach-Object { $_.ipc_name })
                " source_value_set=[$($sourceNames -join ',')]; observed_value_set=[$($mirror.observed_names -join ',')]"
            }
            else { "" }
            Write-Warning (('metric={0} profile={1} source_set={2} mirror_path={3} mirror_selector={4} source={5} observed={6}' -f $drift.Metric, $drift.Profile, $metricSourceSet, $mirror.path, $mirror.selector, $sourceValue, $observedValue) + $setDetail)
        }

        $m = $first.Status.metrics
        Write-Host ("6/6 collected: version={0}; workspace={1}; tauri={2}; rust={3}; frontend={4}/{5} files; manual={6}" -f $m.workspace_version.value, $m.workspace_members.count, $m.tauri_commands.count, $m.test_inventory.rust.registered_cases, $m.test_inventory.frontend.default.runnable_cases, $m.test_inventory.frontend.default.files, $m.manual_pages.page_count)
        Write-Host ("determinism: json={0}; markdown={1}; json_sha256={2}; markdown_sha256={3}" -f $jsonEqual, $markdownEqual, $jsonHash, $markdownHash)
        Write-Host ("collection elapsed_ms: first={0:F1}; second={1:F1}; average={2:F1}; maximum={3:F1}; tooling_setup={4:F1}" -f $elapsed[0], $elapsed[1], $averageMs, $maximumMs, $toolingWatch.Elapsed.TotalMilliseconds)
        foreach ($metricId in @("workspace_version", "workspace_members", "tauri_commands", "test_inventory", "proposal_budgets", "manual_pages")) {
            $firstSeconds = [double]$first.MetricTimings[$metricId] / 1000.0
            $secondSeconds = [double]$second.MetricTimings[$metricId] / 1000.0
            Write-Host ("metric timing_seconds: id={0}; first={1:F3}; second={2:F3}; average={3:F3}; maximum={4:F3}" -f $metricId, $firstSeconds, $secondSeconds, (($firstSeconds + $secondSeconds) / 2.0), ([Math]::Max($firstSeconds, $secondSeconds)))
        }
        foreach ($stepId in @("rust_static", "frontend_static_ast", "vitest_default", "vitest_production_symmetry", "frontend_profile_merge", "cargo_registered", "cargo_ignored", "validation_and_shape", "total")) {
            $firstSeconds = [double]$first.TestInventoryTimings[$stepId] / 1000.0
            $secondSeconds = [double]$second.TestInventoryTimings[$stepId] / 1000.0
            Write-Host ("test_inventory timing_seconds: step={0}; first={1:F3}; second={2:F3}; average={3:F3}; maximum={4:F3}" -f $stepId, $firstSeconds, $secondSeconds, (($firstSeconds + $secondSeconds) / 2.0), ([Math]::Max($firstSeconds, $secondSeconds)))
        }
        Write-Host ("mirror drift anchors: {0}" -f $mirrorDrifts.Count)
        $mirrorExitCode = if ($mirrorDrifts.Count -gt 0) { 1 } else { 0 }
        $scriptExitCode = [Math]::Max([int]$markerGate.ExitCode, [int]$mirrorExitCode)
    }
    }
}
catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    if (-not [string]::IsNullOrWhiteSpace($_.ScriptStackTrace)) {
        [Console]::Error.WriteLine($_.ScriptStackTrace)
    }
    $scriptExitCode = 2
}
finally {
    if ($null -ne $script:SnapshotCacheLock) {
        try { $script:SnapshotCacheLock.Dispose() }
        catch { Write-Warning "snapshot cache lock release failed: $($_.Exception.Message)"; $scriptExitCode = 2 }
        $script:SnapshotCacheLock = $null
    }
    $jobWatch.Stop()
    Write-Host ("total job elapsed_ms (including cleanup): {0:F1}" -f $jobWatch.Elapsed.TotalMilliseconds)
}
exit $scriptExitCode
