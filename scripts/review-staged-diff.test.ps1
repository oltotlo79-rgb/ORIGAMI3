[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$scriptPath = Join-Path $PSScriptRoot "review-staged-diff.ps1"
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-review-staged-diff-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [IO.Path]::GetFullPath((Join-Path $tempBase $sandboxName))
$script:AssertionCount = 0

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$Output = ""
    )
    $script:AssertionCount += 1
    if (-not $Condition) {
        throw "ASSERTION FAILED: $Message`n$Output"
    }
}

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)][AllowNull()]$Actual,
        [Parameter(Mandatory = $true)][AllowNull()]$Expected,
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

# 使い捨てのgitリポジトリをサンドボックス内に作る。本体リポジトリには一切触れない。
function New-DisposableGitRepo {
    param([Parameter(Mandatory = $true)][string]$Name)

    $repoRoot = Join-Path $sandboxRoot $Name
    [void][IO.Directory]::CreateDirectory($repoRoot)
    # Invoke-GitInの戻り値(stdout文字列)を捨てずに置くと、この関数自身の出力
    # ストリームへ混ざり込み、最後のreturn $repoRootと一緒に配列化されてしまう
    # (PowerShellの暗黙のパイプライン出力の実測で発見)。[void]で明示的に捨てる。
    [void](Invoke-GitIn $repoRoot @("init", "--quiet"))
    [void](Invoke-GitIn $repoRoot @("config", "user.email", "test@example.com"))
    [void](Invoke-GitIn $repoRoot @("config", "user.name", "Test User"))
    [IO.File]::WriteAllText((Join-Path $repoRoot "base.txt"), "line1`nline2`n", [Text.UTF8Encoding]::new($false))
    [void](Invoke-GitIn $repoRoot @("add", "base.txt"))
    [void](Invoke-GitIn $repoRoot @("commit", "--quiet", "-m", "initial"))
    return $repoRoot
}

function ConvertTo-ProcessArgumentString {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$ArgumentValues)

    # review-staged-diff.ps1と同じ回避（このPowerShell 5.1環境にProcessStartInfo.
    # ArgumentListが無いことを実測したため、単一文字列Argumentsを自前で組み立てる）。
    $parts = foreach ($value in $ArgumentValues) {
        $escaped = [regex]::Replace($value, '(\\*)"', '$1$1\"')
        $trailingBackslashes = [regex]::Match($escaped, '\\*$').Value
        $escaped = $escaped + $trailingBackslashes
        '"' + $escaped + '"'
    }
    return ($parts -join " ")
}

function Invoke-GitIn {
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
        throw ("git {0} が失敗しました: {1}{2}" -f ($GitArguments -join " "), $stdout, $stderr)
    }
    return $stdout
}

function Invoke-Reviewer {
    param([Parameter(Mandatory = $true)][hashtable]$NamedArgs)

    $argValues = New-Object System.Collections.Generic.List[string]
    $argValues.Add("-NoProfile")
    $argValues.Add("-NonInteractive")
    $argValues.Add("-ExecutionPolicy")
    $argValues.Add("Bypass")
    $argValues.Add("-File")
    $argValues.Add($scriptPath)
    foreach ($key in $NamedArgs.Keys) {
        $argValues.Add("-$key")
        $value = $NamedArgs[$key]
        if ($value -isnot [switch] -and $value -isnot [bool]) {
            $argValues.Add([string]$value)
        }
    }
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = (Get-Process -Id $PID).Path
    $psi.Arguments = ConvertTo-ProcessArgumentString $argValues.ToArray()
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [Text.Encoding]::UTF8
    $psi.UseShellExecute = $false
    $process = [System.Diagnostics.Process]::Start($psi)
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    [pscustomobject]@{ ExitCode = $process.ExitCode; Output = ($stdout + "`n" + $stderr) }
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $sandboxRoot)) { return }
    $resolved = [IO.Path]::GetFullPath($sandboxRoot).TrimEnd([char[]]"\/")
    $parent = [IO.Path]::GetDirectoryName($resolved)
    $leaf = [IO.Path]::GetFileName($resolved)
    if (($parent -ne $tempBase) -or
        (-not [regex]::IsMatch($leaf, '^ori3-review-staged-diff-test-[0-9a-f]{32}$', [Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
        throw "安全でない一時領域の削除を拒否しました: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction SilentlyContinue
    foreach ($receipt in @(Get-ChildItem -LiteralPath ([IO.Path]::GetTempPath()) -Filter "ori3-staged-diff-receipt-*.json" -ErrorAction SilentlyContinue)) {
        if ($script:KnownReceiptPaths -contains $receipt.FullName) {
            Remove-Item -LiteralPath $receipt.FullName -Force -ErrorAction SilentlyContinue
        }
    }
}

if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
    throw "検査本体が見つかりません: $scriptPath"
}

$script:KnownReceiptPaths = New-Object System.Collections.Generic.List[string]
[void][IO.Directory]::CreateDirectory($sandboxRoot)
try {
    Write-Output "[1/7] staged差分が無い場合も正常に表示・記録できる"
    $repoA = New-DisposableGitRepo "no-staged-changes"
    $receiptA = Join-Path $sandboxRoot "receipt-a.json"
    $script:KnownReceiptPaths.Add($receiptA)
    $result = Invoke-Reviewer @{ RepoRoot = $repoA; ReceiptPath = $receiptA }
    Assert-Equal $result.ExitCode 0 "staged差分が無くても正常終了すること" $result.Output
    Assert-Contains $result.Output "staged差分はありません" "差分が無いことを明示すること"
    Assert-True (Test-Path -LiteralPath $receiptA -PathType Leaf) "差分が無くてもreceiptを記録すること"

    Write-Output "[2/7] staged差分の全文を表示し、確認済みreceiptを記録する"
    $repoB = New-DisposableGitRepo "with-staged-changes"
    [IO.File]::WriteAllText((Join-Path $repoB "base.txt"), "line1`nline2-changed`nline3-new`n", [Text.UTF8Encoding]::new($false))
    [void](Invoke-GitIn $repoB @("add", "base.txt"))
    $receiptB = Join-Path $sandboxRoot "receipt-b.json"
    $script:KnownReceiptPaths.Add($receiptB)
    $result = Invoke-Reviewer @{ RepoRoot = $repoB; ReceiptPath = $receiptB }
    Assert-Equal $result.ExitCode 0 "staged差分があれば全文表示のうえ正常終了すること" $result.Output
    Assert-Contains $result.Output "line2-changed" "差分の実際の変更行が全文に含まれること"
    Assert-Contains $result.Output "対象ファイル数: 1" "対象ファイル数を表示すること"
    Assert-True (Test-Path -LiteralPath $receiptB -PathType Leaf) "確認済みreceiptを記録すること"

    Write-Output "[3/7] 確認後に変化が無ければ -Verify は合格する"
    $result = Invoke-Reviewer @{ RepoRoot = $repoB; ReceiptPath = $receiptB; Verify = $true }
    Assert-Equal $result.ExitCode 0 "確認後に変化が無ければ合格すること" $result.Output
    Assert-Contains $result.Output "変わっていません" "不変であることを明示すること"

    Write-Output "[4/7] 確認後にさらにstageすると -Verify は不合格になる(不合格例)"
    [IO.File]::WriteAllText((Join-Path $repoB "extra.txt"), "extra content`n", [Text.UTF8Encoding]::new($false))
    [void](Invoke-GitIn $repoB @("add", "extra.txt"))
    $result = Invoke-Reviewer @{ RepoRoot = $repoB; ReceiptPath = $receiptB; Verify = $true }
    Assert-Equal $result.ExitCode 1 "確認後の追加staged変更を合格扱いしてはならない" $result.Output
    Assert-Contains $result.Output "変わっています" "変化を検出したことを明示すること"

    Write-Output "[5/7] receiptが無い状態での -Verify は不合格になる(不合格例)"
    $repoC = New-DisposableGitRepo "verify-without-receipt"
    $receiptC = Join-Path $sandboxRoot "receipt-c-never-created.json"
    $result = Invoke-Reviewer @{ RepoRoot = $repoC; ReceiptPath = $receiptC; Verify = $true }
    Assert-Equal $result.ExitCode 1 "receiptが無い-Verifyを合格扱いしてはならない" $result.Output
    Assert-Contains $result.Output "receiptが見つかりません" "receipt不在を明示すること"

    Write-Output "[6/7] 別リポジトリのreceiptでの -Verify は不合格になる(不合格例)"
    $repoD = New-DisposableGitRepo "different-repo"
    $result = Invoke-Reviewer @{ RepoRoot = $repoD; ReceiptPath = $receiptB; Verify = $true }
    Assert-Equal $result.ExitCode 1 "別リポジトリのreceiptを合格扱いしてはならない" $result.Output
    Assert-Contains $result.Output "別のリポジトリ" "リポジトリ不一致を明示すること"

    Write-Output "[7/7] gitリポジトリでない場所を指定すると失敗する(不合格例)"
    $notARepo = Join-Path $sandboxRoot "not-a-repo"
    [void][IO.Directory]::CreateDirectory($notARepo)
    $result = Invoke-Reviewer @{ RepoRoot = $notARepo }
    Assert-Equal $result.ExitCode 2 "gitリポジトリでない場所はexit 2にすること" $result.Output
    Assert-Contains $result.Output "見つかりません" "リポジトリ不在を明示すること"

    Write-Output ("review-staged-diff self-test passed: 7 cases, {0} assertions" -f $script:AssertionCount)
}
finally {
    Remove-TestSandbox
}
