# ORIGAMI3取扱説明書を、アプリ内ヘルプの共通内容源から生成する。
# Windows PowerShell 5.1 / PowerShell 7 のどちらでも実行できる。

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$desktop = Join-Path $root "apps\desktop"
$intermediateDir = Join-Path $root "target\manual"
$json = Join-Path $intermediateDir "help-content.json"
$manualDir = Join-Path $root "docs\manual"
$assetsDir = Join-Path $manualDir "assets"
$pdf = Join-Path $manualDir "ORIGAMI3取扱説明書.pdf"

function Assert-NativeSuccess {
    param([string]$Step)
    if ($LASTEXITCODE -ne 0) {
        throw "$Step が失敗しました(終了コード: $LASTEXITCODE)"
    }
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

Write-Host "[OK] $pdf ($pageCount ページ / $($bytes.Length) bytes)" -ForegroundColor Green
