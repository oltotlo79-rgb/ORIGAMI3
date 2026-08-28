[CmdletBinding()]
param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [AllowEmptyCollection()]
    [string[]]$Files = @()
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

function Get-NormalizedGitPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $normalized = $Path.Replace("\", "/")
    while ($normalized.StartsWith("./", [System.StringComparison]::Ordinal)) {
        $normalized = $normalized.Substring(2)
    }
    return $normalized
}

function Get-AddedLines {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $global:LASTEXITCODE = 0
    $diff = @(& git -C $RepositoryRoot -c core.excludesFile= diff --cached --no-color --no-ext-diff --no-textconv --unified=0 -- $Path)
    $status = $LASTEXITCODE
    if ($status -ne 0) {
        throw "git diff --cached に失敗しました（終了コード: $status、対象: $Path）"
    }

    $added = New-Object System.Collections.Generic.List[string]
    foreach ($line in $diff) {
        if ($line.StartsWith("+", [System.StringComparison]::Ordinal) -and
            -not $line.StartsWith("+++", [System.StringComparison]::Ordinal)) {
            $added.Add($line.Substring(1))
        }
    }
    return $added.ToArray()
}

try {
    $repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\..\.."))
    $rustFiles = New-Object System.Collections.Generic.List[string]
    $seenFiles = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($file in @($Files)) {
        if ([string]::IsNullOrWhiteSpace($file)) {
            continue
        }
        $normalized = Get-NormalizedGitPath $file
        if ($normalized.EndsWith(".rs", [System.StringComparison]::OrdinalIgnoreCase) -and $seenFiles.Add($normalized)) {
            $rustFiles.Add($normalized)
        }
    }

    $attributeCount = 0
    $violations = New-Object System.Collections.Generic.List[object]
    $attributePattern = [regex]::new(
        '(?ms)^[ \t]*#\s*\[\s*ignore\b(?<body>[^\]]*)\]',
        [Text.RegularExpressions.RegexOptions]::None
    )
    $reasonPattern = [regex]::new(
        '^\s*=\s*"(?<reason>(?:\\.|[^"\\])*)"\s*$',
        [Text.RegularExpressions.RegexOptions]::Singleline
    )

    foreach ($rustFile in $rustFiles) {
        $addedText = (@(Get-AddedLines -RepositoryRoot $repositoryRoot -Path $rustFile) -join "`n") + "`n"
        foreach ($attribute in $attributePattern.Matches($addedText)) {
            $attributeCount += 1
            $body = $attribute.Groups["body"].Value
            $reasonMatch = $reasonPattern.Match($body)
            if (-not $reasonMatch.Success) {
                $violations.Add([pscustomobject]@{
                    Source = $rustFile
                    Attribute = ($attribute.Value -replace "\s+", " ").Trim()
                    Reason = "理由文のない #[ignore] です"
                })
                continue
            }

            $reason = $reasonMatch.Groups["reason"].Value
            if ($reason -notmatch '[0-9]') {
                $violations.Add([pscustomobject]@{
                    Source = $rustFile
                    Attribute = ($attribute.Value -replace "\s+", " ").Trim()
                    Reason = "理由文に未達の実測値を示す数字がありません"
                })
            }
        }
    }

    if ($violations.Count -gt 0) {
        Write-Output "[NG] #[ignore] 理由検査: 違反が $($violations.Count) 件あります。"
        foreach ($violation in $violations) {
            Write-Output "  対象: $($violation.Source)"
            Write-Output "  属性: $($violation.Attribute)"
            Write-Output "  理由: $($violation.Reason)"
        }
        Write-Output "  #[ignore = \"未達の実測値 ...。数字で示した条件を満たしたら外す\"] の形で記録してください。"
        exit 1
    }

    Write-Output "[OK] #[ignore] 理由検査: 対象Rustファイル $($rustFiles.Count) 件、追加属性 $attributeCount 件、違反 0件"
    exit 0
}
catch {
    Write-Output "[NG] #[ignore] 理由検査を完了できませんでした: $($_.Exception.Message)"
    exit 2
}
