# ORIGAMI3取扱説明書を、アプリ内ヘルプの共通内容源から生成する。
# Windows PowerShell 5.1 / PowerShell 7 のどちらでも実行できる。

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$desktop = Join-Path $root "apps\desktop"
$intermediateDir = Join-Path $root "target\manual"
$json = Join-Path $intermediateDir "help-content.json"
$manualDir = Join-Path $root "docs\manual"
$assetsDir = Join-Path $manualDir "assets"
$helpDir = Join-Path $desktop "src\help"
$packageJson = Join-Path $desktop "package.json"
$pdf = Join-Path $manualDir "ORIGAMI3取扱説明書.pdf"
$receiptPath = Join-Path $manualDir "manual-build-receipt.json"

function Assert-NativeSuccess {
    param([string]$Step)
    if ($LASTEXITCODE -ne 0) {
        throw "$Step が失敗しました(終了コード: $LASTEXITCODE)"
    }
}

function Get-ManualFileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return (($sha.ComputeHash($stream) | ForEach-Object { $_.ToString('x2') }) -join '')
    }
    finally {
        $sha.Dispose()
        $stream.Dispose()
    }
}

function Get-ManualTextSha256 {
    param([Parameter(Mandatory = $true)][string]$Text)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return (($sha.ComputeHash([Text.UTF8Encoding]::new($false).GetBytes($Text)) |
            ForEach-Object { $_.ToString('x2') }) -join '')
    }
    finally { $sha.Dispose() }
}

function Get-ManualReceiptEntries {
    param([Parameter(Mandatory = $true)][string]$Directory)
    if (-not (Test-Path -LiteralPath $Directory -PathType Container)) {
        throw "receipt入力directoryがありません: $Directory"
    }
    $rootPrefix = [IO.Path]::GetFullPath($root).TrimEnd([char[]]'\/') + [IO.Path]::DirectorySeparatorChar
    $entries = foreach ($file in @(Get-ChildItem -LiteralPath $Directory -Recurse -File -Force)) {
        if (($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "receipt入力にreparse pointは使えません: $($file.FullName)"
        }
        $fullPath = [IO.Path]::GetFullPath($file.FullName)
        if (-not $fullPath.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw "receipt入力がrepository外です: $fullPath"
        }
        [ordered]@{
            path = $fullPath.Substring($rootPrefix.Length).Replace('\', '/')
            sha256 = Get-ManualFileSha256 -Path $fullPath
        }
    }
    return @($entries | Sort-Object path)
}

New-Item -ItemType Directory -Path $intermediateDir -Force | Out-Null
New-Item -ItemType Directory -Path $assetsDir -Force | Out-Null

Write-Host "[1/2] ヘルプ内容をJSONへ書き出します" -ForegroundColor Cyan
Push-Location $desktop
try {
    & npm.cmd run export-help -- $json
    Assert-NativeSuccess "ヘルプ内容のJSON書き出し"
}
finally {
    Pop-Location
}

Write-Host "[2/2] A4取扱説明書PDFを組版します" -ForegroundColor Cyan
Push-Location $root
try {
    & cargo run -p ori3-export --bin manual_pdf -- $json $pdf $assetsDir
    Assert-NativeSuccess "取扱説明書PDFの生成"
}
finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $pdf -PathType Leaf)) {
    throw "PDFが生成されませんでした: $pdf"
}
$bytes = [System.IO.File]::ReadAllBytes($pdf)
if ($bytes.Length -lt 5 -or [System.Text.Encoding]::ASCII.GetString($bytes, 0, 5) -ne "%PDF-") {
    throw "生成物がPDFではありません: $pdf"
}
$pdfText = [System.Text.Encoding]::ASCII.GetString($bytes)
$pageCount = ([regex]::Matches($pdfText, "/MediaBox")).Count
if ($pageCount -lt 3) {
    throw "PDFのページ数が不正です: $pageCount"
}

# 更新時刻はcheckout順で変わるため、生成時の入力内容とPDFをhash receiptへ固定する。
$packageVersion = [string](([IO.File]::ReadAllText($packageJson, [Text.UTF8Encoding]::new($false))) | ConvertFrom-Json).version
if ([string]::IsNullOrWhiteSpace($packageVersion)) {
    throw "apps/desktop/package.json のversionを読めません"
}
$receipt = [ordered]@{
    schema = 1
    generated_at_jst = [DateTimeOffset]::UtcNow.ToOffset([TimeSpan]::FromHours(9)).ToString('yyyy-MM-ddTHH:mm:ss.fffffffzzz', [Globalization.CultureInfo]::InvariantCulture)
    inputs = [ordered]@{
        help = @(Get-ManualReceiptEntries -Directory $helpDir)
        assets = @(Get-ManualReceiptEntries -Directory $assetsDir)
        package_version = [ordered]@{
            value = $packageVersion
            sha256 = Get-ManualTextSha256 -Text $packageVersion
        }
    }
    output = [ordered]@{
        path = 'docs/manual/ORIGAMI3取扱説明書.pdf'
        sha256 = Get-ManualFileSha256 -Path $pdf
    }
}
$receiptJson = $receipt | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText($receiptPath, $receiptJson + "`n", [Text.UTF8Encoding]::new($false))

Write-Host "[OK] $pdf ($pageCount ページ / $($bytes.Length) bytes)" -ForegroundColor Green
Write-Host "[OK] $receiptPath (schema=1 / help=$($receipt.inputs.help.Count) / assets=$($receipt.inputs.assets.Count))" -ForegroundColor Green
