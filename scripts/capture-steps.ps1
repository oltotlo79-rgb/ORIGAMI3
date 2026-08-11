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
$viteEntry = Join-Path $root "apps\desktop\node_modules\vite\bin\vite.js"
$tauriCaptureConfig = Join-Path $root "apps\desktop\src-tauri\tauri.capture.conf.json"
$captureTempRoot = Join-Path $root "verification\capture"
$script:actionCounter = 0
$script:ownedApp = $null
$script:ownedLauncher = $null
$script:ownedVite = $null
$script:runSucceeded = $false
$script:targetId = $null
$script:targetGeneration = $null
$script:preserveOwnedHost = $false

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

function Get-DesktopProcesses {
    # The process name is the authority for the 0/1/many startup decision. Do not
    # discard an existing instance merely because its path or title is temporarily
    # unavailable while WebView2 is starting.
    return @(Get-Process -Name "desktop" -ErrorAction SilentlyContinue)
}

function Test-ProcessDescendsFrom {
    param(
        [Parameter(Mandatory = $true)] [int]$ProcessId,
        [Parameter(Mandatory = $true)] [int]$AncestorId
    )

    $seen = New-Object 'System.Collections.Generic.HashSet[int]'
    $currentId = $ProcessId
    while ($currentId -gt 0 -and $seen.Add($currentId)) {
        try {
            $row = @(
                Get-CimInstance Win32_Process `
                    -Filter "ProcessId = $currentId" `
                    -ErrorAction Stop
            ) | Select-Object -First 1
        }
        catch {
            # Some managed Windows sessions deny Win32_Process queries even for
            # the current user. Let the caller use its path/start-time fallback.
            return $null
        }
        if ($null -eq $row) { return $false }
        $parentId = [int]$row.ParentProcessId
        if ($parentId -eq $AncestorId) { return $true }
        $currentId = $parentId
    }
    return $false
}

function Remove-CaptureOrphanedWebViews {
    if (@(Get-DesktopProcesses).Count -ne 0) {
        throw "Refusing to clean WebView2 processes while desktop.exe is running"
    }

    try {
        $all = @(Get-CimInstance Win32_Process -ErrorAction Stop)
    }
    catch {
        # Command lines are required to distinguish our private WebView2 profile
        # from Search, Widgets, Office, and other legitimate WebView2 hosts.
        # Never compensate for a denied query by stopping every WebView2 process.
        $visible = @(Get-Process -Name "msedgewebview2" -ErrorAction SilentlyContinue)
        Write-Milestone "WebView2 command-line inspection is unavailable; left $($visible.Count) unclassified process(es) untouched"
        return
    }
    $normalizedRoot = [System.IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
    $profilePrefix = [System.IO.Path]::GetFullPath(
        (Join-Path $captureTempRoot "wv2-")
    )
    $legacyProfileName = "webview-profile-nosandbox"
    $seeds = @(
        $all | Where-Object {
            if ($_.Name -ine "msedgewebview2.exe" -or
                [string]::IsNullOrWhiteSpace($_.CommandLine)) {
                return $false
            }
            $commandLine = ([string]$_.CommandLine).Replace('/', '\')
            $ownedProfile = $commandLine.IndexOf(
                $profilePrefix,
                [System.StringComparison]::OrdinalIgnoreCase
            ) -ge 0
            $legacyProfile = $commandLine.IndexOf(
                $normalizedRoot,
                [System.StringComparison]::OrdinalIgnoreCase
            ) -ge 0 -and $commandLine.IndexOf(
                $legacyProfileName,
                [System.StringComparison]::OrdinalIgnoreCase
            ) -ge 0
            return $ownedProfile -or $legacyProfile
        }
    )

    $byParent = @{}
    $byId = @{}
    foreach ($row in $all) {
        $processId = [int]$row.ProcessId
        $parentId = [int]$row.ParentProcessId
        $byId[$processId] = $row
        if (-not $byParent.ContainsKey($parentId)) {
            $byParent[$parentId] = New-Object 'System.Collections.Generic.List[int]'
        }
        $byParent[$parentId].Add($processId)
    }

    $targetIds = New-Object 'System.Collections.Generic.HashSet[int]'
    $pending = New-Object 'System.Collections.Generic.Queue[int]'
    foreach ($seed in $seeds) {
        $seedId = [int]$seed.ProcessId
        if ($targetIds.Add($seedId)) { $pending.Enqueue($seedId) }
    }
    while ($pending.Count -gt 0) {
        $parentId = $pending.Dequeue()
        if (-not $byParent.ContainsKey($parentId)) { continue }
        foreach ($childId in $byParent[$parentId]) {
            if ($targetIds.Add($childId)) { $pending.Enqueue($childId) }
        }
    }

    # Stop leaves before their parents. That keeps every target attributable to
    # the captured process tree and makes the reported count deterministic.
    $targetsWithDepth = @()
    foreach ($targetId in $targetIds) {
        $depth = 0
        $cursor = $targetId
        $seen = New-Object 'System.Collections.Generic.HashSet[int]'
        while ($byId.ContainsKey($cursor) -and $seen.Add($cursor)) {
            $parentId = [int]$byId[$cursor].ParentProcessId
            if (-not $targetIds.Contains($parentId)) { break }
            $depth++
            $cursor = $parentId
        }
        $targetsWithDepth += [pscustomobject]@{ ProcessId = $targetId; Depth = $depth }
    }

    $cleaned = 0
    foreach ($target in @($targetsWithDepth | Sort-Object Depth -Descending)) {
        try {
            Stop-Process -Id ([int]$target.ProcessId) -Force -ErrorAction Stop
            $cleaned++
        }
        catch {
            # A parent may have already reaped a child between enumeration and stop.
            # Access-denied and other failures must remain visible instead of being
            # reported as a successful cleanup.
            if ($null -ne (Get-Process -Id ([int]$target.ProcessId) -ErrorAction SilentlyContinue)) {
                throw
            }
        }
    }
    Write-Milestone "cleaned $cleaned orphaned WebView2 process(es)"
}

function Test-TcpPortOpen {
    param([int]$Port)

    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $pending = $client.BeginConnect("127.0.0.1", $Port, $null, $null)
        if (-not $pending.AsyncWaitHandle.WaitOne(500)) { return $false }
        $client.EndConnect($pending)
        return $true
    }
    catch {
        return $false
    }
    finally {
        $client.Close()
    }
}

function Wait-ForCapturePortFree {
    param([int]$TimeoutSeconds = 10, [int]$ConsecutiveChecks = 5)

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $closedCount = 0
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-TcpPortOpen -Port 9222) {
            $closedCount = 0
        }
        else {
            $closedCount++
            if ($closedCount -ge $ConsecutiveChecks) { return }
        }
        Start-Sleep -Milliseconds 250
    }
    throw "CDP port 9222 was not continuously free for $ConsecutiveChecks checks"
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

function Invoke-HiddenNativeCommand {
    param(
        [Parameter(Mandatory = $true)] [string]$Code,
        [Parameter(Mandatory = $true)] [string]$StatusName,
        [Parameter(Mandatory = $true)] [string]$Label,
        [int]$TimeoutSeconds = 300
    )

    $statusPath = Join-Path $runDir "$StatusName.exit"
    $statusLiteral = $statusPath.Replace("'", "''")
    $wrappedCode = @"
`$ErrorActionPreference = "Continue"
`$nativeExitCode = 1
try {
$Code
  if (`$null -ne `$LASTEXITCODE) { `$nativeExitCode = [int]`$LASTEXITCODE }
}
catch {
  `$nativeExitCode = 1
}
[IO.File]::WriteAllText('$statusLiteral', [string]`$nativeExitCode)
exit `$nativeExitCode
"@
    $encodedCode = [Convert]::ToBase64String(
        [Text.Encoding]::Unicode.GetBytes($wrappedCode)
    )
    $helper = Start-Process `
        -FilePath (Get-Command powershell.exe -ErrorAction Stop).Source `
        -ArgumentList "-NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand $encodedCode" `
        -WindowStyle Hidden `
        -PassThru
    try {
        if (-not $helper.WaitForExit($TimeoutSeconds * 1000)) {
            & taskkill.exe /PID $helper.Id /T /F 2>$null | Out-Null
            throw "$Label timed out after $TimeoutSeconds seconds"
        }
        $helper.WaitForExit()
    }
    finally {
        $helper.Dispose()
    }

    if (-not (Test-Path -LiteralPath $statusPath -PathType Leaf)) {
        throw "$Label did not report its exit status"
    }
    $nativeExitCode = 0
    if (-not [int]::TryParse((Read-Utf8 $statusPath).Trim(), [ref]$nativeExitCode) -or
        $nativeExitCode -ne 0) {
        throw "$Label failed (exit $nativeExitCode)"
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
    $npmCommand = Get-Command npm.cmd -ErrorAction Stop
    $desktopDirLiteral = (Join-Path $root "apps\desktop").Replace("'", "''")
    $npmLiteral = $npmCommand.Source.Replace("'", "''")
    $frontendBuildCode = @"
Set-Location -LiteralPath '$desktopDirLiteral'
& '$npmLiteral' run build
"@
    Invoke-HiddenNativeCommand `
        -Code $frontendBuildCode `
        -StatusName "frontend-build" `
        -Label "ORIGAMI3 frontend build" `
        -TimeoutSeconds 180
    Write-Milestone "built the current ORIGAMI3 frontend"

    $viteArgs = @(
        (Quote-NativeArgument $viteEntry),
        "preview",
        "--configLoader", "runner",
        "--host", "127.0.0.1",
        "--port", "1420",
        "--strictPort"
    ) -join " "
    $script:ownedVite = Start-Process `
        -FilePath $nodeCommand.Source `
        -ArgumentList $viteArgs `
        -WorkingDirectory (Join-Path $root "apps\desktop") `
        -WindowStyle Hidden `
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
        [int]$TimeoutSeconds = 120,
        [string]$ExpectedTargetId = "",
        [string]$ExpectedGeneration = ""
    )

    $script:actionCounter++
    $stem = "actions-{0:D4}" -f $script:actionCounter
    $actionPath = Join-Path $runDir "$stem.json"
    $stdoutPath = Join-Path $runDir "$stem.stdout.log"
    $stderrPath = Join-Path $runDir "$stem.stderr.log"
    Write-Utf8NoBom $actionPath (ConvertTo-Json -InputObject $Actions -Depth 8)

    $targetArgument = if (-not [string]::IsNullOrWhiteSpace($ExpectedTargetId)) {
        $ExpectedTargetId
    }
    elseif (-not [string]::IsNullOrWhiteSpace($script:targetId)) {
        [string]$script:targetId
    }
    else {
        "-"
    }
    $generationArgument = if (-not [string]::IsNullOrWhiteSpace($ExpectedGeneration)) {
        $ExpectedGeneration
    }
    elseif (-not [string]::IsNullOrWhiteSpace($script:targetGeneration)) {
        [string]$script:targetGeneration
    }
    else {
        "-"
    }
    $argumentLine = @(
        (Quote-NativeArgument $cdpEntry),
        (Quote-NativeArgument $actionPath),
        (Quote-NativeArgument $endpoint),
        (Quote-NativeArgument $targetArgument),
        (Quote-NativeArgument $generationArgument)
    ) -join " "
    # In managed Windows sessions Start-Process can expose a null ExitCode even
    # after a successful child exits. The old code interpreted null as failure
    # and then called a healthy desktop instance stale. Invoke Node directly so
    # PowerShell's native-command $LASTEXITCODE is authoritative. Every CDP
    # operation also has an internal timeout in capture-steps-cdp.mjs.
    $previousLocation = Get-Location
    $previousErrorAction = $ErrorActionPreference
    try {
        Set-Location -LiteralPath $root
        $ErrorActionPreference = "Continue"
        & $nodeCommand.Source `
            $cdpEntry $actionPath $endpoint $targetArgument $generationArgument `
            1> $stdoutPath 2> $stderrPath
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorAction
        Set-Location -LiteralPath $previousLocation
    }
    $stdout = if (Test-Path -LiteralPath $stdoutPath) { Read-Utf8 $stdoutPath } else { "" }
    $stderr = if (Test-Path -LiteralPath $stderrPath) { Read-Utf8 $stderrPath } else { "" }
    if ($exitCode -ne 0) {
        $detail = ($stderr.Trim() + " " + $stdout.Trim()).Trim()
        if (-not [string]::IsNullOrWhiteSpace($script:targetId) -and
            $detail -match '(?i)(capture generation changed|CDP page target with id)') {
            $script:preserveOwnedHost = $true
            throw "Capture target identity changed; ORIGAMI3 was left running for diagnosis. $detail"
        }
        throw "$Label failed (CDP exit $exitCode).$(Get-AppExitDetail) $detail"
    }

    $records = @()
    foreach ($line in @($stdout -split "`r?`n")) {
        if (-not [string]::IsNullOrWhiteSpace($line)) {
            $records += $line | ConvertFrom-Json
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($script:targetId)) {
        foreach ($record in $records) {
            $actualTargetId = if ($null -ne $record.target) {
                [string]$record.target.id
            }
            else {
                ""
            }
            if ($actualTargetId -ne [string]$script:targetId) {
                $script:preserveOwnedHost = $true
                throw "Capture target changed from $($script:targetId) to $actualTargetId; ORIGAMI3 was left running for diagnosis"
            }
        }
    }
    return @($records)
}

function Get-CaptureApiSample {
    $probe = @(
        [ordered]@{
            act = "eval"
            js = @"
(() => {
  const api = window.__origami3Capture;
  if (!api || typeof api.getStatus !== "function") {
    throw new Error("ORIGAMI3 capture API status is unavailable");
  }
  return api.getStatus();
})()
"@
        }
    )
    $records = @(
        Invoke-CdpActions `
            -Actions $probe `
            -Label "capture API status probe" `
            -TimeoutSeconds 20 `
            -ExpectedTargetId "-" `
            -ExpectedGeneration "-"
    )
    if ($records.Count -ne 1 -or $null -eq $records[0].target) {
        throw "Capture API probe returned no unambiguous target"
    }

    $target = $records[0].target
    $status = $records[0].result
    $targetId = [string]$target.id
    $generation = [string]$status.generation
    $targetUrl = [string]$target.url
    $targetTitle = [string]$target.title
    $statusUrl = [string]$status.url
    $statusTitle = [string]$status.title
    $heartbeat = 0.0
    $heartbeatOk = [double]::TryParse(
        [string]$status.heartbeat,
        [System.Globalization.NumberStyles]::Float,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [ref]$heartbeat
    )

    if ([string]::IsNullOrWhiteSpace($targetId) -or
        [string]::IsNullOrWhiteSpace($generation) -or
        $status.version -ne 1 -or
        $status.ready -ne $true -or
        -not $heartbeatOk -or
        [string]::IsNullOrWhiteSpace($statusUrl) -or
        $statusTitle -ne "ORIGAMI3" -or
        $targetUrl -ne $statusUrl -or
        $targetTitle -ne $statusTitle) {
        throw "Capture API status is not ready or does not match its CDP target"
    }

    return [pscustomobject]@{
        TargetId = $targetId
        Generation = $generation
        Heartbeat = $heartbeat
        Url = $statusUrl
        Title = $statusTitle
    }
}

function Wait-ForCaptureApi {
    param(
        [Parameter(Mandatory = $true)] [System.Diagnostics.Process]$DesktopProcess,
        [int]$TimeoutSeconds = 120
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $previous = $null
    $lastProbeError = "capture target has not appeared"
    while ([DateTime]::UtcNow -lt $deadline) {
        $DesktopProcess.Refresh()
        if ($DesktopProcess.HasExited) {
            throw "ORIGAMI3 process $($DesktopProcess.Id) exited while waiting for its capture API"
        }
        $desktopProcesses = @(Get-DesktopProcesses)
        if ($desktopProcesses.Count -gt 1) {
            throw "More than one desktop.exe process is running; all instances were left untouched"
        }
        if ($desktopProcesses.Count -eq 0 -or
            $desktopProcesses[0].Id -ne $DesktopProcess.Id) {
            throw "The observed ORIGAMI3 process changed while its capture API was starting"
        }

        try {
            $sample = Get-CaptureApiSample
            if ($null -ne $previous -and
                $sample.TargetId -eq $previous.TargetId -and
                $sample.Generation -eq $previous.Generation -and
                $sample.Heartbeat -gt $previous.Heartbeat) {
                $script:targetId = $sample.TargetId
                $script:targetGeneration = $sample.Generation
                Write-Milestone "capture API ready (PID $($DesktopProcess.Id), target $($sample.TargetId), generation $($sample.Generation))"
                return
            }
            if ($null -ne $previous -and
                ($sample.TargetId -ne $previous.TargetId -or
                    $sample.Generation -ne $previous.Generation)) {
                $lastProbeError = "capture target identity changed before it became stable"
            }
            else {
                $lastProbeError = "capture API heartbeat has not advanced yet"
            }
            $previous = $sample
        }
        catch {
            $lastProbeError = [string]$_.Exception.Message
            $previous = $null
        }
        Start-Sleep -Milliseconds 300
    }
    throw "ORIGAMI3 process $($DesktopProcess.Id) did not expose a stable capture API within $TimeoutSeconds seconds; the process was left running. Last probe: $lastProbeError"
}

function Wait-ForOwnedDesktopProcess {
    param([int]$TimeoutSeconds = 120)

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $script:ownedLauncher.Refresh()
        if ($script:ownedLauncher.HasExited) {
            throw "Tauri launcher exited during startup with code $($script:ownedLauncher.ExitCode)"
        }
        $desktopProcesses = @(Get-DesktopProcesses)
        if ($desktopProcesses.Count -gt 1) {
            throw "More than one desktop.exe process appeared during startup"
        }
        if ($desktopProcesses.Count -eq 1) {
            $candidate = $desktopProcesses[0]
            $descendsFromLauncher = Test-ProcessDescendsFrom `
                -ProcessId $candidate.Id `
                -AncestorId $script:ownedLauncher.Id
            $fallbackOwned = $false
            if ($null -eq $descendsFromLauncher) {
                $candidatePath = ""
                $candidateStart = [DateTime]::MinValue
                try { $candidatePath = [System.IO.Path]::GetFullPath($candidate.Path) } catch { }
                try { $candidateStart = $candidate.StartTime } catch { }
                $fallbackOwned =
                    $candidatePath.Equals(
                        [System.IO.Path]::GetFullPath($appExe),
                        [System.StringComparison]::OrdinalIgnoreCase
                    ) -and
                    $candidateStart -ge $script:ownedLauncher.StartTime.AddSeconds(-1)
            }
            if ($descendsFromLauncher -ne $true -and -not $fallbackOwned) {
                throw "A desktop.exe process not owned by this capture run appeared during startup; it was left untouched"
            }
            $script:ownedApp = $candidate
            Write-Milestone "started one ORIGAMI3 process (PID $($script:ownedApp.Id))"
            return $candidate
        }
        Start-Sleep -Milliseconds 250
    }
    throw "ORIGAMI3 desktop.exe did not start within $TimeoutSeconds seconds$(Get-AppExitDetail)"
}

function Start-CaptureApp {
    Start-FrontendServerIfNeeded
    if (-not (Test-Path -LiteralPath $tauriCaptureConfig -PathType Leaf)) {
        throw "Capture configuration is missing: $tauriCaptureConfig"
    }
    $cargoCommand = Get-Command cargo.exe -ErrorAction Stop
    $rootLiteral = $root.Replace("'", "''")
    $cargoLiteral = $cargoCommand.Source.Replace("'", "''")
    $tauriConfig = ConvertTo-Json -Compress -Depth 20 -InputObject (
        Get-Content -LiteralPath $tauriCaptureConfig -Raw -Encoding UTF8 |
            ConvertFrom-Json
    )
    $tauriConfigLiteral = $tauriConfig.Replace("'", "''")
    $desktopBuildCode = @"
Set-Location -LiteralPath '$rootLiteral'
`$env:TAURI_CONFIG = '$tauriConfigLiteral'
& '$cargoLiteral' build -p desktop
"@
    Invoke-HiddenNativeCommand `
        -Code $desktopBuildCode `
        -StatusName "desktop-build" `
        -Label "ORIGAMI3 desktop build" `
        -TimeoutSeconds 300
    if (-not (Test-Path -LiteralPath $appExe -PathType Leaf)) {
        throw "ORIGAMI3 desktop executable is missing after build: $appExe"
    }
    Write-Milestone "built the current ORIGAMI3 desktop"
    Write-Milestone "starting one ORIGAMI3 instance with the default WebView2 runtime"
    # Keep the profile path short and unique to this script process. Chromium
    # creates deeply nested files below it on Windows.
    $profilePath = Join-Path $captureTempRoot "wv2-$PID"
    $previousArgs = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
    $previousProfile = $env:WEBVIEW2_USER_DATA_FOLDER
    try {
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS =
            "--remote-debugging-port=9222 --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"
        $env:WEBVIEW2_USER_DATA_FOLDER = $profilePath
        # Build tools may leave inheritable handles in this PowerShell process;
        # Chromium then cannot create its restricted sandbox token. A fresh,
        # hidden helper owns exactly one desktop.exe and waits for it, keeping
        # process ownership explicit without weakening the WebView2 sandbox.
        $exeLiteral = $appExe.Replace("'", "''")
        $rootLiteral = $root.Replace("'", "''")
        $launchCode =
            "`$app = Start-Process -FilePath '$exeLiteral' -WorkingDirectory '$rootLiteral' -PassThru; `$app.WaitForExit()"
        $encodedLaunch = [Convert]::ToBase64String(
            [Text.Encoding]::Unicode.GetBytes($launchCode)
        )
        $script:ownedLauncher = Start-Process `
            -FilePath (Get-Command powershell.exe -ErrorAction Stop).Source `
            -ArgumentList "-NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand $encodedLaunch" `
            -WindowStyle Hidden `
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
    }

    $desktopProcess = Wait-ForOwnedDesktopProcess -TimeoutSeconds 120
    Wait-ForCaptureApi -DesktopProcess $desktopProcess -TimeoutSeconds 120
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
    if ($script:preserveOwnedHost) {
        Write-Milestone "owned ORIGAMI3 host left running because capture identity changed"
        return
    }
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

$restoreIdentityError = $null
try {
    $desktopProcesses = @(Get-DesktopProcesses)
    if ($desktopProcesses.Count -gt 1) {
        throw "Found $($desktopProcesses.Count) desktop.exe processes; all instances were left untouched because capture would be ambiguous"
    }
    if ($desktopProcesses.Count -eq 1) {
        $existingApp = $desktopProcesses[0]
        Write-Milestone "reusing the existing ORIGAMI3 process (PID $($existingApp.Id)); it will not be restarted"
        Wait-ForCaptureApi -DesktopProcess $existingApp -TimeoutSeconds 120
    }
    else {
        Remove-CaptureOrphanedWebViews
        Wait-ForCapturePortFree -TimeoutSeconds 10 -ConsecutiveChecks 5
        $appeared = @(Get-DesktopProcesses)
        if ($appeared.Count -ne 0) {
            throw "A desktop.exe process appeared before capture startup; it was left untouched"
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
    $captureSteps = @($steps | Sort-Object { [int]$_.number } -Descending)
    $capturedCount = 0

    foreach ($step in $captureSteps) {
        $number = [int]$step.number
        $serial = $number.ToString("D4")
        $capturedCount++
        Write-Milestone "state $capturedCount/$($steps.Count), step ${number}: $($step.name)"
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
        $orderedManifest = @($manifest | Sort-Object { [int]$_.number })
        Write-StepsJson -Entries $orderedManifest -Path $stepsJsonPath
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
    if (-not [string]::IsNullOrWhiteSpace($script:targetId) -and
        -not $script:preserveOwnedHost) {
        try {
            $restore = @([ordered]@{
                act = "eval"
                js = 'window.__origami3Capture && window.__origami3Capture.setView("normal")'
            })
            [void](Invoke-CdpActions -Actions $restore -Label "restore normal view" -TimeoutSeconds 15)
        }
        catch {
            if ($script:preserveOwnedHost) {
                $script:runSucceeded = $false
                $restoreIdentityError = $_.Exception
            }
        }
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
    if ($null -ne $restoreIdentityError) {
        throw $restoreIdentityError
    }
}
