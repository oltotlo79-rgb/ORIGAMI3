<#
.SYNOPSIS
作業ツリーの未コミット状態を refs/wip/<name> へ退避する（中断・再開用）。

.DESCRIPTION
hook を通さない plumbing（read-tree / add / write-tree / commit-tree / update-ref）で、
各作業ツリーの「追跡対象の変更＋未追跡ファイル＋scratchpad の報告書」を1つの commit として
refs/wip/<name> に記録する。作業ツリー自体は一切変更しない。push もされない。

作業ツリー（%TEMP%\ori3-wt-*）が消えても、次のコマンドで内容を復元できる。
    git worktree add <path> refs/wip/<name>
    git checkout refs/wip/<name> -- <path>

除外するもの（利用者指示・規約）:
  docs/competitive-review-2026-08-20.md   触らない（読まない・参照しない・コミットしない）
  traditional_crane_math_bundle/          リポジトリ直下の受領物（追跡対象外のまま置く）
  traditional_crane_complete_cp.png       同上

.PARAMETER Name
退避先の名前（refs/wip/<Name>）。省略時は既定の6ツリーをすべて処理する。

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/snapshot-worktrees.ps1
.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/snapshot-worktrees.ps1 -Name cifix
#>
[CmdletBinding()]
param(
    [string]$Name
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Temp = [Environment]::GetEnvironmentVariable("TEMP")

$Targets = [ordered]@{
    "main-rust-slice" = $RepositoryRoot
    "fixb"            = Join-Path $Temp "ori3-wt-fixb"
    "layer1"          = Join-Path $Temp "ori3-wt-layer1"
    "collapse"        = Join-Path $Temp "ori3-wt-collapse"
    "viewer"          = Join-Path $Temp "ori3-wt-viewer"
    "cifix"           = Join-Path $Temp "ori3-wt-cifix"
}

$Excluded = @(
    "docs/competitive-review-2026-08-20.md",
    "traditional_crane_math_bundle",
    "traditional_crane_complete_cp.png"
)

function Invoke-Git {
    param(
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [string]$IndexFile,
        [switch]$AllowFailure
    )

    $previousIndex = $env:GIT_INDEX_FILE
    if ($IndexFile) { $env:GIT_INDEX_FILE = $IndexFile }
    $previousPreference = $ErrorActionPreference
    try {
        # 改行コードの予告など git の警告は stderr へ出る。PowerShell 5.1 では
        # 外部コマンドの stderr を畳み込むと ErrorRecord になるため、
        # 畳み込まずに終了コードだけで成否を判定する。
        $ErrorActionPreference = "Continue"
        Push-Location -LiteralPath $WorkingDirectory
        try {
            $output = & git @Arguments
            $exitCode = $LASTEXITCODE
        }
        finally {
            Pop-Location
        }
    }
    finally {
        $ErrorActionPreference = $previousPreference
        if ($IndexFile) {
            if ($null -eq $previousIndex) {
                Remove-Item Env:GIT_INDEX_FILE -ErrorAction SilentlyContinue
            }
            else {
                $env:GIT_INDEX_FILE = $previousIndex
            }
        }
    }
    if ($exitCode -ne 0 -and -not $AllowFailure) {
        throw "git $($Arguments -join ' ') failed with exit code ${exitCode}: $output"
    }
    return ($output | Where-Object { $_ -ne $null } | ForEach-Object { $_.ToString() })
}

function Save-Snapshot {
    param(
        [Parameter(Mandatory = $true)][string]$SnapshotName,
        [Parameter(Mandatory = $true)][string]$WorkTree
    )

    if (-not (Test-Path -LiteralPath $WorkTree -PathType Container)) {
        Write-Output "[SKIP] $SnapshotName : 作業ツリーがありません ($WorkTree)"
        return
    }

    $indexFile = Join-Path ([IO.Path]::GetTempPath()) ("ori3-snapshot-" + [Guid]::NewGuid().ToString("N") + ".index")
    try {
        Invoke-Git -WorkingDirectory $WorkTree -Arguments @("read-tree", "HEAD") -IndexFile $indexFile | Out-Null
        Invoke-Git -WorkingDirectory $WorkTree -Arguments @("add", "-A", ".") -IndexFile $indexFile -AllowFailure | Out-Null
        foreach ($path in $Excluded) {
            Invoke-Git -WorkingDirectory $WorkTree -Arguments @("rm", "-r", "-q", "--cached", "--ignore-unmatch", $path) -IndexFile $indexFile -AllowFailure | Out-Null
        }
        # 報告書は .gitignore の対象でも必ず残す（引き継ぎの正本のため）。
        # 画像・フレーム・組み立て成果物まで取り込まないよう、文書と差分だけに限る。
        foreach ($pattern in @("scratchpad/*.md", "scratchpad/*.patch", "scratchpad/*.txt", "scratchpad/**/*.md")) {
            Invoke-Git -WorkingDirectory $WorkTree -Arguments @("add", "-f", "--", $pattern) -IndexFile $indexFile -AllowFailure | Out-Null
        }

        $tree = (Invoke-Git -WorkingDirectory $WorkTree -Arguments @("write-tree") -IndexFile $indexFile) -join ""
        if ($tree -notmatch "^[0-9a-f]{40}$") {
            throw "write-tree が木のIDを返しませんでした: $tree"
        }
        $head = (Invoke-Git -WorkingDirectory $WorkTree -Arguments @("rev-parse", "HEAD")) -join ""
        $stamp = Get-Date -Format "yyyy-MM-dd HH:mm"
        $message = "WIP snapshot $SnapshotName $stamp (no hooks; for resume)"
        $commit = (Invoke-Git -WorkingDirectory $WorkTree -Arguments @("commit-tree", $tree, "-p", $head, "-m", $message)) -join ""
        if ($commit -notmatch "^[0-9a-f]{40}$") {
            throw "commit-tree がコミットのIDを返しませんでした: $commit"
        }
        Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("update-ref", "refs/wip/$SnapshotName", $commit) | Out-Null

        $summary = (Invoke-Git -WorkingDirectory $RepositoryRoot -Arguments @("diff", "--shortstat", $head, $commit)) -join " "
        if ([string]::IsNullOrWhiteSpace($summary)) { $summary = "HEAD と同じ内容" }
        Write-Output "[OK]   $SnapshotName -> $($commit.Substring(0,7))  (HEAD $($head.Substring(0,7)))  $($summary.Trim())"
    }
    finally {
        if (Test-Path -LiteralPath $indexFile) { Remove-Item -LiteralPath $indexFile -Force }
    }
}

if ($Name) {
    if (-not $Targets.Contains($Name)) {
        throw "未知の退避名です: $Name（使える名前: $($Targets.Keys -join ', ')）"
    }
    Save-Snapshot -SnapshotName $Name -WorkTree $Targets[$Name]
}
else {
    foreach ($key in $Targets.Keys) {
        Save-Snapshot -SnapshotName $key -WorkTree $Targets[$key]
    }
}
