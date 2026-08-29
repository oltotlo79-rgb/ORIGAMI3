<#
.SYNOPSIS
統括が作業担当（Claudeのサブエージェント）へ渡す指示文が、規約の必須項目を満たしているか検査します。

.DESCRIPTION
docs/rules/01-役割と委譲.md の §2〜§4、docs/rules/03-品質ゲート.md §10.6、
docs/rules/06-過去の失敗と対策.md §10.7.11 の条文から起こした11項目を、
指示文のテキストから検出します。検出は「該当する記述らしきものが
あるか」を確かめるヒューリスティックであり、実ファイル・実関数が本当に存在するかの
確認そのものは行いません（実在確認は統括が rg 等で行う。01-役割と委譲.md:43）。
このため本検査は単独で「自動」ではなく「半自動」の位置づけです。

検査する11項目と根拠（行番号は2026-08-29時点）:
  1. 実ファイルパスと関数名の実名記載            docs/rules/01-役割と委譲.md:43
  2. 合格条件の数値記載                          docs/rules/01-役割と委譲.md:44
  3. やってはいけないことの列挙（6件）           docs/rules/01-役割と委譲.md:45 ほか
  4. 段階分割と中間報告の義務                    docs/rules/01-役割と委譲.md:46
  5. 過去の失敗と原因                            docs/rules/01-役割と委譲.md:47
  6. 道具の具体的な使い方                        docs/rules/01-役割と委譲.md:48
  7. 成果物の保存先パス                          docs/rules/01-役割と委譲.md:49
  8. 割り当てた作業ツリーの絶対パス              docs/rules/01-役割と委譲.md:30
  9. 報告書ファイルへの継続記録                  docs/rules/01-役割と委譲.md:57
 10. 長時間検査の --skip 4件（cargo test記載時のみ） docs/rules/03-品質ゲート.md:61
 11. 専用のCARGO_TARGET_DIR指定（cargo記載時のみ）    docs/rules/06-過去の失敗と対策.md:241-242

【2026-08-29 追加指示】`.claude/settings.json` のPreToolUse hookが利用者の決定で
外され、`cargo`/`npm`直接実行の自動阻止（§10.7.13）は「自動」から「人」へ戻った。
機械の歯止めが無くなった分の代わりとして、項目11（CARGO_TARGET_DIR指定）を統括の
追加指示により新設した。項目10（--skip 4件）は既存のまま流用する。

【依頼文との食い違い】依頼で示された9項目の一覧には、§4「指示書に『成果は報告書
ファイルへ書く』と記す」に対応する項目が無かった。条文（01-役割と委譲.md:57）を
正本として9番目に追加した。依頼の項目8「割り当てた作業ツリーの絶対パス」はモデル
選択理由の記載を含まない。§2の「起動ごとに選んだモデルと理由を1行で記録する」
（01-役割と委譲.md:31）はClaude自身の記録行為についての定めで、指示文の必須記載
とは条文上読めないため、モデル選択理由は本検査の対象に含めていない。

項目3の6件の内訳のうち、gitへの書き込み禁止／ブラウザ・desktop.exe・配信サーバー
禁止／期待値・許容差を緩めない／Cargo.toml・Cargo.lock・vendor/を変更しない、の
4件は docs/rules/01-役割と委譲.md と docs/rules/02-禁止事項.md に条文がある。
残り2件のうち「docs/competitive-review-2026-08-20.md に触らない」は docs/rules
配下に条文が無く、`scripts/hooks/checks/no-prohibited-doc.ps1` が示す「利用者が
変更を禁止した文書」という運用上の禁止（00-旧規約対応と施行.md:127「利用者指示」）
が根拠である。「悪魔・1分ローズを使わない」は docs/rules 配下のどこにも条文が無く、
唯一の根拠は利用者メモリ `no-devil-no-rose.md`（docs/rules外）である。この2件は
docs/rules を正本とする観点では根拠不足だが、依頼で名指しされたため検査には含め、
根拠の所在をここに明記する。

.PARAMETER InstructionPath
検査する指示文のテキストファイル。複数指定できます（スペースまたはカンマ区切り）。

.PARAMETER MinNumericCriteria
数値の合格条件として認める最小該当件数です。既定は1件（1つも無ければ不合格）。

.PARAMETER ProximityWindow
名詞と動詞の組を認める前後の文字数（近接窓）です。既定は50文字。
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0, ValueFromRemainingArguments = $true)]
    [ValidateNotNullOrEmpty()]
    [string[]]$InstructionPath,

    [ValidateRange(1, 20)]
    [int]$MinNumericCriteria = 1,

    [ValidateRange(10, 200)]
    [int]$ProximityWindow = 50
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

# ---- パターン定義（すべて根拠つき。詳細はレポートを参照） ----

# 項目1: 実ファイルパス（既知の最上位フォルダーで始まり拡張子を持つもの）と、
# 関数名らしきトークン（呼出し/定義/パス形式）。非コード作業は「該当なし」＋理由で免除。
# 註: `docs/competitive-review-2026-08-20.md` は項目3の禁止事項の列挙として
# 触れられるだけで、編集対象になることは規約上あり得ない。これを対象パスとして
# 誤検出しないよう除外する（実測で見つけた誤検出）。
$script:RepoTargetPathPattern = '(?!docs/competitive-review-2026-08-20\.md)(?:crates|apps|scripts|docs|\.github)/[A-Za-z0-9_./+-]*\.[A-Za-z0-9]+'
$script:FunctionLikePattern = '[A-Za-z_][A-Za-z0-9_]*\(\)|[A-Za-z_][A-Za-z0-9_]*::[A-Za-z_][A-Za-z0-9_]*|fn\s+[A-Za-z_][A-Za-z0-9_]*|function\s+[A-Za-z_][A-Za-z0-9_-]*'

# 項目2: 数値と「単位」または「比較記号」の組み合わせ。
# 「1分ローズ」は禁止対象の固有名詞であって数値条件ではないため、"分"の直後に
# "ローズ"が続く場合は単位としてカウントしない（実測で見つけた誤検出を除外）。
$script:NumberPattern = '[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?'
$script:UnitWords = 'ミリ秒|msec|ms|mm|cm|GB|MB|KB|バイト|bytes|件|秒|分(?!ローズ)|層|%|px|度|°|個|本|行|回|倍|m'
$script:JaComparators = '以下|以上|未満|超|以内'
$script:SymComparators = '<=|>=|≦|≧'
$script:NumericCriterionPattern =
    '(?:' + $script:NumberPattern + ')\s*(?:' + $script:UnitWords + '|' + $script:JaComparators + '|' + $script:SymComparators + ')' +
    '|(?:' + $script:SymComparators + ')\s*(?:' + $script:NumberPattern + ')'

# 項目3: 6件のやってはいけないこと（名詞と禁止動詞の近接窓一致）。
# 註: .NETの既定\bは日本語文字も\w扱いのため「gitへの」のように直後に助詞が
# 続く実例で境界判定に失敗する（実測で見つけた誤検出=見逃し）。ASCII文字の
# 連続だけを除外する否定先読み/後読みに置き換えている。
$script:GitNounPattern = '(?<![A-Za-z0-9_])git(?![A-Za-z0-9_])'
$script:GitVerbPattern = '禁止|書き込まな|書き込みしな|書込みしな|させない|しない'
$script:LaunchBanVerbPattern = '起動しない|起動させない|起動禁止|開かない|開かせない|禁止'
$script:ToleranceNounPattern = '期待値|許容差|性能上限|上限値|しきい値|閾値'
$script:RelaxVerbPattern = '緩め|緩和|書き換えない|変更しない|下げない|甘く'
$script:DependencyNounPattern = 'Cargo\.toml|Cargo\.lock|vendor(?![A-Za-z0-9_])'
$script:NoChangeVerbPattern = '変更しない|触らない|編集しない|禁止'
$script:CompetitiveReviewNounPattern = 'competitive-review-2026-08-20'
$script:NoTouchVerbPattern = '触らない|参照しない|読まない|変更しない|禁止|開かない'
$script:DevilRoseNounPattern = '悪魔|1分ローズ|一分ローズ'
$script:DevilRoseVerbPattern = '使わない|扱わない|禁止|用いない|使用しない|対象にしない|標本にしない'

# 項目4: 段階分割と中間報告の義務。
$script:StagePattern = 'Step\s*\d+|第\s*\d*\s*段階|段階|フェーズ|ステップ'
$script:InterimReportPattern = '中間報告|都度報告|各.{0,8}(?:段階|ステップ).{0,10}報告|報告してから|報告を義務'

# 項目5: 過去の失敗と原因。
$script:PastFailurePattern = '過去の失敗|失敗例|不具合|事故|バグ'

# 項目6: 具体的なコマンド（バッククォート区間の中に既知の道具語を含むか）。
$script:CommandWordPattern = 'cargo|npm|(?<![A-Za-z0-9_])rg(?![A-Za-z0-9_])|grep|(?<![A-Za-z0-9_])git(?![A-Za-z0-9_])|powershell|pwsh|Get-Process|Get-NetTCPConnection|Test-Path|Get-Item|Get-ChildItem|scripts/|\.ps1|\.exe|rustc'

# 項目7: 成果物の保存先パス。
$script:SaveVerbPattern = '保存|書き込|出力先|出力する|(?:に|へ)[^\r\n。、]{0,8}書く'
$script:SavePathPattern = '(?:crates|apps|scripts|docs|scratchpad|verification|\.github)/[A-Za-z0-9_./+-]*\.[A-Za-z0-9]+|[A-Za-z]:[\\/][^\s`"]+'

# 項目8: 割り当てた作業ツリーの絶対パス。
$script:WorktreeNounPattern = 'worktree|work\s*tree|作業ツリー|作業木|ワークツリー'
$script:AbsolutePathPattern = '[A-Za-z]:[\\/]|%TEMP%[\\/]'

# 項目9: 報告書ファイルへの継続記録（§4）。
$script:ReportContinuityVerbPattern = '(?:へ|に)[^\r\n。、]{0,10}(?:書く|書き込|記録)|(?:を|が)[^\r\n。、]{0,10}更新|更新し続け'

# 項目11: 専用のCARGO_TARGET_DIR指定（cargo実行の記載時のみ）。
# 根拠: docs/rules/06-過去の失敗と対策.md:241-242「複製内のcargoにもCARGO_TARGET_DIRを
# 設定し、verification/に出力させない」。2026-08-29、統括からの追加指示で新設。
# 発動条件は「cargo <サブコマンド>」の実行記載であり、単なる`Cargo.toml`/`Cargo.lock`
# というファイル名の言及（項目3のCargo.toml/Cargo.lock/vendor禁止の列挙など）では
# 発動しないよう絞り込んでいる（実測で見つけた誤検出を除外）。
$script:CargoInvocationTriggerPattern = 'cargo\s+(?:test|build|check|clippy|run|bench|nextest|fmt|doc|install|add|remove|update)\b'
$script:CargoTargetDirToken = "CARGO_TARGET_DIR"

# 項目10: 長時間検査の除外4件（正本 docs/rules/03-品質ゲート.md:61）。
$script:LongTestSkipNames = @(
    "completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten",
    "named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten",
    "a_safe_coincident_partial_network_appears_after_the_first_fold",
    "the_heaviest_proposal_never_hits_the_time_limit"
)

function Test-WindowedPair {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$NounPattern,
        [Parameter(Mandatory = $true)][string]$VerbPattern,
        [Parameter(Mandatory = $true)][int]$Window
    )

    $options = [Text.RegularExpressions.RegexOptions]::IgnoreCase
    $nounMatches = [regex]::Matches($Text, $NounPattern, $options)
    foreach ($m in $nounMatches) {
        $start = [Math]::Max(0, $m.Index - $Window)
        $stop = [Math]::Min($Text.Length, $m.Index + $m.Length + $Window)
        $slice = $Text.Substring($start, $stop - $start)
        if ([regex]::IsMatch($slice, $VerbPattern, $options)) {
            return $true
        }
    }
    return $false
}

function Test-NoCodeExemption {
    param([Parameter(Mandatory = $true)][string]$Text)

    $match = [regex]::Match($Text, '該当なし(?<reason>[^\r\n]{0,80})')
    if (-not $match.Success) {
        return $false
    }
    $reasonTail = [regex]::Replace($match.Groups["reason"].Value, '[\s、。：:]', '')
    return ($reasonTail.Length -ge 4)
}

function Test-RealPathAndFunctionName {
    param([Parameter(Mandatory = $true)][string]$Text)

    if (Test-NoCodeExemption -Text $Text) {
        return [pscustomobject]@{ Status = "OK"; Detail = "非コード作業の「該当なし」＋理由の記載を検出" }
    }
    $hasPath = [regex]::IsMatch($Text, $script:RepoTargetPathPattern)
    $hasFunction = [regex]::IsMatch($Text, $script:FunctionLikePattern)
    if ($hasPath -and $hasFunction) {
        return [pscustomobject]@{ Status = "OK"; Detail = "実パスと関数名らしき記載を検出" }
    }
    $missingParts = New-Object System.Collections.Generic.List[string]
    if (-not $hasPath) { $missingParts.Add("実ファイルパス") }
    if (-not $hasFunction) { $missingParts.Add("関数名") }
    return [pscustomobject]@{ Status = "NG"; Detail = ("欠落: " + ($missingParts -join "・") + "（「該当なし」＋理由の記載も無し）") }
}

function Test-NumericCriteria {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][int]$MinCount
    )

    $found = [regex]::Matches($Text, $script:NumericCriterionPattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)
    $count = $found.Count
    if ($count -ge $MinCount) {
        return [pscustomobject]@{ Status = "OK"; Detail = "該当パターン $count 件（閾値 $MinCount 件以上）" }
    }
    return [pscustomobject]@{ Status = "NG"; Detail = "該当パターン $count 件（閾値 $MinCount 件未満）" }
}

function Test-ProhibitedActionsEnumerated {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][int]$Window
    )

    $missing = New-Object System.Collections.Generic.List[string]
    $total = 6

    if (-not (Test-WindowedPair -Text $Text -NounPattern $script:GitNounPattern -VerbPattern $script:GitVerbPattern -Window $Window)) {
        $missing.Add("gitへの書き込み禁止")
    }

    $browserOk = (Test-WindowedPair -Text $Text -NounPattern 'ブラウザ' -VerbPattern $script:LaunchBanVerbPattern -Window $Window) -and
                 (Test-WindowedPair -Text $Text -NounPattern 'desktop\.exe' -VerbPattern $script:LaunchBanVerbPattern -Window $Window) -and
                 (Test-WindowedPair -Text $Text -NounPattern '配信サーバー' -VerbPattern $script:LaunchBanVerbPattern -Window $Window)
    if (-not $browserOk) {
        $missing.Add("ブラウザ・desktop.exe・配信サーバーを起動しない")
    }

    if (-not (Test-WindowedPair -Text $Text -NounPattern $script:ToleranceNounPattern -VerbPattern $script:RelaxVerbPattern -Window $Window)) {
        $missing.Add("期待値・許容差を緩めない")
    }

    if (-not (Test-WindowedPair -Text $Text -NounPattern $script:DependencyNounPattern -VerbPattern $script:NoChangeVerbPattern -Window $Window)) {
        $missing.Add("Cargo.toml/Cargo.lock/vendor/を変更しない")
    }

    if (-not (Test-WindowedPair -Text $Text -NounPattern $script:CompetitiveReviewNounPattern -VerbPattern $script:NoTouchVerbPattern -Window $Window)) {
        $missing.Add("docs/competitive-review-2026-08-20.mdに触らない")
    }

    if (-not (Test-WindowedPair -Text $Text -NounPattern $script:DevilRoseNounPattern -VerbPattern $script:DevilRoseVerbPattern -Window $Window)) {
        $missing.Add("悪魔・1分ローズを使わない")
    }

    $passCount = $total - $missing.Count
    if ($missing.Count -eq 0) {
        return [pscustomobject]@{ Status = "OK"; Detail = "$passCount/$total" }
    }
    return [pscustomobject]@{ Status = "NG"; Detail = ("{0}/{1}（欠落: {2}）" -f $passCount, $total, ($missing -join "、")) }
}

function Test-StagedWorkAndInterimReport {
    param([Parameter(Mandatory = $true)][string]$Text)

    $hasStage = [regex]::IsMatch($Text, $script:StagePattern)
    $hasInterimReport = [regex]::IsMatch($Text, $script:InterimReportPattern)
    if ($hasStage -and $hasInterimReport) {
        return [pscustomobject]@{ Status = "OK"; Detail = "段階分割と中間報告義務の両方を検出" }
    }
    $missingParts = New-Object System.Collections.Generic.List[string]
    if (-not $hasStage) { $missingParts.Add("段階分割") }
    if (-not $hasInterimReport) { $missingParts.Add("中間報告の義務") }
    return [pscustomobject]@{ Status = "NG"; Detail = ("欠落: " + ($missingParts -join "・")) }
}

function Test-PastFailureAndCause {
    param([Parameter(Mandatory = $true)][string]$Text)

    $hasFailure = [regex]::IsMatch($Text, $script:PastFailurePattern)
    $hasCause = [regex]::IsMatch($Text, '原因')
    if ($hasFailure -and $hasCause) {
        return [pscustomobject]@{ Status = "OK"; Detail = "過去の失敗例と原因の両方を検出" }
    }
    $missingParts = New-Object System.Collections.Generic.List[string]
    if (-not $hasFailure) { $missingParts.Add("過去の失敗例") }
    if (-not $hasCause) { $missingParts.Add("原因") }
    return [pscustomobject]@{ Status = "NG"; Detail = ("欠落: " + ($missingParts -join "・")) }
}

function Test-ToolUsageSpecifics {
    param([Parameter(Mandatory = $true)][string]$Text)

    $spans = [regex]::Matches($Text, '`([^`\r\n]{1,300})`')
    $commandSpanCount = 0
    foreach ($span in $spans) {
        if ([regex]::IsMatch($span.Groups[1].Value, $script:CommandWordPattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
            $commandSpanCount += 1
        }
    }
    if ($commandSpanCount -ge 1) {
        return [pscustomobject]@{ Status = "OK"; Detail = "具体的なコマンド記載 $commandSpanCount 件" }
    }
    return [pscustomobject]@{ Status = "NG"; Detail = 'コマンド形式の記載（バッククォートで囲んだ具体的なコマンド）が見つかりません' }
}

function Test-DeliverableSavePath {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][int]$Window
    )

    if (Test-WindowedPair -Text $Text -NounPattern $script:SaveVerbPattern -VerbPattern $script:SavePathPattern -Window $Window) {
        return [pscustomobject]@{ Status = "OK"; Detail = "保存先パスの記載を検出" }
    }
    return [pscustomobject]@{ Status = "NG"; Detail = "成果物の保存先パスの記載が見つかりません" }
}

function Test-WorktreeAbsolutePath {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][int]$Window
    )

    if (Test-WindowedPair -Text $Text -NounPattern $script:WorktreeNounPattern -VerbPattern $script:AbsolutePathPattern -Window $Window) {
        return [pscustomobject]@{ Status = "OK"; Detail = "作業ツリーの絶対パスを検出" }
    }
    return [pscustomobject]@{ Status = "NG"; Detail = '割り当てた作業ツリーの絶対パス（例: C:\...）の記載が見つかりません' }
}

function Test-ReportFileContinuity {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][int]$Window
    )

    if (Test-WindowedPair -Text $Text -NounPattern '報告書' -VerbPattern $script:ReportContinuityVerbPattern -Window $Window) {
        return [pscustomobject]@{ Status = "OK"; Detail = "報告書ファイルへ書く旨の記載を検出" }
    }
    return [pscustomobject]@{ Status = "NG"; Detail = "「成果は報告書ファイルへ書く」に相当する記載が見つかりません" }
}

function Test-LongTestSkipList {
    param([Parameter(Mandatory = $true)][string]$Text)

    if (-not [regex]::IsMatch($Text, 'cargo\s+test', [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        return [pscustomobject]@{ Status = "NA"; Detail = "cargo testの言及なし（該当なし）" }
    }
    $missing = New-Object System.Collections.Generic.List[string]
    foreach ($name in $script:LongTestSkipNames) {
        if ($Text.IndexOf($name, [System.StringComparison]::Ordinal) -lt 0) {
            $missing.Add($name)
        }
    }
    if ($missing.Count -eq 0) {
        return [pscustomobject]@{ Status = "OK"; Detail = "4/4 件のskip対象を検出" }
    }
    $foundCount = 4 - $missing.Count
    return [pscustomobject]@{ Status = "NG"; Detail = ("{0}/4 件のみ検出。欠落: {1}" -f $foundCount, ($missing -join ", ")) }
}

function Test-CargoTargetDirSpecified {
    param([Parameter(Mandatory = $true)][string]$Text)

    if (-not [regex]::IsMatch($Text, $script:CargoInvocationTriggerPattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
        return [pscustomobject]@{ Status = "NA"; Detail = "cargo実行の言及なし（該当なし）" }
    }
    if ($Text.IndexOf($script:CargoTargetDirToken, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
        return [pscustomobject]@{ Status = "OK"; Detail = "CARGO_TARGET_DIRの指定を検出" }
    }
    return [pscustomobject]@{ Status = "NG"; Detail = "cargoの言及があるのにCARGO_TARGET_DIRの指定が見つかりません" }
}

function Get-InstructionCheckResults {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][int]$MinNumericCriteria,
        [Parameter(Mandatory = $true)][int]$Window
    )

    $results = New-Object System.Collections.Generic.List[object]

    $r1 = Test-RealPathAndFunctionName -Text $Text
    $results.Add([pscustomobject]@{ Index = 1; Name = "実ファイルパスと関数名の実名記載"; Citation = "01-役割と委譲.md:43"; Status = $r1.Status; Detail = $r1.Detail })

    $r2 = Test-NumericCriteria -Text $Text -MinCount $MinNumericCriteria
    $results.Add([pscustomobject]@{ Index = 2; Name = "合格条件の数値記載"; Citation = "01-役割と委譲.md:44"; Status = $r2.Status; Detail = $r2.Detail })

    $r3 = Test-ProhibitedActionsEnumerated -Text $Text -Window $Window
    $results.Add([pscustomobject]@{ Index = 3; Name = "やってはいけないことの列挙"; Citation = "01-役割と委譲.md:45"; Status = $r3.Status; Detail = $r3.Detail })

    $r4 = Test-StagedWorkAndInterimReport -Text $Text
    $results.Add([pscustomobject]@{ Index = 4; Name = "段階分割と中間報告の義務"; Citation = "01-役割と委譲.md:46"; Status = $r4.Status; Detail = $r4.Detail })

    $r5 = Test-PastFailureAndCause -Text $Text
    $results.Add([pscustomobject]@{ Index = 5; Name = "過去の失敗と原因"; Citation = "01-役割と委譲.md:47"; Status = $r5.Status; Detail = $r5.Detail })

    $r6 = Test-ToolUsageSpecifics -Text $Text
    $results.Add([pscustomobject]@{ Index = 6; Name = "道具の具体的な使い方"; Citation = "01-役割と委譲.md:48"; Status = $r6.Status; Detail = $r6.Detail })

    $r7 = Test-DeliverableSavePath -Text $Text -Window $Window
    $results.Add([pscustomobject]@{ Index = 7; Name = "成果物の保存先パス"; Citation = "01-役割と委譲.md:49"; Status = $r7.Status; Detail = $r7.Detail })

    $r8 = Test-WorktreeAbsolutePath -Text $Text -Window $Window
    $results.Add([pscustomobject]@{ Index = 8; Name = "割り当てた作業ツリーの絶対パス"; Citation = "01-役割と委譲.md:30"; Status = $r8.Status; Detail = $r8.Detail })

    $r9 = Test-ReportFileContinuity -Text $Text -Window $Window
    $results.Add([pscustomobject]@{ Index = 9; Name = "報告書ファイルへの継続記録"; Citation = "01-役割と委譲.md:57"; Status = $r9.Status; Detail = $r9.Detail })

    $r10 = Test-LongTestSkipList -Text $Text
    $results.Add([pscustomobject]@{ Index = 10; Name = "長時間検査の--skip 4件"; Citation = "03-品質ゲート.md:61"; Status = $r10.Status; Detail = $r10.Detail })

    $r11 = Test-CargoTargetDirSpecified -Text $Text
    $results.Add([pscustomobject]@{ Index = 11; Name = "専用のCARGO_TARGET_DIR指定"; Citation = "06-過去の失敗と対策.md:241-242"; Status = $r11.Status; Detail = $r11.Detail })

    return $results.ToArray()
}

function Get-InstructionText {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "指示文ファイルが見つかりません: $Path"
    }
    $resolved = (Resolve-Path -LiteralPath $Path).ProviderPath
    $raw = [IO.File]::ReadAllText($resolved, [Text.Encoding]::UTF8)
    $normalized = $raw.Replace("`r`n", "`n").Replace("`r", "`n")
    return [pscustomobject]@{ Path = $resolved; Text = $normalized }
}

try {
    $fileResults = New-Object System.Collections.Generic.List[object]
    foreach ($rawPath in $InstructionPath) {
        $loaded = Get-InstructionText -Path $rawPath
        $checks = Get-InstructionCheckResults -Text $loaded.Text -MinNumericCriteria $MinNumericCriteria -Window $ProximityWindow
        $fileResults.Add([pscustomobject]@{ Path = $loaded.Path; Checks = $checks })
    }

    $failingFileCount = 0
    $missingItemTotal = 0

    foreach ($fileResult in $fileResults) {
        Write-Output ("[指示書] {0}" -f $fileResult.Path)
        $missingNames = New-Object System.Collections.Generic.List[string]
        foreach ($check in $fileResult.Checks) {
            $marker = "[--]"
            if ($check.Status -eq "OK") { $marker = "[OK]" }
            elseif ($check.Status -eq "NG") { $marker = "[NG]" }
            Write-Output ("  {0} {1}. {2} ({3}) - {4}" -f $marker, $check.Index, $check.Name, $check.Citation, $check.Detail)
            if ($check.Status -eq "NG") {
                $missingNames.Add(("{0}. {1}" -f $check.Index, $check.Name))
            }
        }
        if ($missingNames.Count -gt 0) {
            $failingFileCount += 1
            $missingItemTotal += $missingNames.Count
            Write-Output ("[NG] {0}: 不合格 {1}件 ({2})" -f $fileResult.Path, $missingNames.Count, ($missingNames -join " / "))
        }
        else {
            Write-Output ("[OK] {0}: 全項目合格" -f $fileResult.Path)
        }
    }

    $totalFiles = $fileResults.Count
    if ($failingFileCount -gt 0) {
        Write-Output ("[NG] check-agent-instruction: 対象 {0}件 / 不合格 {1}件 / 不合格項目合計 {2}件" -f $totalFiles, $failingFileCount, $missingItemTotal)
        exit 1
    }
    Write-Output ("[OK] check-agent-instruction: 対象 {0}件 / 全件合格" -f $totalFiles)
    exit 0
}
catch {
    Write-Output ("[NG] 指示文検査を完了できませんでした: {0}" -f $_.Exception.Message)
    exit 2
}
