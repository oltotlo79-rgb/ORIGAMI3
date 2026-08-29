[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$scriptPath = Join-Path $PSScriptRoot "check-agent-instruction.ps1"
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-check-agent-instruction-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [IO.Path]::GetFullPath((Join-Path $tempBase $sandboxName))
$script:AssertionCount = 0
$script:CaseCount = 0

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$Output = ""
    )
    $script:AssertionCount += 1
    if (-not $Condition) {
        throw "ASSERTION FAILED: $Message`n$Output"
    }
}

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)][AllowNull()]$Actual,
        [Parameter(Mandatory = $true)][AllowNull()]$Expected,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$Output = ""
    )
    $script:AssertionCount += 1
    if ($Actual -ne $Expected) {
        throw "ASSERTION FAILED: $Message (expected=$Expected, actual=$Actual)`n$Output"
    }
}

function Assert-Contains {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )
    $script:AssertionCount += 1
    if (-not $Text.Contains($Expected)) {
        throw "ASSERTION FAILED: $Message (missing='$Expected')`n$Text"
    }
}

function Assert-NotContains {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Unexpected,
        [Parameter(Mandatory = $true)][string]$Message
    )
    $script:AssertionCount += 1
    if ($Text.Contains($Unexpected)) {
        throw "ASSERTION FAILED: $Message (unexpected present='$Unexpected')`n$Text"
    }
}

function New-InstructionFixtureFile {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Content
    )
    $path = Join-Path $sandboxRoot $Name
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $path))
    [IO.File]::WriteAllText($path, $Content, [Text.UTF8Encoding]::new($false))
    return $path
}

function Invoke-Check {
    param([Parameter(Mandatory = $true)][string[]]$Paths)

    $script:CaseCount += 1
    $stdoutPath = Join-Path $sandboxRoot ("stdout-{0}.txt" -f $script:CaseCount)
    $stderrPath = Join-Path $sandboxRoot ("stderr-{0}.txt" -f $script:CaseCount)
    $arguments = @("-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $scriptPath) + $Paths
    $powerShellPath = (Get-Process -Id $PID).Path
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = 0
        & $powerShellPath @arguments 1> $stdoutPath 2> $stderrPath
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    $parts = New-Object System.Collections.Generic.List[string]
    if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) { $parts.Add([IO.File]::ReadAllText($stdoutPath)) }
    if (Test-Path -LiteralPath $stderrPath -PathType Leaf) { $parts.Add([IO.File]::ReadAllText($stderrPath)) }
    return [pscustomobject]@{ ExitCode = $exitCode; Output = ($parts -join "`n") }
}

# 完全合格ひな型から、指定した要素だけを欠落させた指示文を組み立てる。
# 単一引数だけをfalse/上書きすれば、その項目だけが不合格になるよう設計している。
function New-InstructionText {
    param(
        [bool]$IncludeTarget = $true,
        [bool]$UseNoCodeExemption = $false,
        [bool]$IncludeNumeric = $true,
        [string]$NumericOverrideText = $null,
        [bool]$IncludeGitBan = $true,
        [bool]$IncludeLaunchBan = $true,
        [bool]$IncludeToleranceBan = $true,
        [bool]$IncludeDependencyBan = $true,
        [bool]$IncludeCompetitiveBan = $true,
        [bool]$IncludeDevilRoseBan = $true,
        [bool]$IncludeStaging = $true,
        [bool]$IncludeFailureCause = $true,
        [bool]$IncludeTool = $true,
        [bool]$IncludeSavePath = $true,
        [bool]$IncludeWorktree = $true,
        [bool]$IncludeReportContinuity = $true,
        [bool]$IncludeCargoTest = $true,
        [bool]$IncludeCargoTargetDir = $true,
        [string[]]$SkipNamesToInclude = @(
            "completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten",
            "named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten",
            "a_safe_coincident_partial_network_appears_after_the_first_fold",
            "the_heaviest_proposal_never_hits_the_time_limit"
        )
    )

    $lines = New-Object System.Collections.Generic.List[string]

    if ($UseNoCodeExemption) {
        $lines.Add('対象: 該当なし。文書のみを整える作業のため対象ファイルはありません。')
    }
    elseif ($IncludeTarget) {
        $lines.Add('対象: crates/ori3-rigid/src/motion.rs の `resolve_step()` 関数を修正してください。')
    }
    else {
        $lines.Add('対象: 直してください。')
    }
    $lines.Add('')

    if (-not [string]::IsNullOrEmpty($NumericOverrideText)) {
        $lines.Add('合格条件: ' + $NumericOverrideText)
    }
    elseif ($IncludeNumeric) {
        $lines.Add('合格条件: 警告件数が0件であること。処理時間が200ms以内であること。誤差が1e-6未満であること。')
    }
    else {
        $lines.Add('合格条件: 警告が出ないこと。処理が高速であること。誤差が小さいこと。')
    }
    $lines.Add('')

    $lines.Add('やってはいけないこと:')
    if ($IncludeGitBan) { $lines.Add('- gitへの書き込み禁止です。commitやpushは絶対にしないでください。') }
    if ($IncludeLaunchBan) { $lines.Add('- ブラウザの窓を開かせないでください。desktop.exeも起動しないでください。配信サーバーも起動しないでください。') }
    if ($IncludeToleranceBan) { $lines.Add('- 期待値や許容差を緩めないでください。') }
    if ($IncludeDependencyBan) { $lines.Add('- Cargo.toml、Cargo.lock、vendor/ を変更しないでください。') }
    if ($IncludeCompetitiveBan) { $lines.Add('- docs/competitive-review-2026-08-20.md には触らないでください。') }
    if ($IncludeDevilRoseBan) { $lines.Add('- 悪魔と1分ローズは使わないでください。') }
    $lines.Add('')

    if ($IncludeStaging) {
        $lines.Add('作業は3段階に分けてください。各段階で中間報告を義務とします。')
    }
    else {
        $lines.Add('作業を進めてください。')
    }
    $lines.Add('')

    if ($IncludeFailureCause) {
        $lines.Add('過去の失敗として、角度を丸めて山谷を消したためori3-layersが壊れた例があります。原因は全体品質ゲートを通さなかったことです。')
    }
    $lines.Add('')

    if ($IncludeTool) {
        $toolLine = '道具の使い方: `rg "resolve_step" crates/ori3-rigid/src` でファイルと関数の実在を確認してください。'
        if ($IncludeCargoTest) {
            $skipParts = New-Object System.Collections.Generic.List[string]
            foreach ($skipName in $SkipNamesToInclude) {
                $skipParts.Add('--skip ' + $skipName)
            }
            $skipTokens = [string]::Join(' ', $skipParts)
            $targetDirClause = ''
            if ($IncludeCargoTargetDir) {
                $targetDirClause = '`$env:CARGO_TARGET_DIR = "%TEMP%\ori3-target-motion"` を設定してから'
            }
            $toolLine = $toolLine + '確認できたら、' + $targetDirClause + '`cargo test -p ori3-rigid -- ' + $skipTokens + '` で検査してください。'
        }
        $lines.Add($toolLine)
    }
    $lines.Add('')

    if ($IncludeSavePath) {
        $lines.Add('成果物は scratchpad/task-report.md に保存してください。')
    }
    if ($IncludeReportContinuity) {
        $lines.Add('報告書は scratchpad/task-report.md へ随時書くこと。完了通知が届かなくても、報告書ファイルを更新し続けてください。')
    }
    $lines.Add('')

    if ($IncludeWorktree) {
        $lines.Add('割り当てた作業ツリー: C:\Users\oltot\AppData\Local\Temp\ori3-wt-motion (worktree)')
    }

    return ($lines -join "`n")
}

function Remove-TestSandbox {
    if (-not (Test-Path -LiteralPath $sandboxRoot)) { return }
    $resolved = [IO.Path]::GetFullPath($sandboxRoot).TrimEnd([char[]]"\/")
    $parent = [IO.Path]::GetDirectoryName($resolved)
    $leaf = [IO.Path]::GetFileName($resolved)
    if (($parent -ne $tempBase) -or
        (-not [regex]::IsMatch($leaf, '^ori3-check-agent-instruction-test-[0-9a-f]{32}$', [Text.RegularExpressions.RegexOptions]::IgnoreCase))) {
        throw "安全でない一時領域の削除を拒否しました: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
    throw "検査本体が見つかりません: $scriptPath"
}

[void][IO.Directory]::CreateDirectory($sandboxRoot)
try {
    Write-Output "[1/13] 完全合格ひな型(cargo test込み)は11項目すべてOKでexit 0"
    $fullPassPath = New-InstructionFixtureFile "full-pass.md" (New-InstructionText)
    $result = Invoke-Check @($fullPassPath)
    Assert-Equal $result.ExitCode 0 "完全合格ひな型はexit 0であること" $result.Output
    for ($i = 1; $i -le 11; $i++) {
        Assert-Contains $result.Output ("[OK] {0}." -f $i) ("項目{0}がOKであること" -f $i)
    }
    Assert-Contains $result.Output "全項目合格" "全項目合格の表示があること"

    Write-Output "[2/13] cargoに触れないひな型は項目10・11がN/Aでも全体合格"
    $noCargoPath = New-InstructionFixtureFile "full-pass-no-cargo.md" (New-InstructionText -IncludeCargoTest $false)
    $result = Invoke-Check @($noCargoPath)
    Assert-Equal $result.ExitCode 0 "cargo未記載でも他項目が揃えば合格であること" $result.Output
    Assert-Contains $result.Output "[--] 10. 長時間検査の--skip 4件 (03-品質ゲート.md:61) - cargo testの言及なし（該当なし）" "項目10がN/A表示になること"
    Assert-Contains $result.Output "[--] 11. 専用のCARGO_TARGET_DIR指定 (06-過去の失敗と対策.md:241-242) - cargo実行の言及なし（該当なし）" "項目11がN/A表示になること"
    Assert-Contains $result.Output "全項目合格" "N/Aは不合格に数えないこと"

    Write-Output "[3/13] 「該当なし」＋理由は項目1を実パス無しでも合格させる"
    $exemptionPath = New-InstructionFixtureFile "exemption.md" (New-InstructionText -UseNoCodeExemption $true)
    $result = Invoke-Check @($exemptionPath)
    Assert-Contains $result.Output "[OK] 1. 実ファイルパスと関数名の実名記載" "非コード作業の「該当なし」免除が効くこと"
    Assert-Contains $result.Output "該当なし" "免除理由の検出詳細を表示すること"

    Write-Output "[4/13] 項目1: 実パスも関数名も無いと不合格"
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item1.md" (New-InstructionText -IncludeTarget $false)))
    Assert-Equal $result.ExitCode 1 "項目1欠落はexit 1であること" $result.Output
    Assert-Contains $result.Output "[NG] 1. 実ファイルパスと関数名の実名記載" "項目1がNGになること"
    Assert-Contains $result.Output "欠落: 実ファイルパス・関数名" "欠落した2要素とも列挙されること"

    Write-Output "[5/13] 項目2: 数値条件の誤検出・見逃しを実測する"
    $numericCases = @(
        @{ Text = "半径は3mm以下であること。"; Expect = "OK"; Label = "単位+比較語(mm以下)は合格" },
        @{ Text = 'gapが `<= 200` であること。'; Expect = "OK"; Label = "記号比較(<=)は合格" },
        @{ Text = "16層で200ms以内であること。"; Expect = "OK"; Label = "規約の例文そのものは合格" },
        @{ Text = "3分以内に完了すること。"; Expect = "OK"; Label = "正当な分単位は合格(誤検出ガードの副作用でないこと)" },
        @{ Text = "1分ローズは使わないこと。"; Expect = "NG"; Label = "「1分ローズ」は数値条件と誤検出しないこと" },
        @{ Text = "処理をVec<f64>で行うこと。バージョンはv0.4.5、日付は2026-08-29を参照。第3項を満たすこと。"; Expect = "NG"; Label = "型/バージョン/日付/章番号は数値条件と誤検出しないこと" }
    )
    foreach ($case in $numericCases) {
        $path = New-InstructionFixtureFile ("numeric-{0}.md" -f [Guid]::NewGuid().ToString("N")) (New-InstructionText -NumericOverrideText $case.Text)
        $result = Invoke-Check @($path)
        $marker = "[{0}] 2." -f $case.Expect
        Assert-Contains $result.Output $marker $case.Label
    }

    Write-Output "[6/13] 項目3: 6件の禁止事項それぞれについて、1件欠落させると不合格になる"
    $prohibitionCases = @(
        @{ Flag = "IncludeGitBan"; Label = "gitへの書き込み禁止" },
        @{ Flag = "IncludeLaunchBan"; Label = "ブラウザ・desktop.exe・配信サーバーを起動しない" },
        @{ Flag = "IncludeToleranceBan"; Label = "期待値・許容差を緩めない" },
        @{ Flag = "IncludeDependencyBan"; Label = "Cargo.toml/Cargo.lock/vendor/を変更しない" },
        @{ Flag = "IncludeCompetitiveBan"; Label = "docs/competitive-review-2026-08-20.mdに触らない" },
        @{ Flag = "IncludeDevilRoseBan"; Label = "悪魔・1分ローズを使わない" }
    )
    foreach ($case in $prohibitionCases) {
        $params = @{ $case.Flag = $false }
        $text = New-InstructionText @params
        $path = New-InstructionFixtureFile ("fail-item3-{0}.md" -f $case.Flag) $text
        $result = Invoke-Check @($path)
        Assert-Equal $result.ExitCode 1 ("{0}を欠くとexit 1であること" -f $case.Label) $result.Output
        Assert-Contains $result.Output "[NG] 3." "項目3がNGになること"
        Assert-Contains $result.Output "5/6" "残り5件は満たしたまま1件だけ欠けること"
        Assert-Contains $result.Output $case.Label ("欠落項目名として『{0}』が名指しされること" -f $case.Label)
    }

    Write-Output "[7/13] 項目4・5・6・7・8・9: それぞれ単独で欠落させると、その項目だけが不合格になる"
    # 項目7(保存先パス)と項目9(報告書への継続記録)は、どちらも「パス+書く系の動詞」を
    # 見るため、報告書の行が保存先パスの記載を兼ねられる（実測で確認した意図した重なり。
    # 「報告書をXへ書く」という一文は保存先の明記でもあるため、これは誤検出ではない）。
    # そのため項目7を単独で欠落させる場合は、保存先パスの行と報告書継続の行の両方を
    # 外し、その場合は項目9も道連れでNGになることをAlsoNGとして明示する。
    $singleItemCases = @(
        @{ Params = @{ IncludeStaging = $false }; Index = 4; Contains = "段階分割"; AlsoNG = @() },
        @{ Params = @{ IncludeFailureCause = $false }; Index = 5; Contains = "過去の失敗例"; AlsoNG = @() },
        @{ Params = @{ IncludeTool = $false }; Index = 6; Contains = "コマンド形式の記載"; AlsoNG = @() },
        @{ Params = @{ IncludeSavePath = $false; IncludeReportContinuity = $false }; Index = 7; Contains = "保存先パス"; AlsoNG = @(9) },
        @{ Params = @{ IncludeWorktree = $false }; Index = 8; Contains = "作業ツリーの絶対パス"; AlsoNG = @() },
        @{ Params = @{ IncludeReportContinuity = $false }; Index = 9; Contains = "報告書ファイルへ書く"; AlsoNG = @() }
    )
    foreach ($case in $singleItemCases) {
        $callParams = $case.Params
        $text = New-InstructionText @callParams
        $path = New-InstructionFixtureFile ("fail-item{0}.md" -f $case.Index) $text
        $result = Invoke-Check @($path)
        Assert-Equal $result.ExitCode 1 ("項目{0}欠落はexit 1であること" -f $case.Index) $result.Output
        Assert-Contains $result.Output ("[NG] {0}." -f $case.Index) ("項目{0}がNGになること" -f $case.Index)
        Assert-Contains $result.Output $case.Contains ("項目{0}の詳細に手掛かり語を含むこと" -f $case.Index)
        # 他の項目まで巻き込んで不合格にしていないことを確認する（分離性）。
        # AlsoNGに列挙した項目（意図した重なりが実測で分かっているもの）は除外する。
        $exemptIndexes = @($case.Index) + @($case.AlsoNG)
        $otherIndexes = @(1..9) | Where-Object { $exemptIndexes -notcontains $_ }
        foreach ($otherIndex in $otherIndexes) {
            Assert-NotContains $result.Output ("[NG] {0}." -f $otherIndex) ("項目{0}を巻き込んで不合格にしないこと（{1}の単独欠落時）" -f $otherIndex, $case.Index)
        }
        foreach ($alsoIndex in @($case.AlsoNG)) {
            Assert-Contains $result.Output ("[NG] {0}." -f $alsoIndex) ("項目{0}は項目{1}と意図して重なりNGになること" -f $alsoIndex, $case.Index)
        }
    }

    Write-Output "[8/13] 項目10: skip対象0/4件・一部欠落(2/4件)を検出する"
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item10-none.md" (New-InstructionText -SkipNamesToInclude @())))
    Assert-Equal $result.ExitCode 1 "skip対象が0件ならexit 1であること" $result.Output
    Assert-Contains $result.Output "[NG] 10. 長時間検査の--skip 4件 (03-品質ゲート.md:61) - 0/4 件のみ検出" "0/4件の検出数を表示すること"
    Assert-Contains $result.Output "[OK] 11." "skip不足はCARGO_TARGET_DIR判定を巻き込まないこと"

    $partialSkips = @(
        "completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten",
        "named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten"
    )
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item10-partial.md" (New-InstructionText -SkipNamesToInclude $partialSkips)))
    Assert-Contains $result.Output "2/4 件のみ検出" "2/4件の検出数を表示すること"
    Assert-Contains $result.Output "a_safe_coincident_partial_network_appears_after_the_first_fold" "欠落したskip名を実名で列挙すること"
    Assert-Contains $result.Output "the_heaviest_proposal_never_hits_the_time_limit" "欠落したもう1件のskip名も実名で列挙すること"

    Write-Output "[9/13] 項目11: CARGO_TARGET_DIR未指定を検出し、項目10には影響しない"
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item11.md" (New-InstructionText -IncludeCargoTargetDir $false)))
    Assert-Equal $result.ExitCode 1 "CARGO_TARGET_DIR欠落はexit 1であること" $result.Output
    Assert-Contains $result.Output "[NG] 11. 専用のCARGO_TARGET_DIR指定 (06-過去の失敗と対策.md:241-242) - cargoの言及があるのにCARGO_TARGET_DIRの指定が見つかりません" "項目11がNGになること"
    Assert-Contains $result.Output "[OK] 10." "CARGO_TARGET_DIR欠落はskip4件判定を巻き込まないこと"

    Write-Output "[10/13] cargo build等cargo testを含まない言及でも、項目11だけは発動する"
    $cargoBuildOnly = '道具: `$env:CARGO_TARGET_DIR = "%TEMP%\ori3-target-x"` を設定してから `cargo build -p ori3-rigid` を実行してください。'
    $result = Invoke-Check @((New-InstructionFixtureFile "cargo-build-only.md" $cargoBuildOnly))
    Assert-Contains $result.Output "[--] 10. 長時間検査の--skip 4件 (03-品質ゲート.md:61) - cargo testの言及なし（該当なし）" "cargo buildはcargo test向けskip判定を発動しないこと"
    Assert-Contains $result.Output "[OK] 11. 専用のCARGO_TARGET_DIR指定 (06-過去の失敗と対策.md:241-242) - CARGO_TARGET_DIRの指定を検出" "cargo buildでもCARGO_TARGET_DIR判定は発動すること"

    Write-Output "[11/13] 複数ファイルを1回の呼び出しで検査できる"
    $secondFailPath = New-InstructionFixtureFile "multi-fail-item5.md" (New-InstructionText -IncludeFailureCause $false)
    $result = Invoke-Check @($fullPassPath, $secondFailPath)
    Assert-Equal $result.ExitCode 1 "1件でも不合格ならexit 1であること" $result.Output
    Assert-Contains $result.Output ("[指示書] {0}" -f $fullPassPath) "1件目のパスが表示されること"
    Assert-Contains $result.Output ("[指示書] {0}" -f $secondFailPath) "2件目のパスが表示されること"
    Assert-Contains $result.Output ("[OK] {0}: 全項目合格" -f $fullPassPath) "1件目は合格として表示されること"
    Assert-Contains $result.Output "対象 2件 / 不合格 1件 / 不合格項目合計 1件" "複数ファイルの集計行が出ること"

    Write-Output "[12/13] 存在しない指示文はexit 2で日本語エラーになる"
    $missingPath = Join-Path $sandboxRoot "does-not-exist.md"
    $result = Invoke-Check @($missingPath)
    Assert-Equal $result.ExitCode 2 "存在しないファイルはexit 2であること" $result.Output
    Assert-Contains $result.Output "指示文ファイルが見つかりません" "日本語のエラー理由を表示すること"

    Write-Output "[13/13] 完全合格ひな型はexit 0、単独不合格はexit 1、存在しないファイルはexit 2の境界を再確認する"
    $result = Invoke-Check @($fullPassPath)
    Assert-Equal $result.ExitCode 0 "合格は0であること(再確認)" $result.Output
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-boundary.md" (New-InstructionText -IncludeWorktree $false)))
    Assert-Equal $result.ExitCode 1 "不合格は1であること(再確認)" $result.Output

    Write-Output ("check-agent-instruction self-test passed: {0} cases, {1} assertions" -f $script:CaseCount, $script:AssertionCount)
}
finally {
    Remove-TestSandbox
}
