#Requires -Version 7
<#
.SYNOPSIS
    Proves a `Test-SetupWizard.ps1` pre-flight refusal changes nothing -- and that
    this control changes nothing either.

.DESCRIPTION
    The wizard proof refuses to start on any remnant it did not create: the app or
    its setup program running, a registered installation, either SpeakEasy Mini
    registry key present with no installation behind it, and anything at the
    install root -- a populated directory, an empty one, or a file. Five of those
    are driven here. Every refusal throws from inside the `try`, so the
    `finally` runs -- and that block stops processes, uninstalls and restores
    configuration. Unguarded, a refusal triggered *by* an existing installation
    would uninstall it, and one triggered by a running app would stop it and roll
    back writes it had made.

    Ownership is therefore claimed at the last check before the first mutation --
    a successful configuration capture -- and every cleanup step is guarded by it.

    **This control's own lifetime is proved too, not just the refusals.** Driving
    them needs an installation, a running process and, for the third case, a
    directory where one would go; creating any of those changes the machine, and a
    silent `--install` also rewrites `config\` because `seed::` resolves the
    profile from the environment rather than from the install root. So the
    complete baseline -- configuration bytes, installed files, the version stamp,
    the Add/Remove Programs entry -- is captured **before the control creates
    anything**, and the outermost `finally` restores only what the control made
    and asserts the operator's configuration is byte-identical.

    The postcondition is independent of the restore it checks.
    `ProfileCapture.ps1`'s `Restore-ConfigCapture` writes bytes; the local
    `Get-ConfigState` re-reads presence and SHA-256 from disk. A restore that
    quietly did nothing cannot satisfy the second by satisfying the first. Neither
    of them deletes a filename it does not recognise: an unknown file is left
    alone and reported.

    **The registry is compared whole**, not by presence. `Get-RegistryKeyState`
    reads every value name, type and datum from the version-stamp key and the
    Add/Remove Programs key -- those two and no others -- so a key that existed
    before has to be identical afterwards, and one this control caused to exist
    has to be gone.

    An installation that was already there is left in place and asserted
    unchanged; one this script created is removed, along with its stamp and its
    Add/Remove Programs entry. The planted directory of CASE 3 is removed file by
    file after its marker is verified, never by a recursive delete of the install
    root: "this control believes it created this" is not a claim on whatever else
    arrived while the run was in progress.

.PARAMETER ArtifactRoot
    Where `Build-LocalInstaller.ps1` put the installer and its `payload\`.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ArtifactRoot
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$artifactFull = if ([IO.Path]::IsPathRooted($ArtifactRoot)) {
    [IO.Path]::GetFullPath($ArtifactRoot)
} else {
    [IO.Path]::GetFullPath((Join-Path $repositoryRoot $ArtifactRoot))
}
$setup = Join-Path $artifactFull 'SpeakEasyMiniSetup.exe'
if (-not (Test-Path -LiteralPath $setup -PathType Leaf)) {
    throw "The single-file installer was not found: $setup"
}
# Before the first live-profile path is derived, and so before anything is
# captured, hashed, planted or installed. This control's whole postcondition is
# that the operator's own machine came back unchanged; run where `%APPDATA%` is
# redirected, it would assert that about a container and prove nothing.
. (Join-Path $PSScriptRoot 'HostProfilePathIdentity.ps1')
Assert-HostProfilePathIdentity -Context 'Test-PreflightRefusalIsInert.ps1'

$installRoot = Join-Path $env:LOCALAPPDATA 'SpeakEasy Mini'
$installedApp = Join-Path $installRoot 'ai-speakeasy-mini.exe'
$dataRoot = Join-Path $env:APPDATA 'ai.speakeasy.mini'
$configRoot = Join-Path $dataRoot 'config'
$stampKey = 'HKCU:\Software\SpeakEasy Mini\LocalDevelopment'
$arpKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ai.speakeasy.mini'
# The same two keys as paths under `HKEY_CURRENT_USER`, for the .NET registry API,
# which is the only way to read a value's *type*. These two and no others: this
# control has no business enumerating the operator's registry.
$stampSubKey = 'Software\SpeakEasy Mini\LocalDevelopment'
$arpSubKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\ai.speakeasy.mini'
$wizard = Join-Path $PSScriptRoot 'Test-SetupWizard.ps1'
# The restore half is shared with the wizard proof and proved without a profile by
# `Test-ProfileCaptureIsScoped.ps1`. The *assertion* half below stays local and
# independent, so a defect cannot satisfy both.
. (Join-Path $PSScriptRoot 'ProfileCapture.ps1')
$trackedFiles = @(Get-TrackedConfigFileNames)

# Presence and SHA-256 only. This is the *assertion* half and is deliberately not
# the thing that restores: it re-reads from disk, so it cannot be satisfied by a
# restore that recorded success without writing.
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

# `Get-ConfigCapture` and `Restore-ConfigCapture` come from `ProfileCapture.ps1`.
# This control used to carry its own pair, and the copy deleted any filename it
# did not recognise -- the same defect the shared one no longer has.

function Assert-Unchanged {
    param([Parameter(Mandatory)]$Before, [Parameter(Mandatory)]$After, [Parameter(Mandatory)][string]$Stage)
    $differences = @()
    foreach ($name in $Before.Keys) {
        # Not `$after`: PowerShell variable names are case-insensitive, so that
        # would overwrite the `$After` parameter on the first iteration and every
        # later lookup would read "(absent)" off a string.
        $afterValue = if ($After.Contains($name)) { $After[$name] } else { '(absent)' }
        if ($Before[$name] -ne $afterValue) { $differences += "$name was $($Before[$name]) and is $afterValue" }
    }
    foreach ($name in $After.Keys) {
        if (-not $Before.Contains($name)) { $differences += "$name appeared" }
    }
    if ($differences.Count -gt 0) {
        throw "${Stage}: configuration changed -- " + ($differences -join '; ')
    }
    Write-Host "  ${Stage}: all $($Before.Count) captured config entries unchanged"
}

function Get-DirectoryState {
    param([Parameter(Mandatory)][string]$Root)
    if (-not (Test-Path -LiteralPath $Root -PathType Container)) { return $null }
    @(Get-ChildItem -LiteralPath $Root -Recurse -File -Force |
        ForEach-Object { '{0}={1}' -f $_.FullName.Substring($Root.Length + 1), (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash } |
        Sort-Object)
}

# **Presence, every value name, every value type and every value datum**, as one
# comparable string. "The key still exists" is not the assertion: a refusal that
# rewrote `DisplayVersion`, or changed a `DWord` into a `String`, would satisfy it
# while having altered the operator's installation record.
function Get-RegistryKeyState {
    param([Parameter(Mandatory)][string]$SubKey)
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($SubKey)
    if ($null -eq $key) { return '(absent)' }
    try {
        $rendered = foreach ($name in @($key.GetValueNames() | Sort-Object)) {
            $kind = $key.GetValueKind($name)
            # Unexpanded, so a value holding `%LOCALAPPDATA%` is compared as the
            # bytes stored rather than as whatever this process resolves it to.
            $data = $key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $text = if ($null -eq $data) { '(null)' }
                elseif ($data -is [byte[]]) { [BitConverter]::ToString($data) }
                elseif ($data -is [string[]]) { $data -join ' ;; ' }
                else { [string]$data }
            '{0} [{1}] = {2}' -f $name, $kind, $text
        }
        'present: ' + (@($rendered) -join ' ;; ')
    } finally {
        $key.Dispose()
    }
}

function Assert-RegistryUnchanged {
    param(
        [Parameter(Mandatory)][string]$Stage,
        [Parameter(Mandatory)][string]$Label,
        [Parameter(Mandatory)][AllowEmptyString()][string]$Before,
        [Parameter(Mandatory)][AllowEmptyString()][string]$After
    )
    if ($Before -ne $After) {
        throw "${Stage}: the $Label key is now <$After> and was <$Before>."
    }
}

# --- Baseline, before this control creates, starts or writes anything ---------
$controlOwnsChanges = $false
$baselineConfigState = $null
$baselineConfigBytes = $null
$baselineInstallState = $null
$baselineStampKey = $null
$baselineArpKey = $null
$createdInstall = $false
$createdDecoy = $false
$createdEmptyDecoy = $false
$createdOrphanKey = $false
# The exact state CASE 5 creates, and the only state its cleanup will delete.
$orphanState = $null

function Remove-PlantedOrphanKey {
    # Refuses unless the key is still exactly what this control made: valueless.
    # Anything else means something wrote to it while the run was in progress, and
    # deleting it would take that with it.
    if (-not (Test-Path -LiteralPath $script:arpKey)) { return 'it is already gone' }
    $now = Get-RegistryKeyState -SubKey $script:arpSubKey
    if ($now -ne $script:orphanState) {
        throw ("$script:arpKey is now <$now> and this control created <$script:orphanState>; " +
            'leaving it in place.')
    }
    Remove-Item -LiteralPath $script:arpKey -Force
    'removed'
}
$startedApp = $null
$bodySucceeded = $false
$decoyState = $null
$caseThreeRan = $false

# **What the planted directory is made of, exactly.** Removal is keyed on this
# list and on a marker file this control writes, not on a `$createdDecoy` flag: a
# recursive delete of the install root because "this control believes it created
# it" removes whatever else arrived while the run was in progress, and the whole
# point of the case is that an unregistered directory belongs to nobody provable.
$decoyMarkerName = '.speakeasy-preflight-control-marker'
$decoyMarkerValue = [Guid]::NewGuid().ToString('N')
$decoyDirectories = @($installRoot, (Join-Path $installRoot 'proof'))
$decoyFiles = @(
    (Join-Path $installRoot $decoyMarkerName)
    (Join-Path $installRoot 'ai-speakeasy-mini.exe')
    (Join-Path $installRoot 'proof\granite-worker.exe')
)

function Remove-PlantedDirectory {
    # Refuses unless this control's own marker is still there holding this run's
    # GUID, then removes exactly the files it created and the directories it
    # created, deepest first and non-recursively. A directory holding anything
    # else refuses to go and names what is in it.
    $markerPath = Join-Path $installRoot $decoyMarkerName
    if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
        throw "There is no control marker at $markerPath, so nothing here is provably this control's; removing nothing."
    }
    $found = (Get-Content -LiteralPath $markerPath -Raw).Trim()
    if ($found -ne $decoyMarkerValue) {
        throw "The marker at $markerPath reads '$found', not this run's '$decoyMarkerValue'; removing nothing."
    }
    foreach ($file in $decoyFiles) {
        if (Test-Path -LiteralPath $file -PathType Leaf) { Remove-Item -LiteralPath $file -Force }
    }
    $deepestFirst = [array]::CreateInstance([string], $decoyDirectories.Count)
    [array]::Copy($decoyDirectories, $deepestFirst, $decoyDirectories.Count)
    [array]::Reverse($deepestFirst)
    foreach ($directory in $deepestFirst) {
        if (-not (Test-Path -LiteralPath $directory -PathType Container)) { continue }
        $left = @(Get-ChildItem -LiteralPath $directory -Force)
        if ($left.Count -gt 0) {
            throw ("$directory still holds $($left.Count) item(s) this control did not create, so it " +
                'stays: ' + ((@($left | ForEach-Object { $_.Name })) -join ', '))
        }
        [IO.Directory]::Delete($directory, $false)
    }
}

Push-Location $repositoryRoot
try {
    if (@(Get-Process -Name 'ai-speakeasy-mini' -ErrorAction SilentlyContinue).Count -gt 0) {
        throw 'SpeakEasy Mini is running; close it and rerun so this control owns what it starts.'
    }
    if (@(Get-Process -Name 'SpeakEasyMiniSetup', 'speakeasy-bootstrapper' -ErrorAction SilentlyContinue).Count -gt 0) {
        throw 'A SpeakEasy Mini setup process is running; close it and rerun so this control owns what it starts.'
    }

    $baselineConfigState = Get-ConfigState -ConfigRoot $configRoot -AlsoTrack $trackedFiles
    $baselineConfigBytes = Get-ConfigCapture -ConfigRoot $configRoot -AlsoTrack $trackedFiles
    $baselineInstallState = Get-DirectoryState -Root $installRoot
    $baselineStampKey = Get-RegistryKeyState -SubKey $stampSubKey
    $baselineArpKey = Get-RegistryKeyState -SubKey $arpSubKey
    $controlOwnsChanges = $true
    Write-Host 'baseline captured before this control changed anything:'
    Write-Host "  config entries : $($baselineConfigState.Count)"
    Write-Host ("  install root   : " +
        $(if ($null -eq $baselineInstallState) { 'absent' } else { "$($baselineInstallState.Count) file(s)" }))
    Write-Host "  version stamp  : $baselineStampKey"
    Write-Host "  ARP entry      : $baselineArpKey"

    # --- Case 3 first: an unregistered install directory ----------------------
    # It has to run before anything is installed, because the registered-install
    # refusal comes first and would mask it. It is therefore only reachable when
    # the machine starts clean -- which is the state every other proof here
    # requires anyway.
    Write-Host ''
    # The Add/Remove Programs key counts too. Left out of this test, CASE 5 would
    # plant a key over one that was already there -- and then delete the
    # operator's on the way out.
    if ($baselineStampKey -ne '(absent)' -or $baselineArpKey -ne '(absent)' -or
        $null -ne $baselineInstallState) {
        Write-Host ('CASES 3-5 NOT RUN: a remnant was already present (stamp key, Add/Remove ' +
            'Programs key or install root), and an earlier refusal fires first. Rerun on a machine ' +
            'with no SpeakEasy Mini installed to cover the three remnant cases.') -ForegroundColor Yellow
    } else {
        Write-Host 'CASE 3: an unregistered install directory is in the way'
        foreach ($directory in $decoyDirectories) {
            New-Item -ItemType Directory -Path $directory -Force | Out-Null
        }
        $createdDecoy = $true
        Set-Content -LiteralPath (Join-Path $installRoot $decoyMarkerName) -Value $decoyMarkerValue -Encoding ascii
        Set-Content -LiteralPath (Join-Path $installRoot 'ai-speakeasy-mini.exe') `
            -Value 'not an executable; a stand-in for the wreckage of an interrupted install' -Encoding ascii
        Set-Content -LiteralPath (Join-Path $installRoot 'proof\granite-worker.exe') `
            -Value 'likewise' -Encoding ascii
        $decoyState = Get-DirectoryState -Root $installRoot
        Write-Host "  planted $($decoyState.Count) file(s) with no version stamp"
        $threw = $false
        $message = $null
        try { & $wizard -ArtifactRoot $artifactFull } catch { $threw = $true; $message = $_.Exception.Message }
        if (-not $threw) {
            throw 'CASE 3: the wizard proof did not refuse over an unregistered install directory.'
        }
        if ($message -notlike '*nothing is registered as installed*') {
            throw "CASE 3: refused for an unrelated reason, so this proves nothing: $message"
        }
        Write-Host "  refused: $message"
        $decoyAfter = Get-DirectoryState -Root $installRoot
        if ($null -eq $decoyAfter) {
            throw 'CASE 3: the refusal deleted the install directory it refused to take over.'
        }
        if (($decoyState -join '|') -ne ($decoyAfter -join '|')) {
            throw 'CASE 3: the refusal altered the files in the install directory it refused to take over.'
        }
        Write-Host "  the $($decoyAfter.Count) planted file(s) are byte-identical and still there"
        Assert-Unchanged -Before $baselineConfigState `
            -After (Get-ConfigState -ConfigRoot $configRoot -AlsoTrack $trackedFiles) -Stage 'CASE 3'
        Assert-RegistryUnchanged -Stage 'CASE 3' -Label 'version stamp' `
            -Before $baselineStampKey -After (Get-RegistryKeyState -SubKey $stampSubKey)
        Assert-RegistryUnchanged -Stage 'CASE 3' -Label 'Add/Remove Programs' `
            -Before $baselineArpKey -After (Get-RegistryKeyState -SubKey $arpSubKey)
        # Removed here rather than in the `finally`, because the cases below need
        # the directory clear. The `finally` still covers a throw before this line.
        Remove-PlantedDirectory
        $createdDecoy = $false
        Write-Host '  removed exactly the files and directories this control planted'
        $caseThreeRan = $true

        # --- Case 4: the unregistered directory is empty ----------------------
        # Emptiness is the interesting half. A directory with files in it is
        # obviously a remnant; an empty one is what a half-finished uninstall
        # leaves, and a pre-flight that enumerated before refusing would wave it
        # through and then delete it in the `finally`.
        Write-Host ''
        Write-Host 'CASE 4: the unregistered install directory is empty'
        New-Item -ItemType Directory -Path $installRoot -Force | Out-Null
        $createdEmptyDecoy = $true
        $threw = $false
        $message = $null
        try { & $wizard -ArtifactRoot $artifactFull } catch { $threw = $true; $message = $_.Exception.Message }
        if (-not $threw) {
            throw 'CASE 4: the wizard proof did not refuse over an empty unregistered install directory.'
        }
        if ($message -notlike '*nothing is registered as installed*' -or $message -notlike '*a directory*') {
            throw "CASE 4: refused for an unrelated reason, so this proves nothing: $message"
        }
        Write-Host "  refused: $message"
        if (-not (Test-Path -LiteralPath $installRoot -PathType Container)) {
            throw 'CASE 4: the refusal removed the empty directory it refused to take over.'
        }
        if (@(Get-ChildItem -LiteralPath $installRoot -Force).Count -ne 0) {
            throw 'CASE 4: the refusal put something into the directory it refused to take over.'
        }
        Write-Host '  the empty directory is still there and still empty'
        Assert-Unchanged -Before $baselineConfigState `
            -After (Get-ConfigState -ConfigRoot $configRoot -AlsoTrack $trackedFiles) -Stage 'CASE 4'
        [IO.Directory]::Delete($installRoot, $false)
        $createdEmptyDecoy = $false
        Write-Host '  removed the empty directory this control created'

        # --- Case 5: a registry key with no installation behind it ------------
        # The Add/Remove Programs key rather than the version stamp, because it
        # has a parent that certainly already exists -- so this control creates
        # exactly one key and removes exactly one key. A stamp key with no
        # `Version` value is the same remnant by the same rule.
        Write-Host ''
        Write-Host 'CASE 5: a registry key with no installation behind it'
        # Absence proved here and now, not inferred from the baseline: the cases
        # above ran in between, and this control deletes the key afterwards.
        if (Test-Path -LiteralPath $arpKey) {
            throw ("CASE 5: $arpKey already exists, so this control cannot plant it and will not " +
                'delete it. Its state is ' + (Get-RegistryKeyState -SubKey $arpSubKey) + '.')
        }
        New-Item -Path $arpKey -Force | Out-Null
        if (-not (Test-Path -LiteralPath $arpKey)) {
            throw "CASE 5: creating $arpKey reported success and the key is not there."
        }
        # Only now. A flag set before the creation succeeded is a `finally` that
        # deletes a key this control never made.
        $createdOrphanKey = $true
        $orphanState = Get-RegistryKeyState -SubKey $arpSubKey
        Write-Host "  planted $arpKey -- $orphanState"
        $threw = $false
        $message = $null
        try { & $wizard -ArtifactRoot $artifactFull } catch { $threw = $true; $message = $_.Exception.Message }
        if (-not $threw) {
            throw 'CASE 5: the wizard proof did not refuse over an orphan registry key.'
        }
        if ($message -notlike '*no installation registered behind it*') {
            throw "CASE 5: refused for an unrelated reason, so this proves nothing: $message"
        }
        Write-Host "  refused: $message"
        Assert-RegistryUnchanged -Stage 'CASE 5' -Label 'Add/Remove Programs' `
            -Before $orphanState -After (Get-RegistryKeyState -SubKey $arpSubKey)
        Assert-RegistryUnchanged -Stage 'CASE 5' -Label 'version stamp' `
            -Before $baselineStampKey -After (Get-RegistryKeyState -SubKey $stampSubKey)
        Assert-Unchanged -Before $baselineConfigState `
            -After (Get-ConfigState -ConfigRoot $configRoot -AlsoTrack $trackedFiles) -Stage 'CASE 5'
        Write-Host "  cleanup: $(Remove-PlantedOrphanKey)"
        $createdOrphanKey = $false
    }

    # --- Set up the remaining two cases ---------------------------------------
    Write-Host ''
    $stamp = (Get-ItemProperty -Path $stampKey -Name Version -ErrorAction SilentlyContinue).Version
    if (-not $stamp) {
        Write-Host 'no installation present; installing one silently for this control'
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try { $output = & $setup --install --install-root $installRoot 2>&1 } finally { $ErrorActionPreference = $previous }
        if ($LASTEXITCODE -ne 0) {
            throw "Silent install failed with exit code ${LASTEXITCODE}: $($output -join ' ')"
        }
        $createdInstall = $true
        $stamp = (Get-ItemProperty -Path $stampKey -Name Version -ErrorAction SilentlyContinue).Version
    }
    if (-not $stamp) { throw 'No version stamp after installing; the refusal cases cannot be set up.' }
    Write-Host "installation present, version $stamp at $installRoot"

    # What the refusals must leave alone. Separate from the baseline: the baseline
    # is what the *control* must put back, this is what a *refusal* must not touch.
    $installedFiles = Get-DirectoryState -Root $installRoot
    $refusalStampKey = Get-RegistryKeyState -SubKey $stampSubKey
    $refusalArpKey = Get-RegistryKeyState -SubKey $arpSubKey
    Write-Host "captured $($installedFiles.Count) installed file(s) and both registry keys for the refusal cases"

    function Assert-InstallationIntact {
        param([Parameter(Mandatory)][string]$Stage)
        if (-not (Test-Path -LiteralPath $installRoot -PathType Container)) {
            throw "${Stage}: the install directory is gone -- the refusal uninstalled it."
        }
        $now = Get-DirectoryState -Root $installRoot
        if (($installedFiles -join '|') -ne ($now -join '|')) {
            throw "${Stage}: the installed files changed."
        }
        # Both keys, whole. A refusal that rewrote one Add/Remove Programs value
        # passes "the stamp still reads 1.8.1" and has still edited the operator's
        # installation record.
        Assert-RegistryUnchanged -Stage $Stage -Label 'version stamp' `
            -Before $refusalStampKey -After (Get-RegistryKeyState -SubKey $stampSubKey)
        Assert-RegistryUnchanged -Stage $Stage -Label 'Add/Remove Programs' `
            -Before $refusalArpKey -After (Get-RegistryKeyState -SubKey $arpSubKey)
        Write-Host ("  ${Stage}: install directory, $($now.Count) file(s) and both registry keys " +
            'unchanged value for value')
    }

    # --- Case 1: an installation already exists -------------------------------
    Write-Host ''
    Write-Host 'CASE 1: an installation already exists'
    # Captured immediately before the refusal, not once at the top: the silent
    # install above writes the seeds and clears the provider record, so a baseline
    # taken earlier would make this case fail for the control's own setup.
    $beforeCase1 = Get-ConfigState -ConfigRoot $configRoot -AlsoTrack $trackedFiles
    $threw = $false
    $message = $null
    try { & $wizard -ArtifactRoot $artifactFull } catch { $threw = $true; $message = $_.Exception.Message }
    if (-not $threw) { throw 'CASE 1: the wizard proof did not refuse over an existing installation.' }
    if ($message -notlike '*already installed*') {
        throw "CASE 1: refused for an unrelated reason, so this proves nothing: $message"
    }
    Write-Host "  refused: $message"
    Assert-InstallationIntact -Stage 'CASE 1'
    Assert-Unchanged -Before $beforeCase1 `
        -After (Get-ConfigState -ConfigRoot $configRoot -AlsoTrack $trackedFiles) -Stage 'CASE 1'

    # --- Case 2: the app is already running -----------------------------------
    Write-Host ''
    Write-Host 'CASE 2: the app is already running'
    $startedApp = Start-Process -FilePath $installedApp -PassThru
    Start-Sleep -Seconds 6
    if ($startedApp.HasExited) {
        throw "CASE 2: the app exited before the refusal could be driven (code $($startedApp.ExitCode))."
    }
    Write-Host "  started the installed app, pid $($startedApp.Id)"
    # Let the launch settle before the baseline is taken. On a control-created
    # installation this first start consumes the seeds and rewrites the
    # dictionary, and that is the app's doing rather than the refusal's -- a
    # baseline taken before it would blame the refusal for the launch.
    $settleDeadline = [DateTime]::UtcNow.AddSeconds(60)
    while ([DateTime]::UtcNow -lt $settleDeadline) {
        $pending = @('install-hotkey.txt', 'install-logging.txt', 'install-retention.txt', 'install-vocabulary.txt' |
            Where-Object { Test-Path -LiteralPath (Join-Path $configRoot $_) })
        if ($pending.Count -eq 0) { break }
        Start-Sleep -Seconds 1
    }
    Start-Sleep -Seconds 5
    $beforeCase2 = Get-ConfigState -ConfigRoot $configRoot -AlsoTrack $trackedFiles
    $threw = $false
    $message = $null
    try { & $wizard -ArtifactRoot $artifactFull } catch { $threw = $true; $message = $_.Exception.Message }
    if (-not $threw) { throw 'CASE 2: the wizard proof did not refuse over a running app.' }
    # The running-app check comes first, so this is the message it must give. If it
    # reported the stamp instead, the order changed and the running app was never
    # the reason.
    if ($message -notlike '*is running*') {
        throw "CASE 2: refused for an unrelated reason, so this proves nothing: $message"
    }
    Write-Host "  refused: $message"
    $stillRunning = @(Get-Process -Id $startedApp.Id -ErrorAction SilentlyContinue)
    if ($stillRunning.Count -eq 0) {
        throw 'CASE 2: the refusal stopped the app it had just refused to disturb.'
    }
    Write-Host "  the app is still running, pid $($startedApp.Id)"
    Assert-InstallationIntact -Stage 'CASE 2'
    Assert-Unchanged -Before $beforeCase2 `
        -After (Get-ConfigState -ConfigRoot $configRoot -AlsoTrack $trackedFiles) -Stage 'CASE 2'

    Write-Host ''
    Write-Host ("pre-flight refusal is inert: passed" +
        $(if ($caseThreeRan) { ' (five cases)' } else { ' (two cases; CASES 3-5 not run, see above)' }))
    $bodySucceeded = $true
}
finally {
    Pop-Location
    if (-not $controlOwnsChanges) {
        Write-Host 'nothing was captured, so nothing was changed and nothing is being restored'
    } else {
        $failures = [Collections.Generic.List[string]]::new()
        try {
            # Only what this control started or created, in the order that makes
            # the next step possible.
            if ($null -ne $startedApp) {
                try {
                    if (@(Get-Process -Id $startedApp.Id -ErrorAction SilentlyContinue).Count -gt 0) {
                        Stop-Process -Id $startedApp.Id -Force
                        Start-Sleep -Seconds 2
                        Write-Host "stopped the app this control started (pid $($startedApp.Id))"
                    }
                } catch { $failures.Add("stopping the app this control started: $($_.Exception.Message)") }
            }
            if ($createdDecoy) {
                try {
                    Remove-PlantedDirectory
                    Write-Host 'removed exactly the files and directories this control planted'
                } catch { $failures.Add("removing the planted install directory: $($_.Exception.Message)") }
            }
            # Non-recursively, so a directory that gained contents refuses rather
            # than taking them with it.
            if ($createdEmptyDecoy) {
                try {
                    if (Test-Path -LiteralPath $installRoot -PathType Container) {
                        [IO.Directory]::Delete($installRoot, $false)
                        Write-Host 'removed the empty directory this control created'
                    }
                } catch { $failures.Add("removing the empty planted directory: $($_.Exception.Message)") }
            }
            if ($createdOrphanKey) {
                try {
                    Write-Host "orphan registry key: $(Remove-PlantedOrphanKey)"
                } catch { $failures.Add("removing the planted registry key: $($_.Exception.Message)") }
            }
            if ($createdInstall) {
                try {
                    $uninstaller = Join-Path $installRoot 'speakeasy-bootstrapper.exe'
                    if (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
                        throw "the uninstaller is not at $uninstaller"
                    }
                    & $uninstaller --uninstall /S --keep-user-data | Out-Null
                    if ($LASTEXITCODE -ne 0) { throw "the uninstaller exited $LASTEXITCODE rather than 0" }
                    Write-Host 'removed the installation this control created'
                } catch { $failures.Add("removing the installation this control created: $($_.Exception.Message)") }
            }
        } finally {
            # Protected by its own `finally` for the same reason the wizard proof's
            # restore is: a cleanup step that throws must not take the operator's
            # configuration down with it.
            try {
                Restore-ConfigCapture -ConfigRoot $configRoot -Capture $baselineConfigBytes
            } catch { $failures.Add("restoring the operator's configuration: $($_.Exception.Message)") }
        }

        # Postconditions, read back from disk and from the registry rather than
        # inferred from the restore having returned.
        try {
            Assert-Unchanged -Before $baselineConfigState `
                -After (Get-ConfigState -ConfigRoot $configRoot -AlsoTrack $trackedFiles) -Stage 'after the control'
        } catch { $failures.Add($_.Exception.Message) }

        $installAfter = Get-DirectoryState -Root $installRoot
        if ($null -eq $baselineInstallState) {
            if ($null -ne $installAfter) {
                $failures.Add("the install root exists and did not before this control: $($installAfter.Count) file(s)")
            } else {
                Write-Host 'install root: absent, as it was before this control'
            }
        } else {
            if ($null -eq $installAfter) {
                $failures.Add('the pre-existing installation is gone.')
            } elseif (($baselineInstallState -join '|') -ne ($installAfter -join '|')) {
                $failures.Add('the pre-existing installation changed.')
            } else {
                Write-Host "install root: the pre-existing $($installAfter.Count) file(s) are byte-identical"
            }
        }

        # Whole-key comparison, both directions. A key that existed before must be
        # identical -- every value name, type and datum -- and a key this control
        # caused to exist must be gone, which `(absent)` on both sides expresses
        # without a second branch.
        $stampAfter = Get-RegistryKeyState -SubKey $stampSubKey
        $arpAfter = Get-RegistryKeyState -SubKey $arpSubKey
        if ($stampAfter -ne $baselineStampKey) {
            $failures.Add("the version stamp key is now <$stampAfter> and was <$baselineStampKey>.")
        }
        if ($arpAfter -ne $baselineArpKey) {
            $failures.Add("the Add/Remove Programs key is now <$arpAfter> and was <$baselineArpKey>.")
        }
        if ($stampAfter -eq $baselineStampKey -and $arpAfter -eq $baselineArpKey) {
            Write-Host 'registry: version stamp and Add/Remove Programs keys both identical, value for value'
        }

        if ($failures.Count -gt 0) {
            $summary = 'This control did not leave the machine as it found it: ' + ($failures -join '; ')
            # Only thrown when nothing else is propagating. A throw from a
            # `finally` replaces the exception already on its way out, and the
            # failure that ended the run is the more informative one.
            if ($bodySucceeded) { throw $summary }
            Write-Warning $summary
        }
    }
}
