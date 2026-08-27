[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ScriptPath = Join-Path $PSScriptRoot "check-proposal-determinism.ps1"
$SandboxRoot = Join-Path ([IO.Path]::GetTempPath()) ("ori3-proposal-determinism-test-" + [Guid]::NewGuid().ToString("N"))
$script:AssertionCount = 0

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $script:AssertionCount += 1
    if (-not $Condition) {
        throw "ASSERTION FAILED: $Message"
    }
}

function Assert-Equal {
    param(
        [AllowNull()]$Actual,
        [AllowNull()]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $script:AssertionCount += 1
    if ($Actual -ne $Expected) {
        throw "ASSERTION FAILED: $Message (expected=$Expected, actual=$Actual)"
    }
}

function Write-TestJson {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    $parent = Split-Path -Parent $Path
    [void][IO.Directory]::CreateDirectory($parent)
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
}

if (-not (Test-Path -LiteralPath $ScriptPath -PathType Leaf)) {
    throw "Required implementation is missing: $ScriptPath"
}

try {
    Write-Host "[1/3] DryRun reports the delegated command and both JSON paths"
    $global:LASTEXITCODE = 0
    $dryRunOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ScriptPath -Mode Full -Resume -DryRun 2>&1 | ForEach-Object { $_.ToString() })
    Assert-Equal $LASTEXITCODE 0 "DryRun must exit successfully"
    Assert-True ($dryRunOutput -contains "[DRY-RUN] existing runner will not be invoked.") "DryRun must state that it does not invoke the runner"
    Assert-True (($dryRunOutput -join "`n") -match "run-proposal-matrix\.ps1.*-Mode Full -Resume") "DryRun must include the Full Resume runner command"
    Assert-True (($dryRunOutput -join "`n") -match "verification.*propose-matrix.*matrix-state\.json") "DryRun must show the Full source JSON"
    Assert-True (($dryRunOutput -join "`n") -match "verification.*improvement-roadmap.*01-proposal.*determinism\.json") "DryRun must show the summary destination"

    . $ScriptPath
    [void][IO.Directory]::CreateDirectory($SandboxRoot)

    Write-Host "[2/3] Write-DeterminismSummary normalizes a fake Performance result"
    $sourcePath = Join-Path $SandboxRoot "source\ci-performance.json"
    $runnerPath = Join-Path $SandboxRoot "run-proposal-matrix.ps1"
    $summaryPath = Join-Path $SandboxRoot "out\determinism.json"
    [IO.File]::WriteAllText($runnerPath, "# fake runner`n", [Text.UTF8Encoding]::new($false))
    $fakeSource = [ordered]@{
        schema = 1
        mode = "Performance"
        started_at = "2026-08-27T01:02:03.0000000+09:00"
        finished_at = "2026-08-27T01:02:05.0000000+09:00"
        input_fingerprint = [ordered]@{
            aggregate_sha256 = "fake-input-sha256"
            files = @([ordered]@{ path = "crates/ori3-propose/src/search.rs"; sha256 = "fake-file-sha256" })
        }
        probe = [ordered]@{
            contract = [ordered]@{
                profile = "release"
                candidates = 4
                requests = 2
                computation = "parallel"
                load = "busy"
                load_threads = 8
                iterations = 100
                candidate_hash = "0123456789abcdef"
                stop_hash = "fedcba9876543210"
                first_candidate_hash = "0011223344556677"
                first_stop = "goal_reached"
                stops = "goal_reached|goal_reached|goal_reached|goal_reached"
            }
            elapsed_seconds = 12.5
            warning_count = 0
        }
    }
    Write-TestJson -Path $sourcePath -Value $fakeSource
    $fakeReachability = [ordered]@{
        method = "fixture"
        command = "rg -n fixture"
        search_roots = @("crates")
        matching_line_count = 4
        matches = @("fixture:1:job_id")
        static_proposal_progress = [ordered]@{
            matching_line_count = 0
            matches = @()
            is_zero = $true
        }
    }
    $summary = Write-DeterminismSummary `
        -RequestedMode Performance `
        -WasResumed $false `
        -RunnerPath $runnerPath `
        -SourcePath $sourcePath `
        -SourceResult ($fakeSource | ConvertTo-Json -Depth 10 | ConvertFrom-Json) `
        -OutputPath $summaryPath `
        -CpuSnapshot ([ordered]@{ processors = @([ordered]@{ name = "fixture cpu"; logical_processors = 8 }); logical_processors_total = 8 }) `
        -RustHost "x86_64-pc-windows-msvc" `
        -Reachability $fakeReachability
    Assert-True (Test-Path -LiteralPath $summaryPath -PathType Leaf) "summary JSON must be written"
    $written = Get-Content -LiteralPath $summaryPath -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-Equal $written.input_fingerprint.aggregate_sha256 "fake-input-sha256" "summary must preserve input fingerprint"
    Assert-Equal $written.measurements.Count 1 "Performance result must produce one measurement"
    Assert-Equal $written.measurements[0].candidate_hash "0123456789abcdef" "summary must preserve candidate hash"
    Assert-Equal $written.measurements[0].stop_hash "fedcba9876543210" "summary must preserve stop hash"
    Assert-Equal $written.measurements[0].load_threads 8 "summary must preserve worker count"
    Assert-Equal $written.rustc.host "x86_64-pc-windows-msvc" "summary must record rust host"
    Assert-Equal $written.reachability.static_proposal_progress.matching_line_count 0 "summary must record zero global PROPOSAL_PROGRESS"
    Assert-True (-not [string]::IsNullOrWhiteSpace([string]$written.source_results[0].reference.sha256)) "summary must hash its source JSON"

    Write-Host "[3/3] Resume is rejected for non-Full modes before a runner can start"
    $global:LASTEXITCODE = 0
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $resumeOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ScriptPath -Mode Validate -Resume -DryRun 2>&1 | ForEach-Object { $_.ToString() })
        $resumeExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    Assert-True ($resumeExitCode -ne 0) "Validate Resume must fail"
    Assert-True (($resumeOutput -join "`n") -match "-ResumeはFullでだけ使えます") "Resume failure must explain the Full-only contract"

    Write-Host "check-proposal-determinism self-test passed: $script:AssertionCount assertions"
}
finally {
    if (Test-Path -LiteralPath $SandboxRoot) {
        $resolvedSandbox = [IO.Path]::GetFullPath($SandboxRoot)
        $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
        if (-not $resolvedSandbox.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing unsafe self-test cleanup: $resolvedSandbox"
        }
        Remove-Item -LiteralPath $resolvedSandbox -Recurse -Force
    }
}
