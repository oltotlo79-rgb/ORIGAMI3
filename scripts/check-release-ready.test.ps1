# 本番リリース関門を本repoで別process実行し、stage 6が実際に接続されたことを検査する。

# stage 1のworkspace version readerは、production関数そのものを隔離fixtureでも検査する。
$ErrorActionPreference = "Stop"
$sut = Join-Path $PSScriptRoot "check-release-ready.ps1"
$snapshotSut = Join-Path $PSScriptRoot "get-roadmap-status.ps1"
$powershellExe = (Get-Process -Id $PID).Path
$global:LASTEXITCODE = 0
$snapshotLines = @(& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $snapshotSut -Format Json)
if ($LASTEXITCODE -ne 0 -or $snapshotLines.Count -ne 1) { throw "production snapshotを取得できません" }
$snapshot = [string]$snapshotLines[0] | ConvertFrom-Json
$previousErrorAction = $ErrorActionPreference
try {
    $ErrorActionPreference = "Continue"
    $global:LASTEXITCODE = 0
    $gateLines = @(& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $snapshotSut -Format Report -RequireComplete 2>&1)
    $gateExitCode = $LASTEXITCODE
}
finally {
    $ErrorActionPreference = $previousErrorAction
}
$global:LASTEXITCODE = 0
$outputLines = @(& $powershellExe -NoProfile -ExecutionPolicy Bypass -File $sut 2>&1)
$exitCode = $LASTEXITCODE
$output = $outputLines -join "`n"
$script:assertions = 0

function Assert-True {
    param([bool]$Condition, [string]$Message)
    $script:assertions++
    if (-not $Condition) { throw "[TEST NG] $Message`n$output" }
}

# 本番と別の正規表現を検査しても回帰を捕まえられないため、PowerShell ASTから
# productionの版数readerをそのまま取り出し、改行・一意性・書式を隔離fixtureで確認する。
$tokens = $null
$parseErrors = $null
$sutAst = [System.Management.Automation.Language.Parser]::ParseFile(
    $sut,
    [ref]$tokens,
    [ref]$parseErrors
)
if ($parseErrors.Count -ne 0) {
    throw "production release gateを構文解析できません: $($parseErrors[0].Message)"
}
$functionDefinitions = @($sutAst.FindAll({
    param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst]
}, $true))
foreach ($functionName in @("Read-Utf8Text", "Get-CargoWorkspaceVersion")) {
    $definitions = @($functionDefinitions | Where-Object { $_.Name -ceq $functionName })
    if ($definitions.Count -ne 1) {
        throw "production functionを1つに特定できません: $functionName count=$($definitions.Count)"
    }
    . ([scriptblock]::Create($definitions[0].Extent.Text))
}

$tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$tempRoot = [IO.Path]::GetFullPath((Join-Path $tempParent ("ori3-release-version-test-" + [Guid]::NewGuid().ToString("N"))))
[void][IO.Directory]::CreateDirectory($tempRoot)
try {
    function Invoke-VersionFixture {
        param([string]$Name, [string]$Content)

        $fixturePath = Join-Path $tempRoot $Name
        [IO.File]::WriteAllText($fixturePath, $Content, (New-Object Text.UTF8Encoding($false)))
        try {
            return [pscustomobject]@{
                Success = $true
                Value = Get-CargoWorkspaceVersion $fixturePath
                Error = $null
            }
        }
        catch {
            return [pscustomobject]@{
                Success = $false
                Value = $null
                Error = $_.Exception.Message
            }
        }
    }

    $lfVersion = Invoke-VersionFixture "lf.toml" "[workspace.package]`nversion = `"0.5.0`"`n[workspace.dependencies]`n"
    $crlfVersion = Invoke-VersionFixture "crlf.toml" "[workspace.package]`r`nversion = `"0.5.0`"`r`n[workspace.dependencies]`r`n"
    $duplicateVersion = Invoke-VersionFixture "duplicate.toml" "[workspace.package]`nversion = `"0.5.0`"`nversion = `"0.6.0`"`n[workspace.dependencies]`n"
    $unquotedVersion = Invoke-VersionFixture "unquoted.toml" "[workspace.package]`nversion = 0.5.0`n[workspace.dependencies]`n"

    Assert-True ($lfVersion.Success -and $lfVersion.Value -ceq "0.5.0") "LFのworkspace versionを読めません: value=$($lfVersion.Value) error=$($lfVersion.Error)"
    Assert-True ($crlfVersion.Success -and $crlfVersion.Value -ceq "0.5.0") "CRLFのworkspace versionを読めません: value=$($crlfVersion.Value) error=$($crlfVersion.Error)"
    Assert-True ((-not $duplicateVersion.Success) -and $duplicateVersion.Error -ceq "[workspace.package] の version を1つに特定できません") "重複workspace versionを拒否しませんでした: value=$($duplicateVersion.Value) error=$($duplicateVersion.Error)"
    Assert-True ((-not $unquotedVersion.Success) -and $unquotedVersion.Error -ceq "[workspace.package] の version を1つに特定できません") "unquoted workspace versionを拒否しませんでした: value=$($unquotedVersion.Value) error=$($unquotedVersion.Error)"
}
finally {
    $resolvedTempRoot = [IO.Path]::GetFullPath($tempRoot).TrimEnd([char[]]"\/")
    if ([IO.Path]::GetDirectoryName($resolvedTempRoot) -cne $tempParent -or
        [IO.Path]::GetFileName($resolvedTempRoot) -notmatch '^ori3-release-version-test-[0-9a-f]{32}$') {
        throw "unsafe version fixture cleanup refused: $resolvedTempRoot"
    }
    [IO.Directory]::Delete($resolvedTempRoot, $true)
}

if ([int]$snapshot.unchecked -gt 0) {
    Assert-True ($gateExitCode -eq 1) "production snapshot完了関門が未チェック$($snapshot.unchecked)件を拒否しませんでした (exit=$gateExitCode)"
    Assert-True ($exitCode -eq 1) "未チェック$($snapshot.unchecked)件がある本番入力をリリース可にしました (exit=$exitCode)"
}
else {
    Assert-True ($gateExitCode -eq 0) "未チェック0件のproduction snapshot完了関門が失敗しました (exit=$gateExitCode)"
}
Assert-True ($gateLines.Count -ge 2 -and [string]$gateLines[0] -ceq [string]$snapshot.report_snapshot_line -and [string]$gateLines[1] -ceq [string]$snapshot.report_progress_line) "production完了関門が報告用snapshot 2行をそのまま返していません"
Assert-True ($output -match '=== BEGIN \(6/6\) ロードマップ全件snapshotと証拠台帳 ===') "stage 6 BEGINがありません"
Assert-True ($output -match '=== END \(6/6\) ロードマップ全件snapshotと証拠台帳 ===') "stage 6 ENDがありません"
Assert-True ($output -match "ROADMAP_STATUS schema=1 .*scope=whole audited=$($snapshot.audited)/$($snapshot.total) partial=false") "全件のsnapshot表示がありません"
Assert-True ($output -match "checked=$($snapshot.checked) unchecked=$($snapshot.unchecked)") "現在snapshotと同じ完了・未完了の実測表示がありません"
Assert-True ($output -match [regex]::Escape([string]$snapshot.report_snapshot_line)) "報告へ貼るRoadmap-Snapshot行が関門出力にありません"
Assert-True ($output -match [regex]::Escape([string]$snapshot.report_progress_line)) "報告へ貼るRoadmap-Progress行が関門出力にありません"
if ([int]$snapshot.unchecked -gt 0) {
    Assert-True ($output -match "ロードマップ完了関門が終了コード1を返したためリリース可ではありません: unchecked=$($snapshot.unchecked)/$($snapshot.total)") "第6段が完了関門の非0を集約した診断がありません"
}
Assert-True (([regex]::Matches($output, '\[FRESH\] roadmap-links\.json|\[FRESH\] roadmap-links\.md|\[FRESH\] manual-acceptance\.md')).Count -eq 3) "証拠台帳3成果物のfreshness表示がありません"
Assert-True ($output -match 'RELEASE_STAGES planned=6 begun=6 ended=6') "全stage実行receiptがありません"

Write-Host "[TEST OK] check-release-ready: $script:assertions assertions; production_exit=$exitCode"
exit 0
