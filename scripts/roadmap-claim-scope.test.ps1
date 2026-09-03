[CmdletBinding()]
param(
    [string]$ScopeScriptPath = '',
    [switch]$ContractOnly
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ScopeScriptPath)) {
    $ScopeScriptPath = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) 'roadmap-claim-scope.ps1'
}
$script:Assertions = 0
$script:FixtureAssertions = 0
$script:AllowedScopes = @('whole', 'bounded', 'local', 'quoted-instruction', 'denied-mention', 'ambiguous')
$script:AllowedKinds = @('universal', 'remainder', 'progress')
$script:AllowedTemporal = @('current', 'past', 'future', 'denied')
$script:RoadmapTotal = 186

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $script:Assertions++
    if (-not $Condition) {
        throw "assertion failed: $Message"
    }
}

function Assert-Equal {
    param(
        [AllowNull()]$Expected,
        [AllowNull()]$Actual,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $script:Assertions++
    $equal = if ($null -eq $Expected -and $null -eq $Actual) {
        $true
    }
    elseif ($null -eq $Expected -or $null -eq $Actual) {
        $false
    }
    else {
        [object]::Equals($Expected, $Actual)
    }
    if (-not $equal) {
        throw "assertion failed: $Message (expected=$Expected actual=$Actual)"
    }
}

function Assert-AstClean {
    param([Parameter(Mandatory = $true)][string]$Path)

    $tokens = $null
    $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile(
        $Path,
        [ref]$tokens,
        [ref]$errors
    )
    Assert-Equal 0 $errors.Count "PowerShell AST errors: $Path"
}

function Get-CheckedFindings {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string[]]$Text,
        [int]$StartLine = 1
    )

    $findings = @(Get-RoadmapScopeAssertions -Text $Text -StartLine $StartLine -RoadmapTotal $script:RoadmapTotal)
    foreach ($finding in $findings) {
        foreach ($propertyName in @('Scope', 'Kind', 'Segment', 'Text', 'Reason', 'Line', 'Trigger', 'Count', 'Temporal')) {
            Assert-True ($finding.PSObject.Properties.Name -contains $propertyName) "finding property exists: $propertyName"
        }
        Assert-True ($script:AllowedScopes -contains [string]$finding.Scope) "scope enum: $($finding.Scope)"
        Assert-True ($script:AllowedKinds -contains [string]$finding.Kind) "kind enum: $($finding.Kind)"
        Assert-True ($script:AllowedTemporal -contains [string]$finding.Temporal) "temporal enum: $($finding.Temporal)"
        Assert-True (-not [string]::IsNullOrWhiteSpace([string]$finding.Segment)) 'Segment is non-empty'
        Assert-True (-not [string]::IsNullOrWhiteSpace([string]$finding.Text)) 'Text is non-empty'
        Assert-True (-not [string]::IsNullOrWhiteSpace([string]$finding.Reason)) 'Reason is non-empty'
        Assert-True (-not [string]::IsNullOrWhiteSpace([string]$finding.Trigger)) 'Trigger is non-empty'
        Assert-True ([int]$finding.Line -ge $StartLine) 'Line is at or after StartLine'
        if ($null -ne $finding.Count) {
            Assert-True ($finding.Count -is [int]) 'Count is int or null'
            Assert-True ([int]$finding.Count -ge 0) 'Count is non-negative'
        }
    }
    return $findings
}

function Assert-SingleFinding {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string[]]$Text,
        [Parameter(Mandatory = $true)][string]$Scope,
        [Parameter(Mandatory = $true)][string]$Kind,
        [Parameter(Mandatory = $true)][string]$Temporal,
        [string]$TriggerPattern = '.*',
        [AllowNull()]$Count,
        [int]$StartLine = 1,
        [int]$ExpectedLine = 1,
        [string]$Label = 'finding'
    )

    $findings = @(Get-CheckedFindings -Text $Text -StartLine $StartLine)
    $matching = @($findings | Where-Object {
        $_.Scope -eq $Scope -and $_.Kind -eq $Kind -and $_.Temporal -eq $Temporal -and $_.Trigger -match $TriggerPattern
    })
    Assert-Equal 1 $matching.Count "$Label matching finding count"
    $finding = $matching[0]
    Assert-Equal $Scope ([string]$finding.Scope) "$Label scope"
    Assert-Equal $Kind ([string]$finding.Kind) "$Label kind"
    Assert-Equal $Temporal ([string]$finding.Temporal) "$Label temporal"
    Assert-Equal $ExpectedLine ([int]$finding.Line) "$Label line"
    if ($PSBoundParameters.ContainsKey('Count')) {
        Assert-Equal $Count $finding.Count "$Label count"
    }
    return $finding
}

function Assert-NoFindings {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string[]]$Text,
        [string]$Label = 'no findings'
    )

    $findings = @(Get-CheckedFindings -Text $Text)
    Assert-Equal 0 $findings.Count $Label
}

function Test-NoneCompatible {
    param([Parameter(Mandatory = $true)][object[]]$Findings)

    foreach ($finding in @($Findings)) {
        if ($finding.Scope -eq 'ambiguous') {
            return $false
        }
        if (($finding.Scope -eq 'whole' -or $finding.Scope -eq 'bounded') -and $finding.Temporal -eq 'current') {
            return $false
        }
    }
    return $true
}

function Assert-NoneCompatible {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string[]]$Text,
        [Parameter(Mandatory = $true)][bool]$Expected,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $findings = @(Get-CheckedFindings -Text $Text)
    Assert-Equal $Expected (Test-NoneCompatible -Findings $findings) "$Label none compatibility"
}

function Invoke-ContractAssertions {
    $resolvedScopeScript = (Resolve-Path -LiteralPath $ScopeScriptPath).Path
    Assert-AstClean -Path $resolvedScopeScript
    Assert-AstClean -Path $PSCommandPath

    . $resolvedScopeScript
    Assert-True ($null -ne (Get-Command Get-RoadmapScopeAssertions -CommandType Function -ErrorAction SilentlyContinue)) 'dot-source exports Get-RoadmapScopeAssertions'

    # The two phrases quoted by the user: the finite group of five agents is
    # local, and merely explaining that the subject is five people has no
    # roadmap-completeness trigger at all.
    [void](Assert-SingleFinding -Text '5担当すべて稼働中' -Scope local -Kind universal -Temporal current -TriggerPattern '^すべて$' -Count 5 -Label 'user quote: five agents')
    Assert-NoFindings -Text '担当5人について言っているだけ' -Label 'user quote: five people only'

    # Eight production records that were falsely rejected by the former bare
    # `all/every` regex.  Fixtures are copied verbatim from their sensitive
    # lines rather than replaced with synthetic wording.
    $recordFixtures = @(
        [pscustomobject]@{
            Name = '2026-09-01 13:05 proposal candidates'
            Lines = @(
                '`acceptance_crane` は終了コード101、10 passed / 15 failed / 4 ignored。**15件はすべて同じ頭の呼び出しで止まっており、無関係な赤は0件。**ただし**15件が全部直るかは不明**で、担当もそう書いている。',
                '拒否時の原子性を検査で固定し、故障注入4本すべてが終了101で赤くなることを確認済み。'
            )
        },
        [pscustomobject]@{
            Name = '2026-09-01 10:56 line accounting'
            Lines = @('**確認したこと**: 全567ファイルで製品+テスト=合計の一致(不一致0件)。外部`#[cfg(test)] mod X;`宣言は今回も`surface_order_acceptance`の1件のみ(213件の.rsファイル全件・85箇所の`#[cfg(test)]`属性を機械的に走査して確認)。`vendor/`・`verification/`配下は今回も0件。')
        },
        [pscustomobject]@{
            Name = '2026-09-01 10:35 crane'
            Lines = @('**`crates/ori3-layers/src/techniques.rs:233` の門である。**中割り折りは、選んだフラップの**すべての面**について「折り線が面を横切っているか」を先に確かめ、1面でも横切っていなければ技法ごと断る。')
        },
        [pscustomobject]@{
            Name = '2026-09-01 10:19 tool rail'
            Lines = @(
                '### 画面側の回帰確認: 5項目すべて保たれていた',
                '| 1 workerでの全件 | **2,350 passed / 1 skipped / 終了0** |',
                '**撮影42枚は、実機確認と必要な修正・再確認がすべて済んだ後に行う。**先に撮ると撮り直しになる。'
            )
        },
        [pscustomobject]@{
            Name = '2026-09-01 07:55 proposal timing and manual'
            Lines = @(
                '## 2026-09-01 07:55 — 「提案が時間内に終わらない」は統括の誤り。説明書は49枚すべて撮り直しになる',
                '故障注入3件               すべて終了コード1、復元済み',
                '> **現在の監視定義5件中、実thread IDを名前に持つのは `01a055cf` の1件だけでした。残り4件は `(707行, sol)` 等で安全に対応づけられません。**',
                '**5件すべてに、実際に送っているスレッドの識別子を入れ直した。**次の走査から全担当に効く。',
                '### 説明書は49枚すべて撮り直しになる'
            )
        },
        [pscustomobject]@{
            Name = '2026-09-01 04:23 stalled agents'
            Lines = @(
                '**走査が `2026-08-31 14:32:00Z` から `19:22:10Z` へ飛んでいる。**その間、5担当すべてが停滞と判定された。',
                '**理由。**厳密な道が2つとも閉じた。そして担当自身が「出力形状への実害はありません」と実測で述べている。**点の役目はどのフラップかを指すことだけで、同じフラップを指すなら以後の計算はすべて同じになる。**',
                '**朝は 168/16/184 だった。項目を2つ増やしたうえで、残りが減っている。**'
            )
        },
        [pscustomobject]@{
            Name = '2026-08-31 22:30 browser integration'
            Lines = @(
                '| 旧形式の作品 | **4契約すべて合格** |',
                '| build / typecheck / lint | すべて終了0 |'
            )
        },
        [pscustomobject]@{
            Name = '2026-08-31 21:43 autosave'
            Lines = @(
                '**そこでWindowsの標準機能 `share_mode(0)` に切り替えた。**`fs2` に期待していた「異常終了の瞬間にOSが錠を外す」性質は同じである。**自前で錠のファイルを作る方式だと落ちたとき錠が残り、次の起動が永久に待たされる。**この性質が要るので、標準機能で代替できた。',
                '**252件の子プロセスの終了までは確認したが、そこで止めて「全件成功」と書かなかった。**進捗を自己申告で報告して89/106と述べ実際は74/106だった過去の失敗と、逆の振る舞いである。'
            )
        }
    )

    $fixtureFindingCount = 0
    foreach ($record in $recordFixtures) {
        $recordFindings = @(Get-CheckedFindings -Text $record.Lines)
        Assert-True $recordFindings.Count "$($record.Name) has scope-sensitive fixture findings"
        Assert-True (Test-NoneCompatible -Findings $recordFindings) "$($record.Name) remains compatible with Roadmap-Claim: none"
        $fixtureFindingCount += $recordFindings.Count
        $script:FixtureAssertions += 2
    }
    Assert-Equal 8 $recordFixtures.Count 'eight production false-positive records are fixed as fixtures'
    Assert-True ($fixtureFindingCount -ge 17) 'all scope-sensitive expressions from eight records'

    # Re-read each complete production record.  Hand-picking only the former
    # `all/every` line would miss a new trigger elsewhere in the same record.
    $reportPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'docs/報告記録.md'
    $utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
    $reportLines = [System.IO.File]::ReadAllLines($reportPath, $utf8Strict)
    $productionHeadings = @(
        '## 2026-09-01 13:05 — 提案の4候補は「本当に閉じない」と確定。折り鶴は数字の出る解を捨てた',
        '## 2026-09-01 10:56 — 行数集計をコミット済み内容(HEAD `1e40fdb`)だけで数え直した',
        '## 2026-09-01 10:35 — 折り鶴の原因を確定。統括が構成を指定せず3時間を空費させた',
        '## 2026-09-01 10:19 — ツールレールの整理と撮影台本を保存。実機確認6件の順番が確定',
        '## 2026-09-01 07:55 — 「提案が時間内に終わらない」は統括の誤り。説明書は49枚すべて撮り直しになる',
        '## 2026-09-01 04:23 — 統括が4時間45分止まり、5担当を待たせた。折り鶴の厳密判定は両方向とも行き止まり',
        '## 2026-08-31 22:30 — ブラウザ版が完成。めり込み検査と提案の比較も完了。7つ目の「守っていない検査」',
        '## 2026-08-31 21:43 — 自動保存のデータ喪失を塞いだ。監視が落ちていたので立て直した'
    )
    $productionFindingCount = 0
    foreach ($heading in $productionHeadings) {
        $indices = New-Object System.Collections.Generic.List[int]
        for ($lineIndex = 0; $lineIndex -lt $reportLines.Count; $lineIndex++) {
            if ([string]::Equals($reportLines[$lineIndex], $heading, [StringComparison]::Ordinal)) {
                $indices.Add($lineIndex)
            }
        }
        Assert-Equal 1 $indices.Count "production heading is unique: $heading"
        $start = [int]$indices[0]
        $end = $reportLines.Count
        for ($lineIndex = $start + 1; $lineIndex -lt $reportLines.Count; $lineIndex++) {
            if ($reportLines[$lineIndex].StartsWith('## ', [StringComparison]::Ordinal)) {
                $end = $lineIndex
                break
            }
        }
        $recordLines = @($reportLines[$start])
        if ($end -gt $start + 1) {
            $recordLines += @($reportLines[($start + 1)..($end - 1)])
        }
        $recordFindings = @(Get-CheckedFindings -Text $recordLines -StartLine ($start + 1))
        Assert-True ($recordFindings.Count -gt 0) "production record has findings: $heading"
        Assert-True (Test-NoneCompatible -Findings $recordFindings) "production record is none-compatible: $heading"
        $productionFindingCount += $recordFindings.Count
    }
    Assert-Equal 8 $productionHeadings.Count 'eight complete production records were checked'

    # Pin every distinct expression class from those records, not just the
    # aggregate none-compatible result.
    [void](Assert-SingleFinding -Text '15件はすべて同じ頭の呼び出しで止まった。acceptance_crane: 10 passed / 15 failed。' -Scope local -Kind universal -Temporal current -Count 15 -Label 'test failure set')
    [void](Assert-SingleFinding -Text '故障注入4本すべてが終了101で赤くなる。' -Scope local -Kind universal -Temporal current -Count 4 -Label 'fault injections')
    [void](Assert-SingleFinding -Text '全567ファイルを照合し、213件の.rsファイル全件を走査した。' -Scope local -Kind universal -Temporal current -Count 213 -Label 'source files')
    [void](Assert-SingleFinding -Text '選んだフラップのすべての面について確かめた。' -Scope local -Kind universal -Temporal current -Label 'flap faces')
    [void](Assert-SingleFinding -Text '画面側の回帰確認: 5項目すべて保たれていた' -Scope local -Kind universal -Temporal current -Count 5 -Label 'viewer regression')
    [void](Assert-SingleFinding -Text '| 1 workerでの全件 | 2,350 passed / 1 skipped / 終了0 |' -Scope local -Kind universal -Temporal current -Label 'one worker all tests')
    [void](Assert-SingleFinding -Text '撮影42枚は、実機確認と必要な修正・再確認がすべて済んだ後に行う。' -Scope local -Kind universal -Temporal future -Count 42 -Label 'future capture condition')
    [void](Assert-SingleFinding -Text '説明書は49枚すべて撮り直しになる' -Scope local -Kind universal -Temporal current -Count 49 -Label 'manual images')
    [void](Assert-SingleFinding -Text '故障注入3件 すべて終了コード1、復元済み' -Scope local -Kind universal -Temporal current -Count 3 -Label 'three injected faults')
    [void](Assert-SingleFinding -Text '現在の監視定義5件中、1件だけが実IDで、残り4件は対応づけられない。' -Scope local -Kind remainder -Temporal current -Count 4 -Label 'watch definitions remainder')
    [void](Assert-SingleFinding -Text '5件すべてに、実際のスレッドの識別子を入れ直した。' -Scope local -Kind universal -Temporal current -Count 5 -Label 'five watch entries')
    [void](Assert-SingleFinding -Text 'その間、5担当すべてが停滞と判定された。' -Scope local -Kind universal -Temporal current -Count 5 -Label 'stalled agents')
    [void](Assert-SingleFinding -Text '同じフラップを指すなら以後の計算はすべて同じになる。' -Scope local -Kind universal -Temporal current -Label 'same calculations')
    [void](Assert-SingleFinding -Text '朝は 168/16/184 だった。項目を2つ増やしたうえで、残りが減っている。' -Scope whole -Kind remainder -Temporal past -Count $null -Label 'past roadmap transition')
    [void](Assert-SingleFinding -Text '| 旧形式の作品 | 4契約すべて合格 |' -Scope local -Kind universal -Temporal current -Count 4 -Label 'legacy contracts')
    [void](Assert-SingleFinding -Text '| build / typecheck / lint | すべて終了0 |' -Scope local -Kind universal -Temporal current -Label 'build checks')
    [void](Assert-SingleFinding -Text '252件の子プロセスを見たが「全件成功」とは報告しない。' -Scope denied-mention -Kind universal -Temporal denied -Count 252 -Label 'not claiming all passed')
    Assert-NoFindings -Text '自前で錠のファイルを作る方式だと落ちたとき錠が残り、次の起動が永久に待たされる。' -Label 'verb nokori is not remainder noun'

    # Fail-closed grammar and roadmap precedence.
    [void](Assert-SingleFinding -Text '正本の残り11件' -Scope whole -Kind remainder -Temporal current -Count 11 -Label 'whole remainder')
    [void](Assert-SingleFinding -Text '正本は175/186' -Scope whole -Kind progress -Temporal current -Count 175 -Label 'whole accounting ratio')
    [void](Assert-SingleFinding -Text '175/186' -Scope whole -Kind progress -Temporal current -Count 175 -Label 'bare current roadmap accounting ratio')
    [void](Assert-SingleFinding -Text '175件済み／11件残り' -Scope whole -Kind progress -Temporal current -Count 175 -Label 'finished remaining accounting')
    [void](Assert-SingleFinding -Text '残り11件' -Scope whole -Kind remainder -Temporal current -Count 11 -Label 'bare numeric remainder')
    [void](Assert-SingleFinding -Text '正本の5担当すべてが稼働中' -Scope whole -Kind universal -Temporal current -Count 5 -Label 'roadmap anchor wins over local binder')
    [void](Assert-SingleFinding -Text '正本734行の3工程すべて完了' -Scope bounded -Kind universal -Temporal current -Count 3 -Label 'roadmap line is bounded')
    [void](Assert-SingleFinding -Text 'Roadmap-Bounds の対象ID内に限定した残りは1件だけです。' -Scope bounded -Kind remainder -Temporal current -Count 1 -Label 'existing bounded wording')
    [void](Assert-SingleFinding -Text 'これが終われば734行が完成し、残りが12件になる。' -Scope whole -Kind remainder -Temporal future -Count 12 -Label 'future whole transition')
    [void](Assert-SingleFinding -Text 'すべて終わった' -Scope ambiguous -Kind universal -Temporal current -Label 'bare universal is ambiguous')
    [void](Assert-SingleFinding -Text '15件すべて終わった' -Scope ambiguous -Kind universal -Temporal current -Count 15 -Label 'generic finite count remains ambiguous')
    [void](Assert-SingleFinding -Text '「正本はすべて完了」と報告した' -Scope whole -Kind universal -Temporal past -Label 'quotation alone is not denial')
    [void](Assert-SingleFinding -Text '「9件すべて塞いだ」は誤りだった' -Scope denied-mention -Kind universal -Temporal denied -Count 9 -Label 'strict correction')
    [void](Assert-SingleFinding -Text @('```text', '正本の残り11件', '```') -Scope whole -Kind remainder -Temporal current -Count 11 -StartLine 40 -ExpectedLine 41 -Label 'code fence cannot hide whole claim')
    [void](Assert-SingleFinding -Text '５担当すべて稼働中' -Scope local -Kind universal -Temporal current -Count 5 -Label 'NFKC full-width finite binder')
    [void](Assert-SingleFinding -Text 'single/flap/all は1/2/4面すべて一致' -Scope local -Kind universal -Temporal current -Count 4 -Label 'local ratio is not whole accounting')
    Assert-NoFindings -Text 'single/flap/all は1/2/4面である' -Label 'naked local ratio has no roadmap finding'
    Assert-NoFindings -Text 'hash更新は48/31である' -Label 'unrelated ratio is not a roadmap trigger'

    $denialOccurrences = @(Get-CheckedFindings -Text '「全件成功」とは報告しないが、全件は完了した。')
    Assert-Equal 2 $denialOccurrences.Count 'denial is bound to one trigger occurrence'
    Assert-Equal 'denied-mention' ([string]$denialOccurrences[0].Scope) 'first occurrence is denied'
    Assert-True ([string]$denialOccurrences[1].Scope -ne 'denied-mention') 'second occurrence is not denied'
    [void](Assert-SingleFinding -Text '正本の残り11件だが、別の修正をした場合は再測定する。' -Scope whole -Kind remainder -Temporal current -Count 11 -Label 'later condition cannot change current claim')
    [void](Assert-SingleFinding -Text '監視定義5件中、実IDは1件だけでした。残り4件は対応づけられません。' -Scope local -Kind remainder -Temporal current -Count 4 -Label 'immediate finite local remainder carry')
    [void](Assert-SingleFinding -Text '残り11件です。5担当すべて稼働中。' -Scope whole -Kind remainder -Temporal current -Count 11 -Label 'later local binder cannot rescue earlier remainder')
    [void](Assert-SingleFinding -Text '5担当が稼働中だが、全件完了した。' -Scope ambiguous -Kind universal -Temporal current -Label 'unrelated local noun cannot rescue ambiguous universal')

    # 統括が指定した追加の正例・負例。scope分類そのものを固定する。
    [void](Assert-SingleFinding -Text '正本186項目すべて完了' -Scope whole -Kind universal -Temporal current -Count 186 -Label 'negative: whole roadmap completion')
    [void](Assert-SingleFinding -Text 'ロードマップ未完了14/186' -Scope whole -Kind remainder -Temporal current -Count 14 -Label 'negative: roadmap unchecked accounting')
    [void](Assert-SingleFinding -Text '残り2件だけ' -Scope whole -Kind remainder -Temporal current -Count 2 -Label 'negative: bare numeric remainder only')
    [void](Assert-SingleFinding -Text '252件の子プロセスの終了までは確認したが、そこで止めて「全件成功」と書かなかった。' -Scope denied-mention -Kind universal -Temporal denied -Count 252 -Label 'positive: production wording of the explicit refusal')
    [void](Assert-SingleFinding -Text '「9件すべて塞いだ」は誤り' -Scope denied-mention -Kind universal -Temporal denied -Count 9 -Label 'positive: production wording of the correction')
    [void](Assert-SingleFinding -Text '現在の監視定義5件中、実thread IDを名前に持つのは1件だけでした。残り4件は対応づけられません。' -Scope local -Kind remainder -Temporal current -Count 4 -Label 'positive: production wording of the watch-definition remainder')
    Assert-NoneCompatible -Text '正本186項目すべて完了' -Expected $false -Label 'whole roadmap completion blocks none'
    Assert-NoneCompatible -Text 'ロードマップ未完了14/186' -Expected $false -Label 'roadmap unchecked accounting blocks none'
    Assert-NoneCompatible -Text '残り2件だけ' -Expected $false -Label 'bare numeric remainder blocks none'
    Assert-NoneCompatible -Text '「9件すべて塞いだ」は誤り' -Expected $true -Label 'explicit correction stays none-compatible'
    Assert-NoneCompatible -Text '252件の子プロセスの終了までは確認したが、そこで止めて「全件成功」と書かなかった。' -Expected $true -Label 'explicit refusal stays none-compatible'
    Assert-NoneCompatible -Text '正本の残り11件' -Expected $false -Label 'current whole blocks none'
    Assert-NoneCompatible -Text 'すべて終わった' -Expected $false -Label 'ambiguous blocks none'
    Assert-NoneCompatible -Text '朝は 168/16/184 だった。項目を2つ増やしたうえで、残りが減っている。' -Expected $true -Label 'past whole does not claim current snapshot'
    Assert-NoneCompatible -Text 'これが終われば734行が完成し、残りが12件になる。' -Expected $true -Label 'future whole does not claim current snapshot'

    # 保留10件（前担当Eの報告書「E. 保留（10件、理由つき）」）。原文から1文字も
    # 変えずに複製し、いずれも Roadmap-Claim: none と両立する(local又は
    # quoted-instruction)ことを固定する。
    $holdbackFixtures = @(
        [pscustomobject]@{
            Name = '2026-09-03 21:48 crane CP subject exclusion'
            Lines = @('**「solver が収束していない」のではなく「solver は1回も反復していない」。**正本の一括collapse は102本の折り目を全て hard で ±180° に固定するため自由変数が0本になり（`solver.rs` 817-824行の `vars` が空）、`iterations=0`。')
        },
        [pscustomobject]@{
            Name = '2026-09-03 07:18 crane CP subject exclusion (への復帰)'
            Lines = @('**solver内部の `1e-13` 基準では正本と同じ既知の `converged=false / best_effort=true` だが、EPS判定・角度一致・継ぎ目・正本への復帰はすべて合格。**')
        },
        [pscustomobject]@{
            Name = '2026-09-03 19:58 quoted-instruction via heading blockquote'
            Lines = @('### 利用者の指示', '', '> **Claudeで全ての作業を再開し続けて。メインエージェントのFable5.1は作業は絶対にせず監視、詳細な指示、判断、Github操作、アプリ立ち上げのみ実施を許可する。サブエージェントで作業をしてサブエージェントではFableは使用せずopus5とsonnet5を使い分け質を落とさない範囲でトークンを節約し作業すること。**')
        },
        [pscustomobject]@{
            Name = '2026-09-02 17:52 folded-crane dictionary noun after heading blockquote'
            Lines = @('### 利用者の要求との対応', '', '> **全ての折り鶴はこれを折れるようにすること。私へ提出する動画もです。**')
        },
        [pscustomobject]@{
            Name = '2026-09-02 04:18 folded-crane dictionary noun inside inline quote'
            Lines = @('利用者の指示「全ての折り鶴はこれを折れるようにすること。動画もです」に直結する。')
        },
        [pscustomobject]@{
            Name = '2026-09-02 02:58 folded-crane dictionary noun inside unattributed quote'
            Lines = @('「アプリが出すすべての折り鶴と、利用者へ提出する動画は、この展開図を折れること」を、以後の前提として指示に入れた。')
        },
        [pscustomobject]@{
            Name = '2026-09-02 02:52 folded-crane dictionary noun after heading blockquote'
            Lines = @('### 利用者の指示との対応', '', '> **全ての折り鶴はこれを折れるようにすることと指示しています。私へ提出する動画もです。**')
        },
        [pscustomobject]@{
            Name = '2026-09-02 10:33 heading itself, 箇所 dictionary noun'
            Lines = @('## 2026-09-02 10:33 — 途中の動きも貫通0。危ない箇所は全て通過。残るは設計。待機の仕組みが実際に働いた')
        },
        [pscustomobject]@{
            Name = '2026-09-03 17:22 対策 dictionary noun'
            Lines = @('物的対策      8件（すべて既存の改定）。実地で働いた')
        },
        [pscustomobject]@{
            Name = '2026-09-02 12:53 手順 dictionary noun inside rules-doc quotation'
            Lines = @('> **達成単位は小さく区切り、「144手順すべて」のような大目標を1回で依頼しない。**')
        }
    )
    foreach ($fixture in $holdbackFixtures) {
        $fixtureFindings = @(Get-CheckedFindings -Text $fixture.Lines)
        Assert-True ($fixtureFindings.Count -gt 0) "holdback fixture has findings: $($fixture.Name)"
        Assert-True (Test-NoneCompatible -Findings $fixtureFindings) "holdback fixture is none-compatible: $($fixture.Name)"
        Assert-True (@($fixtureFindings | Where-Object { $_.Scope -eq 'ambiguous' -or $_.Scope -eq 'whole' -or $_.Scope -eq 'bounded' }).Count -eq 0) "holdback fixture has no residual whole/bounded/ambiguous finding: $($fixture.Name)"
    }
    Assert-Equal 10 $holdbackFixtures.Count 'all ten held-back diagnostics are fixed as fixtures'

    # 負例: roadmapの会計語彙は、除外語(正本CP・正本の展開図等・正本への復帰)に
    # 含まれないかぎり引き続きwholeのまま。緩和していないことを固定する。
    [void](Assert-SingleFinding -Text '正本の未完了は11件で、今朝と同じ。' -Scope whole -Kind remainder -Temporal current -Count 11 -Label 'negative: roadmap 正本の未完了 stays whole')
    [void](Assert-SingleFinding -Text '正本の残りは11件から8件になる。' -Scope whole -Kind remainder -Temporal current -Count 11 -Label 'negative: roadmap 正本の残り stays whole')
    [void](Assert-SingleFinding -Text '正本186項目すべて完了と判定した。' -Scope whole -Kind universal -Temporal current -Count 186 -Label 'negative: 正本+bare number subject stays whole (not excluded)')
    [void](Assert-SingleFinding -Text '正本175/186まで進んだ。' -Scope whole -Kind progress -Temporal current -Count 175 -Label 'negative: 正本 N/186 stays whole')

    # 負例: blockquoteの外側の断言、利用者引用でないblockquoteは、見出し2形の
    # 条件が無ければ従来どおりwhole。quoted-instructionへ広げていないことを固定。
    Assert-NoneCompatible -Text @('### 利用者の要求との対応2', '', '> **正本はすべて完了**') -Expected $false -Label 'negative: blockquote without one of the four required headings stays whole'
    Assert-NoneCompatible -Text @('利用者の指示は明確で、正本はすべて完了と判定できる。') -Expected $false -Label 'negative: mention of 利用者の指示 without an immediate quote stays whole'
    Assert-NoneCompatible -Text @('### 利用者の要求', '', '> **正本はすべて完了した。**') -Expected $false -Label 'negative: heading not among the four required headings stays whole'
    Assert-NoneCompatible -Text @('利用者の指示「進めること」の後、正本の残り11件は変わらない。') -Expected $false -Label 'negative: trigger outside the quoted span stays whole'
    Assert-NoneCompatible -Text @('### 利用者の指示', '', 'メモ: 正本はすべて完了した。') -Expected $false -Label 'negative: non-blockquote line after the heading stays whole'

    # 正例: quoted-instruction は辞書に無い語でも、指定した2形だけで none と両立する。
    [void](Assert-SingleFinding -Text @('### 利用者の指示', '', '> **すべてやり直して、最初から確認すること。**') -Scope quoted-instruction -Kind universal -Temporal current -ExpectedLine 3 -Label 'quoted-instruction: heading blockquote form, no dictionary noun')
    [void](Assert-SingleFinding -Text @('利用者の判断「すべてやり直すこと」を優先する。') -Scope quoted-instruction -Kind universal -Temporal current -Label 'quoted-instruction: inline quote form, no dictionary noun')
    Assert-NoneCompatible -Text @('### 利用者の指示', '', '> **すべてやり直して、最初から確認すること。**') -Expected $true -Label 'quoted-instruction heading form stays none-compatible'
    Assert-NoneCompatible -Text @('利用者の判断「すべてやり直すこと」を優先する。') -Expected $true -Label 'quoted-instruction inline form stays none-compatible'

    # CLI mode is a supported public entry point in addition to dot-sourcing.
    $powershellExe = (Get-Process -Id $PID).Path
    $cliOutput = @(& $powershellExe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $resolvedScopeScript -Text '正本は175/186' -RoadmapTotal $script:RoadmapTotal -AsJson 2>&1)
    Assert-Equal 0 $LASTEXITCODE 'standalone classifier exit code'
    Assert-Equal 1 $cliOutput.Count 'standalone classifier writes one JSON line'
    $cliFinding = [string]$cliOutput[0] | ConvertFrom-Json
    Assert-Equal 'whole' ([string]$cliFinding.Scope) 'standalone classifier JSON scope'
    Assert-Equal 'progress' ([string]$cliFinding.Kind) 'standalone classifier JSON kind'

    return [pscustomobject]@{
        ScopeScript = $resolvedScopeScript
        PowerShell  = $powershellExe
        FixtureFindings = $fixtureFindingCount
    }
}

function Invoke-MutationChecks {
    param(
        [Parameter(Mandatory = $true)][string]$ResolvedScopeScript,
        [Parameter(Mandatory = $true)][string]$PowerShellExe
    )

    $tempParent = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd([char[]]'\/')
    $tempRoot = [System.IO.Path]::GetFullPath((Join-Path $tempParent ('ori3-roadmap-scope-' + [Guid]::NewGuid().ToString('N'))))
    [void][System.IO.Directory]::CreateDirectory($tempRoot)
    try {
        $utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
        $sourceText = [System.IO.File]::ReadAllText($ResolvedScopeScript, $utf8Strict)
        # 統括が求めた5種 (always-local / always-whole / 否定過剰 / temporal過剰 /
        # NFKC除去) に、anchor優先とlocal binderの2種を足した7本。どれか1本でも
        # 緑のまま通るなら、その契約はこのtestで守られていない。
        $mutations = @(
            [pscustomobject]@{
                Name = 'global-anchor-priority'
                Find = 'if ($null -ne $roadmapAnchor) {'
                Replace = 'if ($false -and $null -ne $roadmapAnchor) {'
            },
            [pscustomobject]@{
                Name = 'explicit-local-binder'
                Find = 'if ($null -ne $localBinder) {'
                Replace = 'if ($false -and $null -ne $localBinder) {'
            },
            [pscustomobject]@{
                Name = 'strict-denial-correction'
                Find = 'if (Test-RoadmapScopeDeniedMention -Segment $segment -Trigger $trigger -TriggerIndex $triggerMatch.Index) {'
                Replace = 'if ($false) { # MUTANT: denial disabled'
            },
            [pscustomobject]@{
                Name = 'always-local'
                Find = "-Scope 'ambiguous' -Kind `$definition.Kind"
                Replace = "-Scope 'local' -Kind `$definition.Kind"
            },
            [pscustomobject]@{
                Name = 'always-whole'
                Find = "-Scope 'local' -Kind `$definition.Kind"
                Replace = "-Scope 'whole' -Kind `$definition.Kind"
            },
            [pscustomobject]@{
                Name = 'denial-over-broad'
                Find = "    `$strictDisposition = '(?:"
                Replace = "    return `$true # MUTANT: every mention counts as a denial`n    `$strictDisposition = '(?:"
            },
            [pscustomobject]@{
                Name = 'temporal-over-broad'
                Find = "    return 'current'"
                Replace = "    return 'past' # MUTANT: every assertion becomes past"
            },
            [pscustomobject]@{
                Name = 'nfkc-normalization-removed'
                Find = '$normalizedLine = $line.Normalize($normalizationForm)'
                Replace = '$normalizedLine = $line # MUTANT: NFKC removed'
            },
            [pscustomobject]@{
                # 保留10件の精度追加(2026-09-04)で新設した quoted-instruction を
                # 常に付ける故障注入。負例(見出し無しblockquote・引用外側の断言)が
                # whole から quoted-instruction へ誤って倒れ、赤になることを確かめる。
                Name = 'quoted-instruction-always-attached'
                Find = '$quotedInstructionSource = $null'
                Replace = "`$quotedInstructionSource = 'MUTANT: always-quoted-instruction'"
            },
            [pscustomobject]@{
                # 同じ委譲で追加した正本CP等の除外語を無効化する故障注入。除外語が
                # 主語として扱われなくなり(常にwholeへ戻り)、保留10件のうち
                # 21:48・07:18のfixtureがlocalにならず赤になることを確かめる。
                Name = 'crane-blueprint-exclusion-disabled'
                Find = "`$roadmapSubject = '(?:正本(?!' + `$craneBlueprintReferentExclusion + ')|実装ロードマップ|ロードマップ)'"
                Replace = "`$roadmapSubject = '(?:正本|実装ロードマップ|ロードマップ)' # MUTANT: exclusion disabled"
            }
        )

        $mutationResults = New-Object System.Collections.Generic.List[object]
        foreach ($mutation in $mutations) {
            $occurrences = [regex]::Matches($sourceText, [regex]::Escape([string]$mutation.Find)).Count
            Assert-Equal 1 $occurrences "mutation anchor is unique: $($mutation.Name)"
            $mutatedText = $sourceText.Replace([string]$mutation.Find, [string]$mutation.Replace)
            $mutantPath = Join-Path $tempRoot ($mutation.Name + '.ps1')
            [System.IO.File]::WriteAllText($mutantPath, $mutatedText, (New-Object System.Text.UTF8Encoding($true)))

            $childOutput = @(& $PowerShellExe -NoProfile -NonInteractive -ExecutionPolicy Bypass `
                -File $PSCommandPath -ScopeScriptPath $mutantPath -ContractOnly 2>&1)
            $childExit = $LASTEXITCODE
            Assert-True ($childExit -ne 0) "mutation must make contract tests red: $($mutation.Name)"
            $mutationResults.Add([pscustomobject]@{
                Name = $mutation.Name
                ExitCode = $childExit
                OutputTail = (@($childOutput | Select-Object -Last 2) -join ' | ')
            })
            Write-Host "[MUTATION OK] $($mutation.Name): child exit=$childExit"
        }
        Assert-Equal 10 $mutationResults.Count 'ten isolated mutations were exercised'
        return $mutationResults.ToArray()
    }
    finally {
        $resolvedTemp = [System.IO.Path]::GetFullPath($tempRoot).TrimEnd([char[]]'\/')
        if ([System.IO.Path]::GetDirectoryName($resolvedTemp) -ne $tempParent -or
            [System.IO.Path]::GetFileName($resolvedTemp) -notmatch '^ori3-roadmap-scope-[0-9a-f]{32}$') {
            throw "unsafe mutation cleanup path: $resolvedTemp"
        }
        if (Test-Path -LiteralPath $resolvedTemp) {
            Remove-Item -LiteralPath $resolvedTemp -Recurse -Force
        }
    }
}

try {
    $contractResult = Invoke-ContractAssertions
    $mutationCount = 0
    if (-not $ContractOnly) {
        $mutationResults = @(Invoke-MutationChecks -ResolvedScopeScript $contractResult.ScopeScript -PowerShellExe $contractResult.PowerShell)
        $mutationCount = $mutationResults.Count
    }
    Write-Host "[TEST OK] roadmap-claim-scope: $script:Assertions assertions; production-fixture-findings=$($contractResult.FixtureFindings); mutations=$mutationCount"
    exit 0
}
catch {
    Write-Host "[TEST NG] roadmap-claim-scope: $($_.Exception.Message)" -ForegroundColor Red
    Write-Host $_.ScriptStackTrace -ForegroundColor Red
    exit 1
}
