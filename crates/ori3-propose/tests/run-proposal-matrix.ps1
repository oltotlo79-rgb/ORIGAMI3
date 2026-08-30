[CmdletBinding()]
param(
    [ValidateSet("Validate", "Performance", "Full")]
    [string]$Mode = "Validate",
    [switch]$Resume,

    # 0はCPU数と空き物理memoryから安全側に決める。1以上は自動上限を越えない明示上限。
    [ValidateRange(0, 32)]
    [int]$RegressionParallelism = 0
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$MatrixIterations = 100
$ExpectedCandidateHash = "b5404e822ccd3603"
$ExpectedStopHash = "ea05a0f8b88739bb"
$LocalTargetDir = "C:\Users\oltot\AppData\Local\Temp\ori3-target-speed2"
$RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$OutputRoot = Join-Path $RepositoryRoot "verification\propose-matrix"
$StatePath = Join-Path $OutputRoot "matrix-state.json"
$FullLockPath = Join-Path $OutputRoot "full-controller.lock"
$IsCi = $env:CI -eq "true" -or $env:GITHUB_ACTIONS -eq "true"
$RegressionMemoryPerWorkerBytes = [int64](2GB)

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

    $temporary = "$Path.$([Guid]::NewGuid().ToString('N')).tmp"
    $json = $Value | ConvertTo-Json -Depth 12
    $encoding = [System.Text.UTF8Encoding]::new($false)
    $stream = $null
    $writer = $null
    $backup = $null
    $validated = $false
    try {
        $stream = [System.IO.FileStream]::new(
            $temporary,
            [System.IO.FileMode]::CreateNew,
            [System.IO.FileAccess]::Write,
            [System.IO.FileShare]::None,
            4096,
            [System.IO.FileOptions]::WriteThrough
        )
        $writer = [System.IO.StreamWriter]::new($stream, $encoding)
        $writer.Write($json)
        $writer.Flush()
        $stream.Flush($true)
        $writer.Dispose()
        $writer = $null
        $stream = $null

        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            # WindowsのFile.Replaceはnullのbackup pathを受け付けない実装がある。
            # 同じディレクトリの一意なbackupへ置換し、JSON readback後にだけ消す。
            $backup = "$Path.$([Guid]::NewGuid().ToString('N')).bak"
            [System.IO.File]::Replace($temporary, $Path, $backup, $true)
        }
        else {
            [System.IO.File]::Move($temporary, $Path)
        }

        # JSONが読めないcheckpointを成功扱いしない。
        $null = Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
        $validated = $true
        if ($null -ne $backup -and (Test-Path -LiteralPath $backup)) {
            Remove-Item -LiteralPath $backup -Force
        }
    }
    finally {
        if ($null -ne $writer) {
            $writer.Dispose()
        }
        elseif ($null -ne $stream) {
            $stream.Dispose()
        }
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force
        }
        if ($validated -and $null -ne $backup -and (Test-Path -LiteralPath $backup)) {
            Remove-Item -LiteralPath $backup -Force
        }
    }
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

function Get-MemorySnapshot {
    Add-Type -AssemblyName Microsoft.VisualBasic
    $computer = New-Object Microsoft.VisualBasic.Devices.ComputerInfo
    [pscustomobject]@{
        checked_at = (Get-Date).ToString("o")
        total_physical_bytes = [int64]$computer.TotalPhysicalMemory
        available_physical_bytes = [int64]$computer.AvailablePhysicalMemory
    }
}

function Get-ProcessCount {
    param([Parameter(Mandatory = $true)][string]$Name)
    return @(Get-Process -Name $Name -ErrorAction SilentlyContinue).Count
}

function Get-RegressionParallelismPlan {
    $memory = Get-MemorySnapshot
    $logicalProcessors = [Environment]::ProcessorCount
    $reservedBytes = [int64][Math]::Max(4GB, [Math]::Ceiling($memory.total_physical_bytes * 0.20))
    $usableBytes = [int64][Math]::Max(0, $memory.available_physical_bytes - $reservedBytes)
    $memoryLimit = [int][Math]::Max(1, [Math]::Floor($usableBytes / $RegressionMemoryPerWorkerBytes))
    # OS用に1論理CPUを残し、探索1本につき少なくとも2論理CPUを見込む。
    $cpuLimit = [int][Math]::Max(1, [Math]::Floor(([Math]::Max(1, $logicalProcessors - 1)) / 2))
    $automaticLimit = [int][Math]::Max(1, [Math]::Min($cpuLimit, $memoryLimit))
    $effective = if ($RegressionParallelism -eq 0) {
        $automaticLimit
    }
    else {
        [int][Math]::Min($RegressionParallelism, $automaticLimit)
    }

    [pscustomobject]@{
        requested = $RegressionParallelism
        effective = [int][Math]::Max(1, $effective)
        automatic_limit = $automaticLimit
        cpu_limit = $cpuLimit
        memory_limit = $memoryLimit
        logical_processors = $logicalProcessors
        total_physical_bytes = $memory.total_physical_bytes
        available_physical_bytes = $memory.available_physical_bytes
        reserved_physical_bytes = $reservedBytes
        assumed_bytes_per_worker = $RegressionMemoryPerWorkerBytes
        decided_at = (Get-Date).ToString("o")
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
        "candidate_json_fnv1a64=(?<candidate>[0-9a-f]{16})(?: candidate_json_1e9_fnv1a64=[0-9a-f]{16})? normal_stop_fnv1a64=(?<stop>[0-9a-f]{16})"
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
    param($Fingerprint, $DiskBefore, $ParallelismPlan)

    if ($null -eq $ParallelismPlan) {
        $ParallelismPlan = Get-RegressionParallelismPlan
    }

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
    $regressionDefinitions = @(
        @("completion-search", "acceptance", "completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten"),
        @("named-end-to-end", "end_to_end", "named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten"),
        @("safe-partial-network", "acceptance", "a_safe_coincident_partial_network_appears_after_the_first_fold")
    )
    $regressions = foreach ($definition in $regressionDefinitions) {
        $runs = foreach ($run in 1..$MatrixIterations) {
            [pscustomobject]@{
                run = $run
                status = "pending"
                accepted_attempt = $null
                attempts = @()
            }
        }
        [pscustomobject]@{
            id = $definition[0]
            test_target = $definition[1]
            test_name = $definition[2]
            completed = 0
            total = $MatrixIterations
            status = "pending"
            elapsed_seconds = 0.0
            warning_count = 0
            runs = @($runs)
        }
    }
    $cargoStart = Get-ProcessCount -Name "cargo"
    $rustcStart = Get-ProcessCount -Name "rustc"
    [pscustomobject]@{
        schema = 2
        mode = "Full"
        status = "running"
        started_at = (Get-Date).ToString("o")
        finished_at = $null
        target_dir = $ActiveTargetDir
        matrix_iterations = $MatrixIterations
        expected_candidate_hash = $ExpectedCandidateHash
        expected_stop_hash = $ExpectedStopHash
        logical_processors = [Environment]::ProcessorCount
        input_fingerprint = $Fingerprint
        disk_before = $DiskBefore
        disk_after = $null
        regression_parallelism = $ParallelismPlan
        regression_prebuild = $null
        process_counts = [pscustomobject]@{
            cargo_start = $cargoStart
            rustc_start = $rustcStart
            cargo_max = $cargoStart
            rustc_max = $rustcStart
            regression_workers_max = 0
            cargo_end = $null
            rustc_end = $null
            observed_from = (Get-Date).ToString("o")
            observed_until = $null
        }
        product_hash_contracts = @()
        matrix = @($cells)
        regressions = @($regressions)
        failure = $null
        failures = @()
        summary = $null
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

function Open-FullControllerLock {
    try {
        return [System.IO.File]::Open(
            $FullLockPath,
            [System.IO.FileMode]::OpenOrCreate,
            [System.IO.FileAccess]::ReadWrite,
            [System.IO.FileShare]::None
        )
    }
    catch {
        throw "別のFull controllerが実行中です: $($_.Exception.Message)"
    }
}

function Update-ProcessCounts {
    param([Parameter(Mandatory = $true)]$State)
    $cargo = Get-ProcessCount -Name "cargo"
    $rustc = Get-ProcessCount -Name "rustc"
    if ($cargo -gt [int]$State.process_counts.cargo_max) {
        $State.process_counts.cargo_max = $cargo
    }
    if ($rustc -gt [int]$State.process_counts.rustc_max) {
        $State.process_counts.rustc_max = $rustc
    }
}

function Add-FullFailure {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$RegressionId,
        [Nullable[int]]$Run
    )
    if ([string]::IsNullOrWhiteSpace([string]$State.failure)) {
        $State.failure = $Message
    }
    $State.failures += [pscustomobject]@{
        occurred_at = (Get-Date).ToString("o")
        message = $Message
        regression = $RegressionId
        run = if ($null -eq $Run) { $null } else { [int]$Run }
    }
    $State.status = "failed"
}

function Assert-FullStateContract {
    param([Parameter(Mandatory = $true)]$State)

    foreach ($property in @(
        "schema", "mode", "status", "target_dir", "matrix_iterations",
        "expected_candidate_hash", "expected_stop_hash", "input_fingerprint",
        "regression_parallelism", "process_counts", "product_hash_contracts",
        "matrix", "regressions", "failure", "failures"
    )) {
        if ($State.PSObject.Properties.Name -notcontains $property) {
            throw "保存済みstateに必須propertyがありません: $property"
        }
    }
    if ([int]$State.schema -ne 2) {
        throw "保存済みstateのschemaが2ではありません。schema 1は並列checkpointへ安全に移行できないため再利用しません"
    }
    if ($State.mode -ne "Full" -or [int]$State.matrix_iterations -ne $MatrixIterations) {
        throw "保存済みstateのFull契約または反復回数が一致しません"
    }
    if ($State.expected_candidate_hash -ne $ExpectedCandidateHash -or
        $State.expected_stop_hash -ne $ExpectedStopHash) {
        throw "保存済みstateの期待hashが一致しません"
    }
    if (-not [string]::Equals(
        [IO.Path]::GetFullPath([string]$State.target_dir),
        $ActiveTargetDir,
        [StringComparison]::OrdinalIgnoreCase
    )) {
        throw "保存済みstateのCARGO_TARGET_DIRが現在のFullと一致しません"
    }
    if (@($State.matrix).Count -ne 32) {
        throw "保存済みstateのmatrix cell数が32ではありません"
    }
    $expectedCellIds = foreach ($profile in @("release", "debug")) {
        foreach ($load in @("idle", "busy")) {
            foreach ($computation in @("serial", "parallel")) {
                foreach ($requests in @(1, 2)) {
                    foreach ($candidates in @(1, 4)) {
                        "c${candidates}-r${requests}-${computation}-${load}-${profile}"
                    }
                }
            }
        }
    }
    $actualCellIds = @($State.matrix | ForEach-Object { [string]$_.id })
    if (@($actualCellIds | Sort-Object -Unique).Count -ne 32 -or
        @(Compare-Object -ReferenceObject $expectedCellIds -DifferenceObject $actualCellIds).Count -ne 0) {
        throw "保存済みstateのmatrix cell IDが32通りの契約と一致しません"
    }

    $expectedRegressions = @{
        "completion-search" = @("acceptance", "completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten")
        "named-end-to-end" = @("end_to_end", "named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten")
        "safe-partial-network" = @("acceptance", "a_safe_coincident_partial_network_appears_after_the_first_fold")
    }
    if (@($State.regressions).Count -ne $expectedRegressions.Count) {
        throw "保存済みstateの回帰系列数が3ではありません"
    }
    foreach ($regression in @($State.regressions)) {
        $id = [string]$regression.id
        if (-not $expectedRegressions.ContainsKey($id) -or
            $regression.test_target -ne $expectedRegressions[$id][0] -or
            $regression.test_name -ne $expectedRegressions[$id][1] -or
            [int]$regression.total -ne $MatrixIterations) {
            throw "保存済みstateの回帰定義が一致しません: $id"
        }
        $runs = @($regression.runs)
        if ($runs.Count -ne $MatrixIterations) {
            throw "$id のrun数が100ではありません"
        }
        $runNumbers = @($runs | ForEach-Object { [int]$_.run })
        if (@($runNumbers | Sort-Object -Unique).Count -ne $MatrixIterations -or
            @(Compare-Object -ReferenceObject @(1..$MatrixIterations) -DifferenceObject $runNumbers).Count -ne 0) {
            throw "$id のrun番号が1..100と一致しません"
        }
        foreach ($runState in $runs) {
            if ([string]$runState.status -notin @("pending", "starting", "running", "passed", "failed")) {
                throw "$id/$($runState.run) のstatusが不正です"
            }
            if ($runState.status -eq "passed" -and [string]::IsNullOrWhiteSpace([string]$runState.accepted_attempt)) {
                throw "$id/$($runState.run) はaccepted attemptなしでpassedになっています"
            }
            if ($runState.status -eq "passed") {
                $accepted = @($runState.attempts | Where-Object {
                    $_.token -eq $runState.accepted_attempt -and $_.status -eq "passed"
                })
                if ($accepted.Count -ne 1 -or [int]$accepted[0].exit_code -ne 0 -or
                    [int]$accepted[0].running_one_count -ne 1 -or [int]$accepted[0].exact_ok_count -ne 1) {
                    throw "$id/$($runState.run) のaccepted attempt契約が一致しません"
                }
                $acceptedLog = Join-Path $OutputRoot ([string]$accepted[0].log).Replace('/', '\')
                if (-not (Test-Path -LiteralPath $acceptedLog -PathType Leaf)) {
                    throw "$id/$($runState.run) のaccepted logがありません"
                }
                $actualLogHash = (Get-FileHash -LiteralPath $acceptedLog -Algorithm SHA256).Hash.ToLowerInvariant()
                if ($actualLogHash -ne $accepted[0].log_sha256) {
                    throw "$id/$($runState.run) のaccepted log SHA-256が一致しません"
                }
            }
        }
    }

    $candidateBaselines = @{}
    $firstCandidateHash = $null
    $firstStop = $null
    foreach ($cell in @($State.matrix | Where-Object { $_.status -eq "passed" })) {
        foreach ($field in @("candidate_hash", "stop_hash", "first_candidate_hash", "first_stop")) {
            if ([string]::IsNullOrWhiteSpace([string]$cell.$field)) {
                throw "$($cell.id) は$fieldなしでpassedになっています"
            }
        }
        $key = [string]$cell.candidates
        if ($candidateBaselines.ContainsKey($key)) {
            if ($cell.candidate_hash -ne $candidateBaselines[$key][0] -or
                $cell.stop_hash -ne $candidateBaselines[$key][1]) {
                throw "保存済みpassed matrix cell同士の結果が一致しません"
            }
        }
        else {
            $candidateBaselines[$key] = @($cell.candidate_hash, $cell.stop_hash)
        }
        if ($null -eq $firstCandidateHash) {
            $firstCandidateHash = $cell.first_candidate_hash
            $firstStop = $cell.first_stop
        }
        elseif ($cell.first_candidate_hash -ne $firstCandidateHash -or $cell.first_stop -ne $firstStop) {
            throw "保存済みpassed matrix cellの先頭結果が一致しません"
        }
        if ([int]$cell.candidates -eq 4 -and
            ($cell.candidate_hash -ne $ExpectedCandidateHash -or $cell.stop_hash -ne $ExpectedStopHash)) {
            throw "保存済み4候補cellのhashが1-A契約と一致しません"
        }
    }
    foreach ($contract in @($State.product_hash_contracts)) {
        if ($contract.candidate_hash -ne $ExpectedCandidateHash -or $contract.stop_hash -ne $ExpectedStopHash) {
            throw "保存済み製品hash契約が一致しません"
        }
    }
}

function Reset-InterruptedRunsForResume {
    param([Parameter(Mandatory = $true)]$State)

    if ($State.status -eq "failed" -or @($State.regressions.runs | Where-Object { $_.status -eq "failed" }).Count -gt 0) {
        throw "失敗済みのFullは-Resumeで成功へ変えられません。失敗記録を保ったまま原因を調べてください"
    }
    if ($State.status -eq "passed") {
        throw "保存済みFullは既にpassedです"
    }
    foreach ($regression in @($State.regressions)) {
        foreach ($runState in @($regression.runs | Where-Object { $_.status -in @("starting", "running") })) {
            $attempts = @($runState.attempts)
            if ($attempts.Count -eq 0) {
                throw "$($regression.id)/$($runState.run) はattemptなしで実行中になっています"
            }
            $attempt = $attempts[$attempts.Count - 1]
            if ($null -ne $attempt.process_id) {
                $live = Get-Process -Id ([int]$attempt.process_id) -ErrorAction SilentlyContinue
                if ($null -ne $live) {
                    throw "$($regression.id)/$($runState.run) のprocess $($attempt.process_id) がまだ動いています。終了確認前に再開しません"
                }
            }
            $combinedOutput = ""
            foreach ($relative in @($attempt.stdout, $attempt.stderr)) {
                if (-not [string]::IsNullOrWhiteSpace([string]$relative)) {
                    $absolute = Join-Path $OutputRoot ([string]$relative).Replace('/', '\')
                    if (Test-Path -LiteralPath $absolute -PathType Leaf) {
                        $combinedOutput += [IO.File]::ReadAllText($absolute)
                    }
                }
            }
            if ($combinedOutput -match "(?m)^test result: FAILED") {
                $attempt.status = "failed"
                $attempt.failure = "controller中断後の出力にFAILEDが残っています"
                $runState.status = "failed"
                Add-FullFailure -State $State -Message $attempt.failure -RegressionId $regression.id -Run ([int]$runState.run)
                Write-State -State $State
                throw "失敗した回帰を-Resumeで再試行しません: $($regression.id)/$($runState.run)"
            }
            $attempt.status = "interrupted"
            $attempt.finished_at = (Get-Date).ToString("o")
            $attempt.failure = "controller終了時に完了を採用できなかったため未実行へ戻した"
            $runState.status = "pending"
        }
        $regression.completed = @($regression.runs | Where-Object { $_.status -eq "passed" }).Count
        $regression.status = if ([int]$regression.completed -eq $MatrixIterations) { "passed" } else { "pending" }
    }
}

function Get-RegressionArguments {
    param([Parameter(Mandatory = $true)]$Regression)
    return @(
        "test", "--locked", "-p", "ori3-propose", "--release", "--test", [string]$Regression.test_target,
        [string]$Regression.test_name, "--", "--exact", "--nocapture", "--test-threads=1"
    )
}

function Get-OutputRelativePath {
    param([Parameter(Mandatory = $true)][string]$Path)
    $full = [IO.Path]::GetFullPath($Path)
    $prefix = $OutputRoot.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    if (-not $full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "出力pathがproposal matrix出力先の外です: $full"
    }
    return $full.Substring($prefix.Length).Replace('\', '/')
}

function Write-CombinedRegressionLog {
    param(
        [Parameter(Mandatory = $true)][string]$StdoutPath,
        [Parameter(Mandatory = $true)][string]$StderrPath,
        [Parameter(Mandatory = $true)][string]$LogPath
    )
    $stdout = if (Test-Path -LiteralPath $StdoutPath -PathType Leaf) {
        [IO.File]::ReadAllText($StdoutPath)
    }
    else { "" }
    $stderr = if (Test-Path -LiteralPath $StderrPath -PathType Leaf) {
        [IO.File]::ReadAllText($StderrPath)
    }
    else { "" }
    $combined = "=== stdout ===`r`n$stdout`r`n=== stderr ===`r`n$stderr"
    [IO.File]::WriteAllText($LogPath, $combined, [System.Text.UTF8Encoding]::new($false))
    [pscustomobject]@{
        stdout = $stdout
        stderr = $stderr
        combined = $combined
    }
}

function Test-ExactRegressionSummary {
    param([Parameter(Mandatory = $true)][string]$Output)
    $escape = [string][char]27
    $plain = [regex]::Replace($Output, "$escape\[[0-9;]*[A-Za-z]", "")
    $runningCount = [regex]::Matches($plain, "(?m)^running 1 test\s*$", [Text.RegularExpressions.RegexOptions]::IgnoreCase).Count
    $okPattern = "(?m)^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in .+\s*$"
    $okCount = [regex]::Matches($plain, $okPattern).Count
    [pscustomobject]@{
        passed = ($runningCount -eq 1 -and $okCount -eq 1)
        running_one_count = $runningCount
        exact_ok_count = $okCount
    }
}

function Start-RegressionAttempt {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)]$Regression,
        [Parameter(Mandatory = $true)]$RunState,
        [Parameter(Mandatory = $true)][string]$Fingerprint
    )

    Assert-FingerprintUnchanged -Expected $Fingerprint
    $attemptNumber = @($RunState.attempts).Count + 1
    $token = [Guid]::NewGuid().ToString("N")
    $directory = Join-Path $OutputRoot "logs\regressions\$($Regression.id)"
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    $base = "{0:D3}-a{1:D2}-{2}" -f ([int]$RunState.run), $attemptNumber, $token
    $stdoutPath = Join-Path $directory "$base.stdout.log"
    $stderrPath = Join-Path $directory "$base.stderr.log"
    $logPath = Join-Path $directory "$base.log"
    $arguments = Get-RegressionArguments -Regression $Regression
    $attempt = [pscustomobject]@{
        token = $token
        attempt = $attemptNumber
        status = "starting"
        command = "cargo"
        arguments = @($arguments)
        started_at = (Get-Date).ToString("o")
        process_started_at = $null
        finished_at = $null
        process_id = $null
        exit_code = $null
        elapsed_seconds = $null
        stdout = Get-OutputRelativePath -Path $stdoutPath
        stderr = Get-OutputRelativePath -Path $stderrPath
        log = Get-OutputRelativePath -Path $logPath
        log_sha256 = $null
        warning_count = $null
        fingerprint_before = $Fingerprint
        fingerprint_after = $null
        running_one_count = $null
        exact_ok_count = $null
        failure = $null
    }
    $RunState.attempts += $attempt
    $RunState.status = "starting"
    $Regression.status = "running"
    Write-State -State $State

    $process = $null
    try {
        $process = Start-Process `
            -FilePath "cargo" `
            -ArgumentList $arguments `
            -WorkingDirectory $RepositoryRoot `
            -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath `
            -NoNewWindow `
            -PassThru
        $attempt.process_id = $process.Id
        try {
            $attempt.process_started_at = $process.StartTime.ToString("o")
        }
        catch {
            $attempt.process_started_at = (Get-Date).ToString("o")
        }
        $attempt.status = "running"
        $RunState.status = "running"
        Write-State -State $State
        Write-Host "回帰開始: $($Regression.id) $($RunState.run)/100 pid=$($process.Id)"
        return [pscustomobject]@{
            process = $process
            regression = $Regression
            run_state = $RunState
            attempt = $attempt
            stdout_path = $stdoutPath
            stderr_path = $stderrPath
            log_path = $logPath
            completed = $false
        }
    }
    catch {
        if ($null -ne $process) {
            while (-not $process.HasExited) {
                [Threading.Thread]::Sleep(250)
                Update-ProcessCounts -State $State
            }
            $process.WaitForExit()
            $attempt.exit_code = $process.ExitCode
            try {
                $logs = Write-CombinedRegressionLog -StdoutPath $stdoutPath -StderrPath $stderrPath -LogPath $logPath
                $attempt.log_sha256 = (Get-FileHash -LiteralPath $logPath -Algorithm SHA256).Hash.ToLowerInvariant()
                $attempt.warning_count = [regex]::Matches($logs.combined, "(?im)\bwarn(?:ing)?:").Count
            }
            catch { }
            $process.Dispose()
        }
        $attempt.status = "failed"
        $attempt.finished_at = (Get-Date).ToString("o")
        $attempt.failure = "cargo processの開始または開始checkpointに失敗しました: $($_.Exception.Message)"
        $RunState.status = "failed"
        $Regression.status = "failed"
        Add-FullFailure -State $State -Message $attempt.failure -RegressionId $Regression.id -Run ([int]$RunState.run)
        Write-State -State $State
        throw
    }
}

function Complete-RegressionAttempt {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)]$ActiveItem,
        [Parameter(Mandatory = $true)][string]$Fingerprint
    )
    if ($ActiveItem.completed) {
        try { $ActiveItem.process.Dispose() } catch { }
        return $ActiveItem.run_state.status -eq "failed"
    }

    $process = $ActiveItem.process
    $process.WaitForExit()
    $exitCode = $process.ExitCode
    $finished = Get-Date
    $attempt = $ActiveItem.attempt
    $runState = $ActiveItem.run_state
    $regression = $ActiveItem.regression
    $logs = Write-CombinedRegressionLog `
        -StdoutPath $ActiveItem.stdout_path `
        -StderrPath $ActiveItem.stderr_path `
        -LogPath $ActiveItem.log_path
    $summary = Test-ExactRegressionSummary -Output $logs.combined
    $fingerprintAfter = $null
    $fingerprintFailure = $null
    try {
        $fingerprintAfter = Get-InputFingerprint
        if ($fingerprintAfter.aggregate_sha256 -ne $Fingerprint) {
            $fingerprintFailure = "実行中に入力sourceが変わりました"
        }
    }
    catch {
        $fingerprintFailure = "終了後のsource fingerprintを確認できません: $($_.Exception.Message)"
    }

    $attempt.finished_at = $finished.ToString("o")
    $attempt.exit_code = $exitCode
    $attempt.elapsed_seconds = [math]::Round(($finished - [DateTime]::Parse($attempt.started_at)).TotalSeconds, 3)
    $attempt.warning_count = [regex]::Matches($logs.combined, "(?im)\bwarn(?:ing)?:").Count
    $attempt.log_sha256 = (Get-FileHash -LiteralPath $ActiveItem.log_path -Algorithm SHA256).Hash.ToLowerInvariant()
    $attempt.fingerprint_after = if ($null -eq $fingerprintAfter) { $null } else { $fingerprintAfter.aggregate_sha256 }
    $attempt.running_one_count = $summary.running_one_count
    $attempt.exact_ok_count = $summary.exact_ok_count

    $failure = $null
    if ($exitCode -ne 0) {
        $failure = "cargoが終了コード$exitCodeを返しました"
    }
    elseif ($null -ne $fingerprintFailure) {
        $failure = $fingerprintFailure
    }
    elseif (-not $summary.passed) {
        $failure = "exact test summaryが1 passed/0 failed/0 ignoredではありません(running1=$($summary.running_one_count), exact_ok=$($summary.exact_ok_count))"
    }

    if ($null -eq $failure) {
        $attempt.status = "passed"
        $runState.status = "passed"
        $runState.accepted_attempt = $attempt.token
        $regression.completed = @($regression.runs | Where-Object { $_.status -eq "passed" }).Count
        $regression.elapsed_seconds = [math]::Round([double]$regression.elapsed_seconds + [double]$attempt.elapsed_seconds, 3)
        $regression.warning_count = [int]$regression.warning_count + [int]$attempt.warning_count
        if ([int]$regression.completed -eq $MatrixIterations) {
            $regression.status = "passed"
        }
        Write-Host "回帰合格: $($regression.id) $($runState.run)/100 elapsed=$($attempt.elapsed_seconds)s"
    }
    else {
        $attempt.status = "failed"
        $attempt.failure = $failure
        $runState.status = "failed"
        $regression.status = "failed"
        Add-FullFailure -State $State -Message $failure -RegressionId $regression.id -Run ([int]$runState.run)
        Write-Host "回帰失敗: $($regression.id) $($runState.run)/100 $failure" -ForegroundColor Red
    }
    $ActiveItem.completed = $true
    try {
        Write-State -State $State
    }
    finally {
        $process.Dispose()
    }
    return $null -ne $failure
}

function Invoke-RegressionPrebuild {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Fingerprint
    )
    Clear-MatrixEnvironment
    Assert-FingerprintUnchanged -Expected $Fingerprint
    $arguments = @(
        "test", "--locked", "-p", "ori3-propose", "--release",
        "--test", "acceptance", "--test", "end_to_end", "--no-run"
    )
    $logPath = Join-Path $OutputRoot "logs\regression-prebuild.log"
    $cargo = Invoke-CargoLogged -Arguments $arguments -LogPath $logPath
    if ($cargo.exit_code -ne 0) {
        throw "回帰2 test targetの事前buildが終了コード$($cargo.exit_code)で失敗しました"
    }
    Assert-FingerprintUnchanged -Expected $Fingerprint
    $State.regression_prebuild = [pscustomobject]@{
        command = "cargo"
        arguments = @($arguments)
        elapsed_seconds = $cargo.elapsed_seconds
        warning_count = $cargo.warning_count
        log = Get-OutputRelativePath -Path $logPath
        completed_at = (Get-Date).ToString("o")
        fingerprint = $Fingerprint
    }
    Write-State -State $State
}

function Get-NextPendingRegressionRun {
    param([Parameter(Mandatory = $true)]$State)
    foreach ($regression in @($State.regressions)) {
        foreach ($runState in @($regression.runs)) {
            if ($runState.status -eq "pending") {
                return [pscustomobject]@{
                    regression = $regression
                    run_state = $runState
                }
            }
        }
    }
    return $null
}

function Invoke-RegressionPool {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Fingerprint
    )

    $parallelism = [int]$State.regression_parallelism.effective
    $active = New-Object System.Collections.ArrayList
    $stopDispatch = $false
    $poolError = $null
    Update-ProcessCounts -State $State
    Write-State -State $State
    try {
        while ($true) {
            while (-not $stopDispatch -and $active.Count -lt $parallelism) {
                $next = Get-NextPendingRegressionRun -State $State
                if ($null -eq $next) {
                    break
                }
                $memory = Get-MemorySnapshot
                # 現在のavailableは既存workerの使用分を既に差し引いている。
                # 新しい1本を足した後にもreserveを残せることだけを確認する。
                $requiredAvailable = [int64]$State.regression_parallelism.reserved_physical_bytes +
                    [int64]$State.regression_parallelism.assumed_bytes_per_worker
                if ($memory.available_physical_bytes -lt $requiredAvailable) {
                    if ($active.Count -eq 0) {
                        throw "回帰を1本安全に開始する空きmemoryがありません(available=$($memory.available_physical_bytes), required=$requiredAvailable)"
                    }
                    break
                }
                $item = Start-RegressionAttempt `
                    -State $State `
                    -Regression $next.regression `
                    -RunState $next.run_state `
                    -Fingerprint $Fingerprint
                [void]$active.Add($item)
                if ($active.Count -gt [int]$State.process_counts.regression_workers_max) {
                    $State.process_counts.regression_workers_max = $active.Count
                }
                Update-ProcessCounts -State $State
            }

            if ($active.Count -eq 0) {
                break
            }
            $exited = @($active | Where-Object { $_.process.HasExited })
            if ($exited.Count -eq 0) {
                [Threading.Thread]::Sleep(250)
                Update-ProcessCounts -State $State
                continue
            }
            foreach ($item in $exited) {
                $failed = Complete-RegressionAttempt -State $State -ActiveItem $item -Fingerprint $Fingerprint
                [void]$active.Remove($item)
                Update-ProcessCounts -State $State
                if ($failed) {
                    $stopDispatch = $true
                }
            }
            if ($stopDispatch -and $active.Count -eq 0) {
                break
            }
        }
    }
    catch {
        $poolError = $_
        $stopDispatch = $true
        if ($State.status -ne "failed") {
            Add-FullFailure -State $State -Message "回帰pool controllerが失敗しました: $($_.Exception.Message)"
        }
    }
    finally {
        # 失敗や中断後も、開始済みprocessを強制終了せず自然終了まで回収する。
        while ($active.Count -gt 0) {
            $exited = @($active | Where-Object { $_.process.HasExited })
            if ($exited.Count -eq 0) {
                [Threading.Thread]::Sleep(250)
                Update-ProcessCounts -State $State
                continue
            }
            foreach ($item in $exited) {
                try {
                    $null = Complete-RegressionAttempt -State $State -ActiveItem $item -Fingerprint $Fingerprint
                }
                catch {
                    if ($State.status -ne "failed") {
                        Add-FullFailure -State $State -Message "実行中processの回収に失敗しました: $($_.Exception.Message)"
                    }
                    try { $item.process.Dispose() } catch { }
                }
                [void]$active.Remove($item)
                Update-ProcessCounts -State $State
            }
        }
        $State.process_counts.cargo_end = Get-ProcessCount -Name "cargo"
        $State.process_counts.rustc_end = Get-ProcessCount -Name "rustc"
        $State.process_counts.observed_until = (Get-Date).ToString("o")
        Write-State -State $State
    }

    if ($null -ne $poolError) {
        throw $poolError
    }
    if ($State.status -eq "failed") {
        throw "回帰poolが失敗しました: $($State.failure)"
    }
    foreach ($regression in @($State.regressions)) {
        $passed = @($regression.runs | Where-Object { $_.status -eq "passed" }).Count
        if ($passed -ne $MatrixIterations -or $regression.status -ne "passed") {
            throw "$($regression.id)が100/100 passedではありません(passed=$passed)"
        }
    }
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
    $controllerLock = Open-FullControllerLock
    $state = $null
    try {
        $currentParallelismPlan = Get-RegressionParallelismPlan
        if ($Resume) {
            if (-not (Test-Path -LiteralPath $StatePath)) {
                throw "再開するmatrix-state.jsonがありません"
            }
            $loadedState = Get-Content -LiteralPath $StatePath -Raw -Encoding UTF8 | ConvertFrom-Json
            Assert-FullStateContract -State $loadedState
            Reset-InterruptedRunsForResume -State $loadedState
            Assert-FingerprintUnchanged -Expected $loadedState.input_fingerprint.aggregate_sha256
            $state = $loadedState
            foreach ($cell in $state.matrix) {
                if ($cell.status -eq "running") {
                    $cell.status = "pending"
                }
            }
            # Resumeで並列数を増やさない。現在のCPU/RAM上限または明示上限が低ければ下げる。
            $state.regression_parallelism.effective = [int][Math]::Min(
                [int]$state.regression_parallelism.effective,
                [int]$currentParallelismPlan.effective
            )
            $state.status = "running"
            Write-State -State $state
        }
        else {
            if (Test-Path -LiteralPath $StatePath) {
                throw "matrix-state.jsonが既にあります。続けるなら-Resumeを指定してください"
            }
            $state = New-FullState `
                -Fingerprint (Get-InputFingerprint) `
                -DiskBefore (Get-DiskSnapshot) `
                -ParallelismPlan $currentParallelismPlan
            Write-State -State $state
        }

        $fingerprint = $state.input_fingerprint.aggregate_sha256
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

        # matrixのidle/busy条件を汚さないよう、32 cell完了後にだけ回帰poolを開始する。
        Clear-MatrixEnvironment
        Invoke-RegressionPrebuild -State $state -Fingerprint $fingerprint
        Invoke-RegressionPool -State $state -Fingerprint $fingerprint
        Assert-FingerprintUnchanged -Expected $fingerprint
        $state.summary = [pscustomobject]@{
            matrix_passed = @($state.matrix | Where-Object { $_.status -eq "passed" }).Count
            regression_series_passed = @($state.regressions | Where-Object { $_.status -eq "passed" }).Count
            regression_runs_passed = @($state.regressions.runs | Where-Object { $_.status -eq "passed" }).Count
            regression_runs_failed = @($state.regressions.runs | Where-Object { $_.status -eq "failed" }).Count
            requested_parallelism = $state.regression_parallelism.requested
            effective_parallelism = $state.regression_parallelism.effective
            observed_parallel_workers = $state.process_counts.regression_workers_max
            logical_processors = $state.regression_parallelism.logical_processors
            total_physical_bytes = $state.regression_parallelism.total_physical_bytes
            available_physical_bytes_at_decision = $state.regression_parallelism.available_physical_bytes
            cargo_processes_start = $state.process_counts.cargo_start
            cargo_processes_max = $state.process_counts.cargo_max
            cargo_processes_end = $state.process_counts.cargo_end
            rustc_processes_start = $state.process_counts.rustc_start
            rustc_processes_max = $state.process_counts.rustc_max
            rustc_processes_end = $state.process_counts.rustc_end
            all_started_processes_reaped = $true
        }
        if ($state.summary.matrix_passed -ne 32 -or
            $state.summary.regression_series_passed -ne 3 -or
            $state.summary.regression_runs_passed -ne 300 -or
            $state.summary.regression_runs_failed -ne 0) {
            throw "Fullの最終件数が32 cell・3系列・300回合格と一致しません"
        }
        $state.status = "passed"
    }
    catch {
        if ($null -ne $state -and $state.status -ne "failed") {
            Add-FullFailure -State $state -Message $_.Exception.Message
        }
        throw
    }
    finally {
        try {
            Clear-MatrixEnvironment
            if ($null -ne $state) {
                $state.disk_after = Get-DiskSnapshot
                $state.finished_at = (Get-Date).ToString("o")
                $state.process_counts.cargo_end = Get-ProcessCount -Name "cargo"
                $state.process_counts.rustc_end = Get-ProcessCount -Name "rustc"
                $state.process_counts.observed_until = (Get-Date).ToString("o")
                Write-State -State $state
            }
        }
        finally {
            $controllerLock.Dispose()
        }
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
