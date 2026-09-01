#Requires -Version 7
<#
.SYNOPSIS
    Proves an aborted setup-wizard proof leaves the operator's configuration
    unchanged.

.DESCRIPTION
    `Test-SetupWizard.ps1` drives the real installed application against the real
    profile, because it cannot do otherwise: Tauri resolves its data directory
    through `SHGetKnownFolderPath(FOLDERID_RoamingAppData)`, which ignores the
    `APPDATA` environment variable. It captures `config\` and restores it in its
    `finally`.

    A restore in a `finally` is only worth having if it runs on the failure path,
    and nothing exercises that path by accident. This forces a real mid-run
    failure with `-AbortAfterProfileWrite` -- which throws after the app has
    consumed the seeds, so the configuration really has been rewritten by then --
    and asserts every captured file is byte-identical afterwards.

    It enumerates `config\` itself, before and after, rather than trusting the
    assertion inside the script under test: a restore that reported success
    against its own capture would still pass that one. `logs\speakeasy.log` is
    excluded by design and named as such, because the run appends diagnostic
    evidence to it.

    This installs and uninstalls SpeakEasy Mini for real, and refuses to start if
    an installation is already present.

.PARAMETER ArtifactRoot
    Where `Build-LocalInstaller.ps1` put `SpeakEasyMiniSetup.exe`.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ArtifactRoot
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$dataRoot = Join-Path $env:APPDATA 'ai.speakeasy.mini'
$configRoot = Join-Path $dataRoot 'config'
$seedFiles = @(
    'install-hotkey.txt'
    'install-logging.txt'
    'install-retention.txt'
    'install-vocabulary.txt'
    'install-provider.txt'
)

# Every file in `config\`, plus the seeds whether present or not, so a seed the
# run creates is proved absent again rather than silently tolerated.
function Get-ConfigState {
    param([Parameter(Mandatory)][string]$ConfigRoot, [Parameter(Mandatory)][string[]]$AlsoTrack)
    $names = [Collections.Generic.List[string]]::new()
    if (Test-Path -LiteralPath $ConfigRoot -PathType Container) {
        foreach ($file in Get-ChildItem -LiteralPath $ConfigRoot -File -Force) { $names.Add($file.Name) }
    }
    foreach ($name in $AlsoTrack) { if (-not $names.Contains($name)) { $names.Add($name) } }
    $state = [ordered]@{}
    foreach ($name in ($names | Sort-Object)) {
        $path = Join-Path $ConfigRoot $name
        $state[$name] = if (Test-Path -LiteralPath $path -PathType Leaf) {
            (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
        } else {
            '(absent)'
        }
    }
    $state
}

function Assert-ConfigUnchanged {
    param([Parameter(Mandatory)]$Before, [Parameter(Mandatory)]$After, [Parameter(Mandatory)][string]$Stage)
    $differences = @()
    foreach ($name in $Before.Keys) {
        # Not `$after`: PowerShell variable names are case-insensitive, so that
        # would overwrite the `$After` parameter on the first iteration and every
        # later lookup would read "(absent)" off a string.
        $afterValue = if ($After.Contains($name)) { $After[$name] } else { '(absent)' }
        if ($Before[$name] -ne $afterValue) {
            $differences += "$name was $($Before[$name]) and is $afterValue"
        }
    }
    foreach ($name in $After.Keys) {
        if (-not $Before.Contains($name)) {
            $differences += "$name appeared ($($After[$name]))"
        }
    }
    if ($differences.Count -gt 0) {
        throw "${Stage}: the operator's configuration changed -- " + ($differences -join '; ')
    }
    Write-Host "  ${Stage}: all $($Before.Count) captured config entries byte-identical"
}

$wizard = Join-Path $PSScriptRoot 'Test-SetupWizard.ps1'

Push-Location $repositoryRoot
try {
    $running = @(Get-Process -Name 'ai-speakeasy-mini' -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0) {
        throw 'SpeakEasy Mini is running; close it and rerun.'
    }
    $stamp = (Get-ItemProperty -Path 'HKCU:\Software\SpeakEasy Mini\LocalDevelopment' -Name Version -ErrorAction SilentlyContinue).Version
    if ($stamp) {
        throw "SpeakEasy Mini $stamp is installed; uninstall it and rerun."
    }

    $before = Get-ConfigState -ConfigRoot $configRoot -AlsoTrack $seedFiles
    Write-Host 'operator config before the aborted run:'
    foreach ($name in $before.Keys) { Write-Host "  $name = $($before[$name])" }
    $logPath = Join-Path $dataRoot 'logs\speakeasy.log'
    $logBefore = if (Test-Path -LiteralPath $logPath) { (Get-Item -LiteralPath $logPath).Length } else { 0 }
    Write-Host "diagnostic log before: $logBefore bytes (appended to on purpose; not asserted)"

    Write-Host ''
    Write-Host 'running the wizard proof with a deliberate failure after the config is rewritten'
    $aborted = $false
    $abortMessage = $null
    try {
        & $wizard -ArtifactRoot $ArtifactRoot -AbortAfterProfileWrite
    } catch {
        $aborted = $true
        $abortMessage = $_.Exception.Message
    }
    if (-not $aborted) {
        throw 'The wizard proof completed; -AbortAfterProfileWrite did not abort it, so this control proved nothing.'
    }
    Write-Host ''
    Write-Host "the run aborted as intended: $abortMessage"
    if ($abortMessage -notlike '*AbortAfterProfileWrite*') {
        # It failed for some other reason, so the configuration may never have
        # been rewritten and a byte-identical result below would be vacuous.
        throw "The run failed before reaching the deliberate abort, so this control is inconclusive: $abortMessage"
    }

    $after = Get-ConfigState -ConfigRoot $configRoot -AlsoTrack $seedFiles
    Write-Host 'operator config after the aborted run:'
    foreach ($name in $after.Keys) { Write-Host "  $name = $($after[$name])" }
    Assert-ConfigUnchanged -Before $before -After $after -Stage 'aborted run'

    $logAfter = if (Test-Path -LiteralPath $logPath) { (Get-Item -LiteralPath $logPath).Length } else { 0 }
    Write-Host "diagnostic log after: $logAfter bytes (grew by $($logAfter - $logBefore))"
    if ($logAfter -lt $logBefore) {
        throw "The diagnostic log shrank from $logBefore to $logAfter bytes; the run must append to it, never truncate it."
    }

    # The abort fires after the point a passing run would have uninstalled, so the
    # `finally` had to both remove the installation and restore the configuration.
    if (Test-Path -LiteralPath (Join-Path $env:LOCALAPPDATA 'SpeakEasy Mini')) {
        throw 'The aborted run left the installation behind.'
    }
    $leftoverStamp = (Get-ItemProperty -Path 'HKCU:\Software\SpeakEasy Mini\LocalDevelopment' -Name Version -ErrorAction SilentlyContinue).Version
    if ($leftoverStamp) {
        throw "The aborted run left the version stamp behind: $leftoverStamp"
    }
    Write-Host '  aborted run: installation and version stamp both cleaned up'

    Write-Host ''
    Write-Host 'config restore on abort: passed'
}
finally {
    Pop-Location
}
