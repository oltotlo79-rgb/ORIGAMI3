[CmdletBinding()]
param(
    # 実装コミットより何日前の報告まで許容するか。既定値0は同じ日を要求する。
    [ValidateRange(0, 2147483647)]
    [int]$AllowedDelayDays = 0,

    # 検査用の複製で使う場合だけ指定する。通常は docs/報告記録.md を検査する。
    [string]$ReportPath
)

# ORIGAMI3 利用者への報告記録検査 (Windows PowerShell 5.1 / PowerShell 7 対応)
#
# 記録見出しの正本:
#   ## YYYY-MM-DD HH:mm — 概要

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$root = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot)).TrimEnd([char[]]"\/")
$reportPath = Join-Path $root "docs\報告記録.md"
if ($PSBoundParameters.ContainsKey("ReportPath")) {
    $reportPath = [System.IO.Path]::GetFullPath($ReportPath)
}
$headerPattern = [regex]::new(
    '^## (?<date>\d{4}-\d{2}-\d{2}) (?<time>(?:[01]\d|2[0-3]):[0-5]\d) — (?<title>\S(?:.*\S)?)$'
)
$script:formatProblems = New-Object System.Collections.Generic.List[string]
$script:missingProblems = New-Object System.Collections.Generic.List[string]

function Add-FormatProblem {
    param([string]$Message)

    $script:formatProblems.Add($Message)
}

function Add-MissingProblem {
    param([string]$Message)

    $script:missingProblems.Add($Message)
}

function Read-Utf8Text {
    param([string]$Path)

    $utf8 = [System.Text.UTF8Encoding]::new($false, $true)
    return [System.IO.File]::ReadAllText($Path, $utf8)
}

function Test-RecordHasBody {
    param(
        [string[]]$Lines,
        [int]$StartIndex,
        [int]$EndIndex
    )

    for ($index = $StartIndex + 1; $index -lt $EndIndex; $index++) {
        $trimmed = $Lines[$index].Trim()
        if ($trimmed.Length -eq 0) {
            continue
        }
        # 記録間の水平線だけでは、記録本文があることにならない。
        if ($trimmed -match '^(?:---|\*\*\*|___)$') {
            continue
        }
        return $true
    }
    return $false
}

if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
    Add-MissingProblem "docs/報告記録.md がありません。"
}
else {
    try {
        $content = Read-Utf8Text $reportPath
        $lines = [regex]::Split($content, "\r\n|\n|\r")
        $records = New-Object System.Collections.Generic.List[object]

        for ($lineIndex = 0; $lineIndex -lt $lines.Count; $lineIndex++) {
            $line = $lines[$lineIndex]
            if (-not $line.StartsWith("## ", [System.StringComparison]::Ordinal)) {
                continue
            }

            $match = $headerPattern.Match($line)
            if (-not $match.Success) {
                Add-FormatProblem "$($lineIndex + 1)行目の見出しが書式に合いません: $line"
                continue
            }

            $dateText = $match.Groups["date"].Value
            $recordDate = [datetime]::MinValue
            if (-not [datetime]::TryParseExact(
                $dateText,
                "yyyy-MM-dd",
                [System.Globalization.CultureInfo]::InvariantCulture,
                [System.Globalization.DateTimeStyles]::None,
                [ref]$recordDate
            )) {
                Add-FormatProblem "$($lineIndex + 1)行目の日付が実在しません: $dateText"
                continue
            }

            $records.Add([PSCustomObject]@{
                LineIndex = $lineIndex
                Date      = $recordDate.Date
                Header    = $line
            })
        }

        if ($records.Count -eq 0) {
            Add-MissingProblem "報告記録が1件もありません。"
        }
        else {
            for ($recordIndex = 0; $recordIndex -lt $records.Count; $recordIndex++) {
                $record = $records[$recordIndex]
                $nextLineIndex = $lines.Count
                if ($recordIndex + 1 -lt $records.Count) {
                    $nextLineIndex = $records[$recordIndex + 1].LineIndex
                }
                if (-not (Test-RecordHasBody $lines $record.LineIndex $nextLineIndex)) {
                    Add-FormatProblem "$($record.LineIndex + 1)行目の記録は見出しだけで、本文がありません: $($record.Header)"
                }
            }

            for ($recordIndex = 1; $recordIndex -lt $records.Count; $recordIndex++) {
                $newerRecord = $records[$recordIndex - 1]
                $olderRecord = $records[$recordIndex]
                if ($olderRecord.Date -gt $newerRecord.Date) {
                    Add-FormatProblem "日付が降順ではありません: $($newerRecord.Date.ToString('yyyy-MM-dd')) の後に $($olderRecord.Date.ToString('yyyy-MM-dd')) があります。"
                }
            }

            $global:LASTEXITCODE = 0
            $latestSourceCommitDateLines = @(& git -C $root log -1 --format=%cs -- apps crates)
            $gitStatus = $LASTEXITCODE
            $latestSourceCommitDateText = if ($latestSourceCommitDateLines.Count -gt 0) {
                ([string]$latestSourceCommitDateLines[0]).Trim()
            }
            else {
                ""
            }
            if ($gitStatus -ne 0) {
                Add-FormatProblem "apps/ または crates/ の最新コミット日を取得できませんでした (git の終了コード: $gitStatus)。"
            }
            elseif ($latestSourceCommitDateText.Length -eq 0) {
                Write-Host "[OK] apps/ または crates/ を変更したコミットはまだありません。"
            }
            else {
                $latestSourceCommitDate = [datetime]::MinValue
                if (-not [datetime]::TryParseExact(
                    $latestSourceCommitDateText,
                    "yyyy-MM-dd",
                    [System.Globalization.CultureInfo]::InvariantCulture,
                    [System.Globalization.DateTimeStyles]::None,
                    [ref]$latestSourceCommitDate
                )) {
                    Add-FormatProblem "git が返した最新コミット日を読めません: $latestSourceCommitDateText"
                }
                else {
                    $latestReportDate = $records[0].Date
                    $minimumReportDate = $latestSourceCommitDate.Date.AddDays(-$AllowedDelayDays)
                    if ($latestReportDate -lt $minimumReportDate) {
                        Add-MissingProblem "最新の報告日 $($latestReportDate.ToString('yyyy-MM-dd')) が、apps/ または crates/ の最新コミット日 $($latestSourceCommitDate.ToString('yyyy-MM-dd')) より $AllowedDelayDays 日を超えて古いです。"
                    }
                    else {
                        Write-Host "[OK] 最新の報告日 $($latestReportDate.ToString('yyyy-MM-dd')) は、apps/ または crates/ の最新コミット日 $($latestSourceCommitDate.ToString('yyyy-MM-dd')) に対して許容範囲内です。"
                    }
                }
            }
        }
    }
    catch {
        Add-FormatProblem "docs/報告記録.md を検査できませんでした: $($_.Exception.Message)"
    }
}

foreach ($problem in $script:formatProblems) {
    Write-Host "[NG] $problem" -ForegroundColor Red
}
foreach ($problem in $script:missingProblems) {
    Write-Host "[NG] $problem" -ForegroundColor Red
}

if ($script:formatProblems.Count -gt 0) {
    exit 2
}
if ($script:missingProblems.Count -gt 0) {
    exit 1
}

Write-Host "[OK] 利用者への報告記録の検査に合格しました。" -ForegroundColor Green
exit 0
