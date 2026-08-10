#requires -Version 5.1

<#
.SYNOPSIS
Captures every state of an ORIGAMI3 document through the WebView2 CDP port.

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\capture-steps.ps1 `
  -Document .\sample.ori3 -OutDir .\verification\capture\sample -Views both
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Document,

    [Parameter(Mandatory = $true)]
    [string]$OutDir,

    [ValidateSet("3d", "cp", "both")]
    [string]$Views = "both"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$endpoint = "http://127.0.0.1:9222"
$cdpEntry = Join-Path $PSScriptRoot "capture-steps-cdp.mjs"
$appExe = Join-Path $root "target\debug\desktop.exe"
$tauriEntry = Join-Path $root "apps\desktop\node_modules\@tauri-apps\cli\tauri.js"
$viteEntry = Join-Path $root "apps\desktop\node_modules\vite\bin\vite.js"
$tauriCaptureConfig = Join-Path $root "apps\desktop\src-tauri\tauri.capture.conf.json"
$captureTempRoot = Join-Path $root "verification\capture"
$script:actionCounter = 0
$script:ownedApp = $null
$script:ownedLauncher = $null
$script:ownedVite = $null
$script:runSucceeded = $false

function Write-Milestone {
    param([string]$Message)
    Write-Host "[capture] $Message"
}

function Write-Utf8NoBom {
    param([string]$Path, [string]$Text)
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Text, $encoding)
}

function Read-Utf8 {
    param([string]$Path)
    return [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8)
}

function Quote-NativeArgument {
    param([string]$Value)
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Get-CdpPages {
    try {
        $response = Invoke-WebRequest `
            -Uri "$endpoint/json/list" `
            -UseBasicParsing `
            -TimeoutSec 2
        $targets = @($response.Content | ConvertFrom-Json)
        return @($targets | Where-Object {
            $_.type -eq "page" -and -not [string]::IsNullOrWhiteSpace($_.webSocketDebuggerUrl)
        })
    }
    catch {
        return @()
    }
}

function Get-OrigamiProcesses {
    $expectedExe = [System.IO.Path]::GetFullPath($appExe)
    $matches = @()
    foreach ($process in @(Get-Process -Name "desktop" -ErrorAction SilentlyContinue)) {
        $samePath = $false
        try {
            $samePath = -not [string]::IsNullOrWhiteSpace($process.Path) -and
                [System.IO.Path]::GetFullPath($process.Path).Equals(
                    $expectedExe,
                    [System.StringComparison]::OrdinalIgnoreCase
                )
        }
        catch {
            $samePath = $false
        }
        if ($samePath -or $process.MainWindowTitle -eq "ORIGAMI3") {
            $matches += $process
        }
    }
    return @($matches)
}

function Stop-OrigamiProcesses {
    $processes = @(Get-OrigamiProcesses)
    if ($processes.Count -eq 0) { return }
    Write-Milestone "closing $($processes.Count) stale ORIGAMI3 process(es)"
    foreach ($process in $processes) {
        try { [void]$process.CloseMainWindow() } catch { }
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([DateTime]::UtcNow -lt $deadline) {
        $alive = @($processes | Where-Object { -not $_.HasExited })
        if ($alive.Count -eq 0) { break }
        Start-Sleep -Milliseconds 200
    }
    foreach ($process in $processes) {
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force
        }
    }
}

function Get-AppExitDetail {
    if ($null -ne $script:ownedApp) {
        $script:ownedApp.Refresh()
        if ($script:ownedApp.HasExited) {
            return " ORIGAMI3 exited with code $($script:ownedApp.ExitCode)."
        }
    }
    if ($null -ne $script:ownedLauncher) {
        $script:ownedLauncher.Refresh()
        if ($script:ownedLauncher.HasExited) {
            return " Tauri launcher exited with code $($script:ownedLauncher.ExitCode)."
        }
    }
    return ""
}

function Test-FrontendServer {
    try {
        $response = Invoke-WebRequest `
            -Uri "http://127.0.0.1:1420" `
            -UseBasicParsing `
            -TimeoutSec 2
        return $response.StatusCode -eq 200 -and $response.Content -match '<title>ORIGAMI3</title>'
    }
    catch {
        return $false
    }
}

function Start-FrontendServerIfNeeded {
    if (Test-FrontendServer) {
        Write-Milestone "reusing the ORIGAMI3 frontend server on port 1420"
        return
    }
    if (-not (Test-Path -LiteralPath $viteEntry -PathType Leaf)) {
        throw "Vite is unavailable; run npm install in apps/desktop"
    }
    $viteStdout = Join-Path $runDir "vite.stdout.log"
    $viteStderr = Join-Path $runDir "vite.stderr.log"
    $viteArgs = @(
        (Quote-NativeArgument $viteEntry),
        "--host", "127.0.0.1",
        "--port", "1420",
        "--strictPort"
    ) -join " "
    $script:ownedVite = Start-Process `
        -FilePath $nodeCommand.Source `
        -ArgumentList $viteArgs `
        -WorkingDirectory (Join-Path $root "apps\desktop") `
        -WindowStyle Hidden `
        -RedirectStandardOutput $viteStdout `
        -RedirectStandardError $viteStderr `
        -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    while ([DateTime]::UtcNow -lt $deadline) {
        $script:ownedVite.Refresh()
        if ($script:ownedVite.HasExited) {
            throw "Vite exited during startup with code $($script:ownedVite.ExitCode)"
        }
        if (Test-FrontendServer) {
            Write-Milestone "started the ORIGAMI3 frontend server on port 1420"
            return
        }
        Start-Sleep -Milliseconds 250
    }
    throw "ORIGAMI3 frontend server did not start within 60 seconds"
}

function Invoke-CdpActions {
    param(
        [Parameter(Mandatory = $true)] [object[]]$Actions,
        [Parameter(Mandatory = $true)] [string]$Label,
        [int]$TimeoutSeconds = 120
    )

    $script:actionCounter++
    $stem = "actions-{0:D4}" -f $script:actionCounter
    $actionPath = Join-Path $runDir "$stem.json"
    $stdoutPath = Join-Path $runDir "$stem.stdout.log"
    $stderrPath = Join-Path $runDir "$stem.stderr.log"
    Write-Utf8NoBom $actionPath (ConvertTo-Json -InputObject $Actions -Depth 8)

    $argumentLine = @(
        (Quote-NativeArgument $cdpEntry),
        (Quote-NativeArgument $actionPath),
        (Quote-NativeArgument $endpoint)
    ) -join " "
    $process = Start-Process `
        -FilePath $nodeCommand.Source `
        -ArgumentList $argumentLine `
        -WorkingDirectory $root `
        -WindowStyle Hidden `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath `
        -PassThru

    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "$Label timed out after $TimeoutSeconds seconds.$(Get-AppExitDetail)"
    }
    $process.WaitForExit()
    $stdout = if (Test-Path -LiteralPath $stdoutPath) { Read-Utf8 $stdoutPath } else { "" }
    $stderr = if (Test-Path -LiteralPath $stderrPath) { Read-Utf8 $stderrPath } else { "" }
    if ($process.ExitCode -ne 0) {
        $detail = ($stderr.Trim() + " " + $stdout.Trim()).Trim()
        throw "$Label failed (CDP exit $($process.ExitCode)).$(Get-AppExitDetail) $detail"
    }

    $records = @()
    foreach ($line in @($stdout -split "`r?`n")) {
        if (-not [string]::IsNullOrWhiteSpace($line)) {
            $records += $line | ConvertFrom-Json
        }
    }
    return @($records)
}

function Test-CaptureApi {
    try {
        $probe = @(
            [ordered]@{
                act = "eval"
                js = @"
(async () => {
  const deadline = Date.now() + 10000;
  while (!window.__origami3Capture && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  if (!window.__origami3Capture) throw new Error("ORIGAMI3 capture API is unavailable");
  return { version: window.__origami3Capture.version, title: document.title };
})()
"@
            }
        )
        $result = @(Invoke-CdpActions -Actions $probe -Label "capture API probe" -TimeoutSeconds 20)
        return $result.Count -eq 1 -and
            $result[0].result.version -eq 1 -and
            $result[0].result.title -eq "ORIGAMI3"
    }
    catch {
        return $false
    }
}

function Wait-ForCaptureApi {
    param([int]$TimeoutSeconds = 120)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $webviewDeadline = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($null -eq $script:ownedApp) {
            $started = @(Get-OrigamiProcesses)
            if ($started.Count -gt 1) {
                throw "More than one ORIGAMI3 process appeared during startup"
            }
            if ($started.Count -eq 1) {
                $script:ownedApp = $started[0]
                $webviewDeadline = [DateTime]::UtcNow.AddSeconds(25)
                Write-Milestone "started one ORIGAMI3 process (PID $($script:ownedApp.Id))"
            }
        }
        if ($null -ne $script:ownedApp) {
            $script:ownedApp.Refresh()
            if ($script:ownedApp.HasExited) {
                throw "ORIGAMI3 crashed during startup with exit code $($script:ownedApp.ExitCode)"
            }
        }
        if ($null -ne $script:ownedLauncher) {
            $script:ownedLauncher.Refresh()
            if ($script:ownedLauncher.HasExited -and $null -eq $script:ownedApp) {
                throw "Tauri launcher exited during startup with code $($script:ownedLauncher.ExitCode)"
            }
        }
        $pages = @(Get-CdpPages)
        if ($pages.Count -gt 1) {
            throw "CDP port 9222 exposes multiple page targets; refusing an ambiguous capture"
        }
        if ($pages.Count -eq 1) {
            if (Test-CaptureApi) { return }
        }
        if ($null -ne $webviewDeadline -and [DateTime]::UtcNow -ge $webviewDeadline) {
            throw "WebView2 did not expose CDP port 9222 within 25 seconds"
        }
        Start-Sleep -Milliseconds 250
    }
    throw "ORIGAMI3 did not expose CDP port 9222 within $TimeoutSeconds seconds$(Get-AppExitDetail)"
}

function Start-CaptureApp {
    Start-FrontendServerIfNeeded
    if (-not (Test-Path -LiteralPath $tauriEntry -PathType Leaf)) {
        throw "Tauri CLI is unavailable; run npm install in apps/desktop"
    }
    if (-not (Test-Path -LiteralPath $tauriCaptureConfig -PathType Leaf)) {
        throw "Capture configuration is missing: $tauriCaptureConfig"
    }
    $runtimeRoot = "C:\Program Files (x86)\Microsoft\EdgeWebView\Application"
    $olderRuntimes = @()
    if (Test-Path -LiteralPath $runtimeRoot -PathType Container) {
        $olderRuntimes = @(
            Get-ChildItem -LiteralPath $runtimeRoot -Directory |
                Where-Object {
                    $parsedVersion = $null
                    [Version]::TryParse($_.Name, [ref]$parsedVersion) -and
                        (Test-Path -LiteralPath (Join-Path $_.FullName "msedgewebview2.exe"))
                } |
                Sort-Object { [Version]$_.Name } -Descending |
                Select-Object -Skip 1 -First 2
        )
    }
    $lastStartupError = ""
    foreach ($attempt in 1..3) {
        $runtimeFolder = $null
        if ($attempt -gt 1 -and $olderRuntimes.Count -ge ($attempt - 1)) {
            $runtimeFolder = $olderRuntimes[$attempt - 2].FullName
        }
        $runtimeLabel = if ($null -eq $runtimeFolder) { "default WebView2" } else {
            "WebView2 $($olderRuntimes[$attempt - 2].Name)"
        }
        Write-Milestone "starting ORIGAMI3 (attempt $attempt/3, $runtimeLabel)"
        # Keep the WebView2 profile path short. Chromium creates deeply nested files under it,
        # and long per-run paths can make WebView initialization fail on Windows.
        $profilePath = Join-Path $captureTempRoot "wv2-$attempt"
        if (-not [string]::IsNullOrWhiteSpace($env:ORIGAMI3_CAPTURE_WEBVIEW2_PROFILE)) {
            $overridePath = [System.IO.Path]::GetFullPath(
                $env:ORIGAMI3_CAPTURE_WEBVIEW2_PROFILE
            )
            $allowedProfileRoot = [System.IO.Path]::GetFullPath($captureTempRoot).TrimEnd('\') + '\'
            if (-not $overridePath.StartsWith(
                $allowedProfileRoot,
                [System.StringComparison]::OrdinalIgnoreCase
            )) {
                throw "ORIGAMI3_CAPTURE_WEBVIEW2_PROFILE must be under $captureTempRoot"
            }
            $profilePath = $overridePath
        }
        $launcherStdout = Join-Path $runDir "tauri-$attempt.stdout.log"
        $launcherStderr = Join-Path $runDir "tauri-$attempt.stderr.log"
        $previousArgs = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
        $previousProfile = $env:WEBVIEW2_USER_DATA_FOLDER
        $previousRuntime = $env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER
        try {
            $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS =
                "--remote-debugging-port=9222 --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"
            $env:WEBVIEW2_USER_DATA_FOLDER = $profilePath
            if ($null -ne $runtimeFolder) {
                $env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = $runtimeFolder
            }
            $launcherArgs = @(
                (Quote-NativeArgument $tauriEntry),
                "dev",
                "--no-watch",
                "--no-dev-server-wait",
                "--config",
                (Quote-NativeArgument $tauriCaptureConfig)
            ) -join " "
            $script:ownedLauncher = Start-Process `
                -FilePath $nodeCommand.Source `
                -ArgumentList $launcherArgs `
                -WorkingDirectory (Join-Path $root "apps\desktop") `
                -WindowStyle Hidden `
                -RedirectStandardOutput $launcherStdout `
                -RedirectStandardError $launcherStderr `
                -PassThru
        }
        finally {
            if ($null -eq $previousArgs) {
                Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
            }
            else {
                $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $previousArgs
            }
            if ($null -eq $previousProfile) {
                Remove-Item Env:WEBVIEW2_USER_DATA_FOLDER -ErrorAction SilentlyContinue
            }
            else {
                $env:WEBVIEW2_USER_DATA_FOLDER = $previousProfile
            }
            if ($null -eq $previousRuntime) {
                Remove-Item Env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER -ErrorAction SilentlyContinue
            }
            else {
                $env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = $previousRuntime
            }
        }
        try {
            Wait-ForCaptureApi -TimeoutSeconds 120
            return
        }
        catch {
            $lastStartupError = [string]$_.Exception.Message
            Write-Milestone "startup attempt $attempt/3 failed; restarting the isolated WebView2 host"
            Stop-OwnedHost
            Stop-OrigamiProcesses
            $portDeadline = [DateTime]::UtcNow.AddSeconds(10)
            while (@(Get-CdpPages).Count -gt 0 -and [DateTime]::UtcNow -lt $portDeadline) {
                Start-Sleep -Milliseconds 200
            }
        }
    }
    throw "ORIGAMI3 startup failed three times: $lastStartupError"
}

function Stop-OwnedHost {
    if ($null -ne $script:ownedApp) {
        $script:ownedApp.Refresh()
        if (-not $script:ownedApp.HasExited) {
            try { [void]$script:ownedApp.CloseMainWindow() } catch { }
            if (-not $script:ownedApp.WaitForExit(5000)) {
                Stop-Process -Id $script:ownedApp.Id -Force -ErrorAction SilentlyContinue
            }
        }
        $script:ownedApp = $null
    }
    if ($null -ne $script:ownedLauncher) {
        $script:ownedLauncher.Refresh()
        if (-not $script:ownedLauncher.HasExited) {
            if (-not $script:ownedLauncher.WaitForExit(5000)) {
                # The Tauri CLI owns Cargo and the desktop process. Stop only this
                # launcher's process tree so failed retries cannot leave compilers behind.
                & taskkill.exe /PID $script:ownedLauncher.Id /T /F 2>$null | Out-Null
            }
        }
        $script:ownedLauncher = $null
    }
}

function Stop-OwnedApp {
    Stop-OwnedHost
    if ($null -ne $script:ownedVite) {
        $script:ownedVite.Refresh()
        if (-not $script:ownedVite.HasExited) {
            Stop-Process -Id $script:ownedVite.Id -Force -ErrorAction SilentlyContinue
        }
    }
}

function Write-StepsJson {
    param([object[]]$Entries, [string]$Path)
    $json = ConvertTo-Json -InputObject ([object[]]$Entries) -Depth 6
    Write-Utf8NoBom $Path $json
}

if (-not (Test-Path -LiteralPath $Document -PathType Leaf)) {
    throw "Document does not exist: $Document"
}
$documentPath = (Resolve-Path -LiteralPath $Document).Path
if ([System.IO.Path]::GetExtension($documentPath) -ine ".ori3") {
    throw "Document must have the .ori3 extension: $documentPath"
}

if (-not [System.IO.Path]::IsPathRooted($OutDir)) {
    $OutDir = Join-Path (Get-Location) $OutDir
}
$outPath = [System.IO.Path]::GetFullPath($OutDir)
[void](New-Item -ItemType Directory -Path $outPath -Force)
$outPath = (Resolve-Path -LiteralPath $outPath).Path
[void](New-Item -ItemType Directory -Path $captureTempRoot -Force)
$runId = "run-{0}-{1}" -f ([DateTime]::UtcNow.ToString("yyyyMMdd-HHmmss")), $PID
$runDir = Join-Path $captureTempRoot $runId
[void](New-Item -ItemType Directory -Path $runDir -Force)

if (-not (Test-Path -LiteralPath $cdpEntry -PathType Leaf)) {
    throw "CDP entry script is missing: $cdpEntry"
}
$nodeCommand = Get-Command node.exe -ErrorAction Stop

# Remove only files owned by this tool so old steps cannot masquerade as a new run.
foreach ($file in @(Get-ChildItem -LiteralPath $outPath -File -ErrorAction SilentlyContinue)) {
    if ($file.Name -eq "steps.json" -or $file.Name -match '^step-\d{4}-(3d|cp)\.png$') {
        Remove-Item -LiteralPath $file.FullName -Force
    }
}

Write-Milestone "document: $documentPath"
Write-Milestone "output: $outPath (views=$Views)"

try {
    $pages = @(Get-CdpPages)
    $origamiProcesses = @(Get-OrigamiProcesses)
    $reuseExisting = $pages.Count -eq 1 -and
        $origamiProcesses.Count -le 1 -and
        (Test-CaptureApi)

    if ($reuseExisting) {
        Write-Milestone "reusing the healthy ORIGAMI3 instance on CDP port 9222"
    }
    else {
        if ($pages.Count -gt 0 -and $origamiProcesses.Count -eq 0) {
            throw "CDP port 9222 is occupied by another application"
        }
        Stop-OrigamiProcesses
        $portDeadline = [DateTime]::UtcNow.AddSeconds(10)
        while (@(Get-CdpPages).Count -gt 0 -and [DateTime]::UtcNow -lt $portDeadline) {
            Start-Sleep -Milliseconds 200
        }
        if (@(Get-CdpPages).Count -gt 0) {
            throw "CDP port 9222 remained occupied after closing stale ORIGAMI3 processes"
        }
        Start-CaptureApp
    }

    Write-Milestone "opening the document"
    $documentLiteral = ConvertTo-Json -InputObject $documentPath -Compress
    $openActions = @(
        [ordered]@{
            act = "eval"
            js = @"
(async () => {
  const api = window.__origami3Capture;
  if (!api) throw new Error("ORIGAMI3 capture API is unavailable");
  return await api.openDocument($documentLiteral);
})()
"@
        }
    )
    $openRecords = @(Invoke-CdpActions -Actions $openActions -Label "open document" -TimeoutSeconds 120)
    if ($openRecords.Count -ne 1) {
        throw "Unexpected CDP response while opening the document"
    }
    $documentInfo = $openRecords[0].result
    $steps = @($documentInfo.steps)
    if ($steps.Count -ne ([int]$documentInfo.stepCount + 1)) {
        throw "Capture API returned an inconsistent step list"
    }
    Write-Milestone "document ready: $($documentInfo.stepCount) fold step(s), $($steps.Count) state(s) including step 0"

    $capture3d = $Views -eq "3d" -or $Views -eq "both"
    $captureCp = $Views -eq "cp" -or $Views -eq "both"
    $manifest = @()
    $stepsJsonPath = Join-Path $outPath "steps.json"

    foreach ($step in $steps) {
        $number = [int]$step.number
        $serial = $number.ToString("D4")
        Write-Milestone "state $($number + 1)/$($steps.Count): $($step.name)"
        $actions = @(
            [ordered]@{
                act = "eval"
                js = "window.__origami3Capture.goToStep($number)"
            }
        )
        $images = [ordered]@{}
        if ($capture3d) {
            $name3d = "step-$serial-3d.png"
            $path3d = Join-Path $outPath $name3d
            $actions += [ordered]@{ act = "eval"; js = 'window.__origami3Capture.setView("3d")' }
            $actions += [ordered]@{ act = "shot"; path = $path3d }
            $images["3d"] = $name3d
        }
        if ($captureCp) {
            $nameCp = "step-$serial-cp.png"
            $pathCp = Join-Path $outPath $nameCp
            $actions += [ordered]@{ act = "eval"; js = 'window.__origami3Capture.setView("cp")' }
            $actions += [ordered]@{ act = "shot"; path = $pathCp }
            $images["cp"] = $nameCp
        }
        [void](Invoke-CdpActions -Actions $actions -Label "capture step $number" -TimeoutSeconds 120)
        $manifest += [ordered]@{
            number = $number
            name = [string]$step.name
            images = $images
        }
        Write-StepsJson -Entries $manifest -Path $stepsJsonPath
    }

    $viewCount = [int]$capture3d + [int]$captureCp
    $expectedImages = $steps.Count * $viewCount
    $actualImages = @(
        Get-ChildItem -LiteralPath $outPath -File |
            Where-Object { $_.Name -match '^step-\d{4}-(3d|cp)\.png$' -and $_.Length -gt 0 }
    )
    if ($actualImages.Count -ne $expectedImages) {
        throw "Expected $expectedImages non-empty PNG files, found $($actualImages.Count)"
    }

    $script:runSucceeded = $true
    Write-Milestone "complete: $($steps.Count) state(s), $expectedImages image(s), steps.json"
}
finally {
    if (@(Get-CdpPages).Count -eq 1) {
        try {
            $restore = @([ordered]@{
                act = "eval"
                js = 'window.__origami3Capture && window.__origami3Capture.setView("normal")'
            })
            [void](Invoke-CdpActions -Actions $restore -Label "restore normal view" -TimeoutSeconds 15)
        }
        catch { }
    }
    Stop-OwnedApp

    if ($script:runSucceeded) {
        $tempBase = [System.IO.Path]::GetFullPath($captureTempRoot).TrimEnd('\') + '\'
        $tempRun = [System.IO.Path]::GetFullPath($runDir)
        if ($tempRun.StartsWith($tempBase, [System.StringComparison]::OrdinalIgnoreCase) -and
            (Split-Path -Leaf $tempRun).StartsWith("run-")) {
            Remove-Item -LiteralPath $tempRun -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    else {
        Write-Milestone "failed; diagnostic files kept in $runDir"
    }
}
