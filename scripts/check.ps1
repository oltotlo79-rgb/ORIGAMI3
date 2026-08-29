# ORIGAMI3 一括検査スクリプト(Windows PowerShell 5.1 対応)
# 5つの検査を順に実行し、いずれかが失敗したら非0で終了する。
#   (1) cargo test --workspace(提案の探索の #18〜#21 の4件を --skip する。
#       最適化なしでは現実的な時間で終わらないか、watchdogに当たるため。
#       最適化ありで確かめるコマンドは CLAUDE.md §10.6 の #18〜#21)
#   (2) cargo clippy --workspace --all-targets -- -D warnings
#   (3) apps/desktop で npm run build
#   (4) apps/desktop で npm run lint
#   (5) apps/desktop で npm run test (vitest)

$root = Split-Path -Parent $PSScriptRoot
$receiptHelper = Join-Path $PSScriptRoot "check-receipt.ps1"
$receiptAvailable = $false
$rustW4Arguments = @(
    "test", "--workspace", "--no-fail-fast", "--",
    "--skip", "completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten",
    "--skip", "named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten",
    "--skip", "a_safe_coincident_partial_network_appears_after_the_first_fold",
    "--skip", "the_heaviest_proposal_never_hits_the_time_limit"
)
$clippyArguments = @("clippy", "--workspace", "--all-targets", "--", "-D", "warnings")
$npmBuildArguments = @("run", "build")
$npmLintArguments = @("run", "lint")
$npmTestArguments = @("run", "test")
try {
    if (-not (Test-Path -LiteralPath $receiptHelper -PathType Leaf)) {
        throw "receipt helperが見つかりません: $receiptHelper"
    }
    . $receiptHelper
    # 実行argvとreceiptのrecipeを同じ定義から取る。
    $rustW4Arguments = Get-Ori3RustW4Arguments
    $clippyArguments = Get-Ori3ClippyArguments
    $npmBuildArguments = Get-Ori3NpmBuildArguments
    $npmLintArguments = Get-Ori3NpmLintArguments
    $npmTestArguments = Get-Ori3NpmTestArguments
    $receiptAvailable = $true
}
catch {
    Write-Host "[WARN] receipt判定を使えないため、従来どおり全5検査を実行します: $($_.Exception.Message)" -ForegroundColor Yellow
}

# receipt helperを引数の正本として使う場合も、失敗したtest targetの後ろを
# cargoに続行させ、workspace内の赤を1回の実行ですべて列挙する。
if ($rustW4Arguments -notcontains "--no-fail-fast") {
    $testArgumentSeparator = [Array]::IndexOf([object[]]$rustW4Arguments, "--")
    if ($testArgumentSeparator -lt 0) {
        throw "cargo test引数にtest harnessとの区切り（--）がありません"
    }
    $rustArgumentPrefix = @($rustW4Arguments[0..($testArgumentSeparator - 1)])
    $rustArgumentSuffix = @($rustW4Arguments[$testArgumentSeparator..($rustW4Arguments.Length - 1)])
    $rustW4Arguments = @($rustArgumentPrefix + "--no-fail-fast" + $rustArgumentSuffix)
}

function Invoke-Check {
    param([string]$Name, [string]$Command, [string[]]$CommandArgs)
    Write-Host ""
    Write-Host "=== $Name ===" -ForegroundColor Cyan
    # 直前のコマンドの終了コードが残って偽の合格にならないよう必ずリセットする
    $global:LASTEXITCODE = 0
    # 注意: コマンド不在などの起動失敗は文終了エラーになり、外側のtry/finally配下では
    # 後続の判定行に制御が届かない。ここでcatchして確実に失敗終了させる。
    try {
        & $Command @CommandArgs
    } catch {
        Write-Host "[NG] $Name を起動できませんでした: $($_.Exception.Message)" -ForegroundColor Red
        exit 1
    }
    if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        Write-Host "[NG] $Name が失敗しました(終了コード: $LASTEXITCODE)" -ForegroundColor Red
        exit $LASTEXITCODE
    }
}

Push-Location $root
try {
    # テストが追跡対象のファイルを書き換えていないかを、実行の前後で比べる。
    # 途中経過を書き出すテストが tests/fixtures/ の作品ファイルを毎回上書きしており、
    # テストを走らせるだけで作業ツリーが汚れていた。
    # 同じファイルを別のテストが読むため、実行順で読む内容が変わる状態でもあった。
    $global:LASTEXITCODE = 0
    $beforeTracked = (& git -C $root status --porcelain --untracked-files=no) -join "`n"
    $beforeTrackedStatus = $LASTEXITCODE

    $reuseAllChecks = $false
    $fullReceiptContext = $null
    if ($receiptAvailable) {
        try {
            $fullReceiptContext = New-Ori3ReceiptContext "check-all" $root $null
            $fullHit = Find-Ori3CheckReceipt $fullReceiptContext
            if ($fullHit.IsHit) {
                # hitを見つけた後にも内容/条件を再計算し、確認中の変更で
                # 検査していない内容を省略しない。
                $fullConfirmation = New-Ori3ReceiptContext "check-all" $root $null
                $global:LASTEXITCODE = 0
                $afterReceiptProbeTracked = (& git -C $root status --porcelain --untracked-files=no) -join "`n"
                $afterReceiptProbeStatus = $LASTEXITCODE
                if ($fullConfirmation.EligibilitySha256 -eq $fullReceiptContext.EligibilitySha256 -and
                    $beforeTrackedStatus -eq 0 -and $afterReceiptProbeStatus -eq 0 -and
                    $afterReceiptProbeTracked -eq $beforeTracked) {
                    Write-Ori3ReceiptReuseMessage "手元の全5検査" $fullHit
                    $reuseAllChecks = $true
                }
                else {
                    Write-Host "[RUN] receipt確認中に作業内容/条件が変わったため、全5検査を実行します" -ForegroundColor Yellow
                    $fullReceiptContext = $fullConfirmation
                    $beforeTracked = $afterReceiptProbeTracked
                    $beforeTrackedStatus = $afterReceiptProbeStatus
                }
            }
            else {
                Write-Ori3ReceiptMissMessage "手元の全5検査" $fullHit
            }
        }
        catch {
            Write-Host "[WARN] 全5検査のreceiptを判定できないため実行します: $($_.Exception.Message)" -ForegroundColor Yellow
            $fullReceiptContext = $null
        }
    }

    if ($reuseAllChecks) {
        Write-Host ""
        Write-Host "[OK] 全ての検査に合格済みです (local receipt再利用)" -ForegroundColor Green
    }
    else {
        $reuseRustW4 = $false
        $rustReceiptContext = $null
        $rustReceiptForComposition = $null
        $fullMaximumExpiryUtc = $null
        if ($receiptAvailable) {
            try {
                $contentSnapshot = if ($null -ne $fullReceiptContext) { $fullReceiptContext.Content } else { $null }
                $rustReceiptContext = New-Ori3ReceiptContext "rust-w4" $root $contentSnapshot
                $rustHit = Find-Ori3CheckReceipt $rustReceiptContext
                if ($rustHit.IsHit) {
                    $rustConfirmation = New-Ori3ReceiptContext "rust-w4" $root $null
                    $global:LASTEXITCODE = 0
                    $afterRustProbeTracked = (& git -C $root status --porcelain --untracked-files=no) -join "`n"
                    $afterRustProbeStatus = $LASTEXITCODE
                    if ($rustConfirmation.EligibilitySha256 -eq $rustReceiptContext.EligibilitySha256 -and
                        $beforeTrackedStatus -eq 0 -and $afterRustProbeStatus -eq 0 -and
                        $afterRustProbeTracked -eq $beforeTracked) {
                        Write-Ori3ReceiptReuseMessage "(1/5) Rust W4" $rustHit
                        $reuseRustW4 = $true
                        $rustReceiptForComposition = $rustHit.Receipt
                        $fullMaximumExpiryUtc = [DateTime]::Parse([string]$rustHit.Receipt.expiresAtUtc).ToUniversalTime()
                    }
                    else {
                        Write-Host "[RUN] receipt確認中に作業内容/条件が変わったためRust W4を実行します" -ForegroundColor Yellow
                        $rustReceiptContext = $rustConfirmation
                        $beforeTracked = $afterRustProbeTracked
                        $beforeTrackedStatus = $afterRustProbeStatus
                        try {
                            $fullReceiptContext = New-Ori3ReceiptContext "check-all" $root $rustConfirmation.Content
                        }
                        catch {
                            $fullReceiptContext = $null
                        }
                    }
                }
                else {
                    Write-Ori3ReceiptMissMessage "(1/5) Rust W4" $rustHit
                }
            }
            catch {
                Write-Host "[WARN] Rust W4のreceiptを判定できないため実行します: $($_.Exception.Message)" -ForegroundColor Yellow
                $rustReceiptContext = $null
            }
        }

        if (-not $reuseRustW4) {
            Invoke-Check "(1/5) cargo test --workspace" cargo $rustW4Arguments
        }

        $global:LASTEXITCODE = 0
        $afterTracked = (& git -C $root status --porcelain --untracked-files=no) -join "`n"
        $afterTrackedStatus = $LASTEXITCODE
        if ($afterTracked -ne $beforeTracked) {
            Write-Host ""
            Write-Host "[NG] テストが追跡対象のファイルを書き換えました" -ForegroundColor Red
            Write-Host "実行前:" -ForegroundColor Yellow
            Write-Host $beforeTracked
            Write-Host "実行後:" -ForegroundColor Yellow
            Write-Host $afterTracked
            exit 1
        }
        if ($beforeTrackedStatus -ne 0 -or $afterTrackedStatus -ne 0) {
            # 従来の検査結果は変えないが、guardを検証できない合格を
            # 次回の省略根拠にはしない。
            Write-Host "[WARN] 追跡対象変更guardのgit statusが失敗したためreceiptを記録しません" -ForegroundColor Yellow
            $rustReceiptContext = $null
            $fullReceiptContext = $null
        }
        elseif (-not $reuseRustW4) {
            # The composite receipt must never outlive its oldest component.
            # Start the W4 24-hour window here, before the remaining four checks.
            $fullMaximumExpiryUtc = [DateTime]::UtcNow.AddHours(24)
        }

        if (-not $reuseRustW4 -and $null -ne $rustReceiptContext) {
            try {
                $rustReceiptPath = Write-Ori3CheckReceipt $rustReceiptContext
                Write-Host "[receipt] Rust W4の合格を記録しました: $rustReceiptPath" -ForegroundColor DarkGreen
            }
            catch {
                Write-Host "[WARN] Rust W4は合格しましたがreceiptは記録しません: $($_.Exception.Message)" -ForegroundColor Yellow
            }
        }

        Invoke-Check "(2/5) cargo clippy --workspace --all-targets -- -D warnings" cargo $clippyArguments

        Set-Location (Join-Path $root "apps\desktop")

        Invoke-Check "(3/5) npm run build (apps/desktop)" npm $npmBuildArguments
        Invoke-Check "(4/5) npm run lint (apps/desktop)" npm $npmLintArguments
        Invoke-Check "(5/5) npm run test (apps/desktop)" npm $npmTestArguments

        Set-Location $root
        if ($null -ne $fullReceiptContext) {
            try {
                if ($null -ne $fullMaximumExpiryUtc) {
                    if ($null -ne $rustReceiptForComposition) {
                        $fullReceiptPath = Write-Ori3CheckReceipt $fullReceiptContext -MaximumExpiryUtc $fullMaximumExpiryUtc -ReusedComponentReceipt $rustReceiptForComposition
                    }
                    else {
                        $fullReceiptPath = Write-Ori3CheckReceipt $fullReceiptContext -MaximumExpiryUtc $fullMaximumExpiryUtc
                    }
                }
                else {
                    $fullReceiptPath = Write-Ori3CheckReceipt $fullReceiptContext
                }
                Write-Host "[receipt] 手元の全5検査の合格を記録しました: $fullReceiptPath" -ForegroundColor DarkGreen
            }
            catch {
                Write-Host "[WARN] 全5検査は合格しましたがreceiptは記録しません: $($_.Exception.Message)" -ForegroundColor Yellow
            }
        }

        Write-Host ""
        Write-Host "[OK] 全ての検査に合格しました" -ForegroundColor Green
    }
}
finally {
    Pop-Location
}
exit 0
