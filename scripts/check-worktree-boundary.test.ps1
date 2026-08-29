[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$scriptPath = Join-Path $PSScriptRoot "check-worktree-boundary.ps1"
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-check-worktree-boundary-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [IO.Path]::GetFullPath((Join-Path $tempBase $sandboxName))
$script:AssertionCount = 0
$script:KnownReceiptPaths = New-Object System.Collections.Generic.List[string]

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

function ConvertTo-ProcessArgumentString {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$ArgumentValues)

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

# 使い捨てのgitリポジトリをサンドボックス内に作る。本体リポジトリには一切触れない。
# 「開始時点で既に汚れている」状態を再現するため、commit後にtracked.txtを書き換え、
# untracked.txtを新規追加した状態のまま返す（初期dirty状態）。
function New-DisposableGitRepo {
    param([Parameter(Mandatory = $true)][string]$Name)

    $repoRoot = Join-Path $sandboxRoot $Name
    [void][IO.Directory]::CreateDirectory($repoRoot)
    [void](Invoke-GitIn $repoRoot @("init", "--quiet"))
    [void](Invoke-GitIn $repoRoot @("config", "user.email", "test@example.com"))
    [void](Invoke-GitIn $repoRoot @("config", "user.name", "Test User"))
    [IO.File]::WriteAllText((Join-Path $repoRoot "tracked.txt"), "original content`n", [Text.UTF8Encoding]::new($false))
    [void][IO.Directory]::CreateDirectory((Join-Path $repoRoot "scripts"))
    [IO.File]::WriteAllText((Join-Path $repoRoot "scripts/placeholder.txt"), "placeholder`n", [Text.UTF8Encoding]::new($false))
    [void](Invoke-GitIn $repoRoot @("add", "tracked.txt", "scripts/placeholder.txt"))
    [void](Invoke-GitIn $repoRoot @("commit", "--quiet", "-m", "initial"))

    # ここから「開始時点で既に汚れている」状態を作る(他担当の未コミット作業を模す)。
    [IO.File]::WriteAllText((Join-Path $repoRoot "tracked.txt"), "pre-existing uncommitted edit`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $repoRoot "pre-existing-untracked.txt"), "pre-existing untracked file`n", [Text.UTF8Encoding]::new($false))
    return $repoRoot
}

function Invoke-Checker {
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
        if ($value -is [switch] -or $value -is [bool]) {
            continue
        }
        if ($value -is [array]) {
            foreach ($item in $value) { $argValues.Add([string]$item) }
        }
        else {
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
        (-not [regex]::IsMatch($leaf, '^ori3-check-worktree-boundary-test-[0-9a-f]{32}$', [Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
        throw "安全でない一時領域の削除を拒否しました: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction SilentlyContinue
    foreach ($receipt in @(Get-ChildItem -LiteralPath ([IO.Path]::GetTempPath()) -Filter "ori3-worktree-boundary-receipt-*.json" -ErrorAction SilentlyContinue)) {
        if ($script:KnownReceiptPaths -contains $receipt.FullName) {
            Remove-Item -LiteralPath $receipt.FullName -Force -ErrorAction SilentlyContinue
        }
    }
}

if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
    throw "検査本体が見つかりません: $scriptPath"
}

[void][IO.Directory]::CreateDirectory($sandboxRoot)
try {
    Write-Output "[1/9] -Init は -AllowedPaths 無しでは失敗する(不合格例)"
    $repo1 = New-DisposableGitRepo "no-allowed-paths"
    $receipt1 = Join-Path $sandboxRoot "receipt1.json"
    $result = Invoke-Checker @{ RepoRoot = $repo1; ReceiptPath = $receipt1; Init = $true }
    Assert-Equal $result.ExitCode 2 "AllowedPaths無しの-Initを合格扱いしてはならない" $result.Output
    Assert-Contains $result.Output "AllowedPaths" "AllowedPaths必須である旨を明示すること"

    Write-Output "[2/9] -Init は開始時点の既存差分(2件)を基準として記録する"
    $repo2 = New-DisposableGitRepo "init-records-baseline"
    $receipt2 = Join-Path $sandboxRoot "receipt2.json"
    $script:KnownReceiptPaths.Add($receipt2)
    $result = Invoke-Checker @{ RepoRoot = $repo2; ReceiptPath = $receipt2; Init = $true; AllowedPaths = @("scripts/*") }
    Assert-Equal $result.ExitCode 0 "正常なInitは合格すること" $result.Output
    Assert-Contains $result.Output "既存差分 2 件" "開始時点の既存差分2件(tracked.txtとpre-existing-untracked.txt)を記録すること"
    Assert-True (Test-Path -LiteralPath $receipt2 -PathType Leaf) "receiptファイルを作成すること"

    Write-Output "[3/9] 変更が無ければ -Verify は合格する"
    $result = Invoke-Checker @{ RepoRoot = $repo2; ReceiptPath = $receipt2; Verify = $true }
    Assert-Equal $result.ExitCode 0 "変更が無ければ合格すること" $result.Output
    Assert-Contains $result.Output "[OK]" "合格を明示すること"

    Write-Output "[4/9] 許可pattern内の新規pathは -Verify に合格する"
    [IO.File]::WriteAllText((Join-Path $repo2 "scripts/new-tool.ps1"), "new tool`n", [Text.UTF8Encoding]::new($false))
    $result = Invoke-Checker @{ RepoRoot = $repo2; ReceiptPath = $receipt2; Verify = $true }
    Assert-Equal $result.ExitCode 0 "許可pattern内の新規pathは合格扱いにすること" $result.Output

    Write-Output "[5/9] 許可pattern外の新規pathは -Verify に不合格になる(不合格例)"
    [IO.File]::WriteAllText((Join-Path $repo2 "outside-scope.txt"), "should not be touched here`n", [Text.UTF8Encoding]::new($false))
    $result = Invoke-Checker @{ RepoRoot = $repo2; ReceiptPath = $receipt2; Verify = $true }
    Assert-Equal $result.ExitCode 1 "許可範囲外のpathを合格扱いしてはならない" $result.Output
    Assert-Contains $result.Output "許可範囲外のpathを変更しました: outside-scope.txt" "許可範囲外のpathを名指しすること"
    Remove-Item -LiteralPath (Join-Path $repo2 "outside-scope.txt") -Force

    Write-Output "[6/9] 開始時点で既に汚れていたファイルをさらに変更すると -Verify は不合格になる(不合格例)"
    [IO.File]::WriteAllText((Join-Path $repo2 "tracked.txt"), "pre-existing uncommitted edit -- further changed by an agent`n", [Text.UTF8Encoding]::new($false))
    $result = Invoke-Checker @{ RepoRoot = $repo2; ReceiptPath = $receipt2; Verify = $true }
    Assert-Equal $result.ExitCode 1 "既存差分のさらなる変更を合格扱いしてはならない" $result.Output
    Assert-Contains $result.Output "既存差分を変更しました: tracked.txt" "既存差分の変更を名指しすること"

    Write-Output "[7/9] receiptが無い状態での -Verify は不合格になる(不合格例)"
    $repo3 = New-DisposableGitRepo "verify-without-receipt"
    $receipt3 = Join-Path $sandboxRoot "receipt3-never-created.json"
    $result = Invoke-Checker @{ RepoRoot = $repo3; ReceiptPath = $receipt3; Verify = $true }
    Assert-Equal $result.ExitCode 1 "receiptが無い-Verifyを合格扱いしてはならない" $result.Output
    Assert-Contains $result.Output "receiptが見つかりません" "receipt不在を明示すること"

    Write-Output "[8/9] 別リポジトリのreceiptでの -Verify は不合格になる(不合格例)"
    $repo4 = New-DisposableGitRepo "different-repo"
    $result = Invoke-Checker @{ RepoRoot = $repo4; ReceiptPath = $receipt2; Verify = $true }
    Assert-Equal $result.ExitCode 1 "別リポジトリのreceiptを合格扱いしてはならない" $result.Output
    Assert-Contains $result.Output "別のリポジトリ" "リポジトリ不一致を明示すること"

    Write-Output "[9/9] gitリポジトリでない場所を指定すると失敗する(不合格例)"
    $notARepo = Join-Path $sandboxRoot "not-a-repo"
    [void][IO.Directory]::CreateDirectory($notARepo)
    $result = Invoke-Checker @{ RepoRoot = $notARepo; Verify = $true }
    Assert-Equal $result.ExitCode 2 "gitリポジトリでない場所はexit 2にすること" $result.Output
    Assert-Contains $result.Output "見つかりません" "リポジトリ不在を明示すること"

    Write-Output ("check-worktree-boundary self-test passed: 9 cases, {0} assertions" -f $script:AssertionCount)
}
finally {
    Remove-TestSandbox
}
