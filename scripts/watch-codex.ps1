# ORIGAMI3 Codex停滞監視 (Windows PowerShell 5.1対応)
# codexプロセスは1秒ごと、成果物はIntervalSecondsごとに確認する。

[CmdletBinding()]
param(
    [int]$StallMinutes = 30,
    [int]$MaxMinutes = 240,
    [int]$IntervalSeconds = 600,
    [switch]$NeedsApp
)

# NeedsApp指定時はapps/desktopも成果物の最新ファイル監視対象に含める。
# desktop.exeの起動や存在は要求しない。

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
$repoRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$excludedDirectoryNames = @(".git", "target", "node_modules", "dist", "vendor")
$excludedPathPrefixes = @(
    [IO.Path]::GetFullPath((Join-Path $repoRoot "verification\ci-repro")),
    [IO.Path]::GetFullPath((Join-Path $repoRoot "verification\tear-check-target"))
)

function Get-CodexProcessCount {
    return @(Get-Process -Name "codex" -ErrorAction SilentlyContinue).Count
}

function Assert-CodexRunning {
    $count = Get-CodexProcessCount
    if ($count -eq 0) {
        Write-Output "NO_AGENT_RUNNING"
        exit 2
    }
    return $count
}

function Test-IsExcludedDirectory {
    param([string]$Path)

    $fullPath = [IO.Path]::GetFullPath($Path)
    $leaf = Split-Path -Leaf $fullPath
    if ($excludedDirectoryNames -contains $leaf) {
        return $true
    }
    foreach ($prefix in $excludedPathPrefixes) {
        if ([string]::Equals($fullPath, $prefix, [StringComparison]::OrdinalIgnoreCase) -or
            $fullPath.StartsWith($prefix + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            return $true
        }
    }
    return $false
}

function Get-LatestFileMarker {
    param([string]$Root)

    if (-not (Test-Path -LiteralPath $Root)) {
        return "$Root|MISSING"
    }

    $rootItem = Get-Item -LiteralPath $Root -Force
    if (-not $rootItem.PSIsContainer) {
        return "$($rootItem.FullName)|$($rootItem.LastWriteTimeUtc.Ticks)|$($rootItem.Length)"
    }

    $latest = $null
    $processedFiles = 0
    $pending = New-Object 'System.Collections.Generic.Stack[string]'
    $pending.Push($rootItem.FullName)

    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        if (Test-IsExcludedDirectory $directory) {
            continue
        }

        try {
            $filePaths = @([IO.Directory]::EnumerateFiles($directory))
        }
        catch {
            continue
        }
        foreach ($filePath in $filePaths) {
            $processedFiles++
            if (($processedFiles % 128) -eq 0) {
                $null = Assert-CodexRunning
            }
            try {
                $file = Get-Item -LiteralPath $filePath -Force
                if ($null -eq $latest -or $file.LastWriteTimeUtc -gt $latest.LastWriteTimeUtc) {
                    $latest = $file
                }
            }
            catch {
                # 一時ファイルが列挙直後に消えた場合は、次回のスナップショットで再評価する。
            }
        }

        try {
            $childDirectories = @([IO.Directory]::EnumerateDirectories($directory))
        }
        catch {
            continue
        }
        foreach ($childDirectory in $childDirectories) {
            if (-not (Test-IsExcludedDirectory $childDirectory)) {
                $pending.Push($childDirectory)
            }
        }
        $null = Assert-CodexRunning
    }

    if ($null -eq $latest) {
        return "$($rootItem.FullName)|EMPTY"
    }
    return "$($latest.FullName)|$($latest.LastWriteTimeUtc.Ticks)|$($latest.Length)"
}

function Get-ArtifactSnapshot {
    $global:LASTEXITCODE = 0
    $statusLines = @(& git -C $repoRoot status --porcelain=v1 --untracked-files=all 2>$null)
    if ($LASTEXITCODE -ne 0) {
        throw "git status failed with exit code $LASTEXITCODE"
    }

    $targets = @(
        (Join-Path $repoRoot "crates"),
        (Join-Path $repoRoot "docs"),
        (Join-Path $repoRoot "scripts"),
        (Join-Path $repoRoot "verification"),
        (Join-Path $repoRoot "scratchpad")
    )
    if ($NeedsApp) {
        $targets += Join-Path $repoRoot "apps\desktop"
    }

    $parts = New-Object 'System.Collections.Generic.List[string]'
    $parts.Add("git=" + ($statusLines -join "`n"))
    foreach ($target in $targets) {
        $parts.Add((Get-LatestFileMarker $target))
        $null = Assert-CodexRunning
    }
    return $parts -join "`n---`n"
}

try {
    if ($StallMinutes -le 0) {
        throw "StallMinutes must be greater than zero"
    }
    if ($MaxMinutes -le 0) {
        throw "MaxMinutes must be greater than zero"
    }
    if ($IntervalSeconds -le 0) {
        throw "IntervalSeconds must be greater than zero"
    }

    $agentCount = Assert-CodexRunning

    $startedAt = [DateTime]::UtcNow
    $lastProgressAt = $startedAt
    $previousSnapshot = Get-ArtifactSnapshot
    $nextArtifactCheck = $startedAt.AddSeconds($IntervalSeconds)

    while ($true) {
        $agentCount = Assert-CodexRunning

        $now = [DateTime]::UtcNow
        $elapsedMinutes = [int][Math]::Floor(($now - $startedAt).TotalMinutes)
        if (($now - $startedAt).TotalMinutes -ge $MaxMinutes) {
            Write-Output "MAX_TIME_REACHED elapsed=$($elapsedMinutes)min agents=$agentCount"
            exit 3
        }

        if ($now -ge $nextArtifactCheck) {
            $currentSnapshot = Get-ArtifactSnapshot
            if ($currentSnapshot -cne $previousSnapshot) {
                $lastProgressAt = $now
                $previousSnapshot = $currentSnapshot
            }

            $idleMinutes = [int][Math]::Floor(($now - $lastProgressAt).TotalMinutes)
            if (($now - $lastProgressAt).TotalMinutes -ge $StallMinutes) {
                Write-Output "STALLED idle=$($idleMinutes)min agents=$agentCount"
                exit 1
            }

            Write-Output "STILL_WORKING idle=$($idleMinutes)min agents=$agentCount"
            $nextArtifactCheck = $now.AddSeconds($IntervalSeconds)
        }

        Start-Sleep -Seconds 1
    }
}
catch {
    $message = ($_.Exception.Message -replace '[\r\n]+', ' ').Trim()
    Write-Output "MONITOR_ERROR message=$message"
    exit 4
}
