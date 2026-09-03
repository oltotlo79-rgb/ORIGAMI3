[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [AllowNull()]
    [AllowEmptyCollection()]
    [AllowEmptyString()]
    [string[]]$Text,

    [ValidateRange(1, 2147483647)]
    [int]$StartLine = 1,

    [ValidateRange(0, 2147483647)]
    [int]$RoadmapTotal = 0,

    [switch]$AsJson
)

# Deterministic scope classifier for natural-language roadmap claims.
#
# This script deliberately recognizes a small grammar instead of guessing the
# meaning of arbitrary Japanese.  An expression that is neither explicitly
# roadmap-wide/bounded nor explicitly bound to a finite local subject is
# returned as `ambiguous`.  Callers must fail closed on that result.
#
# The file can be dot-sourced and Get-RoadmapScopeAssertions called directly,
# or invoked as a script with -Text (and optionally -AsJson).

if ($MyInvocation.InvocationName -ne '.') {
    Set-StrictMode -Version 2.0
    $ErrorActionPreference = 'Stop'
}

function ConvertTo-RoadmapScopeLines {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [AllowEmptyString()]
        [AllowNull()]
        [string[]]$Values
    )

    $lines = New-Object System.Collections.Generic.List[string]
    foreach ($value in @($Values)) {
        if ($null -eq $value) {
            $lines.Add('')
            continue
        }
        foreach ($line in [regex]::Split([string]$value, "\r\n|\n|\r")) {
            $lines.Add([string]$line)
        }
    }
    return $lines.ToArray()
}

function Get-RoadmapScopeSegments {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Line,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$NormalizedLine
    )

    # Keep a Markdown table row together: its left cell is commonly the local
    # subject and its right cell the `all passed` predicate.  Ordinary prose is
    # split at sentence terminators so a subject in another sentence cannot be
    # borrowed accidentally.
    if ($NormalizedLine.TrimStart().StartsWith('|', [StringComparison]::Ordinal)) {
        return ,([pscustomobject]@{
            Original   = $Line
            Normalized = $NormalizedLine
            Index      = 0
        })
    }

    $matches = [regex]::Matches($NormalizedLine, '[^。！？]+(?:[。！？]+|$)')
    if ($matches.Count -eq 0) {
        return ,([pscustomobject]@{
            Original   = $Line
            Normalized = $NormalizedLine
            Index      = 0
        })
    }

    $segments = New-Object System.Collections.Generic.List[object]
    foreach ($match in $matches) {
        $normalizedSegment = [string]$match.Value
        if ([string]::IsNullOrWhiteSpace($normalizedSegment)) {
            continue
        }
        # NFKC can change character width, so the unmodified complete source
        # line is retained as Text while Segment is the exact normalized unit
        # used for classification.
        $segments.Add([pscustomobject]@{
            Original   = $Line
            Normalized = $normalizedSegment.Trim()
            Index      = [int]$match.Index
        })
    }
    return $segments.ToArray()
}

function Get-NearestRoadmapScopeInteger {
    param(
        [Parameter(Mandatory = $true)][string]$Segment,
        [Parameter(Mandatory = $true)][int]$TriggerIndex
    )

    $nearestValue = $null
    $nearestDistance = [int]::MaxValue
    foreach ($match in [regex]::Matches($Segment, '[0-9]+')) {
        $distance = [Math]::Abs([int]$match.Index - $TriggerIndex)
        if ($distance -lt $nearestDistance) {
            $parsed = 0
            if ([int]::TryParse($match.Value, [ref]$parsed)) {
                $nearestValue = $parsed
                $nearestDistance = $distance
            }
        }
    }
    return $nearestValue
}

function Get-RoadmapScopeCount {
    param(
        [Parameter(Mandatory = $true)][string]$Segment,
        [Parameter(Mandatory = $true)][string]$Kind,
        [Parameter(Mandatory = $true)][string]$Trigger,
        [Parameter(Mandatory = $true)][int]$TriggerIndex
    )

    if ($Kind -eq 'remainder') {
        $suffixCount = [regex]::Match($Segment, '(?<count>[0-9]+)\s*件\s*(?:残り|残件)')
        if ($suffixCount.Success) {
            return [int]$suffixCount.Groups['count'].Value
        }
        $fromTrigger = $Segment.Substring($TriggerIndex)
        $followingCount = [regex]::Match(
            $fromTrigger,
            '^(?:残作業(?:の本当の数)?|残件|残り|未チェック(?:件数)?|未完了(?:件数)?|解消対象として残(?:す件数|る))(?:は|が|[:：])?\s*(?<count>[0-9]+)'
        )
        if ($followingCount.Success) {
            return [int]$followingCount.Groups['count'].Value
        }
        return $null
    }
    if ($Kind -eq 'universal') {
        $beforeTrigger = $Segment.Substring(0, $TriggerIndex)
        $boundTotal = [regex]::Match(
            $beforeTrigger,
            '(?<count>[0-9]+)\s*(?:件|人|名|本|種|組|枚|か所|箇所|面|層|項目|つ|担当|契約|検査)?[^0-9]*$'
        )
        if ($boundTotal.Success) {
            return [int]$boundTotal.Groups['count'].Value
        }
    }
    return Get-NearestRoadmapScopeInteger -Segment $Segment -TriggerIndex $TriggerIndex
}

function Get-RoadmapScopeTemporal {
    param(
        [Parameter(Mandatory = $true)][string]$Segment,
        [Parameter(Mandatory = $true)][int]$TriggerIndex,
        [Parameter(Mandatory = $true)][string]$Trigger
    )

    $beforeTrigger = $Segment.Substring(0, $TriggerIndex)
    $clauseStart = 0
    foreach ($breaker in [regex]::Matches($beforeTrigger, '(?:だが|ですが|しかし|一方|ただし|ものの|けれども?|のに)[、，]')) {
        $clauseStart = [int]$breaker.Index + [int]$breaker.Length
    }
    $beforeTriggerClause = $beforeTrigger.Substring($clauseStart)
    $fromTrigger = $Segment.Substring($TriggerIndex)

    # A condition scopes this assertion only when it precedes the trigger, or
    # when the predicate immediately following this occurrence states that
    # the asserted state is future.  An unrelated `if ...` later in the same
    # sentence must not turn a current claim into a future one.
    if ($beforeTriggerClause -match '(?:終われば|終えれば|完了すれば|した場合|になれば|すれば|できれば|これから|今後)[^。！？]*$' -or
        $fromTrigger -match ('^' + [regex]::Escape($Trigger) + '.{0,24}?(?:済んだ後|完了した後|終わった後|になる予定|となる予定|になる見込み|になるはず|になれば)')) {
        return 'future'
    }
    if ($beforeTriggerClause -match '(?:朝は|以前|当時|先ほど|修正前|変更前|直前は|過去(?:には|は|の))[^。！？]*$' -or
        $fromTrigger -match ('^' + [regex]::Escape($Trigger) + '.{0,32}?(?:だった|であった|と報告した|と述べた|と書いた)')) {
        return 'past'
    }
    return 'current'
}

function Test-RoadmapScopeDeniedMention {
    param(
        [Parameter(Mandatory = $true)][string]$Segment,
        [Parameter(Mandatory = $true)][string]$Trigger,
        [Parameter(Mandatory = $true)][int]$TriggerIndex
    )

    $escapedTrigger = [regex]::Escape($Trigger)
    $strictDisposition = '(?:誤り(?:だった|である|です)?|間違い(?:だった|である|です)?|事実ではない|成立しない|正しくない|取り下げ(?:る|た|ます)|訂正(?:する|した|します))'
    $quotedDenial = '[「『][^」』]*' + $escapedTrigger + '[^」』]*[」』]\s*(?:は|が|を)?\s*' + $strictDisposition
    foreach ($denialMatch in [regex]::Matches($Segment, $quotedDenial, [Text.RegularExpressions.RegexOptions]::CultureInvariant)) {
        if ($TriggerIndex -ge $denialMatch.Index -and $TriggerIndex -lt $denialMatch.Index + $denialMatch.Length) {
            return $true
        }
    }

    # Do not exempt a quotation by itself.  The sensitive expression and the
    # exact refusal/correction must occur in the same segment.
    $directDenial = '^' + $escapedTrigger + '.{0,48}?(?:(?:とは|とはまだ|と(?:は)?)(?:報告|断定|主張|記載|記録)?(?:しない|していない|しません|しなかった|していません|できない)|と書かなかった|は未確認|かは不明)'
    return [regex]::IsMatch($Segment.Substring($TriggerIndex), $directDenial, [Text.RegularExpressions.RegexOptions]::CultureInvariant)
}

function Get-RoadmapScopeAnchor {
    param(
        [Parameter(Mandatory = $true)][string]$Segment,
        [Parameter(Mandatory = $true)][string]$Kind,
        [Parameter(Mandatory = $true)][string]$Trigger,
        [Parameter(Mandatory = $true)][int]$TriggerIndex,
        [AllowNull()]$Count
    )

    $exactBounds = [regex]::Match($Segment, '(?i:Roadmap-Bounds)')
    if ($exactBounds.Success) {
        return [pscustomobject]@{ Scope = 'bounded'; Text = $exactBounds.Value }
    }

    # `(707行, sol)` のような監視定義の行番号はroadmap項目ではない。裸の`NNN行`を
    # bounded anchorにすると局所の残件断言をroadmapの限定主張へ誤って昇格させる。
    # 正本/ロードマップを直接名指しした行番号だけをbounded anchorとする。
    $roadmapLinePattern = '(?:正本|実装ロードマップ|ロードマップ)\s*[0-9]{3,4}\s*行'
    $boundedPattern = '(?i:(?:TEST|MANUAL|ADDITIONAL)\.[A-Z0-9][A-Z0-9._-]*)|(?i:M[0-9]+(?:\.[A-Z0-9][A-Z0-9_-]*)+)|' + $roadmapLinePattern
    $bounded = [regex]::Match($Segment, $boundedPattern, [Text.RegularExpressions.RegexOptions]::CultureInvariant)
    # `734行が完成し、残り12件になる` first names one item, then makes an
    # unbounded release remainder assertion.  The latter is whole, not bounded.
    $itemThenWholeRemainder = $Kind -eq 'remainder' -and $null -ne $Count -and
        $bounded.Success -and $bounded.Index -lt $TriggerIndex -and
        $Segment.Substring($bounded.Index + $bounded.Length, $TriggerIndex - ($bounded.Index + $bounded.Length)) -match '(?:完成|完了)'
    if ($bounded.Success -and -not $itemThenWholeRemainder) {
        return [pscustomobject]@{ Scope = 'bounded'; Text = $bounded.Value }
    }

    $escapedTrigger = [regex]::Escape($Trigger)
    $roadmapSubject = '(?:正本|実装ロードマップ|ロードマップ)'
    # `正本が求めた4/4` cites the source of a local criterion; it does not use
    # the roadmap as the asserted mother set.  Require the subject to lead
    # directly to the sensitive expression/state instead of accepting a bare
    # occurrence of the word 正本 anywhere in the sentence.
    $subjectAssertion = [regex]::Match(
        $Segment,
        $roadmapSubject + '(?!\s*(?:(?:の|は|が|では|について)\s*)?(?:求めた|定めた|指定した))' +
            '\s*(?:の|は|が|では|について)?\s*[^。！？]{0,32}?' + $escapedTrigger,
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if ($subjectAssertion.Success) {
        return [pscustomobject]@{ Scope = 'whole'; Text = $subjectAssertion.Value }
    }

    $releaseAnchor = [regex]::Match(
        $Segment,
        'リリース(?:まで)?(?:の)?(?:範囲|残作業|関門|準備|妨げ|停止理由)',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if ($releaseAnchor.Success) {
        return [pscustomobject]@{ Scope = 'whole'; Text = $releaseAnchor.Value }
    }
    $machineWhole = [regex]::Match($Segment, '(?i:Roadmap-(?:Snapshot|Progress))')
    if ($machineWhole.Success) {
        return [pscustomobject]@{ Scope = 'whole'; Text = $machineWhole.Value }
    }

    # These nouns are reserved for roadmap/release accounting even without an
    # adjacent explicit subject.  Numeric bare `残りN件` is handled only after
    # a possible finite local binder has been considered by the caller.
    $reservedWhole = [regex]::Match($Segment, '残作業(?:の本当の数)?|解消対象として残(?:す件数|る)')
    if ($reservedWhole.Success) {
        return [pscustomobject]@{ Scope = 'whole'; Text = $reservedWhole.Value }
    }
    return $null
}

function Get-RoadmapScopeLocalNounPattern {
    return '(?:担当|エージェント|検査|テスト|契約|標本|候補|方式|変異|ファイル|生成物|画像|参照|経路|プロセス|コマンド|故障注入|監視定義|版数|撮影|説明書|スレッド|thread\s*ID|計算|結果|出力|build|typecheck|lint|worker|フラップ|折り線|姿勢|三角形|面|層|辺|頂点|頂角|(?i:[A-Z0-9_.-]*(?:test|acceptance)[A-Z0-9_.-]*|passed|failed))'
}

function Get-ExplicitFiniteLocalContext {
    param([Parameter(Mandatory = $true)][string]$Segment)

    $localNoun = Get-RoadmapScopeLocalNounPattern
    $finitePatterns = @(
        ($localNoun + '\s*(?:は|が|の|を|で|中|:)?\s*(?<count>[0-9]+)\s*(?:件|人|名|本|種|組|枚|か所|箇所|面|層|項目|つ)?'),
        ('(?<count>[0-9]+)\s*(?:件|本|種|組|枚|人|名|面|層|項目)?(?:の)?\s*' + $localNoun),
        '(?:説明書|画像|撮影)\s*(?:は|が|の)?\s*(?<count>[0-9]+)\s*枚',
        '版数.{0,16}(?<count>[0-9]+)\s*(?:か所|箇所)',
        '(?<count>[0-9]+)\s*(?:人|名|枚|か所|箇所|層|面|種)'
    )
    foreach ($pattern in $finitePatterns) {
        $match = [regex]::Match($Segment, $pattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase -bor [Text.RegularExpressions.RegexOptions]::CultureInvariant)
        if ($match.Success) {
            return [pscustomobject]@{
                Text = $match.Value
                Total = [int]$match.Groups['count'].Value
                CanCarryRemainder = $Segment.Substring($match.Index, [Math]::Min($Segment.Length - $match.Index, $match.Length + 8)) -match '(?:中|のうち)'
            }
        }
    }

    # Generic counters (`件`, `項目`) are local only when the same segment
    # supplies a concrete non-roadmap topic, as in
    # `画面側の回帰確認: 5項目すべて` or `監視定義5件中の残り4件`.
    $topicCount = [regex]::Match($Segment, '(?<topic>[^。！？|:：]{2,40})[:：].{0,24}?[0-9]+\s*(?:件|項目)')
    if ($topicCount.Success -and $topicCount.Groups['topic'].Value -match '(?:画面|回帰|検査|監視|撮影|説明書|担当|故障|版数|生成|画像|参照)') {
        $countMatch = [regex]::Match($topicCount.Value, '(?<count>[0-9]+)\s*(?:件|項目)')
        return [pscustomobject]@{
            Text = $topicCount.Value
            Total = [int]$countMatch.Groups['count'].Value
            CanCarryRemainder = $Segment.Substring($topicCount.Index, [Math]::Min($Segment.Length - $topicCount.Index, $topicCount.Length + 8)) -match '(?:中|のうち)'
        }
    }
    $namedGenericCount = [regex]::Match($Segment, $localNoun + '.{0,20}?(?<count>[0-9]+)\s*(?:件|項目)')
    if ($namedGenericCount.Success) {
        return [pscustomobject]@{
            Text = $namedGenericCount.Value
            Total = [int]$namedGenericCount.Groups['count'].Value
            CanCarryRemainder = $Segment.Substring($namedGenericCount.Index, [Math]::Min($Segment.Length - $namedGenericCount.Index, $namedGenericCount.Length + 8)) -match '(?:中|のうち)'
        }
    }
    return $null
}

function Get-ExplicitFiniteLocalBinder {
    param([Parameter(Mandatory = $true)][string]$Segment)

    $context = Get-ExplicitFiniteLocalContext -Segment $Segment
    if ($null -eq $context) { return $null }
    return [string]$context.Text
}

function Get-ExplicitLocalBinder {
    param([Parameter(Mandatory = $true)][string]$Segment)

    $localNoun = Get-RoadmapScopeLocalNounPattern
    $finite = Get-ExplicitFiniteLocalBinder -Segment $Segment
    if ($null -ne $finite) {
        return $finite
    }

    # A concrete local subject may bind `all` without a numeric count.
    $directLocalSubject = [regex]::Match($Segment, $localNoun + '.{0,20}?(?:すべて|全て|全件)')
    if ($directLocalSubject.Success) {
        return $directLocalSubject.Value
    }
    $countThenNamedSubject = [regex]::Match($Segment, '[0-9]+\s*件.{0,20}?(?:すべて|全て|全件).{0,40}?' + $localNoun)
    if ($countThenNamedSubject.Success) {
        return $countThenNamedSubject.Value
    }

    # Markdown table rows and prose sometimes spell out the finite set as an
    # enumeration rather than a number: `build / typecheck / lint | all ...`.
    $enumeration = [regex]::Match($Segment, '(?:[\p{L}\p{N}_.+`-]+\s*(?:/|／|・)\s*){1,}[\p{L}\p{N}_.+`-]+.{0,32}?(?:すべて|全て|全件)')
    if ($enumeration.Success) {
        return $enumeration.Value
    }
    $localRatio = [regex]::Match($Segment, $localNoun + '.{0,48}?[0-9]+\s*[／/]\s*[0-9]+')
    if ($localRatio.Success) {
        return $localRatio.Value
    }
    $ratioThenLocal = [regex]::Match($Segment, '[0-9]+\s*[／/]\s*[0-9]+.{0,48}?' + $localNoun)
    if ($ratioThenLocal.Success) {
        return $ratioThenLocal.Value
    }
    return $null
}

function Get-ExplicitLocalBinderForTrigger {
    param(
        [Parameter(Mandatory = $true)][string]$Segment,
        [Parameter(Mandatory = $true)][string]$Kind,
        [Parameter(Mandatory = $true)][string]$Trigger,
        [Parameter(Mandatory = $true)][int]$TriggerIndex,
        [AllowNull()]$Count
    )

    $beforeTrigger = $Segment.Substring(0, $TriggerIndex)
    $clauseStart = 0
    foreach ($breaker in [regex]::Matches($beforeTrigger, '(?:だが|ですが|しかし|一方|ただし|ものの|けれども?|のに)[、，]')) {
        $clauseStart = [int]$breaker.Index + [int]$breaker.Length
    }
    $prefixWithTrigger = $Segment.Substring($clauseStart, $TriggerIndex + $Trigger.Length - $clauseStart)
    $beforeBinder = Get-ExplicitLocalBinder -Segment $prefixWithTrigger
    if ($null -ne $beforeBinder) {
        if ($Kind -ne 'remainder') {
            return $beforeBinder
        }
        $finite = Get-ExplicitFiniteLocalContext -Segment $prefixWithTrigger
        if ($null -ne $finite -and $null -ne $Count -and [int]$Count -le [int]$finite.Total) {
            return $beforeBinder
        }
    }

    # Some Japanese binders follow the quantifier (`すべての面`,
    # `全567ファイル`, `4/4で候補1件`).  Only a short, direct continuation
    # from this exact occurrence is accepted; a later unrelated noun cannot
    # retroactively turn an ambiguous expression into local scope.
    $fromTrigger = $Segment.Substring($TriggerIndex)
    # 逆接をまたいだ先の名詞は別の主題である。`5担当が稼働中だが、全件完了した`の
    # `5担当`と同じ理由で、後続側でも逆接より先は借りない。
    $afterBreaker = [regex]::Match($fromTrigger, '(?:だが|ですが|しかし|一方|ただし|ものの|けれども?|のに)[、，]')
    if ($afterBreaker.Success) {
        $fromTrigger = $fromTrigger.Substring(0, $afterBreaker.Index)
    }
    $localNoun = Get-RoadmapScopeLocalNounPattern
    $afterPattern = if ($Kind -eq 'progress') {
        '^' + [regex]::Escape($Trigger) + '.{0,24}?' + $localNoun
    }
    elseif ($Kind -eq 'universal') {
        # `N件すべてに、実際に送っているスレッドの識別子` のように、数で母集合を
        # 明示した全数だけが読点をまたいで直後の局所名詞へ結び付く。数の無い裸の
        # `全件` は読点をまたげず、`全件完了し、検査を止めた` はambiguousのままになる。
        $adjacentCount = [regex]::Match(
            $Segment.Substring(0, $TriggerIndex),
            '(?<![0-9])[0-9]+\s*(?:件|人|名|本|種|組|枚|か所|箇所|面|層|項目|つ|担当|契約|検査|回)?\s*(?:は|が|も|の)?\s*$'
        )
        if ($adjacentCount.Success) {
            '^' + [regex]::Escape($Trigger) + '(?:\s*の|\s*(?:[0-9]+)?)?[^。！？]{0,20}?' + $localNoun
        }
        else {
            '^' + [regex]::Escape($Trigger) + '(?:\s*の|\s*(?:[0-9]+)?)?[^。！？、，]{0,12}?' + $localNoun
        }
    }
    else {
        # remainderは緩めない。`残り11件だけで、検査は完了した` のような文で
        # 後続の局所名詞を借りると、roadmap全体の残件断言を見逃す。
        '^' + [regex]::Escape($Trigger) + '(?:\s*の|\s*(?:[0-9]+)?)?\s*' + $localNoun
    }
    $afterBinder = [regex]::Match($fromTrigger, $afterPattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase -bor [Text.RegularExpressions.RegexOptions]::CultureInvariant)
    if ($afterBinder.Success) {
        return $afterBinder.Value
    }
    return $null
}

function Get-CountedExecutionResultBinder {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$NormalizedLine,
        [Parameter(Mandatory = $true)][string]$Segment,
        [Parameter(Mandatory = $true)][string]$Kind,
        [AllowNull()]$Count
    )

    # `10 passed / 15 failed / 4 ignored。15件はすべて同じ頭の呼び出しで止まった`の
    # ように、同じ行が実行結果としてN件を報告し、断言側もN件を明示している場合だけ
    # 有限の母集合とみなす。数がどこにも報告されていなければ借りない。
    if ($Kind -ne 'universal' -or $null -eq $Count) {
        return $null
    }
    $countText = [regex]::Escape([string]$Count)
    if ($Segment -notmatch ('(?<![0-9])' + $countText + '\s*件')) {
        return $null
    }
    $context = [regex]::Match(
        $NormalizedLine,
        '(?i:[A-Z0-9_.-]*(?:test|acceptance)[A-Z0-9_.-]*|passed|failed|ignored|skipped)|終了コード'
    )
    if (-not $context.Success) {
        return $null
    }
    $executionCount = [regex]::Match(
        $NormalizedLine,
        '(?<![0-9])' + $countText + '\s*(?:passed|failed|ignored|skipped)\b',
        [Text.RegularExpressions.RegexOptions]::IgnoreCase -bor [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if (-not $executionCount.Success) {
        return $null
    }
    return "same-line-execution-result:$($context.Value):$($executionCount.Value)"
}

function New-RoadmapScopeAssertion {
    param(
        [Parameter(Mandatory = $true)][string]$Scope,
        [Parameter(Mandatory = $true)][string]$Kind,
        [Parameter(Mandatory = $true)][string]$Segment,
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Reason,
        [Parameter(Mandatory = $true)][int]$Line,
        [Parameter(Mandatory = $true)][string]$Trigger,
        [AllowNull()]$Count,
        [Parameter(Mandatory = $true)][string]$Temporal
    )

    $allowedScopes = @('whole', 'bounded', 'local', 'denied-mention', 'ambiguous')
    $allowedKinds = @('universal', 'remainder', 'progress')
    if ($allowedScopes -notcontains $Scope) {
        throw "invalid roadmap scope: $Scope"
    }
    if ($allowedKinds -notcontains $Kind) {
        throw "invalid roadmap scope kind: $Kind"
    }
    return [pscustomobject][ordered]@{
        Scope    = $Scope
        Kind     = $Kind
        Segment  = $Segment
        Text     = $Text
        Reason   = $Reason
        Line     = $Line
        Trigger  = $Trigger
        Count    = $Count
        Temporal = $Temporal
    }
}

function Get-RoadmapScopeAssertions {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true, Position = 0, ValueFromPipeline = $true)]
        [AllowEmptyCollection()]
        [AllowEmptyString()]
        [AllowNull()]
        [string[]]$Text,

        [ValidateRange(1, 2147483647)]
        [int]$StartLine = 1,

        [ValidateRange(0, 2147483647)]
        [int]$RoadmapTotal = 0
    )

    begin {
        $inputValues = New-Object System.Collections.Generic.List[string]
    }
    process {
        foreach ($value in @($Text)) {
            $inputValues.Add([string]$value)
        }
    }
    end {
        $lines = @(ConvertTo-RoadmapScopeLines -Values $inputValues.ToArray())
        $findings = New-Object System.Collections.Generic.List[object]
        $normalizationForm = [Text.NormalizationForm]::FormKC
        $universalPattern = 'すべて|全て|全件|全部完了|これで全部|これが全部'
        # `錠が残り、次の起動...` is a verb, not a remainder assertion.
        # Require a noun-like continuation for the otherwise ambiguous `残り`.
        $remainderPattern = '残作業(?:の本当の数)?|残件|[0-9]+\s*件\s*残り|残り(?=(?:は|が|の|を|も|だけ|のみ|[:：]|\s*[0-9]))|未チェック(?:件数)?|未完了(?:件数)?|解消対象として残(?:す件数|る)'
        # A ratio is scope-sensitive only when its grammar ties it to roadmap
        # accounting.  Arbitrary test/hash ratios (87/102, 4/4, 1/200, ...)
        # are not completeness words.  The current audited total supplied by
        # the caller makes the otherwise terse `175/186` unambiguous.
        $explicitRoadmapRatio = '(?:正本|実装ロードマップ|ロードマップ)(?!\s*(?:(?:の|は|が|では|について)\s*)?(?:求めた|定めた|指定した))\s*(?:の|は|が|では)?\s*[^。！？]{0,24}?[0-9]+\s*[／/]\s*[0-9]+'
        $finishedRemainingAccounting = '[0-9]+\s*件済み\s*[／/]\s*[0-9]+\s*件残り'
        $currentTotalRatio = '(?!)'
        if ($RoadmapTotal -gt 0) {
            $currentTotalRatio = '(?<![0-9／/])[0-9]+\s*[／/]\s*' + [regex]::Escape([string]$RoadmapTotal) + '(?![0-9／/])'
        }
        $progressPattern = '進捗率|' + $finishedRemainingAccounting + '|' + $explicitRoadmapRatio + '|' + $currentTotalRatio

        for ($lineIndex = 0; $lineIndex -lt $lines.Count; $lineIndex++) {
            $line = [string]$lines[$lineIndex]
            if ([string]::IsNullOrWhiteSpace($line)) {
                continue
            }
            $normalizedLine = $line.Normalize($normalizationForm)
            $pastRoadmapAccountingContext = $normalizedLine -match '(?:朝は|当時|以前)[^。！？]*[0-9]+\s*[／/]\s*[0-9]+\s*[／/]\s*[0-9]+[^。！？]*(?:だった|であった)[。！？][^。！？]*(?:増やした|減らした|進んだ|後退した)(?:うえで|後)[^。！？]*残り(?:は|が)(?:減|増)'
            $previousLocalContext = $null
            foreach ($segmentInfo in @(Get-RoadmapScopeSegments -Line $line -NormalizedLine $normalizedLine)) {
                $segment = [string]$segmentInfo.Normalized
                $segmentLocalContext = Get-ExplicitFiniteLocalContext -Segment $segment
                $segmentTestContext = [regex]::Match(
                    $segment,
                    '(?i:[A-Z0-9_.-]*(?:test|acceptance)[A-Z0-9_.-]*|passed|failed)|終了コード'
                )
                $triggerDefinitions = @(
                    [pscustomobject]@{ Kind = 'universal'; Pattern = $universalPattern },
                    [pscustomobject]@{ Kind = 'remainder'; Pattern = $remainderPattern },
                    [pscustomobject]@{ Kind = 'progress'; Pattern = $progressPattern }
                )

                foreach ($definition in $triggerDefinitions) {
                    foreach ($triggerMatch in [regex]::Matches($segment, [string]$definition.Pattern)) {
                        $trigger = [string]$triggerMatch.Value
                        $count = Get-RoadmapScopeCount -Segment $segment -Kind $definition.Kind -Trigger $trigger -TriggerIndex $triggerMatch.Index
                        if (Test-RoadmapScopeDeniedMention -Segment $segment -Trigger $trigger -TriggerIndex $triggerMatch.Index) {
                            $findings.Add((New-RoadmapScopeAssertion `
                                -Scope 'denied-mention' -Kind $definition.Kind -Segment $segment -Text $segmentInfo.Original `
                                -Reason "strict-denial-or-correction:$trigger" -Line ($StartLine + $lineIndex) `
                                -Trigger $trigger -Count $count -Temporal 'denied'))
                            continue
                        }

                        $temporal = Get-RoadmapScopeTemporal -Segment $segment -TriggerIndex $triggerMatch.Index -Trigger $trigger
                        $isPastTransitionSegment = $segment -match '(?:増やした|減らした|進んだ|後退した)(?:うえで|後).{0,40}残り(?:は|が)(?:減|増)'
                        if ($temporal -eq 'current' -and $pastRoadmapAccountingContext -and $isPastTransitionSegment -and $definition.Kind -eq 'remainder') {
                            $temporal = 'past'
                        }
                        $roadmapAnchor = Get-RoadmapScopeAnchor `
                            -Segment $segment -Kind $definition.Kind -Trigger $trigger `
                            -TriggerIndex $triggerMatch.Index -Count $count
                        if ($null -eq $roadmapAnchor -and $pastRoadmapAccountingContext -and $isPastTransitionSegment -and $definition.Kind -eq 'remainder') {
                            $roadmapAnchor = [pscustomobject]@{
                                Scope = 'whole'
                                Text = 'past-roadmap-accounting-transition'
                            }
                        }
                        if ($null -ne $roadmapAnchor) {
                            $findings.Add((New-RoadmapScopeAssertion `
                                -Scope $roadmapAnchor.Scope -Kind $definition.Kind -Segment $segment -Text $segmentInfo.Original `
                                -Reason "explicit-roadmap-$($roadmapAnchor.Scope)-anchor:$($roadmapAnchor.Text)" `
                                -Line ($StartLine + $lineIndex) -Trigger $trigger -Count $count -Temporal $temporal))
                            continue
                        }

                        $localBinder = Get-ExplicitLocalBinderForTrigger `
                            -Segment $segment -Kind $definition.Kind -Trigger $trigger `
                            -TriggerIndex $triggerMatch.Index -Count $count
                        if ($null -eq $localBinder -and $definition.Kind -eq 'remainder' -and
                            $null -ne $count -and $null -ne $previousLocalContext -and
                            [bool]$previousLocalContext.CanCarryRemainder -and
                            [int]$count -le [int]$previousLocalContext.Total) {
                            $localBinder = "previous-finite-local-context:$($previousLocalContext.Text)"
                        }
                        if ($null -eq $localBinder -and $definition.Kind -eq 'universal' -and
                            $segmentTestContext.Success -and $null -ne $count -and
                            $segment -match ('(?<![0-9])' + [regex]::Escape([string]$count) + '\s*(?:件|passed|failed)')) {
                            $localBinder = "same-segment-counted-test-context:$($segmentTestContext.Value):$count"
                        }
                        if ($null -eq $localBinder) {
                            $localBinder = Get-CountedExecutionResultBinder `
                                -NormalizedLine $normalizedLine -Segment $segment `
                                -Kind $definition.Kind -Count $count
                        }
                        if ($null -ne $localBinder) {
                            $findings.Add((New-RoadmapScopeAssertion `
                                -Scope 'local' -Kind $definition.Kind -Segment $segment -Text $segmentInfo.Original `
                                -Reason "explicit-finite-local-binder:$localBinder" -Line ($StartLine + $lineIndex) `
                                -Trigger $trigger -Count $count -Temporal $temporal))
                            continue
                        }

                        if ($definition.Kind -eq 'remainder' -and $null -ne $count) {
                            $findings.Add((New-RoadmapScopeAssertion `
                                -Scope 'whole' -Kind $definition.Kind -Segment $segment -Text $segmentInfo.Original `
                                -Reason 'explicit-roadmap-whole-anchor:bare-numeric-remainder' `
                                -Line ($StartLine + $lineIndex) -Trigger $trigger -Count $count -Temporal $temporal))
                            continue
                        }
                        if ($definition.Kind -eq 'progress') {
                            $ratio = [regex]::Match($trigger, '(?<checked>[0-9]+)\s*[／/]\s*(?<total>[0-9]+)')
                            $isCurrentTotal = $ratio.Success -and $RoadmapTotal -gt 0 -and
                                [int]$ratio.Groups['total'].Value -eq $RoadmapTotal
                            $isFinishedRemaining = $trigger -match '件済み.*件残り'
                            if ($isCurrentTotal -or $isFinishedRemaining) {
                                $anchorText = if ($isCurrentTotal) { "current-roadmap-total:$RoadmapTotal" } else { 'finished-remaining-accounting' }
                                $findings.Add((New-RoadmapScopeAssertion `
                                    -Scope 'whole' -Kind $definition.Kind -Segment $segment -Text $segmentInfo.Original `
                                    -Reason "explicit-roadmap-whole-anchor:$anchorText" `
                                    -Line ($StartLine + $lineIndex) -Trigger $trigger -Count $count -Temporal $temporal))
                                continue
                            }
                        }

                        $findings.Add((New-RoadmapScopeAssertion `
                            -Scope 'ambiguous' -Kind $definition.Kind -Segment $segment -Text $segmentInfo.Original `
                            -Reason 'no-explicit-scope-anchor' -Line ($StartLine + $lineIndex) `
                            -Trigger $trigger -Count $count -Temporal $temporal))
                    }
                }
                if ($null -ne $segmentLocalContext -and [bool]$segmentLocalContext.CanCarryRemainder) {
                    $previousLocalContext = $segmentLocalContext
                }
                else {
                    $previousLocalContext = $null
                }
            }
        }
        return $findings.ToArray()
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    if (-not $PSBoundParameters.ContainsKey('Text')) {
        throw 'Specify -Text when invoking roadmap-claim-scope.ps1 directly.'
    }
    $result = @(Get-RoadmapScopeAssertions -Text $Text -StartLine $StartLine -RoadmapTotal $RoadmapTotal)
    if ($AsJson) {
        if ($result.Count -eq 0) {
            Write-Output '[]'
        }
        else {
            Write-Output ($result | ConvertTo-Json -Depth 5 -Compress)
        }
    }
    else {
        Write-Output $result
    }
}
