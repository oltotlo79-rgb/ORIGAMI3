[CmdletBinding()]
param(
    [ValidateSet("Validate", "Performance", "Full")]
    [string]$Mode = "Validate",
    [switch]$Resume
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$MatrixIterations = 100
$ExpectedCandidateHash = "b5404e822ccd3603"
$ExpectedStopHash = "ea05a0f8b88739bb"
$LocalTargetDir = "C:\Users\oltot\AppData\Local\Temp\ori3-target-propose1d"
$RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$OutputRoot = Join-Path $RepositoryRoot "verification\propose-matrix"
$StatePath = Join-Path $OutputRoot "matrix-state.json"
$IsCi = $env:CI -eq "true" -or $env:GITHUB_ACTIONS -eq "true"

if ($Resume -and $Mode -ne "Full") {
    throw "-ResumeはFullでだけ使えます"
}
if ($Mode -eq "Full" -and $IsCi) {
    throw "FullはCIでは実行しません。ほかの作業を止めたリリース直前に手元で実行してください"
}

if ($Mode -in @("Validate", "Full")) {
    if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
        $env:CARGO_TARGET_DIR = $LocalTargetDir
    }
    $ActiveTargetDir = [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
    if (-not [string]::Equals(
        $ActiveTargetDir,
        [IO.Path]::GetFullPath($LocalTargetDir),
        [StringComparison]::OrdinalIgnoreCase
    )) {
        throw "Validate/FullのCARGO_TARGET_DIRが指定先と違います"
    }
}
elseif (-not [string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
    # check-ci.ps1 は複製専用targetを設定する。Performanceはそれを上書きしない。
    $ActiveTargetDir = [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
}
elseif ($IsCi) {
    # GitHub Actionsではrust-cacheが扱う既定のrepository targetを使う。
    $ActiveTargetDir = [IO.Path]::GetFullPath((Join-Path $RepositoryRoot "target"))
}
else {
    $env:CARGO_TARGET_DIR = $LocalTargetDir
    $ActiveTargetDir = [IO.Path]::GetFullPath($LocalTargetDir)
}

function Write-JsonAtomic {
    param(
        [Parameter(Mandatory = $true)]$Value,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $temporary = "$Path.tmp"
    $json = $Value | ConvertTo-Json -Depth 12
    [System.IO.File]::WriteAllText($temporary, $json, [System.Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporary -Destination $Path -Force
}

function Get-DiskSnapshot {
    $drive = [System.IO.DriveInfo]::new("C:\")
    [pscustomobject]@{
        checked_at = (Get-Date).ToString("o")
        total_bytes = $drive.TotalSize
        free_bytes = $drive.AvailableFreeSpace
        free_gib = [math]::Round($drive.AvailableFreeSpace / 1GB, 3)
    }
}

function Convert-BytesToHex {
    param([byte[]]$Bytes)
    -join ($Bytes | ForEach-Object { $_.ToString("x2") })
}

function Get-InputFingerprint {
    $paths = @(
        Get-ChildItem -LiteralPath (Join-Path $RepositoryRoot "crates\ori3-propose\src") -File -Filter "*.rs"
        foreach ($crate in @("ori3-model", "ori3-geometry", "ori3-cp", "ori3-rigid", "ori3-layers")) {
            Get-ChildItem -LiteralPath (Join-Path $RepositoryRoot "crates\$crate\src") -File -Filter "*.rs" -Recurse
            Get-Item -LiteralPath (Join-Path $RepositoryRoot "crates\$crate\Cargo.toml")
        }
        Get-ChildItem -LiteralPath (Join-Path $RepositoryRoot "crates\ori3-propose\tests\fixtures") -File
        Get-Item -LiteralPath (Join-Path $RepositoryRoot "crates\ori3-propose\tests\acceptance.rs")
        Get-Item -LiteralPath (Join-Path $RepositoryRoot "crates\ori3-propose\tests\end_to_end.rs")
        Get-Item -LiteralPath (Join-Path $RepositoryRoot "crates\ori3-propose\tests\proposal_matrix.rs")
        Get-Item -LiteralPath (Join-Path $RepositoryRoot "crates\ori3-propose\tests\run-proposal-matrix.ps1")
        Get-ChildItem -LiteralPath (Join-Path $RepositoryRoot "apps\desktop\src-tauri\src") -File -Filter "*.rs" -Recurse
        Get-Item -LiteralPath (Join-Path $RepositoryRoot "apps\desktop\src-tauri\Cargo.toml")
        Get-Item -LiteralPath (Join-Path $RepositoryRoot "apps\desktop\src-tauri\build.rs")
        Get-Item -LiteralPath (Join-Path $RepositoryRoot "Cargo.toml")
        Get-Item -LiteralPath (Join-Path $RepositoryRoot "Cargo.lock")
        Get-Item -LiteralPath (Join-Path $RepositoryRoot "crates\ori3-propose\Cargo.toml")
    ) | Sort-Object FullName -Unique

    $files = foreach ($path in $paths) {
        $relativePath = $path.FullName
        $rootPrefix = $RepositoryRoot.TrimEnd("\") + "\"
        if ($relativePath.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            $relativePath = $relativePath.Substring($rootPrefix.Length)
        }
        [ordered]@{
            path = $relativePath.Replace("\", "/")
            sha256 = (Get-FileHash -LiteralPath $path.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    $text = $files | ConvertTo-Json -Depth 4 -Compress
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $aggregate = Convert-BytesToHex -Bytes ($sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($text)))
    }
    finally {
        $sha.Dispose()
    }
    [pscustomobject]@{
        aggregate_sha256 = $aggregate
        files = $files
    }
}

function Clear-MatrixEnvironment {
    foreach ($name in @(
        "ORI3_PROPOSAL_MATRIX_CANDIDATES",
        "ORI3_PROPOSAL_MATRIX_REQUESTS",
        "ORI3_PROPOSAL_MATRIX_COMPUTATION",
        "ORI3_PROPOSAL_MATRIX_LOAD",
        "ORI3_PROPOSAL_MATRIX_PROFILE",
        "ORI3_PROPOSAL_MATRIX_ITERATIONS"
    )) {
        [Environment]::SetEnvironmentVariable($name, $null, "Process")
    }
}

function Invoke-CargoLogged {
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$LogPath
    )

    $started = Get-Date
    $previousErrorAction = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = @(& cargo @Arguments 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorAction
    }
    $text = ($output | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
    [System.IO.File]::WriteAllText($LogPath, $text + [Environment]::NewLine, [System.Text.UTF8Encoding]::new($false))
    foreach ($line in $output) {
        Write-Host $line
    }
    [pscustomobject]@{
        exit_code = $exitCode
        elapsed_seconds = [math]::Round(((Get-Date) - $started).TotalSeconds, 3)
        output = $text
        warning_count = @($output | Where-Object { $_.ToString() -match "(?i)\bwarn(?:ing)?:" }).Count
    }
}

function Parse-MatrixResult {
    param([Parameter(Mandatory = $true)][string]$Output)

    $pattern = "PROPOSAL_MATRIX_RESULT profile=(?<profile>\w+) candidates=(?<candidates>\d+) requests=(?<requests>\d+) computation=(?<computation>\w+) load=(?<load>\w+) load_threads=(?<load_threads>\d+) iterations=(?<iterations>\d+) candidate_hash=(?<candidate_hash>[0-9a-f]{16}) stop_hash=(?<stop_hash>[0-9a-f]{16}) first_candidate_hash=(?<first_candidate_hash>[0-9a-f]{16}) first_stop=(?<first_stop>[a-z_]+) stops=(?<stops>[a-z_|]+)"
    $match = [regex]::Match($Output, $pattern)
    if (-not $match.Success) {
        throw "matrix probeの結果行を読めません"
    }
    [pscustomobject]@{
        profile = $match.Groups["profile"].Value
        candidates = [int]$match.Groups["candidates"].Value
        requests = [int]$match.Groups["requests"].Value
        computation = $match.Groups["computation"].Value
        load = $match.Groups["load"].Value
        load_threads = [int]$match.Groups["load_threads"].Value
        iterations = [int]$match.Groups["iterations"].Value
        candidate_hash = $match.Groups["candidate_hash"].Value
        stop_hash = $match.Groups["stop_hash"].Value
        first_candidate_hash = $match.Groups["first_candidate_hash"].Value
        first_stop = $match.Groups["first_stop"].Value
        stops = $match.Groups["stops"].Value
    }
}

function Invoke-MatrixProbe {
    param(
        [Parameter(Mandatory = $true)][int]$Candidates,
        [Parameter(Mandatory = $true)][int]$Requests,
        [Parameter(Mandatory = $true)][string]$Computation,
        [Parameter(Mandatory = $true)][string]$Load,
        [Parameter(Mandatory = $true)][string]$Profile,
        [Parameter(Mandatory = $true)][bool]$FullRun,
        [Parameter(Mandatory = $true)][string]$LogPath
    )

    $env:ORI3_PROPOSAL_MATRIX_CANDIDATES = "$Candidates"
    $env:ORI3_PROPOSAL_MATRIX_REQUESTS = "$Requests"
    $env:ORI3_PROPOSAL_MATRIX_COMPUTATION = $Computation
    $env:ORI3_PROPOSAL_MATRIX_LOAD = $Load
    $env:ORI3_PROPOSAL_MATRIX_PROFILE = $Profile
    if ($FullRun) {
        $env:ORI3_PROPOSAL_MATRIX_ITERATIONS = "$MatrixIterations"
    }
    else {
        [Environment]::SetEnvironmentVariable("ORI3_PROPOSAL_MATRIX_ITERATIONS", $null, "Process")
    }

    $arguments = @("test", "--locked", "-p", "ori3-propose", "--test", "proposal_matrix")
    if ($Profile -eq "release") {
        $arguments += "--release"
    }
    $arguments += @("proposal_matrix_contract", "--", "--exact", "--nocapture", "--test-threads=1")
    $cargo = Invoke-CargoLogged -Arguments $arguments -LogPath $LogPath
    if ($cargo.exit_code -ne 0) {
        throw "matrix probeが終了コード$($cargo.exit_code)で失敗しました"
    }
    [pscustomobject]@{
        contract = Parse-MatrixResult -Output $cargo.output
        elapsed_seconds = $cargo.elapsed_seconds
        warning_count = $cargo.warning_count
    }
}

function Invoke-ProductHashContract {
    param(
        [Parameter(Mandatory = $true)][string]$Profile,
        [Parameter(Mandatory = $true)][string]$LogPath
    )

    Clear-MatrixEnvironment
    $arguments = @("test", "--locked", "-p", "desktop", "--lib")
    if ($Profile -eq "release") {
        $arguments += "--release"
    }
    $arguments += @(
        "commands::tests::proposal_candidates_are_the_same_computed_together_or_one_by_one",
        "--",
        "--exact",
        "--nocapture",
        "--test-threads=1"
    )
    $cargo = Invoke-CargoLogged -Arguments $arguments -LogPath $LogPath
    if ($cargo.exit_code -ne 0) {
        throw "製品hash契約が終了コード$($cargo.exit_code)で失敗しました"
    }
    $match = [regex]::Match(
        $cargo.output,
        "candidate_json_fnv1a64=(?<candidate>[0-9a-f]{16}) normal_stop_fnv1a64=(?<stop>[0-9a-f]{16})"
    )
    if (-not $match.Success) {
        throw "製品hash契約の結果行を読めません"
    }
    $candidate = $match.Groups["candidate"].Value
    $stop = $match.Groups["stop"].Value
    if ($candidate -ne $ExpectedCandidateHash -or $stop -ne $ExpectedStopHash) {
        throw "1-A契約hashが変わりました: candidate=$candidate stop=$stop"
    }
    [pscustomobject]@{
        profile = $Profile
        candidate_hash = $candidate
        stop_hash = $stop
        elapsed_seconds = $cargo.elapsed_seconds
        warning_count = $cargo.warning_count
    }
}

function New-FullState {
    param($Fingerprint, $DiskBefore)

    $cells = foreach ($profile in @("release", "debug")) {
        foreach ($load in @("idle", "busy")) {
            foreach ($computation in @("serial", "parallel")) {
                foreach ($requests in @(1, 2)) {
                    foreach ($candidates in @(1, 4)) {
                        $id = "c${candidates}-r${requests}-${computation}-${load}-${profile}"
                        [pscustomobject]@{
                            id = $id
                            candidates = $candidates
                            requests = $requests
                            computation = $computation
                            load = $load
                            profile = $profile
                            iterations = $MatrixIterations
                            status = "pending"
                            elapsed_seconds = $null
                            warning_count = $null
                            candidate_hash = $null
                            stop_hash = $null
                            first_candidate_hash = $null
                            first_stop = $null
                            stops = $null
                            load_threads = $null
                            log = "logs/$id.log"
                        }
                    }
                }
            }
        }
    }
    $regressions = @(
        [pscustomobject]@{
            id = "completion-search"
            test_target = "acceptance"
            test_name = "completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten"
            completed = 0
            total = $MatrixIterations
            status = "pending"
            elapsed_seconds = 0.0
            warning_count = 0
        },
        [pscustomobject]@{
            id = "named-end-to-end"
            test_target = "end_to_end"
            test_name = "named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten"
            completed = 0
            total = $MatrixIterations
            status = "pending"
            elapsed_seconds = 0.0
            warning_count = 0
        },
        [pscustomobject]@{
            id = "safe-partial-network"
            test_target = "acceptance"
            test_name = "a_safe_coincident_partial_network_appears_after_the_first_fold"
            completed = 0
            total = $MatrixIterations
            status = "pending"
            elapsed_seconds = 0.0
            warning_count = 0
        }
    )
    [pscustomobject]@{
        schema = 1
        mode = "Full"
        status = "running"
        started_at = (Get-Date).ToString("o")
        finished_at = $null
        target_dir = $LocalTargetDir
        matrix_iterations = $MatrixIterations
        expected_candidate_hash = $ExpectedCandidateHash
        expected_stop_hash = $ExpectedStopHash
        logical_processors = [Environment]::ProcessorCount
        input_fingerprint = $Fingerprint
        disk_before = $DiskBefore
        disk_after = $null
        product_hash_contracts = @()
        matrix = @($cells)
        regressions = $regressions
        failure = $null
    }
}

function Assert-FingerprintUnchanged {
    param([Parameter(Mandatory = $true)][string]$Expected)
    $now = Get-InputFingerprint
    if ($now.aggregate_sha256 -ne $Expected) {
        throw "実行中に入力sourceが変わりました。現在のcellを証拠にせず停止します"
    }
}

function Write-State {
    param([Parameter(Mandatory = $true)]$State)
    Write-JsonAtomic -Value $State -Path $StatePath
}

function Run-Validation {
    $validation = [ordered]@{
        schema = 1
        mode = "Validate"
        started_at = (Get-Date).ToString("o")
        target_dir = $LocalTargetDir
        disk_before = Get-DiskSnapshot
        input_fingerprint = Get-InputFingerprint
        probes = @()
        product_hash_contract = $null
        full_run_plan = $null
        disk_after = $null
        finished_at = $null
    }
    $plannedState = New-FullState -Fingerprint $validation.input_fingerprint -DiskBefore $validation.disk_before
    $validation.full_run_plan = [ordered]@{
        matrix_cells = @($plannedState.matrix).Count
        iterations_per_cell = $MatrixIterations
        matrix_iterations = @($plannedState.matrix).Count * $MatrixIterations
        matrix = @($plannedState.matrix | Select-Object id, candidates, requests, computation, load, profile, iterations, status)
        regressions = @($plannedState.regressions | Select-Object id, test_target, test_name, completed, total, status)
    }
    $debugLog = Join-Path $OutputRoot "validate-debug.log"
    $releaseLog = Join-Path $OutputRoot "validate-release-busy.log"
    $hashLog = Join-Path $OutputRoot "validate-product-hash-release.log"
    $validation.probes += Invoke-MatrixProbe -Candidates 1 -Requests 1 -Computation "serial" -Load "idle" -Profile "debug" -FullRun $false -LogPath $debugLog
    $validation.probes += Invoke-MatrixProbe -Candidates 4 -Requests 2 -Computation "parallel" -Load "busy" -Profile "release" -FullRun $false -LogPath $releaseLog
    $validation.product_hash_contract = Invoke-ProductHashContract -Profile "release" -LogPath $hashLog
    Assert-FingerprintUnchanged -Expected $validation.input_fingerprint.aggregate_sha256
    $validation.disk_after = Get-DiskSnapshot
    $validation.finished_at = (Get-Date).ToString("o")
    Write-JsonAtomic -Value $validation -Path (Join-Path $OutputRoot "validation.json")
    Write-JsonAtomic -Value $validation.full_run_plan -Path (Join-Path $OutputRoot "planned-full-run.json")
    Write-Host "Validate完了: 1回probe 2件、製品hash契約 1件。100回受入の代用ではありません。"
}

function Run-Performance {
    $performance = [ordered]@{
        schema = 1
        mode = "Performance"
        started_at = (Get-Date).ToString("o")
        target_dir = $ActiveTargetDir
        disk_before = Get-DiskSnapshot
        input_fingerprint = Get-InputFingerprint
        probe = $null
        product_hash_contract = $null
        warning_count = $null
        disk_after = $null
        finished_at = $null
    }

    $probeLog = Join-Path $OutputRoot "performance-release-busy.log"
    $hashLog = Join-Path $OutputRoot "performance-product-hash-release.log"
    $performance.probe = Invoke-MatrixProbe -Candidates 4 -Requests 2 -Computation "parallel" -Load "busy" -Profile "release" -FullRun $false -LogPath $probeLog
    $contract = $performance.probe.contract
    if ($contract.iterations -ne 1 -or
        $contract.candidates -ne 4 -or
        $contract.requests -ne 2 -or
        $contract.computation -ne "parallel" -or
        $contract.load -ne "busy" -or
        $contract.profile -ne "release") {
        throw "Performance境界が4候補・2要求・並列・混雑・release・1回ではありません"
    }
    if ($contract.load_threads -ne [Environment]::ProcessorCount) {
        throw "Performance境界の負荷thread数が論理CPU数と違います"
    }
    if ($contract.candidate_hash -ne $ExpectedCandidateHash -or $contract.stop_hash -ne $ExpectedStopHash) {
        throw "Performance境界の1-A契約hashが変わりました"
    }

    $performance.product_hash_contract = Invoke-ProductHashContract -Profile "release" -LogPath $hashLog
    Assert-FingerprintUnchanged -Expected $performance.input_fingerprint.aggregate_sha256
    $performance.warning_count = [int]$performance.probe.warning_count + [int]$performance.product_hash_contract.warning_count
    $performance.disk_after = Get-DiskSnapshot
    $performance.finished_at = (Get-Date).ToString("o")
    Write-JsonAtomic -Value $performance -Path (Join-Path $OutputRoot "ci-performance.json")
    Write-Host "Performance完了: releaseの4候補・2要求・並列・混雑を1回と、製品hash契約を確認しました。100回受入の代用ではありません。"
}

function Run-Full {
    if ($Resume) {
        if (-not (Test-Path -LiteralPath $StatePath)) {
            throw "再開するmatrix-state.jsonがありません"
        }
        $state = Get-Content -LiteralPath $StatePath -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($state.matrix_iterations -ne $MatrixIterations) {
            throw "保存済みstateの反復回数が100ではありません"
        }
        Assert-FingerprintUnchanged -Expected $state.input_fingerprint.aggregate_sha256
        foreach ($cell in $state.matrix) {
            if ($cell.status -eq "running") {
                $cell.status = "pending"
            }
        }
        foreach ($regression in $state.regressions) {
            if ($regression.status -eq "running") {
                $regression.status = "pending"
            }
        }
        $state.status = "running"
        $state.failure = $null
        Write-State -State $state
    }
    else {
        if (Test-Path -LiteralPath $StatePath) {
            throw "matrix-state.jsonが既にあります。続けるなら-Resumeを指定してください"
        }
        $state = New-FullState -Fingerprint (Get-InputFingerprint) -DiskBefore (Get-DiskSnapshot)
        Write-State -State $state
    }

    $fingerprint = $state.input_fingerprint.aggregate_sha256
    try {
        foreach ($profile in @("release", "debug")) {
            $existing = @($state.product_hash_contracts | Where-Object { $_.profile -eq $profile })
            if ($existing.Count -eq 0) {
                $log = Join-Path $OutputRoot "logs\product-hash-$profile.log"
                $productHash = Invoke-ProductHashContract -Profile $profile -LogPath $log
                Assert-FingerprintUnchanged -Expected $fingerprint
                $state.product_hash_contracts += $productHash
                Write-State -State $state
            }
        }

        $baselines = @{}
        $firstCandidateHash = $null
        $firstStop = $null
        foreach ($passed in @($state.matrix | Where-Object { $_.status -eq "passed" })) {
            $key = "$($passed.candidates)"
            if (-not $baselines.ContainsKey($key)) {
                $baselines[$key] = @($passed.candidate_hash, $passed.stop_hash)
            }
            if ($null -eq $firstCandidateHash) {
                $firstCandidateHash = $passed.first_candidate_hash
                $firstStop = $passed.first_stop
            }
        }

        foreach ($cell in $state.matrix) {
            if ($cell.status -eq "passed") {
                continue
            }
            Assert-FingerprintUnchanged -Expected $fingerprint
            $cell.status = "running"
            Write-State -State $state
            $log = Join-Path $OutputRoot $cell.log
            $probe = Invoke-MatrixProbe -Candidates $cell.candidates -Requests $cell.requests -Computation $cell.computation -Load $cell.load -Profile $cell.profile -FullRun $true -LogPath $log
            Assert-FingerprintUnchanged -Expected $fingerprint
            $contract = $probe.contract
            if ($contract.iterations -ne $MatrixIterations) {
                throw "$($cell.id): 100回走っていません"
            }
            $key = "$($cell.candidates)"
            if ($baselines.ContainsKey($key)) {
                if ($contract.candidate_hash -ne $baselines[$key][0] -or $contract.stop_hash -ne $baselines[$key][1]) {
                    throw "$($cell.id): 他cellと結果または停止理由が違います"
                }
            }
            else {
                $baselines[$key] = @($contract.candidate_hash, $contract.stop_hash)
            }
            if ($null -eq $firstCandidateHash) {
                $firstCandidateHash = $contract.first_candidate_hash
                $firstStop = $contract.first_stop
            }
            elseif ($contract.first_candidate_hash -ne $firstCandidateHash -or $contract.first_stop -ne $firstStop) {
                throw "$($cell.id): 先頭候補または先頭停止理由が候補1/4で一致しません"
            }
            if ($cell.candidates -eq 4 -and $contract.candidate_hash -ne $ExpectedCandidateHash) {
                throw "$($cell.id): 4候補の候補JSON hashが1-A契約と違います"
            }
            if ($cell.candidates -eq 4 -and $contract.stop_hash -ne $ExpectedStopHash) {
                throw "$($cell.id): 4候補の通常停止理由hashが1-A契約と違います"
            }
            if ($cell.load -eq "idle" -and $contract.load_threads -ne 0) {
                throw "$($cell.id): 空きcellが負荷threadを作りました"
            }
            if ($cell.load -eq "busy" -and $contract.load_threads -ne [Environment]::ProcessorCount) {
                throw "$($cell.id): 混雑cellの負荷thread数が論理CPU数と違います"
            }
            $cell.elapsed_seconds = $probe.elapsed_seconds
            $cell.warning_count = $probe.warning_count
            $cell.candidate_hash = $contract.candidate_hash
            $cell.stop_hash = $contract.stop_hash
            $cell.first_candidate_hash = $contract.first_candidate_hash
            $cell.first_stop = $contract.first_stop
            $cell.stops = $contract.stops
            $cell.load_threads = $contract.load_threads
            $cell.status = "passed"
            Write-State -State $state
        }

        foreach ($regression in $state.regressions) {
            if ($regression.status -eq "passed") {
                continue
            }
            $regression.status = "running"
            Write-State -State $state
            for ($run = [int]$regression.completed + 1; $run -le $MatrixIterations; $run++) {
                Assert-FingerprintUnchanged -Expected $fingerprint
                $directory = Join-Path $OutputRoot "logs\regressions\$($regression.id)"
                New-Item -ItemType Directory -Path $directory -Force | Out-Null
                $log = Join-Path $directory ("{0:D3}.log" -f $run)
                $arguments = @(
                    "test", "--locked", "-p", "ori3-propose", "--release", "--test", $regression.test_target,
                    $regression.test_name, "--", "--exact", "--nocapture", "--test-threads=1"
                )
                $cargo = Invoke-CargoLogged -Arguments $arguments -LogPath $log
                if ($cargo.exit_code -ne 0) {
                    throw "$($regression.id)の$run/100回目が失敗しました"
                }
                Assert-FingerprintUnchanged -Expected $fingerprint
                $regression.completed = $run
                $regression.elapsed_seconds = [math]::Round([double]$regression.elapsed_seconds + $cargo.elapsed_seconds, 3)
                $regression.warning_count = [int]$regression.warning_count + $cargo.warning_count
                Write-State -State $state
            }
            $regression.status = "passed"
            Write-State -State $state
        }

        $state.status = "passed"
    }
    catch {
        $state.status = "failed"
        $state.failure = $_.Exception.Message
        throw
    }
    finally {
        Clear-MatrixEnvironment
        $state.disk_after = Get-DiskSnapshot
        $state.finished_at = (Get-Date).ToString("o")
        Write-State -State $state
    }
}

New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $OutputRoot "logs") -Force | Out-Null
Push-Location $RepositoryRoot
try {
    switch ($Mode) {
        "Validate" { Run-Validation }
        "Performance" { Run-Performance }
        "Full" { Run-Full }
    }
}
finally {
    Clear-MatrixEnvironment
    Pop-Location
}
