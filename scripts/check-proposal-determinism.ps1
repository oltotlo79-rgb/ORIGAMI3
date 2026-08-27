[CmdletBinding()]
param(
    [ValidateSet("Validate", "Performance", "Full")]
    [string]$Mode = "Validate",

    [switch]$Resume,

    [switch]$DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$MatrixRunnerPath = Join-Path $RepositoryRoot "crates\ori3-propose\tests\run-proposal-matrix.ps1"
$MatrixOutputRoot = Join-Path $RepositoryRoot "verification\propose-matrix"
$DeterminismOutputPath = Join-Path $RepositoryRoot "verification\improvement-roadmap\01-proposal\determinism.json"
$ReachabilityPattern = "PROPOSAL_PROGRESS|SearchDeadline|TimeCap|job_id|jobId|ProposalJobs"
$ReachabilityRoots = @("crates", "apps/desktop/src-tauri/src", "apps/desktop/src")

function Get-OptionalProperty {
    param(
        [AllowNull()]
        $Object,

        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    if ($null -eq $Object) {
        return $null
    }

    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Get-MatrixSourcePath {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet("Validate", "Performance", "Full")]
        [string]$RequestedMode,

        [Parameter(Mandatory = $true)]
        [string]$OutputRoot
    )

    switch ($RequestedMode) {
        "Validate" { return Join-Path $OutputRoot "validation.json" }
        "Performance" { return Join-Path $OutputRoot "ci-performance.json" }
        "Full" { return Join-Path $OutputRoot "matrix-state.json" }
    }
}

function Get-FileReference {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "参照元が見つかりません: $Path"
    }

    $resolved = (Resolve-Path -LiteralPath $Path).Path
    [ordered]@{
        path = $resolved
        sha256 = (Get-FileHash -LiteralPath $resolved -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

function Get-CpuSnapshot {
    $processors = @(
        Get-CimInstance -ClassName Win32_Processor | ForEach-Object {
            [ordered]@{
                name = [string]$_.Name
                logical_processors = [int]$_.NumberOfLogicalProcessors
            }
        }
    )
    if ($processors.Count -eq 0) {
        throw "Win32_ProcessorからCPU情報を取得できません"
    }

    [ordered]@{
        processors = $processors
        logical_processors_total = [int](($processors | Measure-Object -Property logical_processors -Sum).Sum)
    }
}

function Get-RustHost {
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = @(& rustc -vV 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($exitCode -ne 0) {
        throw "rustc -vVが終了コード$exitCodeで失敗しました"
    }

    $hostLine = @($output | Where-Object { $_.ToString() -match "^host:\s+" } | Select-Object -First 1)
    if ($hostLine.Count -ne 1) {
        throw "rustc -vVのhost行を読めません"
    }
    return ($hostLine[0].ToString() -replace "^host:\s+", "").Trim()
}

function Get-ReachabilityScan {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Repository,

        [Parameter(Mandatory = $true)]
        [string]$Pattern,

        [Parameter(Mandatory = $true)]
        [string[]]$RelativeRoots
    )

    $rootPaths = @($RelativeRoots | ForEach-Object { Join-Path $Repository $_ })
    $rg = Get-Command rg -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -ne $rg) {
        $previousPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = "Continue"
            Push-Location -LiteralPath $Repository
            try {
                $matches = @(& $rg.Source -n $Pattern @($RelativeRoots) 2>$null)
                $matchExitCode = $LASTEXITCODE
                $staticMatches = @(& $rg.Source -n "\bstatic\s+(?:mut\s+)?PROPOSAL_PROGRESS\b" @($RelativeRoots) 2>$null)
                $staticExitCode = $LASTEXITCODE
            }
            finally {
                Pop-Location
            }
        }
        finally {
            $ErrorActionPreference = $previousPreference
        }
        if ($matchExitCode -gt 1 -or $staticExitCode -gt 1) {
            throw "rgによる到達経路の走査に失敗しました"
        }
        $method = "rg"
    }
    else {
        $files = @($rootPaths | ForEach-Object {
            Get-ChildItem -LiteralPath $_ -File -Recurse
        })
        $matches = @($files | Select-String -Pattern $Pattern | ForEach-Object {
            "{0}:{1}:{2}" -f $_.Path, $_.LineNumber, $_.Line.TrimEnd()
        })
        $staticMatches = @($files | Select-String -Pattern "\bstatic\s+(?:mut\s+)?PROPOSAL_PROGRESS\b" | ForEach-Object {
            "{0}:{1}:{2}" -f $_.Path, $_.LineNumber, $_.Line.TrimEnd()
        })
        $method = "Select-String"
    }

    if ($staticMatches.Count -ne 0) {
        throw "製品用の大域static PROPOSAL_PROGRESSが$($staticMatches.Count)件見つかりました"
    }

    [ordered]@{
        method = $method
        command = "rg -n `"$Pattern`" crates apps/desktop/src-tauri/src apps/desktop/src"
        search_roots = $RelativeRoots
        matching_line_count = $matches.Count
        matches = @($matches | ForEach-Object { $_.ToString() })
        static_proposal_progress = [ordered]@{
            matching_line_count = $staticMatches.Count
            matches = @($staticMatches | ForEach-Object { $_.ToString() })
            is_zero = $true
        }
    }
}

function New-DeterminismMeasurement {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SourceMode,

        [Parameter(Mandatory = $true)]
        $Contract,

        [AllowNull()]
        $ElapsedSeconds,

        [AllowNull()]
        $WarningCount,

        [AllowNull()]
        $Status
    )

    [ordered]@{
        source_mode = $SourceMode
        status = $Status
        profile = Get-OptionalProperty -Object $Contract -Name "profile"
        candidates = Get-OptionalProperty -Object $Contract -Name "candidates"
        requests = Get-OptionalProperty -Object $Contract -Name "requests"
        computation = Get-OptionalProperty -Object $Contract -Name "computation"
        load = Get-OptionalProperty -Object $Contract -Name "load"
        load_threads = Get-OptionalProperty -Object $Contract -Name "load_threads"
        iterations = Get-OptionalProperty -Object $Contract -Name "iterations"
        candidate_hash = Get-OptionalProperty -Object $Contract -Name "candidate_hash"
        stop_hash = Get-OptionalProperty -Object $Contract -Name "stop_hash"
        first_candidate_hash = Get-OptionalProperty -Object $Contract -Name "first_candidate_hash"
        first_stop = Get-OptionalProperty -Object $Contract -Name "first_stop"
        stops = Get-OptionalProperty -Object $Contract -Name "stops"
        elapsed_seconds = $ElapsedSeconds
        warning_count = $WarningCount
    }
}

function Get-DeterminismMeasurements {
    param(
        [Parameter(Mandatory = $true)]
        $SourceResult
    )

    $sourceMode = [string](Get-OptionalProperty -Object $SourceResult -Name "mode")
    $measurements = @()
    $probes = Get-OptionalProperty -Object $SourceResult -Name "probes"
    if ($null -ne $probes) {
        foreach ($probe in @($probes)) {
            $measurements += New-DeterminismMeasurement `
                -SourceMode $sourceMode `
                -Contract (Get-OptionalProperty -Object $probe -Name "contract") `
                -ElapsedSeconds (Get-OptionalProperty -Object $probe -Name "elapsed_seconds") `
                -WarningCount (Get-OptionalProperty -Object $probe -Name "warning_count") `
                -Status "passed"
        }
    }

    $probe = Get-OptionalProperty -Object $SourceResult -Name "probe"
    if ($null -ne $probe) {
        $measurements += New-DeterminismMeasurement `
            -SourceMode $sourceMode `
            -Contract (Get-OptionalProperty -Object $probe -Name "contract") `
            -ElapsedSeconds (Get-OptionalProperty -Object $probe -Name "elapsed_seconds") `
            -WarningCount (Get-OptionalProperty -Object $probe -Name "warning_count") `
            -Status "passed"
    }

    $matrix = Get-OptionalProperty -Object $SourceResult -Name "matrix"
    if ($null -ne $matrix) {
        foreach ($cell in @($matrix)) {
            $measurements += New-DeterminismMeasurement `
                -SourceMode $sourceMode `
                -Contract $cell `
                -ElapsedSeconds (Get-OptionalProperty -Object $cell -Name "elapsed_seconds") `
                -WarningCount (Get-OptionalProperty -Object $cell -Name "warning_count") `
                -Status (Get-OptionalProperty -Object $cell -Name "status")
        }
    }

    if ($measurements.Count -eq 0) {
        throw "matrix runnerのJSONにprobeまたはmatrixがありません"
    }
    return @($measurements)
}

function Write-JsonAtomic {
    param(
        [Parameter(Mandatory = $true)]
        $Value,

        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $parent = Split-Path -Parent $Path
    [void][IO.Directory]::CreateDirectory($parent)
    $temporary = "$Path.$([Guid]::NewGuid().ToString('N')).tmp"
    $json = $Value | ConvertTo-Json -Depth 14
    $encoding = [Text.UTF8Encoding]::new($false)
    try {
        [IO.File]::WriteAllText($temporary, $json, $encoding)
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            [IO.File]::Replace($temporary, $Path, $null, $true)
        }
        else {
            [IO.File]::Move($temporary, $Path)
        }
        $null = Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
    }
    finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force
        }
    }
}

function Write-DeterminismSummary {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet("Validate", "Performance", "Full")]
        [string]$RequestedMode,

        [Parameter(Mandatory = $true)]
        [bool]$WasResumed,

        [Parameter(Mandatory = $true)]
        [string]$RunnerPath,

        [Parameter(Mandatory = $true)]
        [string]$SourcePath,

        [Parameter(Mandatory = $true)]
        $SourceResult,

        [Parameter(Mandatory = $true)]
        [string]$OutputPath,

        [Parameter(Mandatory = $true)]
        $CpuSnapshot,

        [Parameter(Mandatory = $true)]
        [string]$RustHost,

        [Parameter(Mandatory = $true)]
        $Reachability
    )

    $inputFingerprint = Get-OptionalProperty -Object $SourceResult -Name "input_fingerprint"
    if ($null -eq $inputFingerprint) {
        throw "matrix runnerのJSONにinput_fingerprintがありません"
    }
    $measurements = @(Get-DeterminismMeasurements -SourceResult $SourceResult)
    $profiles = @($measurements | ForEach-Object { $_.profile } | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } | Sort-Object -Unique)

    $summary = [ordered]@{
        schema = 1
        artifact = "1-D proposal determinism summary"
        generated_at = (Get-Date).ToString("o")
        requested = [ordered]@{
            mode = $RequestedMode
            resume = $WasResumed
        }
        runner = Get-FileReference -Path $RunnerPath
        source_results = @(
            [ordered]@{
                reference = Get-FileReference -Path $SourcePath
                mode = Get-OptionalProperty -Object $SourceResult -Name "mode"
                started_at = Get-OptionalProperty -Object $SourceResult -Name "started_at"
                finished_at = Get-OptionalProperty -Object $SourceResult -Name "finished_at"
            }
        )
        input_fingerprint = $inputFingerprint
        measurements = $measurements
        profiles = $profiles
        cpu = $CpuSnapshot
        rustc = [ordered]@{ host = $RustHost }
        reachability = $Reachability
    }
    Write-JsonAtomic -Value $summary -Path $OutputPath
    return $summary
}

function Format-MatrixRunnerCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RunnerPath,

        [Parameter(Mandatory = $true)]
        [string]$RequestedMode,

        [Parameter(Mandatory = $true)]
        [bool]$WasResumed
    )

    $quotedPath = "'" + $RunnerPath.Replace("'", "''") + "'"
    $command = "& $quotedPath -Mode $RequestedMode"
    if ($WasResumed) {
        $command += " -Resume"
    }
    return $command
}

function Invoke-DeterminismRun {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet("Validate", "Performance", "Full")]
        [string]$RequestedMode,

        [Parameter(Mandatory = $true)]
        [bool]$WasResumed,

        [Parameter(Mandatory = $true)]
        [bool]$IsDryRun
    )

    if ($WasResumed -and $RequestedMode -ne "Full") {
        throw "-ResumeはFullでだけ使えます"
    }
    if (-not (Test-Path -LiteralPath $MatrixRunnerPath -PathType Leaf)) {
        throw "既存matrix runnerが見つかりません: $MatrixRunnerPath"
    }

    $sourcePath = Get-MatrixSourcePath -RequestedMode $RequestedMode -OutputRoot $MatrixOutputRoot
    $command = Format-MatrixRunnerCommand -RunnerPath $MatrixRunnerPath -RequestedMode $RequestedMode -WasResumed $WasResumed
    if ($IsDryRun) {
        Write-Output "[DRY-RUN] existing runner will not be invoked."
        Write-Output "[DRY-RUN] command: $command"
        Write-Output "[DRY-RUN] source JSON: $sourcePath"
        Write-Output "[DRY-RUN] summary JSON: $DeterminismOutputPath"
        return
    }

    $runnerArguments = @("-Mode", $RequestedMode)
    if ($WasResumed) {
        $runnerArguments += "-Resume"
    }
    $global:LASTEXITCODE = 0
    & $MatrixRunnerPath @runnerArguments
    if ($LASTEXITCODE -ne 0) {
        throw "既存matrix runnerが終了コード$LASTEXITCODEで失敗しました"
    }

    if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
        throw "既存matrix runnerの出力が見つかりません: $sourcePath"
    }
    $sourceResult = Get-Content -LiteralPath $sourcePath -Raw -Encoding UTF8 | ConvertFrom-Json
    $cpu = Get-CpuSnapshot
    $rustHost = Get-RustHost
    $reachability = Get-ReachabilityScan -Repository $RepositoryRoot -Pattern $ReachabilityPattern -RelativeRoots $ReachabilityRoots
    $summary = Write-DeterminismSummary `
        -RequestedMode $RequestedMode `
        -WasResumed $WasResumed `
        -RunnerPath $MatrixRunnerPath `
        -SourcePath $sourcePath `
        -SourceResult $sourceResult `
        -OutputPath $DeterminismOutputPath `
        -CpuSnapshot $cpu `
        -RustHost $rustHost `
        -Reachability $reachability
    Write-Output "[OK] determinism summary: $DeterminismOutputPath ($($summary.measurements.Count) measurement(s))"
}

if ($MyInvocation.InvocationName -ne ".") {
    Invoke-DeterminismRun -RequestedMode $Mode -WasResumed $Resume.IsPresent -IsDryRun $DryRun.IsPresent
}
