[CmdletBinding()]
param(
    [string]$RepositoryRoot = "",
    [string]$DefinitionPath = "",
    [switch]$FailOnDecrease
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

function Test-HasProperty {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string]$Name
    )

    return $null -ne $Object.PSObject.Properties[$Name]
}

function Get-RequiredString {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Context
    )

    if (-not (Test-HasProperty -Object $Object -Name $Name)) {
        throw "$Context に必須項目 '$Name' がありません。"
    }
    $value = [string]$Object.$Name
    if ([string]::IsNullOrWhiteSpace($value)) {
        throw "$Context の '$Name' が空です。"
    }
    return $value
}

function Get-RequiredNonNegativeInteger {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Context
    )

    if (-not (Test-HasProperty -Object $Object -Name $Name)) {
        throw "$Context に必須項目 '$Name' がありません。"
    }
    $value = 0
    if (-not [int]::TryParse([string]$Object.$Name, [ref]$value) -or $value -lt 0) {
        throw "$Context の '$Name' は0以上の整数でなければなりません。"
    }
    return $value
}

function ConvertTo-RepositoryPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $normalized = $Path.Replace("\", "/")
    while ($normalized.StartsWith("./", [StringComparison]::Ordinal)) {
        $normalized = $normalized.Substring(2)
    }
    return $normalized.TrimStart([char[]]"/")
}

function Get-SafeRepositoryPath {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$RelativePath,
        [Parameter(Mandatory = $true)][string]$Context
    )

    $normalized = ConvertTo-RepositoryPath $RelativePath
    if ([string]::IsNullOrWhiteSpace($normalized) -or [IO.Path]::IsPathRooted($RelativePath)) {
        throw "$Context のパスはリポジトリ相対でなければなりません: $RelativePath"
    }
    if (@($normalized -split '/') -contains "..") {
        throw "$Context のパスに '..' は使えません: $RelativePath"
    }

    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd([char[]]"\/")
    $candidate = [IO.Path]::GetFullPath((Join-Path $rootFull $normalized.Replace("/", [IO.Path]::DirectorySeparatorChar)))
    $prefix = $rootFull + [IO.Path]::DirectorySeparatorChar
    if (-not $candidate.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Context のパスがリポジトリ外を指しています: $RelativePath"
    }
    return $candidate
}

function New-CheckedRegex {
    param(
        [Parameter(Mandatory = $true)][string]$Pattern,
        [Parameter(Mandatory = $true)][string]$Context
    )

    try {
        return [regex]::new(
            $Pattern,
            [Text.RegularExpressions.RegexOptions]::Multiline -bor
                [Text.RegularExpressions.RegexOptions]::CultureInvariant,
            [TimeSpan]::FromSeconds(2)
        )
    }
    catch {
        throw "$Context の正規表現が不正です: $($_.Exception.Message)"
    }
}

function Hide-RustCommentsAndStrings {
    param([Parameter(Mandatory = $true)][string]$Text)

    if ($Text.Length -eq 0) {
        return $Text
    }

    $source = $Text.ToCharArray()
    $masked = $Text.ToCharArray()
    $length = $source.Length
    $index = 0

    while ($index -lt $length) {
        # 行コメント。doc commentも同じ扱いにする。
        if (($source[$index] -eq '/') -and ($index + 1 -lt $length) -and ($source[$index + 1] -eq '/')) {
            while ($index -lt $length -and $source[$index] -ne "`r" -and $source[$index] -ne "`n") {
                $masked[$index] = [char]32
                $index += 1
            }
            continue
        }

        # Rustで入れ子にできるブロックコメント。
        if (($source[$index] -eq '/') -and ($index + 1 -lt $length) -and ($source[$index + 1] -eq '*')) {
            $depth = 1
            $masked[$index] = [char]32
            $masked[$index + 1] = [char]32
            $index += 2
            while ($index -lt $length -and $depth -gt 0) {
                if (($source[$index] -eq '/') -and ($index + 1 -lt $length) -and ($source[$index + 1] -eq '*')) {
                    $masked[$index] = [char]32
                    $masked[$index + 1] = [char]32
                    $depth += 1
                    $index += 2
                    continue
                }
                if (($source[$index] -eq '*') -and ($index + 1 -lt $length) -and ($source[$index + 1] -eq '/')) {
                    $masked[$index] = [char]32
                    $masked[$index + 1] = [char]32
                    $depth -= 1
                    $index += 2
                    continue
                }
                if ($source[$index] -ne "`r" -and $source[$index] -ne "`n") {
                    $masked[$index] = [char]32
                }
                $index += 1
            }
            continue
        }

        # r"..."、r#"..."#、br#"..."# を行位置を保って隠す。
        $rawStart = -1
        $rIndex = -1
        if ($source[$index] -eq 'r') {
            $rawStart = $index
            $rIndex = $index
        }
        elseif (($source[$index] -eq 'b') -and ($index + 1 -lt $length) -and ($source[$index + 1] -eq 'r')) {
            $rawStart = $index
            $rIndex = $index + 1
        }
        if ($rIndex -ge 0) {
            $delimiterIndex = $rIndex + 1
            $hashCount = 0
            while ($delimiterIndex -lt $length -and $source[$delimiterIndex] -eq '#') {
                $hashCount += 1
                $delimiterIndex += 1
            }
            if ($delimiterIndex -lt $length -and $source[$delimiterIndex] -eq '"') {
                $closing = '"' + ('#' * $hashCount)
                $closingIndex = $Text.IndexOf($closing, $delimiterIndex + 1, [StringComparison]::Ordinal)
                if ($closingIndex -lt 0) {
                    $rawEnd = $length - 1
                }
                else {
                    $rawEnd = $closingIndex + $closing.Length - 1
                }
                for ($cursor = $rawStart; $cursor -le $rawEnd; $cursor += 1) {
                    if ($source[$cursor] -ne "`r" -and $source[$cursor] -ne "`n") {
                        $masked[$cursor] = [char]32
                    }
                }
                $index = $rawEnd + 1
                continue
            }
        }

        # 通常の文字列とbyte文字列。先頭のbは残っても検査regexには影響しない。
        if ($source[$index] -eq '"') {
            $masked[$index] = [char]32
            $index += 1
            while ($index -lt $length) {
                if ($source[$index] -eq '\') {
                    $masked[$index] = [char]32
                    if ($index + 1 -lt $length) {
                        $index += 1
                        if ($source[$index] -ne "`r" -and $source[$index] -ne "`n") {
                            $masked[$index] = [char]32
                        }
                    }
                    $index += 1
                    continue
                }
                if ($source[$index] -eq '"') {
                    $masked[$index] = [char]32
                    $index += 1
                    break
                }
                if ($source[$index] -ne "`r" -and $source[$index] -ne "`n") {
                    $masked[$index] = [char]32
                }
                $index += 1
            }
            continue
        }

        $index += 1
    }

    return -join $masked
}

function Get-PatternFiles {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)]$Definition,
        [Parameter(Mandatory = $true)][regex]$FileRegex,
        [Parameter(Mandatory = $true)][string]$Context
    )

    if (-not (Test-HasProperty -Object $Definition -Name "roots")) {
        throw "$Context に必須項目 'roots' がありません。"
    }
    $roots = @($Definition.roots)
    if ($roots.Count -eq 0) {
        throw "$Context の roots が空です。"
    }

    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    $results = New-Object System.Collections.Generic.List[object]
    foreach ($configuredRoot in $roots) {
        $relativeRoot = Get-RequiredString -Object ([pscustomobject]@{ value = $configuredRoot }) -Name "value" -Context "$Context roots"
        $fullRoot = Get-SafeRepositoryPath -Root $Root -RelativePath $relativeRoot -Context "$Context roots"
        if (-not (Test-Path -LiteralPath $fullRoot)) {
            continue
        }

        $candidates = @()
        if (Test-Path -LiteralPath $fullRoot -PathType Leaf) {
            $candidates = @((Get-Item -LiteralPath $fullRoot))
        }
        else {
            $candidates = @(Get-ChildItem -LiteralPath $fullRoot -File -Recurse)
        }
        foreach ($candidate in $candidates) {
            $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd([char[]]"\/")
            $relative = $candidate.FullName.Substring($rootFull.Length).TrimStart([char[]]"\/").Replace("\", "/")
            if ($FileRegex.IsMatch($relative) -and $seen.Add($relative)) {
                $results.Add([pscustomobject]@{
                    Path = $relative
                    FullPath = $candidate.FullName
                })
            }
        }
    }
    return $results.ToArray()
}

function Get-LineNumber {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][int]$Index
    )

    if ($Index -le 0) {
        return 1
    }
    return ([regex]::Matches($Text.Substring(0, $Index), "`n")).Count + 1
}

try {
    if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
        $RepositoryRoot = Join-Path $PSScriptRoot "..\..\.."
    }
    $RepositoryRoot = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd([char[]]"\/")
    if (-not (Test-Path -LiteralPath $RepositoryRoot -PathType Container)) {
        throw "リポジトリのルートが存在しません: $RepositoryRoot"
    }

    if ([string]::IsNullOrWhiteSpace($DefinitionPath)) {
        $DefinitionPath = Join-Path $RepositoryRoot ".github\known-defect-shapes.json"
    }
    $DefinitionPath = [IO.Path]::GetFullPath($DefinitionPath)
    if (-not (Test-Path -LiteralPath $DefinitionPath -PathType Leaf)) {
        throw "既知形の定義ファイルが存在しません: $DefinitionPath"
    }

    $json = [IO.File]::ReadAllText($DefinitionPath)
    $catalog = $json | ConvertFrom-Json
    if (-not (Test-HasProperty -Object $catalog -Name "patterns")) {
        throw "定義ファイルに patterns がありません。"
    }
    $definitions = @($catalog.patterns)
    if ($definitions.Count -eq 0) {
        throw "定義ファイルの patterns が空です。"
    }

    $increaseCount = 0
    $decreaseCount = 0
    $inventoryDriftCount = 0
    $seenIds = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal)

    foreach ($definition in $definitions) {
        $id = Get-RequiredString -Object $definition -Name "id" -Context "patterns項目"
        $context = "patterns[$id]"
        if (-not $seenIds.Add($id)) {
            throw "pattern id が重複しています: $id"
        }
        [void](Get-RequiredString -Object $definition -Name "reason" -Context $context)
        $patternText = Get-RequiredString -Object $definition -Name "regex" -Context $context
        $filePatternText = Get-RequiredString -Object $definition -Name "filePattern" -Context $context
        $registeredCount = Get-RequiredNonNegativeInteger -Object $definition -Name "registeredCount" -Context $context
        $measuredRawCount = Get-RequiredNonNegativeInteger -Object $definition -Name "measuredRawCount" -Context $context
        $patternRegex = New-CheckedRegex -Pattern $patternText -Context $context
        $fileRegex = New-CheckedRegex -Pattern $filePatternText -Context "$context filePattern"

        $files = @(Get-PatternFiles -Root $RepositoryRoot -Definition $definition -FileRegex $fileRegex -Context $context)
        $rawByPath = @{}
        $maskedByPath = @{}
        $rawCount = 0
        $samples = New-Object System.Collections.Generic.List[string]
        foreach ($file in $files) {
            $sourceText = [IO.File]::ReadAllText($file.FullPath)
            $maskedText = Hide-RustCommentsAndStrings -Text $sourceText
            $maskedByPath[$file.Path] = $maskedText
            $matches = @($patternRegex.Matches($maskedText))
            $rawByPath[$file.Path] = $matches.Count
            $rawCount += $matches.Count
            foreach ($match in $matches) {
                if ($samples.Count -ge 5) {
                    break
                }
                $line = Get-LineNumber -Text $maskedText -Index $match.Index
                $samples.Add("$($file.Path):$line")
            }
        }

        if (-not (Test-HasProperty -Object $definition -Name "exceptions")) {
            throw "$context に必須項目 'exceptions' がありません。"
        }
        $exceptions = @($definition.exceptions)
        $registeredAllowance = 0
        $currentAllowance = 0
        $seenExactExceptionLocations = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
        foreach ($exception in $exceptions) {
            $exceptionContext = "$context exceptions"
            $exceptionPath = ConvertTo-RepositoryPath (Get-RequiredString -Object $exception -Name "path" -Context $exceptionContext)
            [void](Get-SafeRepositoryPath -Root $RepositoryRoot -RelativePath $exceptionPath -Context $exceptionContext)
            [void](Get-RequiredString -Object $exception -Name "reason" -Context $exceptionContext)
            $allowedCount = Get-RequiredNonNegativeInteger -Object $exception -Name "allowedCount" -Context $exceptionContext
            $registeredAllowance += $allowedCount

            $actualExceptionCount = 0
            $isRegexException = Test-HasProperty -Object $exception -Name "regex"
            if ($isRegexException) {
                $exceptionPattern = Get-RequiredString -Object $exception -Name "regex" -Context $exceptionContext
                $exceptionRegex = New-CheckedRegex -Pattern $exceptionPattern -Context $exceptionContext
                if ($maskedByPath.ContainsKey($exceptionPath)) {
                    $actualExceptionCount = @($exceptionRegex.Matches([string]$maskedByPath[$exceptionPath])).Count
                }
            }
            elseif ($rawByPath.ContainsKey($exceptionPath)) {
                $actualExceptionCount = [int]$rawByPath[$exceptionPath]
            }

            $currentAllowance += [Math]::Min($actualExceptionCount, $allowedCount)
            if ($isRegexException -and $actualExceptionCount -gt $allowedCount) {
                Write-Output "[NG] $id の許可例外が増加しました: $exceptionPath は $allowedCount 件まで、現在 $actualExceptionCount 件。"
            }

            $hasExactLine = Test-HasProperty -Object $exception -Name "line"
            $hasExactMacro = Test-HasProperty -Object $exception -Name "macro"
            if ($hasExactLine -ne $hasExactMacro) {
                throw "$exceptionContext の個別例外は line と macro の両方を指定してください: $exceptionPath"
            }
            if ($hasExactLine) {
                if (-not $isRegexException -or $allowedCount -ne 1) {
                    throw "$exceptionContext の個別例外は regex を持ち、allowedCount=1でなければなりません: $exceptionPath"
                }
                $exactLine = Get-RequiredNonNegativeInteger -Object $exception -Name "line" -Context $exceptionContext
                $exactMacro = Get-RequiredString -Object $exception -Name "macro" -Context $exceptionContext
                if ($exactLine -lt 1) {
                    throw "$exceptionContext の line は1以上でなければなりません。"
                }
                $locationKey = "$exceptionPath`:$exactLine"
                if (-not $seenExactExceptionLocations.Add($locationKey)) {
                    throw "$exceptionContext の個別例外の場所が重複しています: $locationKey"
                }

                $identityPresent = $false
                if ($maskedByPath.ContainsKey($exceptionPath)) {
                    $exactLines = @(([string]$maskedByPath[$exceptionPath]) -split "`r?`n")
                    if ($exactLine -le $exactLines.Count -and $exactLines[$exactLine - 1].Contains($exactMacro)) {
                        $identityPresent = $true
                    }
                }
                if (-not $identityPresent) {
                    $inventoryDriftCount += 1
                    Write-Output "[要確認] $id の誤検知例外台帳が移動または消失しました: $exceptionPath`:$exactLine ($exactMacro)。型と理由を再分類してください。"
                }
            }
        }

        if (($measuredRawCount - $registeredAllowance) -ne $registeredCount) {
            throw "$context の件数が矛盾しています: measuredRawCount $measuredRawCount - 例外許容 $registeredAllowance != registeredCount $registeredCount"
        }

        if (Test-HasProperty -Object $definition -Name "candidateInventory") {
            $inventory = @($definition.candidateInventory)
            if ($inventory.Count -ne $registeredCount) {
                throw "$context の candidateInventory は $($inventory.Count) 件ですが、registeredCount は $registeredCount 件です。"
            }
            $macroCounts = @{}
            foreach ($candidate in $inventory) {
                $candidatePath = ConvertTo-RepositoryPath (Get-RequiredString -Object $candidate -Name "path" -Context "$context candidateInventory")
                $candidateMacro = Get-RequiredString -Object $candidate -Name "macro" -Context "$context candidateInventory"
                [void](Get-RequiredString -Object $candidate -Name "shape" -Context "$context candidateInventory")
                $candidateLine = Get-RequiredNonNegativeInteger -Object $candidate -Name "line" -Context "$context candidateInventory"
                if ($candidateLine -lt 1) {
                    throw "$context candidateInventory の line は1以上でなければなりません。"
                }
                if (-not $macroCounts.ContainsKey($candidateMacro)) {
                    $macroCounts[$candidateMacro] = 0
                }
                $macroCounts[$candidateMacro] += 1

                $candidateFullPath = Get-SafeRepositoryPath -Root $RepositoryRoot -RelativePath $candidatePath -Context "$context candidateInventory"
                $identityPresent = $false
                if (Test-Path -LiteralPath $candidateFullPath -PathType Leaf) {
                    $candidateLines = @([IO.File]::ReadAllLines($candidateFullPath))
                    if ($candidateLine -le $candidateLines.Count -and $candidateLines[$candidateLine - 1].Contains($candidateMacro)) {
                        $identityPresent = $true
                    }
                }
                if (-not $identityPresent) {
                    $inventoryDriftCount += 1
                    Write-Output "[要確認] $id の候補台帳が移動または消失しました: $candidatePath`:$candidateLine ($candidateMacro)。件数ラチェットの置換見逃しを目視確認してください。"
                }
            }

            if (Test-HasProperty -Object $definition -Name "registeredBreakdown") {
                foreach ($property in $definition.registeredBreakdown.PSObject.Properties) {
                    $expected = [int]$property.Value
                    $actual = 0
                    if ($macroCounts.ContainsKey($property.Name)) {
                        $actual = [int]$macroCounts[$property.Name]
                    }
                    if ($actual -ne $expected) {
                        throw "$context の registeredBreakdown[$($property.Name)]=$expected と台帳件数 $actual が一致しません。"
                    }
                }
            }
        }

        $currentCount = $rawCount - $currentAllowance
        if ($currentCount -gt $registeredCount) {
            $increaseCount += 1
            $delta = $currentCount - $registeredCount
            Write-Output "[NG] 既知の欠陥形 '$id' が増加しました: $registeredCount -> $currentCount（+$delta、raw=$rawCount、許可例外=$currentAllowance）。"
            Write-Output "  理由: $($definition.reason)"
            if ($samples.Count -gt 0) {
                Write-Output "  現在の一致例: $($samples -join ', ')"
            }
        }
        elseif ($currentCount -lt $registeredCount) {
            $decreaseCount += 1
            $delta = $registeredCount - $currentCount
            Write-Output "[減少] 既知の欠陥形 '$id' が減りました: $registeredCount -> $currentCount（-$delta、raw=$rawCount、許可例外=$currentAllowance）。"
            Write-Output "  改善を固定するため、$DefinitionPath の registeredCount / measuredRawCount / 例外枠を実測に合わせて更新してください。"
        }
        else {
            Write-Output "[OK] 既知の欠陥形 '$id': $currentCount 件（登録 $registeredCount、raw=$rawCount、許可例外=$currentAllowance）"
        }
    }

    if ($increaseCount -gt 0) {
        Write-Output "[NG] 既知の欠陥形ラチェット: 増加 $increaseCount 種、減少 $decreaseCount 種、台帳移動 $inventoryDriftCount 件"
        exit 1
    }
    if ($FailOnDecrease -and ($decreaseCount -gt 0 -or $inventoryDriftCount -gt 0)) {
        Write-Output "[NG] 既知の欠陥形ラチェット: -FailOnDecrease により、減少または台帳移動を定義更新まで失敗扱いにします。"
        exit 1
    }

    Write-Output "[OK] 既知の欠陥形ラチェット: 定義 $($definitions.Count) 種、増加 0種、減少 $decreaseCount 種、台帳移動 $inventoryDriftCount 件"
    exit 0
}
catch {
    Write-Output "[NG] 既知の欠陥形ラチェットを完了できませんでした: $($_.Exception.Message)"
    exit 2
}
