[CmdletBinding()]
param(
    [int[]]$ProcessId = @(),
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'

function Get-AppPath {
    param([Parameter(Mandatory)][string]$Executable)

    $registryPaths = @(
        "HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\$Executable",
        "HKLM:\Software\Microsoft\Windows\CurrentVersion\App Paths\$Executable",
        "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\$Executable"
    )
    foreach ($registryPath in $registryPaths) {
        if (Test-Path -LiteralPath $registryPath) {
            $candidate = (Get-ItemProperty -LiteralPath $registryPath).'(default)'
            if ($candidate -and (Test-Path -LiteralPath $candidate)) {
                return (Resolve-Path -LiteralPath $candidate).Path
            }
        }
    }
    $command = Get-Command $Executable -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command -and $command.Source -and (Test-Path -LiteralPath $command.Source)) {
        return (Resolve-Path -LiteralPath $command.Source).Path
    }
    return $null
}

$targets = @(
    [pscustomobject]@{ name = 'Notepad'; executable = 'notepad.exe' },
    [pscustomobject]@{ name = 'VS Code'; executable = 'Code.exe' },
    [pscustomobject]@{ name = 'Word'; executable = 'WINWORD.EXE' },
    [pscustomobject]@{ name = 'Chrome'; executable = 'chrome.exe' },
    [pscustomobject]@{ name = 'Excel'; executable = 'EXCEL.EXE' },
    [pscustomobject]@{ name = 'Windows Terminal'; executable = 'wt.exe' }
)

$applications = foreach ($target in $targets) {
    $path = Get-AppPath -Executable $target.executable
    $version = if ($path) { (Get-Item -LiteralPath $path).VersionInfo.ProductVersion } else { $null }
    [ordered]@{
        name = $target.name
        executable = $target.executable
        discovery = if ($path) { 'found' } else { 'not-found' }
        path = $path
        product_version = $version
        launched_by_probe = $false
        content_read = $false
        input_injected = $false
        commit_on_finish = 'untested-interactive'
        reidentification = 'untested-interactive'
        selection_and_caret = 'untested-interactive'
        uipi_and_password = 'untested-interactive'
        user_input_invalidation = 'untested-interactive'
        clipboard_race = 'untested-interactive'
        ambiguous_send_input = 'untested-interactive'
    }
}

$processes = foreach ($id in $ProcessId) {
    $process = Get-Process -Id $id -ErrorAction SilentlyContinue
    if (-not $process) {
        [ordered]@{ process_id = $id; identity_status = 'not-found' }
        continue
    }
    $path = try { $process.Path } catch { $null }
    $startTimeUtc = try { $process.StartTime.ToUniversalTime().ToString('o') } catch { $null }
    [ordered]@{
        process_id = $process.Id
        identity_status = if ($path -and $startTimeUtc) { 'snapshot-complete' } else { 'snapshot-incomplete-refuse' }
        process_name = $process.ProcessName
        executable_path = $path
        start_time_utc = $startTimeUtc
        main_window_handle = $process.MainWindowHandle.ToInt64()
        has_exited = $process.HasExited
        mutation_authorized = $false
    }
}

$record = [ordered]@{
    schema_version = 1
    probe = 'phase-0b-read-only-target-discovery'
    timestamp_utc = (Get-Date).ToUniversalTime().ToString('o')
    machine = $env:COMPUTERNAME
    applications = @($applications)
    process_identity_snapshots = @($processes)
    safety = [ordered]@{
        mutation_performed = $false
        application_launched = $false
        window_activated = $false
        clipboard_accessed = $false
        keyboard_or_mouse_input_injected = $false
        document_content_read = $false
    }
    qualification = 'none-read-only-discovery-is-not-delivery-evidence'
    remaining_manual_gate = 'Use disposable documents in a resettable standard-user lane to run commit, invalidation, clipboard-race, password, UIPI, and ambiguous-input cases.'
}

$json = $record | ConvertTo-Json -Depth 8
if ($OutputPath) {
    $parent = Split-Path -Parent $OutputPath
    if ($parent -and -not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent | Out-Null
    }
    Set-Content -LiteralPath $OutputPath -Value $json -Encoding UTF8
}
$json