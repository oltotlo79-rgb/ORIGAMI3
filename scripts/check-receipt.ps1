# ORIGAMI3 local check receipt helper (Windows PowerShell 5.1 compatible)
#
# A receipt is only an optimization. Any error while collecting or validating it
# must make the caller run the original check. CI and check-ci.ps1 do not call
# this file. In addition, CI-like environments are explicitly denied below.

[CmdletBinding()]
param(
    [switch]$RunRustW4,
    [string]$RepoRoot,
    [string]$GateStatusPath
)

Set-StrictMode -Version 2.0

$script:Ori3ReceiptSchemaVersion = 1
$script:Ori3ReceiptLifetimeHours = 24
$script:Ori3ReceiptDirectoryName = ".origami3\check-receipts"
$script:Ori3Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Get-Ori3Sha256HexFromBytes {
    param([byte[]]$Bytes)

    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $hash = $sha.ComputeHash($Bytes)
    }
    finally {
        $sha.Dispose()
    }
    return ([System.BitConverter]::ToString($hash)).Replace("-", "").ToLowerInvariant()
}

function Get-Ori3Sha256HexFromText {
    param([string]$Text)
    return Get-Ori3Sha256HexFromBytes $script:Ori3Utf8NoBom.GetBytes($Text)
}

function Get-Ori3FileSha256Hex {
    param([string]$LiteralPath)

    $before = Get-Item -LiteralPath $LiteralPath -Force -ErrorAction Stop
    if (($before.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "reparse pointはreceiptの対象にできません: $LiteralPath"
    }
    if ($before.PSIsContainer) {
        throw "directoryはreceiptの対象にできません: $LiteralPath"
    }

    $sha = [System.Security.Cryptography.SHA256]::Create()
    $stream = New-Object System.IO.FileStream(
        $LiteralPath,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::Read
    )
    try {
        $hash = $sha.ComputeHash($stream)
    }
    finally {
        $stream.Dispose()
        $sha.Dispose()
    }

    $after = Get-Item -LiteralPath $LiteralPath -Force -ErrorAction Stop
    if ($before.Length -ne $after.Length -or
        $before.LastWriteTimeUtc.Ticks -ne $after.LastWriteTimeUtc.Ticks) {
        throw "読み取り中にファイルが変更されました: $LiteralPath"
    }

    return ([System.BitConverter]::ToString($hash)).Replace("-", "").ToLowerInvariant()
}

function ConvertTo-Ori3CanonicalText {
    param([System.Collections.IDictionary]$Fields)

    $builder = New-Object System.Text.StringBuilder
    foreach ($key in $Fields.Keys) {
        $keyText = [string]$key
        $valueText = [string]$Fields[$key]
        $key64 = [System.Convert]::ToBase64String($script:Ori3Utf8NoBom.GetBytes($keyText))
        $value64 = [System.Convert]::ToBase64String($script:Ori3Utf8NoBom.GetBytes($valueText))
        [void]$builder.Append($key64)
        [void]$builder.Append(":")
        [void]$builder.Append($value64)
        [void]$builder.Append("`n")
    }
    return $builder.ToString()
}

function Get-Ori3CanonicalHash {
    param([System.Collections.IDictionary]$Fields)
    return Get-Ori3Sha256HexFromText (ConvertTo-Ori3CanonicalText $Fields)
}

function Resolve-Ori3RepoRoot {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "RepoRootが指定されていません"
    }
    $resolved = (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path
    return [System.IO.Path]::GetFullPath($resolved).TrimEnd([char[]]@('\', '/'))
}

function Get-Ori3ResolvedCommand {
    param([string]$Name)

    $command = Get-Command -Name $Name -ErrorAction Stop | Select-Object -First 1
    if ($command.CommandType -ne [System.Management.Automation.CommandTypes]::Application -and
        $command.CommandType -ne [System.Management.Automation.CommandTypes]::ExternalScript) {
        throw "外部commandとして解決できません: $Name ($($command.CommandType))"
    }
    if ([string]::IsNullOrWhiteSpace($command.Source)) {
        throw "command pathを解決できません: $Name"
    }
    return [System.IO.Path]::GetFullPath($command.Source)
}

function ConvertTo-Ori3ProcessArgument {
    param([string]$Argument)

    if ($Argument.Length -eq 0) {
        return '""'
    }
    if ($Argument -notmatch '[\s"]') {
        return $Argument
    }
    # Windows CreateProcess quoting: double backslashes before a quote and at
    # the end of a quoted argument.
    $escaped = [System.Text.RegularExpressions.Regex]::Replace($Argument, '(\\*)"', '$1$1\"')
    $escaped = [System.Text.RegularExpressions.Regex]::Replace($escaped, '(\\+)$', '$1$1')
    return '"' + $escaped + '"'
}

function Invoke-Ori3CapturedCommand {
    param(
        [string]$Executable,
        [string[]]$Arguments,
        [string]$WorkingDirectory
    )

    $fileName = $Executable
    $processArguments = @($Arguments)
    $extension = [System.IO.Path]::GetExtension($Executable)
    if ($extension -ieq ".ps1") {
        $powershellExecutable = Join-Path $PSHOME "powershell.exe"
        if (-not (Test-Path -LiteralPath $powershellExecutable -PathType Leaf)) {
            throw "PowerShell child processを解決できません: $powershellExecutable"
        }
        $fileName = $powershellExecutable
        $processArguments = @("-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $Executable) + $Arguments
    }
    elseif ($extension -ieq ".cmd" -or $extension -ieq ".bat") {
        throw "receipt条件取得でcmd/batは実行しません: $Executable"
    }

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $fileName
    $startInfo.Arguments = (($processArguments | ForEach-Object { ConvertTo-Ori3ProcessArgument ([string]$_) }) -join " ")
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = $script:Ori3Utf8NoBom
    $startInfo.StandardErrorEncoding = $script:Ori3Utf8NoBom

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw "commandを起動できません: $Executable"
        }
        $stdout = $process.StandardOutput.ReadToEnd()
        $stderr = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) {
            throw "commandが失敗しました ($($process.ExitCode)): $Executable $($Arguments -join ' ')`n$stderr"
        }
        $parts = @($stdout.Trim(), $stderr.Trim()) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
        return ($parts -join "`n").Trim()
    }
    finally {
        $process.Dispose()
    }
}

function Invoke-Ori3GitFileList {
    param([string]$Root)

    $gitPath = Get-Ori3ResolvedCommand "git"
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $gitPath
    $startInfo.Arguments = "-c core.quotepath=false ls-files --cached --others --exclude-standard -z"
    $startInfo.WorkingDirectory = $Root
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = $script:Ori3Utf8NoBom
    $startInfo.StandardErrorEncoding = $script:Ori3Utf8NoBom
    $startInfo.EnvironmentVariables["GIT_OPTIONAL_LOCKS"] = "0"

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw "git ls-filesを起動できません"
        }
        $stdout = $process.StandardOutput.ReadToEnd()
        $stderr = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) {
            throw "git ls-filesが失敗しました ($($process.ExitCode)): $stderr"
        }
    }
    finally {
        $process.Dispose()
    }

    $pathList = New-Object System.Collections.Generic.List[string]
    foreach ($path in $stdout.Split([char]0)) {
        if ($path.Length -gt 0) {
            $pathList.Add($path)
        }
    }
    $pathList.Sort([System.StringComparer]::Ordinal)
    return $pathList.ToArray()
}

function Get-Ori3WorktreeSnapshot {
    param([string]$Root)

    $paths = Invoke-Ori3GitFileList $Root
    $seen = New-Object "System.Collections.Generic.HashSet[string]" ([System.StringComparer]::Ordinal)
    $manifest = New-Object System.Text.StringBuilder
    $fileCount = 0
    $totalBytes = [Int64]0
    $rootPrefix = $Root + [System.IO.Path]::DirectorySeparatorChar
    $comparison = if ($env:OS -eq "Windows_NT") {
        [System.StringComparison]::OrdinalIgnoreCase
    }
    else {
        [System.StringComparison]::Ordinal
    }

    foreach ($relativeGitPath in $paths) {
        if (-not $seen.Add($relativeGitPath)) {
            throw "git ls-filesが重複pathを返しました: $relativeGitPath"
        }
        if ([System.IO.Path]::IsPathRooted($relativeGitPath)) {
            throw "git ls-filesが絶対pathを返しました: $relativeGitPath"
        }

        $relativePlatformPath = $relativeGitPath.Replace('/', [System.IO.Path]::DirectorySeparatorChar)
        $fullPath = [System.IO.Path]::GetFullPath((Join-Path $Root $relativePlatformPath))
        if (-not $fullPath.StartsWith($rootPrefix, $comparison)) {
            throw "repository外のpathはreceiptの対象にできません: $relativeGitPath"
        }

        $path64 = [System.Convert]::ToBase64String($script:Ori3Utf8NoBom.GetBytes($relativeGitPath))
        if (Test-Path -LiteralPath $fullPath -PathType Leaf) {
            $item = Get-Item -LiteralPath $fullPath -Force -ErrorAction Stop
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "reparse pointはreceiptの対象にできません: $relativeGitPath"
            }
            $fileHash = Get-Ori3FileSha256Hex $fullPath
            $fileLength = [Int64]$item.Length
            $totalBytes += $fileLength
            [void]$manifest.Append("F|")
            [void]$manifest.Append($path64)
            [void]$manifest.Append("|")
            [void]$manifest.Append($fileLength)
            [void]$manifest.Append("|")
            [void]$manifest.Append($fileHash)
            [void]$manifest.Append("`n")
        }
        elseif (Test-Path -LiteralPath $fullPath -PathType Container) {
            throw "directory/submoduleはreceiptの対象にできません: $relativeGitPath"
        }
        else {
            # Tracked deletions are part of the tested worktree state.
            [void]$manifest.Append("M|")
            [void]$manifest.Append($path64)
            [void]$manifest.Append("`n")
        }
        $fileCount++
    }

    return [pscustomobject]@{
        Sha256 = Get-Ori3Sha256HexFromText $manifest.ToString()
        FileCount = $fileCount
        TotalBytes = $totalBytes
    }
}

function Get-Ori3OptionalInputHash {
    param([string]$Root)

    $paths = New-Object System.Collections.Generic.List[string]
    $envNames = @(".env", ".env.local", ".env.test", ".env.test.local", ".env.production", ".env.production.local")
    foreach ($name in $envNames) {
        $paths.Add((Join-Path $Root $name))
        $paths.Add((Join-Path (Join-Path $Root "apps\desktop") $name))
    }
    $paths.Add((Join-Path $Root "apps\desktop\node_modules\.package-lock.json"))
    $paths.Add((Join-Path $Root ".cargo\config"))
    $paths.Add((Join-Path $Root ".cargo\config.toml"))
    $paths.Add((Join-Path $Root ".npmrc"))
    $paths.Add((Join-Path $Root "apps\desktop\.npmrc"))

    # Cargo searches .cargo/config(.toml) from cwd through every ancestor.
    $ancestor = [System.IO.Directory]::GetParent($Root)
    while ($null -ne $ancestor) {
        $paths.Add((Join-Path $ancestor.FullName ".cargo\config"))
        $paths.Add((Join-Path $ancestor.FullName ".cargo\config.toml"))
        $ancestor = $ancestor.Parent
    }

    $profileRoot = $env:USERPROFILE
    if ([string]::IsNullOrWhiteSpace($profileRoot)) {
        $profileRoot = $env:HOME
    }
    if (-not [string]::IsNullOrWhiteSpace($profileRoot)) {
        $paths.Add((Join-Path $profileRoot ".cargo\config"))
        $paths.Add((Join-Path $profileRoot ".cargo\config.toml"))
        $paths.Add((Join-Path $profileRoot ".npmrc"))
    }
    if (-not [string]::IsNullOrWhiteSpace($env:CARGO_HOME)) {
        $paths.Add((Join-Path $env:CARGO_HOME "config"))
        $paths.Add((Join-Path $env:CARGO_HOME "config.toml"))
    }
    if (-not [string]::IsNullOrWhiteSpace($env:NPM_CONFIG_USERCONFIG)) {
        $paths.Add($env:NPM_CONFIG_USERCONFIG)
    }

    $unique = New-Object "System.Collections.Generic.HashSet[string]" ([System.StringComparer]::OrdinalIgnoreCase)
    $records = New-Object System.Collections.Generic.List[string]
    foreach ($path in $paths) {
        $fullPath = [System.IO.Path]::GetFullPath($path)
        if (-not $unique.Add($fullPath)) {
            continue
        }
        $pathHash = Get-Ori3Sha256HexFromText $fullPath
        if (Test-Path -LiteralPath $fullPath -PathType Leaf) {
            $item = Get-Item -LiteralPath $fullPath -Force -ErrorAction Stop
            $records.Add("$pathHash|F|$($item.Length)|$(Get-Ori3FileSha256Hex $fullPath)")
        }
        elseif (Test-Path -LiteralPath $fullPath -PathType Container) {
            throw "条件ファイルがdirectoryです: $fullPath"
        }
        else {
            $records.Add("$pathHash|M")
        }
    }
    $recordArray = $records.ToArray()
    [System.Array]::Sort($recordArray, [System.StringComparer]::Ordinal)
    return Get-Ori3Sha256HexFromText ($recordArray -join "`n")
}

function Get-Ori3RelevantEnvironment {
    $exactNames = @(
        "PATH", "PATHEXT", "TEMP", "TMP", "TMPDIR", "USERPROFILE", "HOME",
        "CI", "GITHUB_ACTIONS", "TZ", "LANG", "CC", "CXX", "AR", "CL", "LINK",
        "LIB", "LIBPATH", "INCLUDE", "CFLAGS", "CXXFLAGS", "LDFLAGS",
        "ZERO_OWNER_VERBOSE", "SOURCE_DATE_EPOCH"
    )
    $exact = New-Object "System.Collections.Generic.HashSet[string]" ([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($name in $exactNames) {
        [void]$exact.Add($name)
    }
    $prefixes = @("RUST", "CARGO", "NODE", "NPM_CONFIG_", "VITE_", "VITEST_", "TAURI_", "ORI3_", "LC_")
    $environment = [System.Environment]::GetEnvironmentVariables()
    $selectedNames = New-Object System.Collections.Generic.List[string]
    foreach ($rawName in $environment.Keys) {
        $name = [string]$rawName
        $include = $exact.Contains($name)
        if (-not $include) {
            foreach ($prefix in $prefixes) {
                if ($name.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
                    $include = $true
                    break
                }
            }
        }
        if ($include) {
            $selectedNames.Add($name)
        }
    }
    $names = $selectedNames.ToArray()
    [System.Array]::Sort($names, [System.StringComparer]::OrdinalIgnoreCase)
    $fields = [ordered]@{}
    foreach ($name in $names) {
        $fields[$name.ToUpperInvariant()] = [string]$environment[$name]
    }
    return [pscustomobject]@{
        Sha256 = Get-Ori3CanonicalHash $fields
        Names = ($names -join ",")
    }
}

function Test-Ori3ReceiptEnvironmentAllowed {
    if (-not [string]::IsNullOrWhiteSpace($env:CI) -or
        -not [string]::IsNullOrWhiteSpace($env:GITHUB_ACTIONS)) {
        throw "CI環境ではreceiptを使用しません"
    }
    if ($env:OS -ne "Windows_NT") {
        throw "DPAPIで機械・利用者に縛るため、Windows以外ではreceiptを使用しません"
    }
}

function Get-Ori3MachineCondition {
    param([string]$Root)

    Test-Ori3ReceiptEnvironmentAllowed
    $machineGuid = (Get-ItemProperty -LiteralPath "HKLM:\SOFTWARE\Microsoft\Cryptography" -Name MachineGuid -ErrorAction Stop).MachineGuid
    $cpu = Get-ItemProperty -LiteralPath "HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0" -ErrorAction Stop
    $sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    if ([string]::IsNullOrWhiteSpace($machineGuid) -or
        [string]::IsNullOrWhiteSpace($sid) -or
        [string]::IsNullOrWhiteSpace($cpu.ProcessorNameString) -or
        [string]::IsNullOrWhiteSpace($cpu.Identifier)) {
        throw "機械/CPUの識別情報を完全に取得できません"
    }

    $osDescription = [System.Environment]::OSVersion.VersionString
    $osArchitecture = $env:PROCESSOR_ARCHITECTURE
    $processArchitecture = $env:PROCESSOR_ARCHITEW6432
    if ([string]::IsNullOrWhiteSpace($processArchitecture)) {
        $processArchitecture = $osArchitecture
    }
    try {
        $osDescription = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
        $osArchitecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
        $processArchitecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
    }
    catch {
        # Windows PowerShell 5.1 on older .NET may not expose RuntimeInformation.
    }

    $machineFields = [ordered]@{
        MachineGuid = [string]$machineGuid
        UserSid = [string]$sid
        RepoRoot = $Root
    }
    $machineHash = Get-Ori3CanonicalHash $machineFields
    return [pscustomobject]@{
        MachineHash = $machineHash
        Os = "$osDescription; os-arch=$osArchitecture; process-arch=$processArchitecture"
        Cpu = "$($cpu.ProcessorNameString); vendor=$($cpu.VendorIdentifier); id=$($cpu.Identifier); logical=$([System.Environment]::ProcessorCount)"
        RepoRoot = $Root
    }
}

function Get-Ori3RustW4Arguments {
    return @(
        "test", "--workspace", "--no-fail-fast", "--",
        "--skip", "completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten",
        "--skip", "named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten",
        "--skip", "a_safe_coincident_partial_network_appears_after_the_first_fold",
        "--skip", "the_heaviest_proposal_never_hits_the_time_limit"
    )
}

function Get-Ori3ClippyArguments {
    return @("clippy", "--workspace", "--all-targets", "--", "-D", "warnings")
}

function Get-Ori3NpmBuildArguments {
    return @("run", "build")
}

function Get-Ori3NpmLintArguments {
    return @("run", "lint")
}

function Get-Ori3NpmTestArguments {
    return @("run", "test")
}

function Get-Ori3ReceiptRecipe {
    param([ValidateSet("rust-w4", "check-all")][string]$Kind)

    $rustArgs = Get-Ori3RustW4Arguments
    $clippyArgs = Get-Ori3ClippyArguments
    $npmBuildArgs = Get-Ori3NpmBuildArguments
    $npmLintArgs = Get-Ori3NpmLintArguments
    $npmTestArgs = Get-Ori3NpmTestArguments
    if ($Kind -eq "rust-w4") {
        $text = "cwd=.|cargo|" + ($rustArgs -join "`n") + "`ntracked-mutation-guard=v1"
        $checks = @("cargo test --workspace (W4: exact 4 --skip)")
    }
    else {
        $text = @(
            "cwd=.|cargo|" + ($rustArgs -join "`n"),
            "cwd=.|cargo|" + ($clippyArgs -join "`n"),
            "cwd=apps/desktop|npm|" + ($npmBuildArgs -join "`n"),
            "cwd=apps/desktop|npm|" + ($npmLintArgs -join "`n"),
            "cwd=apps/desktop|npm|" + ($npmTestArgs -join "`n"),
            "tracked-mutation-guard=v1"
        ) -join "`n---`n"
        $checks = @(
            "(1/5) cargo test --workspace (W4: exact 4 --skip)",
            "(2/5) cargo clippy --workspace --all-targets -- -D warnings",
            "(3/5) npm run build (apps/desktop)",
            "(4/5) npm run lint (apps/desktop)",
            "(5/5) npm run test (apps/desktop)"
        )
    }
    return [pscustomobject]@{
        Sha256 = Get-Ori3Sha256HexFromText $text
        Checks = $checks
    }
}

function Get-Ori3PowerShellCondition {
    $policyRows = @(Get-ExecutionPolicy -List | Sort-Object { [string]$_.Scope } | ForEach-Object {
        "$($_.Scope)=$($_.ExecutionPolicy)"
    })
    $edition = if ($PSVersionTable.ContainsKey("PSEdition")) { [string]$PSVersionTable.PSEdition } else { "Desktop" }
    $clr = if ($PSVersionTable.ContainsKey("CLRVersion")) { [string]$PSVersionTable.CLRVersion } else { "(none)" }
    $culture = [System.Globalization.CultureInfo]::CurrentCulture
    $uiCulture = [System.Globalization.CultureInfo]::CurrentUICulture
    $timeZone = [System.TimeZoneInfo]::Local
    try {
        $timeZoneIdentity = $timeZone.ToSerializedString()
    }
    catch {
        $timeZoneIdentity = "$($timeZone.Id)|$($timeZone.BaseUtcOffset)|$($timeZone.SupportsDaylightSavingTime)"
    }
    $fields = [ordered]@{
        Version = $PSVersionTable.PSVersion.ToString()
        Edition = $edition
        Clr = $clr
        LanguageMode = [string]$ExecutionContext.SessionState.LanguageMode
        ExecutionPolicies = ($policyRows -join ";")
        Culture = "$($culture.Name)|$($culture.Calendar.GetType().FullName)"
        UiCulture = $uiCulture.Name
        TimeZone = $timeZoneIdentity
    }
    return [pscustomobject]@{
        Sha256 = Get-Ori3CanonicalHash $fields
        PowerShell = "PowerShell $($fields.Version) $edition; policy=$($policyRows -join ',')"
        Locale = "culture=$($culture.Name); ui=$($uiCulture.Name); timezone=$($timeZone.Id)"
    }
}

function Get-Ori3ReceiptConditions {
    param(
        [ValidateSet("rust-w4", "check-all")][string]$Kind,
        [string]$Root
    )

    $machine = Get-Ori3MachineCondition $Root
    $powerShell = Get-Ori3PowerShellCondition
    $environment = Get-Ori3RelevantEnvironment
    $cargoPath = Get-Ori3ResolvedCommand "cargo"
    $rustcPath = Get-Ori3ResolvedCommand "rustc"
    $cargoVersion = Invoke-Ori3CapturedCommand $cargoPath @("-Vv") $Root
    $rustcVersion = Invoke-Ori3CapturedCommand $rustcPath @("-vV") $Root
    $supplementalHash = Get-Ori3OptionalInputHash $Root

    $nodePath = ""
    $nodeVersion = ""
    $npmPath = ""
    $npmVersion = ""
    $clippyVersion = ""
    if ($Kind -eq "check-all") {
        $nodePath = Get-Ori3ResolvedCommand "node"
        $npmPath = Get-Ori3ResolvedCommand "npm"
        $nodeVersion = Invoke-Ori3CapturedCommand $nodePath @("--version") $Root
        $npmVersion = Invoke-Ori3CapturedCommand $npmPath @("--version") $Root
        $clippyVersion = Invoke-Ori3CapturedCommand $cargoPath @("clippy", "-V") $Root
    }

    $conditionFields = [ordered]@{
        Machine = $machine.MachineHash
        RepoRoot = $machine.RepoRoot
        Os = $machine.Os
        Cpu = $machine.Cpu
        PowerShell = $powerShell.Sha256
        CargoPath = $cargoPath
        CargoVersion = $cargoVersion
        RustcPath = $rustcPath
        RustcVersion = $rustcVersion
        NodePath = $nodePath
        NodeVersion = $nodeVersion
        NpmPath = $npmPath
        NpmVersion = $npmVersion
        ClippyVersion = $clippyVersion
        Environment = $environment.Sha256
        SupplementalInputs = $supplementalHash
    }
    $rustcDisplay = @($rustcVersion -split "`n" | Where-Object { $_.StartsWith("rustc ") } | Select-Object -First 1)
    $cargoDisplay = @($cargoVersion -split "`n" | Where-Object { $_.StartsWith("cargo ") } | Select-Object -First 1)
    $clippyDisplay = @($clippyVersion -split "`n" | Where-Object { $_ -match "^(clippy|cargo-clippy) " } | Select-Object -First 1)
    if ($rustcDisplay.Count -eq 0) { $rustcDisplay = @(($rustcVersion -split "`n")[0]) }
    if ($cargoDisplay.Count -eq 0) { $cargoDisplay = @(($cargoVersion -split "`n")[0]) }
    if ($clippyVersion -and $clippyDisplay.Count -eq 0) { $clippyDisplay = @(($clippyVersion -split "`n")[0]) }

    return [pscustomobject]@{
        Sha256 = Get-Ori3CanonicalHash $conditionFields
        SafeDisplay = [ordered]@{
            Os = $machine.Os
            Cpu = $machine.Cpu
            Machine = $machine.MachineHash.Substring(0, 12)
            PowerShell = $powerShell.PowerShell
            Locale = $powerShell.Locale
            Rustc = $rustcDisplay[0]
            Cargo = $cargoDisplay[0]
            Node = if ($nodeVersion) { $nodeVersion } else { "(not used)" }
            Npm = if ($npmVersion) { $npmVersion } else { "(not used)" }
            Clippy = if ($clippyVersion) { $clippyDisplay[0] } else { "(not used)" }
            EnvironmentNames = $environment.Names
        }
    }
}

function New-Ori3ReceiptContext {
    param(
        [ValidateSet("rust-w4", "check-all")][string]$Kind,
        [string]$Root,
        $ContentSnapshot
    )

    $resolvedRoot = Resolve-Ori3RepoRoot $Root
    if ($null -eq $ContentSnapshot) {
        $ContentSnapshot = Get-Ori3WorktreeSnapshot $resolvedRoot
    }
    $recipe = Get-Ori3ReceiptRecipe $Kind
    $conditions = Get-Ori3ReceiptConditions $Kind $resolvedRoot
    $eligibilityFields = [ordered]@{
        Schema = [string]$script:Ori3ReceiptSchemaVersion
        CheckId = $Kind
        Content = $ContentSnapshot.Sha256
        Recipe = $recipe.Sha256
        Conditions = $conditions.Sha256
    }
    return [pscustomobject]@{
        Kind = $Kind
        Root = $resolvedRoot
        Content = $ContentSnapshot
        Recipe = $recipe
        Conditions = $conditions
        EligibilitySha256 = Get-Ori3CanonicalHash $eligibilityFields
    }
}

function Get-Ori3ReceiptStorePath {
    param([string]$Root)
    return Join-Path $Root $script:Ori3ReceiptDirectoryName
}

function Get-Ori3ReceiptPath {
    param($Context)
    return Join-Path (Get-Ori3ReceiptStorePath $Context.Root) ($Context.Kind + ".json")
}

function Set-Ori3GateStatus {
    param(
        [string]$Root,
        [string]$Path,
        [ValidateSet("helper-ready", "cargo-started")][string]$Status
    )
    if ([string]::IsNullOrWhiteSpace($Path)) {
        return
    }
    $store = [System.IO.Path]::GetFullPath((Get-Ori3ReceiptStorePath $Root)).TrimEnd([char[]]@('\', '/'))
    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $prefix = $store + [System.IO.Path]::DirectorySeparatorChar
    if (-not $fullPath.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "gate status pathがreceipt directory外です"
    }
    [System.IO.Directory]::CreateDirectory($store) | Out-Null
    [System.IO.File]::WriteAllText($fullPath, $Status + "`n", $script:Ori3Utf8NoBom)
}

function Get-Ori3SigningEntropy {
    return $script:Ori3Utf8NoBom.GetBytes("ORIGAMI3/check-receipt/dpapi/v1")
}

function Get-Ori3SigningKey {
    param(
        [string]$Root,
        [switch]$Create
    )

    Test-Ori3ReceiptEnvironmentAllowed
    Add-Type -AssemblyName System.Security -ErrorAction Stop
    $store = Get-Ori3ReceiptStorePath $Root
    $keyPath = Join-Path $store "local-signing-key.dpapi"
    if (-not (Test-Path -LiteralPath $keyPath -PathType Leaf)) {
        if (-not $Create) {
            throw "この機械・利用者の署名鍵がありません"
        }
        [System.IO.Directory]::CreateDirectory($store) | Out-Null
        $key = New-Object byte[] 32
        $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
        try {
            $rng.GetBytes($key)
        }
        finally {
            $rng.Dispose()
        }
        $protected = [System.Security.Cryptography.ProtectedData]::Protect(
            $key,
            (Get-Ori3SigningEntropy),
            [System.Security.Cryptography.DataProtectionScope]::CurrentUser
        )
        $temporary = "$keyPath.$PID.$([Guid]::NewGuid().ToString('N')).tmp"
        $createdKey = $false
        try {
            [System.IO.File]::WriteAllBytes($temporary, $protected)
            if (Test-Path -LiteralPath $keyPath -PathType Leaf) {
                Remove-Item -LiteralPath $temporary -Force
            }
            else {
                try {
                    # File.Move fails atomically when another process created the
                    # destination after the check above.
                    [System.IO.File]::Move($temporary, $keyPath)
                    $createdKey = $true
                }
                catch [System.IO.IOException] {
                    if (-not (Test-Path -LiteralPath $keyPath -PathType Leaf)) {
                        throw
                    }
                    Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
                }
            }
        }
        finally {
            if (Test-Path -LiteralPath $temporary -PathType Leaf) {
                Remove-Item -LiteralPath $temporary -Force
            }
        }
        if ($createdKey) {
            return $key
        }
        # Another process won the key-creation race. Use the stored key rather
        # than signing with the losing process's unstored random bytes.
        return Get-Ori3SigningKey $Root
    }

    $item = Get-Item -LiteralPath $keyPath -Force -ErrorAction Stop
    if ($item.Length -le 0 -or $item.Length -gt 4096) {
        throw "署名鍵ファイルの大きさが不正です"
    }
    $protectedBytes = [System.IO.File]::ReadAllBytes($keyPath)
    $keyBytes = [System.Security.Cryptography.ProtectedData]::Unprotect(
        $protectedBytes,
        (Get-Ori3SigningEntropy),
        [System.Security.Cryptography.DataProtectionScope]::CurrentUser
    )
    if ($keyBytes.Length -ne 32) {
        throw "署名鍵の形式が不正です"
    }
    return $keyBytes
}

function Get-Ori3ReceiptDisplayHash {
    param($Receipt)
    $fields = [ordered]@{
        Os = [string]$Receipt.conditions.os
        Cpu = [string]$Receipt.conditions.cpu
        Machine = [string]$Receipt.conditions.machine
        PowerShell = [string]$Receipt.conditions.powerShell
        Locale = [string]$Receipt.conditions.locale
        Rustc = [string]$Receipt.conditions.rustc
        Cargo = [string]$Receipt.conditions.cargo
        Node = [string]$Receipt.conditions.node
        Npm = [string]$Receipt.conditions.npm
        Clippy = [string]$Receipt.conditions.clippy
        EnvironmentNames = [string]$Receipt.conditions.environmentNames
    }
    return Get-Ori3CanonicalHash $fields
}

function Get-Ori3ReceiptChecksHash {
    param($Checks)
    $fields = [ordered]@{}
    $index = 0
    foreach ($check in @($Checks)) {
        $fields[[string]$index] = [string]$check
        $index++
    }
    return Get-Ori3CanonicalHash $fields
}

function Get-Ori3ReceiptSignature {
    param(
        $Receipt,
        [byte[]]$Key
    )
    $fields = [ordered]@{
        SchemaVersion = [string]$Receipt.schemaVersion
        CheckId = [string]$Receipt.checkId
        Result = [string]$Receipt.result
        PassedAtUtc = [string]$Receipt.passedAtUtc
        ExpiresAtUtc = [string]$Receipt.expiresAtUtc
        ContentSha256 = [string]$Receipt.contentSha256
        ContentFileCount = [string]$Receipt.contentFileCount
        RecipeSha256 = [string]$Receipt.recipeSha256
        ConditionsSha256 = [string]$Receipt.conditionsSha256
        EligibilitySha256 = [string]$Receipt.eligibilitySha256
        ChecksSha256 = Get-Ori3ReceiptChecksHash $Receipt.checks
        DisplaySha256 = Get-Ori3ReceiptDisplayHash $Receipt
        HeadAtPass = [string]$Receipt.headAtPass
        ReusedComponentCheckId = [string]$Receipt.reusedComponentCheckId
        ReusedComponentPassedAtUtc = [string]$Receipt.reusedComponentPassedAtUtc
        ReusedComponentExpiresAtUtc = [string]$Receipt.reusedComponentExpiresAtUtc
    }
    $hmac = New-Object System.Security.Cryptography.HMACSHA256(,$Key)
    try {
        $hash = $hmac.ComputeHash($script:Ori3Utf8NoBom.GetBytes((ConvertTo-Ori3CanonicalText $fields)))
    }
    finally {
        $hmac.Dispose()
    }
    return ([System.BitConverter]::ToString($hash)).Replace("-", "").ToLowerInvariant()
}

function Test-Ori3FixedTimeHexEqual {
    param([string]$Left, [string]$Right)
    if ($null -eq $Left -or $null -eq $Right -or $Left.Length -ne $Right.Length) {
        return $false
    }
    $difference = 0
    for ($index = 0; $index -lt $Left.Length; $index++) {
        $difference = $difference -bor ([int][char]$Left[$index] -bxor [int][char]$Right[$index])
    }
    return $difference -eq 0
}

function Read-Ori3ReceiptJson {
    param([string]$Path)
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if ($item.Length -le 0 -or $item.Length -gt 65536) {
        throw "receipt JSONの大きさが不正です"
    }
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $text = $script:Ori3Utf8NoBom.GetString($bytes)
    return $text | ConvertFrom-Json -ErrorAction Stop
}

function Find-Ori3CheckReceipt {
    param($Context)

    $path = Get-Ori3ReceiptPath $Context
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        return [pscustomobject]@{ IsHit = $false; Reason = "記録がありません"; Receipt = $null; Path = $path }
    }
    try {
        $receipt = Read-Ori3ReceiptJson $path
        if ([int]$receipt.schemaVersion -ne $script:Ori3ReceiptSchemaVersion) { throw "schemaが違います" }
        if ([string]$receipt.checkId -ne $Context.Kind) { throw "check IDが違います" }
        if ([string]$receipt.result -ne "passed") { throw "合格記録ではありません" }
        if ([string]$receipt.contentSha256 -ne $Context.Content.Sha256 -or
            [int]$receipt.contentFileCount -ne [int]$Context.Content.FileCount) { throw "作業内容が違います" }
        if ([string]$receipt.recipeSha256 -ne $Context.Recipe.Sha256) { throw "検査コマンド/意味が違います" }
        if ((Get-Ori3ReceiptChecksHash $receipt.checks) -ne (Get-Ori3ReceiptChecksHash $Context.Recipe.Checks)) {
            throw "表示用の検査一覧が違います"
        }
        if ([string]$receipt.conditionsSha256 -ne $Context.Conditions.Sha256) { throw "実行条件が違います" }
        if ([string]$receipt.eligibilitySha256 -ne $Context.EligibilitySha256) { throw "総合fingerprintが違います" }

        $passedAt = [DateTime]::Parse(
            [string]$receipt.passedAtUtc,
            [System.Globalization.CultureInfo]::InvariantCulture,
            [System.Globalization.DateTimeStyles]::RoundtripKind
        ).ToUniversalTime()
        $expiresAt = [DateTime]::Parse(
            [string]$receipt.expiresAtUtc,
            [System.Globalization.CultureInfo]::InvariantCulture,
            [System.Globalization.DateTimeStyles]::RoundtripKind
        ).ToUniversalTime()
        $now = [DateTime]::UtcNow
        if ($passedAt -gt $now.AddMinutes(5)) { throw "合格時刻が未来です" }
        if ($expiresAt -gt $passedAt.AddHours($script:Ori3ReceiptLifetimeHours).AddSeconds(1)) {
            throw "期限が24時間を超えています"
        }
        if ($expiresAt -le $passedAt) { throw "期限が合格時刻以前です" }
        if ($expiresAt -le $now) { throw "24時間の期限が切れています" }

        if (-not [string]::IsNullOrWhiteSpace([string]$receipt.reusedComponentCheckId)) {
            if ([string]$receipt.reusedComponentCheckId -ne "rust-w4") { throw "再利用component IDが不正です" }
            $componentPassedAt = [DateTime]::Parse([string]$receipt.reusedComponentPassedAtUtc).ToUniversalTime()
            $componentExpiresAt = [DateTime]::Parse([string]$receipt.reusedComponentExpiresAtUtc).ToUniversalTime()
            if ($componentPassedAt -gt $passedAt.AddMinutes(5)) { throw "component合格時刻が不正です" }
            if ($expiresAt -gt $componentExpiresAt) { throw "複合receiptがcomponentより長く有効です" }
        }

        $key = Get-Ori3SigningKey $Context.Root
        $expectedSignature = Get-Ori3ReceiptSignature $receipt $key
        if (-not (Test-Ori3FixedTimeHexEqual ([string]$receipt.signatureHmacSha256) $expectedSignature)) {
            throw "この機械・利用者の署名を検証できません"
        }
        return [pscustomobject]@{ IsHit = $true; Reason = ""; Receipt = $receipt; Path = $path }
    }
    catch {
        return [pscustomobject]@{ IsHit = $false; Reason = $_.Exception.Message; Receipt = $null; Path = $path }
    }
}

function Invoke-Ori3AtomicJsonWrite {
    param(
        [string]$Path,
        $Value
    )
    $directory = Split-Path -Parent $Path
    [System.IO.Directory]::CreateDirectory($directory) | Out-Null
    $temporary = "$Path.$PID.$([Guid]::NewGuid().ToString('N')).tmp"
    try {
        $json = $Value | ConvertTo-Json -Depth 8
        [System.IO.File]::WriteAllText($temporary, $json, $script:Ori3Utf8NoBom)
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            $backup = "$temporary.backup"
            try {
                [System.IO.File]::Replace($temporary, $Path, $backup, $true)
            }
            finally {
                if (Test-Path -LiteralPath $backup -PathType Leaf) {
                    Remove-Item -LiteralPath $backup -Force
                }
            }
        }
        else {
            Move-Item -LiteralPath $temporary -Destination $Path
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporary -PathType Leaf) {
            Remove-Item -LiteralPath $temporary -Force
        }
    }
}

function Get-Ori3HeadForDisplay {
    param([string]$Root)
    try {
        $gitPath = Get-Ori3ResolvedCommand "git"
        return Invoke-Ori3CapturedCommand $gitPath @("rev-parse", "HEAD") $Root
    }
    catch {
        return "(unavailable; display only)"
    }
}

function Write-Ori3CheckReceipt {
    param(
        $ExpectedContext,
        [Nullable[DateTime]]$MaximumExpiryUtc,
        $ReusedComponentReceipt
    )

    $current = New-Ori3ReceiptContext $ExpectedContext.Kind $ExpectedContext.Root $null
    if ($current.EligibilitySha256 -ne $ExpectedContext.EligibilitySha256) {
        throw "検査中に作業内容または実行条件が変わったため記録しません"
    }

    $now = [DateTime]::UtcNow
    $expiresAt = $now.AddHours($script:Ori3ReceiptLifetimeHours)
    if ($null -ne $MaximumExpiryUtc) {
        $boundedExpiry = $MaximumExpiryUtc.Value.ToUniversalTime()
        if ($boundedExpiry -lt $expiresAt) {
            $expiresAt = $boundedExpiry
        }
    }
    if ($expiresAt -le $now) {
        throw "再利用した構成要素の期限が切れたため複合receiptを記録しません"
    }

    $reusedComponentCheckId = ""
    $reusedComponentPassedAtUtc = ""
    $reusedComponentExpiresAtUtc = ""
    if ($null -ne $ReusedComponentReceipt) {
        if ([string]$ReusedComponentReceipt.checkId -ne "rust-w4" -or
            [string]$ReusedComponentReceipt.result -ne "passed") {
            throw "複合receiptの再利用componentが不正です"
        }
        $reusedComponentCheckId = [string]$ReusedComponentReceipt.checkId
        $reusedComponentPassedAtUtc = [string]$ReusedComponentReceipt.passedAtUtc
        $reusedComponentExpiresAtUtc = [string]$ReusedComponentReceipt.expiresAtUtc
    }

    $receipt = [ordered]@{
        schemaVersion = $script:Ori3ReceiptSchemaVersion
        checkId = $ExpectedContext.Kind
        result = "passed"
        passedAtUtc = $now.ToString("o", [System.Globalization.CultureInfo]::InvariantCulture)
        expiresAtUtc = $expiresAt.ToString("o", [System.Globalization.CultureInfo]::InvariantCulture)
        contentSha256 = $ExpectedContext.Content.Sha256
        contentFileCount = $ExpectedContext.Content.FileCount
        recipeSha256 = $ExpectedContext.Recipe.Sha256
        conditionsSha256 = $ExpectedContext.Conditions.Sha256
        eligibilitySha256 = $ExpectedContext.EligibilitySha256
        checks = $ExpectedContext.Recipe.Checks
        conditions = [ordered]@{
            os = $ExpectedContext.Conditions.SafeDisplay.Os
            cpu = $ExpectedContext.Conditions.SafeDisplay.Cpu
            machine = $ExpectedContext.Conditions.SafeDisplay.Machine
            powerShell = $ExpectedContext.Conditions.SafeDisplay.PowerShell
            locale = $ExpectedContext.Conditions.SafeDisplay.Locale
            rustc = $ExpectedContext.Conditions.SafeDisplay.Rustc
            cargo = $ExpectedContext.Conditions.SafeDisplay.Cargo
            node = $ExpectedContext.Conditions.SafeDisplay.Node
            npm = $ExpectedContext.Conditions.SafeDisplay.Npm
            clippy = $ExpectedContext.Conditions.SafeDisplay.Clippy
            environmentNames = $ExpectedContext.Conditions.SafeDisplay.EnvironmentNames
        }
        headAtPass = Get-Ori3HeadForDisplay $ExpectedContext.Root
        reusedComponentCheckId = $reusedComponentCheckId
        reusedComponentPassedAtUtc = $reusedComponentPassedAtUtc
        reusedComponentExpiresAtUtc = $reusedComponentExpiresAtUtc
        signatureHmacSha256 = ""
    }
    $key = Get-Ori3SigningKey $ExpectedContext.Root -Create
    $receipt.signatureHmacSha256 = Get-Ori3ReceiptSignature ([pscustomobject]$receipt) $key
    Invoke-Ori3AtomicJsonWrite (Get-Ori3ReceiptPath $ExpectedContext) $receipt
    return Get-Ori3ReceiptPath $ExpectedContext
}

function Write-Ori3ReceiptReuseMessage {
    param(
        [string]$What,
        $Hit
    )
    $receipt = $Hit.Receipt
    $passedAt = [DateTime]::Parse([string]$receipt.passedAtUtc).ToLocalTime()
    $expiresAt = [DateTime]::Parse([string]$receipt.expiresAtUtc).ToLocalTime()
    Write-Host ""
    Write-Host "[REUSE] $What は完全に同じ合格receiptがあるため再実行しません" -ForegroundColor Green
    foreach ($check in $receipt.checks) {
        Write-Host "  - $check"
    }
    Write-Host "  合格時刻: $($passedAt.ToString('yyyy-MM-dd HH:mm:ss zzz'))"
    Write-Host "  有効期限: $($expiresAt.ToString('yyyy-MM-dd HH:mm:ss zzz')) (24時間以内)"
    Write-Host "  内容SHA-256: $(([string]$receipt.contentSha256).Substring(0, 16))... ($($receipt.contentFileCount) files)"
    Write-Host "  OS: $($receipt.conditions.os)"
    Write-Host "  CPU: $($receipt.conditions.cpu)"
    Write-Host "  PowerShell: $($receipt.conditions.powerShell)"
    Write-Host "  locale/timezone: $($receipt.conditions.locale)"
    Write-Host "  Rust: $($receipt.conditions.rustc) / $($receipt.conditions.cargo)"
    if ([string]$receipt.checkId -eq "check-all") {
        Write-Host "  Node: $($receipt.conditions.node) / npm $($receipt.conditions.npm) / $($receipt.conditions.clippy)"
    }
    Write-Host "  機械・利用者ID: $($receipt.conditions.machine) (DPAPI署名検証済み)"
    Write-Host "  参考HEAD: $($receipt.headAtPass) (SHAは判定に使用しない)"
    if (-not [string]::IsNullOrWhiteSpace([string]$receipt.reusedComponentCheckId)) {
        $componentPassedAt = [DateTime]::Parse([string]$receipt.reusedComponentPassedAtUtc).ToLocalTime()
        Write-Host "  構成要素: $($receipt.reusedComponentCheckId) は $($componentPassedAt.ToString('yyyy-MM-dd HH:mm:ss zzz')) の合格receiptを再利用"
    }
    Write-Host "  receipt: $($Hit.Path)"
}

function Write-Ori3ReceiptMissMessage {
    param([string]$What, $Hit)
    Write-Host "[RUN] $What receiptは再利用できません: $($Hit.Reason)" -ForegroundColor Yellow
}

function Get-Ori3TrackedStatus {
    param([string]$Root)
    $gitPath = Get-Ori3ResolvedCommand "git"
    return Invoke-Ori3CapturedCommand $gitPath @("status", "--porcelain", "--untracked-files=no") $Root
}

function Invoke-Ori3RustW4Gate {
    param([string]$Root, [string]$StatusPath)

    $resolvedRoot = Resolve-Ori3RepoRoot $Root
    $expectedContext = $null
    $trackedGuardAvailable = $true
    try {
        $beforeTracked = Get-Ori3TrackedStatus $resolvedRoot
    }
    catch {
        # This guard did not exist in the old pre-commit hook. A guard setup
        # failure must disable receipt reuse and run the old W4, not skip it.
        Write-Host "[WARN] 追跡対象の変更guardを開始できないため、receiptを使わずRust W4を実行します: $($_.Exception.Message)" -ForegroundColor Yellow
        $trackedGuardAvailable = $false
        $beforeTracked = ""
    }

    if ($trackedGuardAvailable) {
      try {
        $expectedContext = New-Ori3ReceiptContext "rust-w4" $resolvedRoot $null
        $hit = Find-Ori3CheckReceipt $expectedContext
        if ($hit.IsHit) {
            $confirmation = New-Ori3ReceiptContext "rust-w4" $resolvedRoot $null
            $afterProbeTracked = Get-Ori3TrackedStatus $resolvedRoot
            if ($confirmation.EligibilitySha256 -eq $expectedContext.EligibilitySha256 -and
                $afterProbeTracked -eq $beforeTracked) {
                Write-Ori3ReceiptReuseMessage "Rust W4" $hit
                return 0
            }
            Write-Host "[RUN] receipt確認中に作業内容/条件が変わったためRust W4を実行します" -ForegroundColor Yellow
            $expectedContext = $confirmation
            $beforeTracked = $afterProbeTracked
        }
        else {
            Write-Ori3ReceiptMissMessage "Rust W4" $hit
        }
      }
      catch {
        Write-Host "[WARN] receiptを判定できないためRust W4を実行します: $($_.Exception.Message)" -ForegroundColor Yellow
        $expectedContext = $null
      }
    }

    $cargoPath = $null
    try {
        $cargoPath = Get-Ori3ResolvedCommand "cargo"
    }
    catch {
        Write-Host "[NG] cargoが見つからないためcommitを中止します" -ForegroundColor Red
        return 127
    }
    Write-Host "[..] Rustの変更を含むため、作業ツリー全体のテストを実行します (W4 exact 4 --skip)"
    Set-Ori3GateStatus $resolvedRoot $StatusPath "cargo-started"
    $global:LASTEXITCODE = 0
    $pushedLocation = $false
    try {
        Push-Location $resolvedRoot
        $pushedLocation = $true
        $rustW4Arguments = Get-Ori3RustW4Arguments
        & $cargoPath @rustW4Arguments
        $status = $LASTEXITCODE
    }
    catch {
        Write-Host "[NG] cargo test --workspaceを起動できません: $($_.Exception.Message)" -ForegroundColor Red
        return 1
    }
    finally {
        if ($pushedLocation) {
            Pop-Location
        }
    }
    if ($status -ne 0) {
        Write-Host "[NG] cargo test --workspaceが失敗したためcommitを中止します (終了コード: $status)" -ForegroundColor Red
        return $status
    }

    if ($trackedGuardAvailable) {
        try {
            $afterTracked = Get-Ori3TrackedStatus $resolvedRoot
        }
        catch {
            Write-Host "[NG] 検査後の追跡対象変更guardを確認できません: $($_.Exception.Message)" -ForegroundColor Red
            return 1
        }
        if ($afterTracked -ne $beforeTracked) {
            Write-Host "[NG] Rust W4が追跡対象のファイルを書き換えたためcommitを中止します" -ForegroundColor Red
            Write-Host "実行前:"
            Write-Host $beforeTracked
            Write-Host "実行後:"
            Write-Host $afterTracked
            return 1
        }
    }

    if ($null -ne $expectedContext) {
        try {
            $path = Write-Ori3CheckReceipt $expectedContext
            Write-Host "[receipt] Rust W4の合格を記録しました: $path" -ForegroundColor DarkGreen
        }
        catch {
            Write-Host "[WARN] 検査は合格しましたがreceiptは記録しません: $($_.Exception.Message)" -ForegroundColor Yellow
        }
    }
    Write-Host "[OK] 作業ツリー全体のRustテストに合格しました" -ForegroundColor Green
    return 0
}

# Dot-sourcing exposes the functions to check.ps1. Direct execution is only the
# pre-commit W4 gate, so a future argv change cannot diverge between the hook,
# the receipt recipe, and check.ps1.
if ($MyInvocation.InvocationName -ne ".") {
    if (-not $RunRustW4) {
        Write-Host "[NG] -RunRustW4を指定してください" -ForegroundColor Red
        exit 64
    }
    try {
        $resolvedCliRoot = Resolve-Ori3RepoRoot $RepoRoot
        Set-Ori3GateStatus $resolvedCliRoot $GateStatusPath "helper-ready"
        $exitCode = Invoke-Ori3RustW4Gate $resolvedCliRoot $GateStatusPath
        exit $exitCode
    }
    catch {
        Write-Host "[NG] Rust W4 gateが予期しないエラーで停止しました: $($_.Exception.Message)" -ForegroundColor Red
        exit 125
    }
}
