[CmdletBinding()]
param()

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$scriptPath = Join-Path $PSScriptRoot "check-agent-instruction.ps1"
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]"\/")
$sandboxName = "ori3-check-agent-instruction-test-{0}" -f [Guid]::NewGuid().ToString("N")
$sandboxRoot = [IO.Path]::GetFullPath((Join-Path $tempBase $sandboxName))
$fixtureRepository = Join-Path $sandboxRoot "fixture-repository"
$evidenceTargetRelativePath = "scripts/sample-target.ps1"
$evidenceTargetBody = "function Invoke-SampleTarget {"
$evidenceTargetLine = 2
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

function ConvertTo-ProcessArgumentString {
    param([Parameter(Mandatory = $true)][string[]]$Values)

    $parts = foreach ($value in $Values) {
        $escaped = [regex]::Replace($value, '(\\*)"', '$1$1\\"')
        $trailingBackslashes = [regex]::Match($escaped, '\\*$').Value
        $escaped = $escaped + $trailingBackslashes
        '"' + $escaped + '"'
    }
    return ($parts -join " ")
}

function Invoke-Check {
    param(
        [string[]]$Paths = @(),
        [AllowNull()][string]$StandardInput = $null,
        [switch]$VerifyLiveBaseline,
        [string]$ExpectedModel = "",
        [string]$GitExecutable = ""
    )

    $useStandardInput = $PSBoundParameters.ContainsKey("StandardInput")
    if (-not $useStandardInput -and @($Paths).Count -eq 0) {
        throw "Invoke-CheckにはPathsまたはStandardInputが必要です"
    }
    $script:CaseCount += 1
    $arguments = New-Object System.Collections.Generic.List[string]
    foreach ($part in @("-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $scriptPath)) {
        $arguments.Add($part)
    }
    if ($useStandardInput) {
        $arguments.Add("-ReadFromStdin")
    }
    else {
        foreach ($path in $Paths) { $arguments.Add($path) }
    }
    $arguments.Add("-RepositoryRoot")
    $arguments.Add($fixtureRepository)
    if ($VerifyLiveBaseline) { $arguments.Add("-VerifyLiveBaseline") }
    if (-not [string]::IsNullOrWhiteSpace($ExpectedModel)) {
        $arguments.Add("-ExpectedModel")
        $arguments.Add($ExpectedModel)
    }
    if (-not [string]::IsNullOrWhiteSpace($GitExecutable)) {
        $arguments.Add("-GitExecutable")
        $arguments.Add($GitExecutable)
    }

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = (Get-Process -Id $PID).Path
    $startInfo.Arguments = ConvertTo-ProcessArgumentString -Values $arguments.ToArray()
    $startInfo.WorkingDirectory = $sandboxRoot
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.RedirectStandardInput = $useStandardInput
    $startInfo.StandardOutputEncoding = [Text.Encoding]::UTF8
    $startInfo.StandardErrorEncoding = [Text.Encoding]::UTF8
    $process = [Diagnostics.Process]::Start($startInfo)
    if ($useStandardInput) {
        $inputBytes = (New-Object Text.UTF8Encoding($false)).GetBytes($StandardInput)
        if ($inputBytes.Length -gt 0) {
            $process.StandardInput.BaseStream.Write($inputBytes, 0, $inputBytes.Length)
        }
        $process.StandardInput.BaseStream.Close()
    }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    return [pscustomobject]@{ ExitCode = $process.ExitCode; Output = ($stdout + $stderr) }
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
        [bool]$IncludeModelSelection = $true,
        [string]$ModelSelectionLine = "モデル選択: opus | 理由: 原因切分けと意味契約の判断が必要なため。",
        [bool]$IncludeBaseline = $true,
        [bool]$IncludeBaselineWorktree = $true,
        [bool]$IncludeBaselineHead = $true,
        [bool]$IncludeBaselineDirty = $true,
        [bool]$IncludeBaselineCommand = $true,
        [bool]$IncludeBaselineResult = $true,
        [string]$BaselineWorktree = "C:\Users\oltot\AppData\Local\Temp\ori3-wt-motion",
        [string]$BaselineHead = "1111111111111111111111111111111111111111",
        [int]$BaselineDirty = 3,
        [string]$BaselineCommand = "measure-target --all",
        [string]$BaselineResult = "exit=1; 17 passed / 8 failed / 4 ignored",
        [bool]$IncludeEvidence = $true,
        [bool]$IncludeEvidenceTarget = $true,
        [bool]$IncludeEvidenceCommand = $true,
        [bool]$IncludeEvidenceOutput = $true,
        [bool]$IncludeStructuredNoCodeExemption = $true,
        [string]$NoCodeEvidenceReason = "文書だけを整える作業のため",
        [string]$EvidenceTargetPath = "scripts/sample-target.ps1",
        [string]$EvidenceSymbol = "Invoke-SampleTarget()",
        [string]$EvidenceCommand = "rg -n --fixed-strings --glob '!docs/competitive-review-2026-08-20.md' 'Invoke-SampleTarget' scripts/sample-target.ps1",
        [string]$EvidenceOutput = "scripts/sample-target.ps1:2:function Invoke-SampleTarget {",
        [string[]]$RequestedDeliverables = @("対象関数の修正と検査結果を報告する"),
        [string[]]$SkipNamesToInclude = @(
            "completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten",
            "named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten",
            "a_safe_coincident_partial_network_appears_after_the_first_fold",
            "the_heaviest_proposal_never_hits_the_time_limit"
        )
    )

    $lines = New-Object System.Collections.Generic.List[string]

    $lines.Add('依頼項目:')
    foreach ($deliverable in $RequestedDeliverables) {
        $lines.Add('- ' + $deliverable)
    }
    $lines.Add('')

    if ($UseNoCodeExemption) {
        $lines.Add('対象: 該当なし。文書のみを整える作業のため対象ファイルはありません。')
    }
    elseif ($IncludeTarget) {
        $lines.Add('対象: scripts/sample-target.ps1 の `Invoke-SampleTarget()` 関数を修正してください。')
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
    $lines.Add('')

    if ($IncludeModelSelection) {
        $lines.Add($ModelSelectionLine)
    }
    $lines.Add('')

    if ($IncludeBaseline) {
        $lines.Add('投入時ベースライン:')
        if ($IncludeBaselineWorktree) { $lines.Add('- worktree: `' + $BaselineWorktree + '`') }
        if ($IncludeBaselineHead) { $lines.Add('- HEAD: `' + $BaselineHead + '`') }
        if ($IncludeBaselineDirty) { $lines.Add('- 未コミット件数: `' + $BaselineDirty + '`') }
        if ($IncludeBaselineCommand) { $lines.Add('- 対象検査command: `' + $BaselineCommand + '`') }
        if ($IncludeBaselineResult) { $lines.Add('- 対象検査baseline: ' + $BaselineResult) }
    }
    $lines.Add('')

    if ($UseNoCodeExemption) {
        if ($IncludeStructuredNoCodeExemption) {
            $lines.Add('対象実名: 該当なし :: ' + $NoCodeEvidenceReason)
        }
    }
    elseif ($IncludeEvidence) {
        if ($IncludeEvidenceTarget) { $lines.Add('対象実名: `' + $EvidenceTargetPath + '` :: `' + $EvidenceSymbol + '`') }
        if ($IncludeEvidenceCommand) { $lines.Add('実在確認コマンド: `' + $EvidenceCommand + '`') }
        if ($IncludeEvidenceOutput) { $lines.Add('実在確認出力: `' + $EvidenceOutput + '`') }
    }

    return ($lines -join "`n")
}

function Initialize-LiveBaselineStub {
    $worktree = Join-Path $sandboxRoot "live-baseline-worktree"
    [void][IO.Directory]::CreateDirectory($worktree)
    $stubPath = Join-Path $sandboxRoot "git-read-only-stub.cmd"
    $stubHead = "2222222222222222222222222222222222222222"
    $stubText = @"
@echo off
setlocal
:scan
if "%~1"=="" goto unexpected
if /I "%~1"=="rev-parse" goto head
if /I "%~1"=="status" goto status
shift
goto scan
:head
echo $stubHead
exit /b 0
:status
echo  M tracked-a.txt
echo ?? untracked-b.txt
exit /b 0
:unexpected
echo unexpected git stub arguments 1>&2
exit /b 9
"@
    [IO.File]::WriteAllText($stubPath, $stubText.Replace("`n", "`r`n"), [Text.ASCIIEncoding]::new())
    return [pscustomobject]@{ Path = $worktree; Head = $stubHead; DirtyCount = 2; GitExecutable = $stubPath }
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
[void][IO.Directory]::CreateDirectory((Join-Path $fixtureRepository "scripts"))
$evidenceTargetFullPath = Join-Path $fixtureRepository ($evidenceTargetRelativePath.Replace('/', [IO.Path]::DirectorySeparatorChar))
[IO.File]::WriteAllText(
    $evidenceTargetFullPath,
    "# fixture`n$evidenceTargetBody`n    return`n}`n",
    [Text.UTF8Encoding]::new($false)
)
try {
    Write-Output "[1/21] 完全合格ひな型(cargo test込み)は15項目すべてOKでexit 0"
    $fullPassPath = New-InstructionFixtureFile "full-pass.md" (New-InstructionText)
    $result = Invoke-Check @($fullPassPath)
    Assert-Equal $result.ExitCode 0 "完全合格ひな型はexit 0であること" $result.Output
    for ($i = 1; $i -le 15; $i++) {
        Assert-Contains $result.Output ("[OK] {0}." -f $i) ("項目{0}がOKであること" -f $i)
    }
    Assert-Contains $result.Output "全項目合格" "全項目合格の表示があること"

    Write-Output "[2/21] cargoに触れないひな型は項目10・11がN/Aでも全体合格"
    $noCargoPath = New-InstructionFixtureFile "full-pass-no-cargo.md" (New-InstructionText -IncludeCargoTest $false)
    $result = Invoke-Check @($noCargoPath)
    Assert-Equal $result.ExitCode 0 "cargo未記載でも他項目が揃えば合格であること" $result.Output
    Assert-Contains $result.Output "[--] 10. 長時間検査の--skip 4件 (03-品質ゲート.md:61) - cargo testの言及なし（該当なし）" "項目10がN/A表示になること"
    Assert-Contains $result.Output "[--] 11. 専用のCARGO_TARGET_DIR指定 (06-過去の失敗と対策.md:241-242) - cargo実行の言及なし（該当なし）" "項目11がN/A表示になること"
    Assert-Contains $result.Output "全項目合格" "N/Aは不合格に数えないこと"

    Write-Output "[3/21] 「該当なし」＋理由は項目1・14を実パス無しでも合格させる"
    $exemptionPath = New-InstructionFixtureFile "exemption.md" (New-InstructionText -UseNoCodeExemption $true)
    $result = Invoke-Check @($exemptionPath)
    Assert-Contains $result.Output "[OK] 1. 実ファイルパスと関数名の実名記載" "非コード作業の「該当なし」免除が効くこと"
    Assert-Contains $result.Output "[OK] 14. 対象実名ごとの実在確認証拠" "非コード作業は実在証拠も該当なしになること"
    Assert-Contains $result.Output "該当なし" "免除理由の検出詳細を表示すること"

    Write-Output "[4/21] 項目1: 実パスも関数名も無いと不合格"
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item1.md" (New-InstructionText -IncludeTarget $false -IncludeEvidence $false)))
    Assert-Equal $result.ExitCode 1 "項目1欠落はexit 1であること" $result.Output
    Assert-Contains $result.Output "[NG] 1. 実ファイルパスと関数名の実名記載" "項目1がNGになること"
    Assert-Contains $result.Output "欠落: 実ファイルパス・関数名" "欠落した2要素とも列挙されること"

    Write-Output "[5/21] 項目2: 数値条件の誤検出・見逃しを実測する"
    $numericCases = @(
        @{ Text = "半径は3mm以下であること。"; Expect = "OK"; Label = "単位+比較語(mm以下)は合格" },
        @{ Text = 'gapが `<= 200` であること。'; Expect = "OK"; Label = "記号比較(<=)は合格" },
        @{ Text = "16層で200ms以内であること。"; Expect = "OK"; Label = "規約の例文そのものは合格" },
        @{ Text = "3分以内に完了すること。"; Expect = "OK"; Label = "正当な分単位は合格(誤検出ガードの副作用でないこと)" },
        @{ Text = "1分ローズは使わないこと。"; Expect = "NG"; Label = "「1分ローズ」は数値条件と誤検出しないこと" },
        @{ Text = "処理をVec<f64>で行うこと。バージョンはv0.4.5、日付は2026-08-29を参照。第3項を満たすこと。"; Expect = "NG"; Label = "型/バージョン/日付/章番号は数値条件と誤検出しないこと" }
    )
    foreach ($case in $numericCases) {
        $path = New-InstructionFixtureFile ("numeric-{0}.md" -f [Guid]::NewGuid().ToString("N")) (New-InstructionText -NumericOverrideText $case.Text -IncludeBaseline $false)
        $result = Invoke-Check @($path)
        $marker = "[{0}] 2." -f $case.Expect
        Assert-Contains $result.Output $marker $case.Label
    }

    Write-Output "[6/21] 項目3: 6件の禁止事項それぞれについて、1件欠落させると不合格になる"
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

    Write-Output "[7/21] 項目4・5・6・7・8・9: それぞれ単独で欠落させると、その項目だけが不合格になる"
    # 項目7(保存先パス)と項目9(報告書への継続記録)は、どちらも「パス+書く系の動詞」を
    # 見るため、報告書の行が保存先パスの記載を兼ねられる（実測で確認した意図した重なり。
    # 「報告書をXへ書く」という一文は保存先の明記でもあるため、これは誤検出ではない）。
    # そのため項目7を単独で欠落させる場合は、保存先パスの行と報告書継続の行の両方を
    # 外し、その場合は項目9も道連れでNGになることをAlsoNGとして明示する。
    $singleItemCases = @(
        @{ Params = @{ IncludeStaging = $false }; Index = 4; Contains = "段階分割"; AlsoNG = @() },
        @{ Params = @{ IncludeFailureCause = $false }; Index = 5; Contains = "過去の失敗例"; AlsoNG = @() },
        @{ Params = @{ IncludeTool = $false; IncludeEvidence = $false }; Index = 6; Contains = "コマンド形式の記載"; AlsoNG = @() },
        @{ Params = @{ IncludeSavePath = $false; IncludeReportContinuity = $false }; Index = 7; Contains = "保存先パス"; AlsoNG = @(9) },
        @{ Params = @{ IncludeWorktree = $false; IncludeBaselineWorktree = $false }; Index = 8; Contains = "作業ツリーの絶対パス"; AlsoNG = @() },
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

    Write-Output "[8/21] 項目10: skip対象0/4件・一部欠落(2/4件)を検出する"
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

    Write-Output "[9/21] 項目11: CARGO_TARGET_DIR未指定を検出し、項目10には影響しない"
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item11.md" (New-InstructionText -IncludeCargoTargetDir $false)))
    Assert-Equal $result.ExitCode 1 "CARGO_TARGET_DIR欠落はexit 1であること" $result.Output
    Assert-Contains $result.Output "[NG] 11. 専用のCARGO_TARGET_DIR指定 (06-過去の失敗と対策.md:241-242) - cargoの言及があるのにCARGO_TARGET_DIRの指定が見つかりません" "項目11がNGになること"
    Assert-Contains $result.Output "[OK] 10." "CARGO_TARGET_DIR欠落はskip4件判定を巻き込まないこと"

    Write-Output "[10/21] cargo build等cargo testを含まない言及でも、項目11だけは発動する"
    $cargoBuildOnly = '道具: `$env:CARGO_TARGET_DIR = "%TEMP%\ori3-target-x"` を設定してから `cargo build -p ori3-rigid` を実行してください。'
    $result = Invoke-Check @((New-InstructionFixtureFile "cargo-build-only.md" $cargoBuildOnly))
    Assert-Contains $result.Output "[--] 10. 長時間検査の--skip 4件 (03-品質ゲート.md:61) - cargo testの言及なし（該当なし）" "cargo buildはcargo test向けskip判定を発動しないこと"
    Assert-Contains $result.Output "[OK] 11. 専用のCARGO_TARGET_DIR指定 (06-過去の失敗と対策.md:241-242) - CARGO_TARGET_DIRの指定を検出" "cargo buildでもCARGO_TARGET_DIR判定は発動すること"

    Write-Output "[11/21] 複数ファイルを1回の呼び出しで検査できる"
    $secondFailPath = New-InstructionFixtureFile "multi-fail-item5.md" (New-InstructionText -IncludeFailureCause $false)
    $result = Invoke-Check @($fullPassPath, $secondFailPath)
    Assert-Equal $result.ExitCode 1 "1件でも不合格ならexit 1であること" $result.Output
    Assert-Contains $result.Output ("[指示書] {0}" -f $fullPassPath) "1件目のパスが表示されること"
    Assert-Contains $result.Output ("[指示書] {0}" -f $secondFailPath) "2件目のパスが表示されること"
    Assert-Contains $result.Output ("[OK] {0}: 全項目合格" -f $fullPassPath) "1件目は合格として表示されること"
    Assert-Contains $result.Output "対象 2件 / 不合格 1件 / 不合格項目合計 1件" "複数ファイルの集計行が出ること"

    Write-Output "[12/21] 存在しない指示文はexit 2で日本語エラーになる"
    $missingPath = Join-Path $sandboxRoot "does-not-exist.md"
    $result = Invoke-Check @($missingPath)
    Assert-Equal $result.ExitCode 2 "存在しないファイルはexit 2であること" $result.Output
    Assert-Contains $result.Output "指示文ファイルが見つかりません" "日本語のエラー理由を表示すること"

    Write-Output "[13/21] 完全合格ひな型はexit 0、単独不合格はexit 1、存在しないファイルはexit 2の境界を再確認する"
    $result = Invoke-Check @($fullPassPath)
    Assert-Equal $result.ExitCode 0 "合格は0であること(再確認)" $result.Output
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-boundary.md" (New-InstructionText -IncludeWorktree $false -IncludeBaselineWorktree $false)))
    Assert-Equal $result.ExitCode 1 "不合格は1であること(再確認)" $result.Output

    Write-Output "[14/21] 項目12: モデルと理由を同一行で必須化し、実モデル不一致を検出する"
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item12-missing.md" (New-InstructionText -IncludeModelSelection $false)))
    Assert-Equal $result.ExitCode 1 "モデル選択行の欠落はexit 1であること" $result.Output
    Assert-Contains $result.Output "[NG] 12. モデル選択と同一行の理由" "項目12がNGになること"

    $splitModelLine = "モデル選択: opus`n理由: 原因切分けが必要なため。"
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item12-split.md" (New-InstructionText -ModelSelectionLine $splitModelLine)))
    Assert-Equal $result.ExitCode 1 "理由を次行へ分離するとexit 1であること" $result.Output
    Assert-Contains $result.Output "[NG] 12." "同一行でない理由を拒否すること"

    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item12-empty-reason.md" (New-InstructionText -ModelSelectionLine "モデル選択: opus | 理由:   ")))
    Assert-Equal $result.ExitCode 1 "同一行でも空理由はexit 1であること" $result.Output
    Assert-Contains $result.Output "[NG] 12." "空理由を拒否すること"

    $modelMismatchPath = New-InstructionFixtureFile "fail-item12-model-mismatch.md" (New-InstructionText)
    $result = Invoke-Check -Paths @($modelMismatchPath) -ExpectedModel "sonnet"
    Assert-Equal $result.ExitCode 1 "記録opus/実sonnetの不一致はexit 1であること" $result.Output
    Assert-Contains $result.Output "記録モデル opus と実際のモデル sonnet が一致しません" "モデル不一致の実名を表示すること"

    $sonnetPath = New-InstructionFixtureFile "pass-item12-sonnet.md" (New-InstructionText -ModelSelectionLine "モデル選択: sonnet | 理由: 機械的な集計だけを行うため。")
    $result = Invoke-Check -Paths @($sonnetPath) -ExpectedModel "sonnet"
    Assert-Equal $result.ExitCode 0 "sonnetと同一行の理由はexit 0であること" $result.Output
    Assert-Contains $result.Output "[OK] 12." "sonnetの正当な選択を合格にすること"

    $gptPath = New-InstructionFixtureFile "pass-item12-gpt-5.6-sol.md" (New-InstructionText -ModelSelectionLine "モデル選択: gpt-5.6-sol | 理由: Codex復帰時の意味契約の判断が必要なため。")
    $result = Invoke-Check -Paths @($gptPath) -ExpectedModel "gpt-5.6-sol"
    Assert-Equal $result.ExitCode 0 "gpt-5.6-solと同一行の理由はexit 0であること" $result.Output
    Assert-Contains $result.Output "[OK] 12." "gpt-5.6-solの正当な選択と実モデル一致を合格にすること"

    $result = Invoke-Check -Paths @($gptPath) -ExpectedModel "opus"
    Assert-Equal $result.ExitCode 1 "記録gpt-5.6-sol/実opusの不一致はexit 1であること" $result.Output
    Assert-Contains $result.Output "記録モデル gpt-5.6-sol と実際のモデル opus が一致しません" "gpt-5.6-solのモデル不一致を表示すること"

    Write-Output "[15/21] 項目13: baselineの5要素、終了コード、数値実測、40/64hexを検査する"
    $baselineMissingCases = @(
        @{ Flag = "IncludeBaselineWorktree"; Label = "worktree" },
        @{ Flag = "IncludeBaselineHead"; Label = "HEAD(40/64hex)" },
        @{ Flag = "IncludeBaselineDirty"; Label = "未コミット件数" },
        @{ Flag = "IncludeBaselineCommand"; Label = "対象検査command" },
        @{ Flag = "IncludeBaselineResult"; Label = "対象検査baseline" }
    )
    foreach ($case in $baselineMissingCases) {
        $params = @{ $case.Flag = $false }
        $path = New-InstructionFixtureFile ("fail-item13-{0}.md" -f $case.Flag) (New-InstructionText @params)
        $result = Invoke-Check @($path)
        Assert-Equal $result.ExitCode 1 ("baselineの{0}欠落はexit 1であること" -f $case.Label) $result.Output
        Assert-Contains $result.Output "[NG] 13." ("{0}欠落で項目13がNGになること" -f $case.Label)
        Assert-Contains $result.Output $case.Label ("項目13が欠落要素{0}を表示すること" -f $case.Label)
    }

    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item13-no-exit.md" (New-InstructionText -BaselineResult "17 passed / 8 failed")))
    Assert-Equal $result.ExitCode 1 "終了コード無しbaselineはexit 1であること" $result.Output
    Assert-Contains $result.Output "exit/終了コードがありません" "終了コード欠落の理由を表示すること"
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item13-no-measurement.md" (New-InstructionText -BaselineResult "exit=0")))
    Assert-Equal $result.ExitCode 1 "終了コード以外の数値実測無しはexit 1であること" $result.Output
    Assert-Contains $result.Output "終了コード以外の数値実測がありません" "数値実測欠落の理由を表示すること"

    $head64 = "a" * 64
    $result = Invoke-Check @((New-InstructionFixtureFile "pass-item13-head64.md" (New-InstructionText -BaselineHead $head64 -BaselineResult "終了コード: 0; 0 failures")))
    Assert-Equal $result.ExitCode 0 "64hex HEADと日本語終了コードはexit 0であること" $result.Output
    Assert-Contains $result.Output "[OK] 13." "64hex HEADを合格にすること"

    Write-Output "[16/21] 項目14: target/command/output欠落、rg除外漏れ、古い行、本文不一致を拒否する"
    $evidenceCases = @(
        @{ Name = "missing-target"; Params = @{ IncludeEvidenceTarget = $false }; Expected = "構造化した対象実名/実在確認コマンド/実在確認出力がありません" },
        @{ Name = "missing-command"; Params = @{ IncludeEvidenceCommand = $false }; Expected = "command/outputが1:1ではありません" },
        @{ Name = "missing-output"; Params = @{ IncludeEvidenceOutput = $false }; Expected = "command/outputが1:1ではありません" },
        @{ Name = "missing-exclusion"; Params = @{ EvidenceCommand = "rg -n --fixed-strings 'Invoke-SampleTarget' scripts/sample-target.ps1" }; Expected = "--glob '!docs/competitive-review-2026-08-20.md' がありません" },
        @{ Name = "command-path-mismatch"; Params = @{ EvidenceCommand = "rg -n --fixed-strings --glob '!docs/competitive-review-2026-08-20.md' 'Invoke-SampleTarget' scripts/other-target.ps1" }; Expected = "commandに対象pathがありません" },
        @{ Name = "command-symbol-mismatch"; Params = @{ EvidenceCommand = "rg -n --fixed-strings --glob '!docs/competitive-review-2026-08-20.md' 'Old-SampleTarget' scripts/sample-target.ps1" }; Expected = "commandに対象symbolがありません" },
        @{ Name = "output-path-mismatch"; Params = @{ EvidenceOutput = "scripts/other-target.ps1:2:function Invoke-SampleTarget {" }; Expected = "出力path scripts/other-target.ps1 が対象pathと一致しません" },
        @{ Name = "stale-line"; Params = @{ EvidenceOutput = "scripts/sample-target.ps1:1:function Invoke-SampleTarget {" }; Expected = "出力本文が実ファイル1行目と一致しません" },
        @{ Name = "wrong-body"; Params = @{ EvidenceOutput = "scripts/sample-target.ps1:2:function Old-SampleTarget {" }; Expected = "出力本文が実ファイル2行目と一致しません" },
        @{
            Name = "output-symbol-mismatch"
            Params = @{
                EvidenceSymbol = "Other-SampleTarget()"
                EvidenceCommand = "rg -n --fixed-strings --glob '!docs/competitive-review-2026-08-20.md' 'Other-SampleTarget' scripts/sample-target.ps1"
            }
            Expected = "実ファイル行に対象symbolがありません"
        },
        @{
            Name = "prohibited-document"
            Params = @{
                EvidenceTargetPath = "docs/competitive-review-2026-08-20.md"
                EvidenceCommand = "rg -n --fixed-strings --glob '!docs/competitive-review-2026-08-20.md' 'Invoke-SampleTarget' docs/competitive-review-2026-08-20.md"
                EvidenceOutput = "docs/competitive-review-2026-08-20.md:1:Invoke-SampleTarget"
            }
            Expected = "禁止文書なので対象実名にできません"
        }
    )
    foreach ($case in $evidenceCases) {
        $evidenceParams = $case.Params
        $path = New-InstructionFixtureFile ("fail-item14-{0}.md" -f $case.Name) (New-InstructionText @evidenceParams)
        $result = Invoke-Check @($path)
        Assert-Equal $result.ExitCode 1 ("実在証拠{0}はexit 1であること" -f $case.Name) $result.Output
        Assert-Contains $result.Output "[NG] 14." ("実在証拠{0}で項目14がNGになること" -f $case.Name)
        Assert-Contains $result.Output $case.Expected ("実在証拠{0}の拒否理由を表示すること" -f $case.Name)
    }

    $grepPath = New-InstructionFixtureFile "pass-item14-grep.md" (New-InstructionText -EvidenceCommand "grep -n 'Invoke-SampleTarget' scripts/sample-target.ps1")
    $result = Invoke-Check @($grepPath)
    Assert-Equal $result.ExitCode 0 "grepの構造化証拠はexit 0であること" $result.Output
    Assert-Contains $result.Output "[OK] 14." "grepを許可すること"

    Write-Output "[16b/21] 項目14: 該当なし免除は単独の構造化1組・4文字以上の理由だけ許可する"
    $plainNoCodeText = (New-InstructionText -IncludeEvidence $false) + "`n補足: 実在確認は該当なしです。"
    $plainNoCodePath = New-InstructionFixtureFile "fail-item14-plain-no-code.md" $plainNoCodeText
    $result = Invoke-Check @($plainNoCodePath)
    Assert-Equal $result.ExitCode 1 "本文中だけの該当なしはexit 1であること" $result.Output
    Assert-Contains $result.Output "構造化した対象実名/実在確認コマンド/実在確認出力がありません" "本文の該当なしでbypassしないこと"

    $missingStructuredNoCodePath = New-InstructionFixtureFile "fail-item14-no-code-without-structure.md" (New-InstructionText -UseNoCodeExemption $true -IncludeStructuredNoCodeExemption $false)
    $result = Invoke-Check @($missingStructuredNoCodePath)
    Assert-Equal $result.ExitCode 1 "非コード作業でも構造化免除無しはexit 1であること" $result.Output
    Assert-Contains $result.Output "[NG] 14." "従来の任意位置の該当なしでbypassしないこと"

    $shortReasonPath = New-InstructionFixtureFile "fail-item14-no-code-short-reason.md" (New-InstructionText -UseNoCodeExemption $true -NoCodeEvidenceReason "短い")
    $result = Invoke-Check @($shortReasonPath)
    Assert-Equal $result.ExitCode 1 "4文字未満の構造化免除理由はexit 1であること" $result.Output
    Assert-Contains $result.Output "4文字以上必要です" "短い免除理由を拒否すること"

    $noCodeWithEvidence = (New-InstructionText -UseNoCodeExemption $true) + @'

実在確認コマンド: `rg -n --glob '!docs/competitive-review-2026-08-20.md' 'unused' scripts/sample-target.ps1`
実在確認出力: `scripts/sample-target.ps1:2:function Invoke-SampleTarget {`
'@
    $noCodeWithEvidencePath = New-InstructionFixtureFile "fail-item14-no-code-with-command-output.md" $noCodeWithEvidence
    $result = Invoke-Check @($noCodeWithEvidencePath)
    Assert-Equal $result.ExitCode 1 "構造化免除へのcommand/output付加はexit 1であること" $result.Output
    Assert-Contains $result.Output "実在確認command/outputを付けられません" "免除と証拠の混在を拒否すること"

    $noCodeMixedTarget = (New-InstructionText -UseNoCodeExemption $true) + @'

対象実名: `scripts/sample-target.ps1` :: `Invoke-SampleTarget()`
実在確認コマンド: `rg -n --fixed-strings --glob '!docs/competitive-review-2026-08-20.md' 'Invoke-SampleTarget' scripts/sample-target.ps1`
実在確認出力: `scripts/sample-target.ps1:2:function Invoke-SampleTarget {`
'@
    $noCodeMixedPath = New-InstructionFixtureFile "fail-item14-no-code-mixed-target.md" $noCodeMixedTarget
    $result = Invoke-Check @($noCodeMixedPath)
    Assert-Equal $result.ExitCode 1 "構造化免除と実名対象の混在はexit 1であること" $result.Output
    Assert-Contains $result.Output "他の対象実名と混在できません" "免除と対象実名の混在を拒否すること"

    Write-Output "[17/21] 項目14: 複数対象を全件1:1照合し、2件目だけの欠落も拒否する"
    $secondTargetPath = Join-Path $fixtureRepository "scripts/second-target.rs"
    [IO.File]::WriteAllText($secondTargetPath, "fn second_target() {}`n", [Text.UTF8Encoding]::new($false))
    $secondEvidence = @'
対象実名: `scripts/second-target.rs` :: `second_target()`
実在確認コマンド: `rg -n --fixed-strings --glob '!docs/competitive-review-2026-08-20.md' 'second_target' scripts/second-target.rs`
実在確認出力: `scripts/second-target.rs:1:fn second_target() {}`
'@
    $multiplePath = New-InstructionFixtureFile "pass-item14-multiple.md" ((New-InstructionText) + "`n" + $secondEvidence)
    $result = Invoke-Check @($multiplePath)
    Assert-Equal $result.ExitCode 0 "2対象とも1:1ならexit 0であること" $result.Output
    Assert-Contains $result.Output "2対象のcommand/output/path/line/body/symbolを実ファイルと照合" "2対象を全件照合した件数を表示すること"

    $secondEvidenceWithoutOutput = @'
対象実名: `scripts/second-target.rs` :: `second_target()`
実在確認コマンド: `rg -n --fixed-strings --glob '!docs/competitive-review-2026-08-20.md' 'second_target' scripts/second-target.rs`
'@
    $multipleMissingPath = New-InstructionFixtureFile "fail-item14-multiple-second-missing.md" ((New-InstructionText) + "`n" + $secondEvidenceWithoutOutput)
    $result = Invoke-Check @($multipleMissingPath)
    Assert-Equal $result.ExitCode 1 "2件目だけoutput欠落でもexit 1であること" $result.Output
    Assert-Contains $result.Output "scripts/second-target.rs::second_target() はcommand/outputが1:1ではありません" "欠落した2件目を実名表示すること"

    Write-Output "[18/21] 標準入力modeを新processで実行し、空stdinはexit 2にする"
    $result = Invoke-Check -StandardInput (New-InstructionText)
    Assert-Equal $result.ExitCode 0 "stdinの完全指示書はexit 0であること" $result.Output
    Assert-Contains $result.Output "[指示書] <stdin>" "stdin入力であることを表示すること"
    Assert-Contains $result.Output "[OK] <stdin>: 全項目合格" "stdinでも15項目すべて合格すること"
    $result = Invoke-Check -StandardInput ""
    Assert-Equal $result.ExitCode 2 "空stdinはexit 2であること" $result.Output
    Assert-Contains $result.Output "標準入力の指示文が空です" "空stdinの理由を日本語表示すること"

    Write-Output "[19/21] -VerifyLiveBaselineがgitを書かないstubのHEADと未コミット件数を照合する"
    $liveBaseline = Initialize-LiveBaselineStub
    $liveText = New-InstructionText -BaselineWorktree $liveBaseline.Path -BaselineHead $liveBaseline.Head -BaselineDirty $liveBaseline.DirtyCount
    $livePath = New-InstructionFixtureFile "pass-item13-live.md" $liveText
    $result = Invoke-Check -Paths @($livePath) -VerifyLiveBaseline -GitExecutable $liveBaseline.GitExecutable
    Assert-Equal $result.ExitCode 0 "live HEAD/dirty一致はexit 0であること" $result.Output
    Assert-Contains $result.Output "live照合一致" "live照合実施を表示すること"

    $wrongHeadText = New-InstructionText -BaselineWorktree $liveBaseline.Path -BaselineHead ("0" * 40) -BaselineDirty $liveBaseline.DirtyCount
    $wrongHeadPath = New-InstructionFixtureFile "fail-item13-live-head.md" $wrongHeadText
    $result = Invoke-Check -Paths @($wrongHeadPath) -VerifyLiveBaseline -GitExecutable $liveBaseline.GitExecutable
    Assert-Equal $result.ExitCode 1 "live HEAD不一致はexit 1であること" $result.Output
    Assert-Contains $result.Output "記録HEAD" "live HEAD不一致の両値を表示すること"

    $wrongDirtyText = New-InstructionText -BaselineWorktree $liveBaseline.Path -BaselineHead $liveBaseline.Head -BaselineDirty 3
    $wrongDirtyPath = New-InstructionFixtureFile "fail-item13-live-dirty.md" $wrongDirtyText
    $result = Invoke-Check -Paths @($wrongDirtyPath) -VerifyLiveBaseline -GitExecutable $liveBaseline.GitExecutable
    Assert-Equal $result.ExitCode 1 "live未コミット件数不一致はexit 1であること" $result.Output
    Assert-Contains $result.Output "記録した未コミット件数 3 と実測 2 が一致しません" "live dirty不一致の両値を表示すること"

    Write-Output "[20/21] 既存14項目と新しい成果物上限を同じ集計・exit境界で扱う"
    $finalPass = Invoke-Check @($fullPassPath)
    Assert-Equal $finalPass.ExitCode 0 "最終完全合格はexit 0であること" $finalPass.Output
    Assert-Contains $finalPass.Output "対象 1件 / 全件合格" "最終集計が全件合格であること"

    Write-Output "[21/21] 項目15: 4成果物は拒否し、3成果物は許可し、件数×観点の直積も数える"
    $fourDeliverables = @("結果1", "結果2", "結果3", "結果4")
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item15-four.md" (New-InstructionText -RequestedDeliverables $fourDeliverables)))
    Assert-Equal $result.ExitCode 1 "4成果物の委譲はexit 1であること" $result.Output
    Assert-Contains $result.Output "[NG] 15. 1委譲の独立成果物上限" "4成果物で項目15がNGになること"
    Assert-Contains $result.Output "現在 4件、上限 3件" "拒否文に現在4件と上限3件を表示すること"

    $threeDeliverables = @("結果1", "結果2", "結果3")
    $result = Invoke-Check @((New-InstructionFixtureFile "pass-item15-three.md" (New-InstructionText -RequestedDeliverables $threeDeliverables)))
    Assert-Equal $result.ExitCode 0 "3成果物の委譲はexit 0であること" $result.Output
    Assert-Contains $result.Output "[OK] 15. 1委譲の独立成果物上限" "3成果物で項目15がOKになること"
    Assert-Contains $result.Output "現在 3件、上限 3件" "許可文に現在3件と上限3件を表示すること"

    $productText = (New-InstructionText) + "`n`n設計案2件 × 4観点をそれぞれ独立に報告してください。"
    $result = Invoke-Check @((New-InstructionFixtureFile "fail-item15-product.md" $productText))
    Assert-Equal $result.ExitCode 1 "設計案2件×4観点は8成果物としてexit 1であること" $result.Output
    Assert-Contains $result.Output "現在 8件、上限 3件" "直積8件と上限3件を表示すること"
    Assert-Contains $result.Output "判定根拠: 件数と観点の直積" "直積を判定根拠として表示すること"

    Write-Output ("check-agent-instruction self-test passed: {0} cases, {1} assertions" -f $script:CaseCount, $script:AssertionCount)
}
finally {
    Remove-TestSandbox
}
