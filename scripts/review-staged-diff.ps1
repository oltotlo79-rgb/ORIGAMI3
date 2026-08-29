<#
.SYNOPSIS
コミット前に staged diff の全文を表示し、確認済みハッシュをreceiptへ記録します。
確認後に staged 内容が変わっていないかを -Verify で検査します。

.DESCRIPTION
規約 docs/rules/06-過去の失敗と対策.md §10.7.1「差分を読まずにコミットし、
コミット済みの内容で組み立てられなくなった」への対策です。`git diff --stat`
の行数だけを見て中身を読まなかった失敗を防ぐため、①`git diff --cached`の全文を
必ず出力し、②その内容のSHA-256を「確認済み」としてreceiptへ記録し、③コミット
直前に-Verifyでreceipt保存後に staged 内容が変わっていないことを確かめます。

`git diff --cached`の取得は`System.Diagnostics.Process`へ`StandardOutputEncoding`を
明示してUTF-8で復号します（PowerShellの`&`によるネイティブコマンド捕捉は
呼び出し元セッションの`[Console]::OutputEncoding`に依存し、既定コードページが
UTF-8でない環境では日本語が化けることを`check-rules-split.ps1`の調査で実測した
ため、同じ危険を避けています）。

.PARAMETER RepoRoot
検査対象のgitリポジトリのルート。既定はこのscriptの2階層上（ORIGAMI3のルート）。

.PARAMETER ReceiptPath
確認済みハッシュを記録するreceiptファイルの絶対パス。既定は
`%TEMP%/ori3-staged-diff-receipt-<RepoRootのSHA-256先頭16桁>.json`
（リポジトリの外だけを使い、`.git`配下を含めリポジトリ内へは一切書き込まない。
複数worktreeを並行して使っても、RepoRootごとに別ファイルへ分かれる）。

.PARAMETER Verify
このスイッチを指定すると、表示・記録を行わず、現在のstaged diffのハッシュが
receiptの確認済みハッシュと一致するかどうかだけを判定します。
#>
[CmdletBinding()]
param(
    [string]$RepoRoot,
    [string]$ReceiptPath,
    [switch]$Verify
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
# 呼び出し元がこのscriptをredirect付きの子processとして起動した場合、
# このscript自身のWrite-Output/Write-Hostの符号化は[Console]::OutputEncodingに
# 従う。呼び出し元がProcessStartInfo.StandardOutputEncodingでUTF-8を指定していても、
# 子（このscript自身）が別の既定コードページで書き出せば文字化けする
# （check-rules-split.ps1の調査と同じ原理、書く側と読む側の両方を合わせる必要がある）。
[Console]::OutputEncoding = [Text.UTF8Encoding]::new()

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Split-Path -Parent $PSScriptRoot
}
$RepoRoot = [IO.Path]::GetFullPath($RepoRoot).TrimEnd([char[]]"\/")

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

if ([string]::IsNullOrWhiteSpace($ReceiptPath)) {
    $repoRootHashPrefix = (Get-Sha256HexOfString $RepoRoot).Substring(0, 16)
    $ReceiptPath = Join-Path ([IO.Path]::GetTempPath()) ("ori3-staged-diff-receipt-{0}.json" -f $repoRootHashPrefix)
}

function ConvertTo-ProcessArgumentString {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$ArgumentValues)

    # このPowerShell 5.1 / .NET Framework環境ではProcessStartInfo.ArgumentList
    # プロパティが無いこと（PropertyNotFoundStrict）を実測したため、昔ながらの
    # 単一文字列Argumentsを自前で組み立てる。値は二重引用符で囲み、内部の
    # 二重引用符だけを`\"`へ、末尾の連続する`\`は引用符の直前でだけ2倍にする
    # （Windowsのコマンドライン引数解析規則）。
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

    # PowerShellの`&`によるネイティブコマンド捕捉は[Console]::OutputEncodingに
    # 依存する（check-rules-split.ps1の調査で実測済み）。System.Diagnostics.Process
    # へStandardOutputEncodingを明示することで、呼び出し元セッションの設定に
    # 左右されず常にUTF-8で復号する。
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = "git"
    $psi.WorkingDirectory = $RepoRoot
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

if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot ".git"))) {
    Write-Output ("[NG] gitリポジトリが見つかりません: {0}" -f $RepoRoot)
    exit 2
}

$diffText = Get-GitCommandText -RepoRoot $RepoRoot -GitArguments @("diff", "--cached")
$currentHash = Get-Sha256HexOfString $diffText
$changedFiles = @((Get-GitCommandText -RepoRoot $RepoRoot -GitArguments @("diff", "--cached", "--name-only")) -split "`n" |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) })

if ($Verify) {
    if (-not (Test-Path -LiteralPath $ReceiptPath -PathType Leaf)) {
        Write-Output "[NG] 確認済みreceiptが見つかりません。先に -Verify を付けずに実行し、全文を読んでから確認してください。"
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
    if ($receipt.Sha256 -ne $currentHash) {
        Write-Output "[NG] 確認後にstaged内容が変わっています。全文を読み直して再確認してください。"
        Write-Output ("     確認済みハッシュ: {0}" -f $receipt.Sha256)
        Write-Output ("     現在のハッシュ:   {0}" -f $currentHash)
        exit 1
    }
    Write-Output ("[OK] staged内容は確認時点から変わっていません（対象 {0} 件、確認日時 {1}）。" -f $changedFiles.Count, $receipt.ConfirmedAtUtc)
    exit 0
}

Write-Output "===== git diff --cached (全文) ====="
if ([string]::IsNullOrEmpty($diffText)) {
    Write-Output "(staged差分はありません)"
}
else {
    Write-Output $diffText
}
Write-Output "===== ここまで ====="
Write-Output ("対象ファイル数: {0}" -f $changedFiles.Count)
foreach ($file in $changedFiles) {
    Write-Output ("  - {0}" -f $file)
}

$receiptObject = [pscustomobject]@{
    RepoRoot = $RepoRoot
    Sha256 = $currentHash
    FileCount = $changedFiles.Count
    ConfirmedAtUtc = [DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ssZ")
}
$receiptDirectory = Split-Path -Parent $ReceiptPath
[void][IO.Directory]::CreateDirectory($receiptDirectory)
[IO.File]::WriteAllText($ReceiptPath, ($receiptObject | ConvertTo-Json), [Text.UTF8Encoding]::new($false))

Write-Output ("[OK] 全文を表示し、確認済みreceiptを記録しました（SHA-256: {0}）。" -f $currentHash)
Write-Output "     コミット直前に同じ引数へ -Verify を付けて実行し、内容が変わっていないことを確かめてください。"
exit 0
