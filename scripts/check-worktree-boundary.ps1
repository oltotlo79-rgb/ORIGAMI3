<#
.SYNOPSIS
作業ツリーの開始時点の未コミット差分と、許可されたpathの一覧（manifest）を記録し、
後から「既存差分が変更されていないか」「許可範囲外のpathが変更されていないか」を
検査します。

.DESCRIPTION
規約 docs/rules/06-過去の失敗と対策.md §10.7.2「`git checkout --` で他の未コミット
作業を巻き添えにした」、および §2「担当のgit禁止」「触ってよいファイルの限定」への
対策です。委譲の開始時点で -Init を実行して基準（baseline）を記録し、担当の作業後に
-Verify を実行して、①開始時点で既に汚れていたファイルの内容がさらに変わっていないか、
②開始時点で汚れていなかった新規のpathが、許可されたpath manifestの範囲内かを検査します。

内容の同一性はSHA-256で判定します。`git status --porcelain`とファイル内容の取得は
`System.Diagnostics.Process`へ`StandardOutputEncoding`を明示してUTF-8で復号します
（check-rules-split.ps1の調査で実測した、PowerShellの`&`によるネイティブコマンド
捕捉が呼び出し元セッションの`[Console]::OutputEncoding`に依存する問題を避けるため）。

.PARAMETER RepoRoot
検査対象のgit作業ツリーのルート。既定はこのscriptの2階層上（ORIGAMI3のルート）。
committed済みの本体でもgit worktreeでも使えます。

.PARAMETER ReceiptPath
基準を記録するreceiptファイルの絶対パス。既定は
`%TEMP%/ori3-worktree-boundary-receipt-<RepoRootのSHA-256先頭16桁>.json`
（リポジトリの外だけを使い、リポジトリ内へは一切書き込まない）。

.PARAMETER Init
このスイッチを指定すると、現在の`git status --porcelain`を基準として記録します。
`-AllowedPaths` の指定が必須です。

.PARAMETER AllowedPaths
-Init と併用し、担当が変更してよいpathのパターン一覧を指定します（RepoRoot相対、
`/`区切り、`*`によるワイルドカードを使えます。例: `scripts/*`, `docs/rules/00-*.md`）。

.PARAMETER Verify
このスイッチを指定すると、記録済みの基準と現在の状態を比較し、
①基準時点で既に汚れていたpathの内容がさらに変わっていないか、
②基準時点で汚れていなかった新規のpathが許可pattern内かを検査します。
#>
[CmdletBinding()]
param(
    [string]$RepoRoot,
    [string]$ReceiptPath,
    [switch]$Init,
    [string[]]$AllowedPaths,
    [switch]$Verify
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
# 呼び出し元セッションの[Console]::OutputEncodingに関わらず、このscript自身の
# Write-Output/Write-Hostを常にUTF-8で書き出す（review-staged-diff.ps1と同じ対策）。
[Console]::OutputEncoding = [Text.UTF8Encoding]::new()

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Split-Path -Parent $PSScriptRoot
}
$RepoRoot = [IO.Path]::GetFullPath($RepoRoot).TrimEnd([char[]]"\/")

function ConvertTo-ProcessArgumentString {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$ArgumentValues)

    # このPowerShell 5.1 / .NET Framework環境にProcessStartInfo.ArgumentListが
    # 無いこと（PropertyNotFoundStrict）を実測したため、単一文字列Argumentsを
    # 自前で組み立てる（review-staged-diff.ps1と同じ回避）。
    $parts = foreach ($value in $ArgumentValues) {
        $escaped = [regex]::Replace($value, '(\\*)"', '$1$1\"')
        $trailingBackslashes = [regex]::Match($escaped, '\\*$').Value
        $escaped = $escaped + $trailingBackslashes
        '"' + $escaped + '"'
    }
    return ($parts -join " ")
}

function Get-GitCommandText {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string[]]$GitArguments
    )

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = "git"
    $psi.Arguments = ConvertTo-ProcessArgumentString (@("-C", $RepoRoot) + $GitArguments)
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [Text.Encoding]::UTF8
    $psi.UseShellExecute = $false

    $process = [System.Diagnostics.Process]::Start($psi)
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) {
        throw ("git {0} が失敗しました (終了コード: {1}): {2}" -f ($GitArguments -join " "), $process.ExitCode, $stderr)
    }
    return $stdout
}

function Get-Sha256HexOfFile {
    param([Parameter(Mandatory = $true)][string]$Path)

    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $stream = [IO.File]::OpenRead($Path)
        try {
            $hashBytes = $sha256.ComputeHash($stream)
        }
        finally {
            $stream.Dispose()
        }
    }
    finally {
        $sha256.Dispose()
    }
    return -join ($hashBytes | ForEach-Object { $_.ToString("x2") })
}

function Get-Sha256HexOfString {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    $bytes = [Text.Encoding]::UTF8.GetBytes($Text)
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        return -join ($sha256.ComputeHash($bytes) | ForEach-Object { $_.ToString("x2") })
    }
    finally {
        $sha256.Dispose()
    }
}

# `git status --porcelain`の1行から、状態コード(先頭2文字)とpath部分を取り出す。
# rename(`R  old -> new`)はnew側を対象pathとして扱う。引用符付きpath
# （空白や非ASCIIを含む場合にgitが付ける）はTrimQuoteだけ簡易対応する。
function ConvertFrom-PorcelainLine {
    param([Parameter(Mandatory = $true)][string]$Line)

    if ($Line.Length -lt 4) {
        return $null
    }
    $status = $Line.Substring(0, 2)
    $pathPart = $Line.Substring(3)
    $arrowIndex = $pathPart.IndexOf(" -> ", [StringComparison]::Ordinal)
    if ($arrowIndex -ge 0) {
        $pathPart = $pathPart.Substring($arrowIndex + 4)
    }
    if ($pathPart.StartsWith('"') -and $pathPart.EndsWith('"') -and $pathPart.Length -ge 2) {
        $pathPart = $pathPart.Substring(1, $pathPart.Length - 2)
    }
    [pscustomobject]@{
        Status = $status
        Path = $pathPart.Replace('\', '/')
    }
}

function Get-WorktreeDirtyEntries {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $statusText = Get-GitCommandText -RepoRoot $RepoRoot -GitArguments @("status", "--porcelain")
    $entries = [Collections.Generic.List[object]]::new()
    foreach ($line in ($statusText -split "`n")) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        $parsed = ConvertFrom-PorcelainLine $line
        if ($null -eq $parsed) { continue }
        $isDeleted = $parsed.Status.Contains("D")
        $fullPath = Join-Path $RepoRoot ($parsed.Path.Replace('/', [IO.Path]::DirectorySeparatorChar))
        $hash = $null
        if (-not $isDeleted -and (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
            $hash = Get-Sha256HexOfFile $fullPath
        }
        $entries.Add([pscustomobject]@{
                Path = $parsed.Path
                Status = $parsed.Status
                Deleted = $isDeleted
                Sha256 = $hash
            })
    }
    return $entries.ToArray()
}

function Test-PathMatchesAnyPattern {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Patterns
    )

    foreach ($pattern in $Patterns) {
        if ($Path -like $pattern) {
            return $true
        }
    }
    return $false
}

if ([string]::IsNullOrWhiteSpace($ReceiptPath)) {
    $repoRootHashPrefix = (Get-Sha256HexOfString $RepoRoot).Substring(0, 16)
    $ReceiptPath = Join-Path ([IO.Path]::GetTempPath()) ("ori3-worktree-boundary-receipt-{0}.json" -f $repoRootHashPrefix)
}

if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot ".git"))) {
    Write-Output ("[NG] gitリポジトリが見つかりません: {0}" -f $RepoRoot)
    exit 2
}

if ($Init -and $Verify) {
    Write-Output "[NG] -Init と -Verify は同時に指定できません。"
    exit 2
}

if ($Init) {
    if ($null -eq $AllowedPaths -or $AllowedPaths.Count -eq 0) {
        Write-Output "[NG] -Init には -AllowedPaths（許可pathのpattern一覧、1件以上）が必要です。"
        exit 2
    }
    $baseline = Get-WorktreeDirtyEntries -RepoRoot $RepoRoot
    $receiptObject = [pscustomobject]@{
        RepoRoot = $RepoRoot
        AllowedPaths = @($AllowedPaths)
        Baseline = @($baseline)
        RecordedAtUtc = [DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ssZ")
    }
    $receiptDirectory = Split-Path -Parent $ReceiptPath
    [void][IO.Directory]::CreateDirectory($receiptDirectory)
    [IO.File]::WriteAllText($ReceiptPath, ($receiptObject | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
    Write-Output ("[OK] 基準を記録しました（既存差分 {0} 件、許可pattern {1} 件）。" -f $baseline.Count, $AllowedPaths.Count)
    exit 0
}

if ($Verify) {
    if (-not (Test-Path -LiteralPath $ReceiptPath -PathType Leaf)) {
        Write-Output "[NG] 基準receiptが見つかりません。先に -Init を実行してください。"
        exit 1
    }
    $receipt = $null
    try {
        $receipt = (Get-Content -LiteralPath $ReceiptPath -Raw -Encoding UTF8) | ConvertFrom-Json
    }
    catch {
        Write-Output ("[NG] receiptを読み取れません: {0}" -f $_.Exception.Message)
        exit 2
    }
    if ($receipt.RepoRoot -ne $RepoRoot) {
        Write-Output ("[NG] receiptは別のリポジトリのものです (receipt: {0} / 現在: {1})。作り直してください。" -f $receipt.RepoRoot, $RepoRoot)
        exit 1
    }

    $baselineByPath = @{}
    foreach ($entry in @($receipt.Baseline)) {
        $baselineByPath[$entry.Path] = $entry
    }
    $allowedPatterns = @($receipt.AllowedPaths)
    $current = Get-WorktreeDirtyEntries -RepoRoot $RepoRoot

    $violations = [Collections.Generic.List[string]]::new()
    foreach ($entry in $current) {
        if ($baselineByPath.ContainsKey($entry.Path)) {
            $base = $baselineByPath[$entry.Path]
            $baseDeleted = [bool]$base.Deleted
            if ($baseDeleted -ne $entry.Deleted) {
                $violations.Add("既存差分の状態が変わりました(削除⇔復元): $($entry.Path)")
            }
            elseif (-not $entry.Deleted -and $base.Sha256 -ne $entry.Sha256) {
                $violations.Add("既存差分を変更しました: $($entry.Path)")
            }
        }
        else {
            if (-not (Test-PathMatchesAnyPattern -Path $entry.Path -Patterns $allowedPatterns)) {
                $violations.Add("許可範囲外のpathを変更しました: $($entry.Path)")
            }
        }
    }

    if ($violations.Count -gt 0) {
        Write-Output ("[NG] 作業ツリー境界の違反 {0} 件:" -f $violations.Count)
        foreach ($violation in $violations) {
            Write-Output ("     - {0}" -f $violation)
        }
        exit 1
    }
    Write-Output ("[OK] 既存差分の変更・許可範囲外のpath変更はありません（現在の差分 {0} 件、許可pattern {1} 件）。" -f $current.Count, $allowedPatterns.Count)
    exit 0
}

Write-Output "[NG] -Init または -Verify のいずれかを指定してください。"
exit 2
