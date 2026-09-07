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
foreach ($functionName in @(
    "Read-Utf8Text",
    "Get-RelativePath",
    "Get-JstDate",
    "Get-CargoWorkspaceVersion",
    "Get-FileSha256",
    "Get-Utf8TextSha256",
    "Get-ManualReceiptFileEntries",
    "Compare-ManualReceiptFileGroup",
    "Get-ManualBuildReceiptStatus"
)) {
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

# receiptは入力内容とPDFをhashで結ぶ。JSONのLF/CRLFやhost timezoneに依存せず、
# 欠落・入力変更・PDF変更をそれぞれfail-closedで拒否する。
$receiptRoot = [IO.Path]::GetFullPath((Join-Path $tempParent ("ori3-release-receipt-test-" + [Guid]::NewGuid().ToString("N"))))
$receiptHelp = Join-Path $receiptRoot "apps\desktop\src\help"
$receiptAssets = Join-Path $receiptRoot "docs\manual\assets"
$receiptPdf = Join-Path $receiptRoot "docs\manual\manual.pdf"
$receiptJsonPath = Join-Path $receiptRoot "docs\manual\manual-build-receipt.json"
$receiptPackage = Join-Path $receiptRoot "apps\desktop\package.json"
[void][IO.Directory]::CreateDirectory($receiptHelp)
[void][IO.Directory]::CreateDirectory($receiptAssets)
$utf8NoBom = New-Object Text.UTF8Encoding($false)
try {
    [IO.File]::WriteAllText((Join-Path $receiptHelp "index.ts"), "export const help = 1;`n", $utf8NoBom)
    [IO.File]::WriteAllText((Join-Path $receiptAssets "screen.txt"), "screen`n", $utf8NoBom)
    [IO.File]::WriteAllText($receiptPdf, "%PDF-1.4 fixture`n", $utf8NoBom)
    [IO.File]::WriteAllText($receiptPackage, '{"version":"0.5.0"}', $utf8NoBom)
    $root = $receiptRoot

    function New-ReceiptFixtureJson {
        $version = "0.5.0"
        $fixtureReceipt = [ordered]@{
            schema = 1
            generated_at_jst = "2026-09-07T12:00:00.0000000+09:00"
            inputs = [ordered]@{
                help = @(Get-ManualReceiptFileEntries -Root $receiptRoot -Directory $receiptHelp)
                assets = @(Get-ManualReceiptFileEntries -Root $receiptRoot -Directory $receiptAssets)
                package_version = [ordered]@{ value = $version; sha256 = Get-Utf8TextSha256 -Text $version }
            }
            output = [ordered]@{
                path = "docs/manual/manual.pdf"
                sha256 = Get-FileSha256 -Path $receiptPdf
            }
        }
        return ($fixtureReceipt | ConvertTo-Json -Depth 8)
    }

    function Get-ReceiptFixtureStatus {
        return Get-ManualBuildReceiptStatus -Root $receiptRoot -ReceiptPath $receiptJsonPath `
            -PdfPath $receiptPdf -HelpPath $receiptHelp -AssetsPath $receiptAssets -PackageJsonPath $receiptPackage
    }

    $receiptJson = New-ReceiptFixtureJson
    [IO.File]::WriteAllText($receiptJsonPath, $receiptJson.Replace("`r`n", "`n") + "`n", $utf8NoBom)
    $lfReceipt = Get-ReceiptFixtureStatus
    Assert-True ($lfReceipt.IsFresh) "LF receiptの正しい入力/PDFを拒否しました: $($lfReceipt.Reasons -join '; ')"

    [IO.File]::WriteAllText($receiptJsonPath, $receiptJson.Replace("`r`n", "`n").Replace("`n", "`r`n") + "`r`n", $utf8NoBom)
    $crlfReceipt = Get-ReceiptFixtureStatus
    Assert-True ($crlfReceipt.IsFresh) "CRLF receiptの正しい入力/PDFを拒否しました: $($crlfReceipt.Reasons -join '; ')"

    [IO.File]::WriteAllText((Join-Path $receiptHelp "index.ts"), "export const help = 2;`n", $utf8NoBom)
    $changedHelp = Get-ReceiptFixtureStatus
    Assert-True ((-not $changedHelp.IsFresh) -and ($changedHelp.Reasons -match '^help: hash mismatch: apps/desktop/src/help/index\.ts$')) "help hash不一致を項目名つきで拒否しませんでした: $($changedHelp.Reasons -join '; ')"
    [IO.File]::WriteAllText((Join-Path $receiptHelp "index.ts"), "export const help = 1;`n", $utf8NoBom)

    [IO.File]::WriteAllText((Join-Path $receiptAssets "new-screen.txt"), "new`n", $utf8NoBom)
    $extraAsset = Get-ReceiptFixtureStatus
    Assert-True ((-not $extraAsset.IsFresh) -and ($extraAsset.Reasons -match '^assets: unexpected input: docs/manual/assets/new-screen\.txt$')) "assets追加を項目名つきで拒否しませんでした: $($extraAsset.Reasons -join '; ')"
    Remove-Item -LiteralPath (Join-Path $receiptAssets "new-screen.txt") -Force

    [IO.File]::AppendAllText($receiptPdf, "changed`n", $utf8NoBom)
    $changedPdf = Get-ReceiptFixtureStatus
    Assert-True ((-not $changedPdf.IsFresh) -and ($changedPdf.Reasons -match '^pdf: hash mismatch: docs/manual/manual\.pdf$')) "PDF hash不一致を項目名つきで拒否しませんでした: $($changedPdf.Reasons -join '; ')"

    Remove-Item -LiteralPath $receiptJsonPath -Force
    $missingReceipt = Get-ReceiptFixtureStatus
    Assert-True ((-not $missingReceipt.IsFresh) -and ($missingReceipt.Reasons -match '^receipt missing: docs/manual/manual-build-receipt\.json$')) "receipt欠落を拒否しませんでした: $($missingReceipt.Reasons -join '; ')"

    $sameInstantUtc = [DateTimeOffset]::Parse("2026-09-06T15:30:00+00:00")
    $sameInstantJst = [DateTimeOffset]::Parse("2026-09-07T00:30:00+09:00")
    Assert-True ((Get-JstDate -Instant $sameInstantUtc) -eq (Get-JstDate -Instant $sameInstantJst)) "UTC/JST表現でrelease日が変わりました"
    Assert-True ((Get-JstDate -Instant $sameInstantUtc).ToString('yyyy-MM-dd') -ceq '2026-09-07') "JST日付への変換が違います"
}
finally {
    $resolvedReceiptRoot = [IO.Path]::GetFullPath($receiptRoot).TrimEnd([char[]]"\/")
    if ([IO.Path]::GetDirectoryName($resolvedReceiptRoot) -cne $tempParent -or
        [IO.Path]::GetFileName($resolvedReceiptRoot) -notmatch '^ori3-release-receipt-test-[0-9a-f]{32}$') {
        throw "unsafe receipt fixture cleanup refused: $resolvedReceiptRoot"
    }
    Remove-Item -LiteralPath $resolvedReceiptRoot -Recurse -Force
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
Assert-True ($output -match 'MANUAL_FRESHNESS stage=2 basis=receipt fresh=False') "検査2のreceipt判定表示がありません"
Assert-True ($output -match 'MANUAL_FRESHNESS stage=4 basis=receipt fresh=False') "検査4のreceipt判定表示がありません"
Assert-True ($output -match 'receipt missing: docs/manual/manual-build-receipt\.json') "本体のreceipt欠落理由がありません"
Assert-True ($output -match 'RELEASE_STAGES planned=6 begun=6 ended=6') "全stage実行receiptがありません"

Write-Host "[TEST OK] check-release-ready: $script:assertions assertions; production_exit=$exitCode"
exit 0
