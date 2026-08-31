[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ArtifactRoot,
    [string]$InstallRoot,
    [switch]$KeepInstall
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$targetRoot = [IO.Path]::GetFullPath((Join-Path $repositoryRoot 'target'))
$artifactRoot = [IO.Path]::GetFullPath($ArtifactRoot)
if (-not $artifactRoot.StartsWith($targetRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'ArtifactRoot must remain under the workspace target directory.'
}
if (-not $InstallRoot) {
    $InstallRoot = Join-Path $targetRoot "installer-lifecycle\$PID"
}
$installRoot = [IO.Path]::GetFullPath($InstallRoot)
$lifecycleRoot = Join-Path $targetRoot 'installer-lifecycle'
if (-not $installRoot.StartsWith($lifecycleRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'InstallRoot must remain under target\installer-lifecycle.'
}
if (Test-Path -LiteralPath $installRoot) {
    throw "InstallRoot already exists; use a fresh ephemeral path: $installRoot"
}

# There is no `*-setup.exe` any more. `apps/bootstrapper` installs SpeakEasy:
# it places files, writes the HKCU version stamp and its refusals, registers in
# Add/Remove Programs, creates the Start Menu shortcut and uninstalls. The
# artifact root holds the bootstrapper and the `payload/` directory it installs.
$installer = Join-Path $artifactRoot 'speakeasy-bootstrapper.exe'
if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
    throw "The bootstrapper was not found in $artifactRoot."
}
if (-not (Test-Path -LiteralPath (Join-Path $artifactRoot 'payload') -PathType Container)) {
    throw "No payload directory in $artifactRoot; the bootstrapper would have nothing to install."
}
$productVersion = & (Join-Path $PSScriptRoot 'Get-ProductVersion.ps1')

function Invoke-Installer {
    # `--install` rather than a wizard click. The refusals below are proven by
    # exit code, and a refusal only a human can observe is one nothing verifies.
    #
    # The call operator, deliberately, and NOT `Start-Process -ArgumentList`.
    # `Start-Process` joins an argument array with spaces and quotes nothing, so
    # this repository's own path -- `C:\Coding Projects\...` -- was delivered as
    # two arguments. Measured 2026-08-15: the installer took `C:\Coding` as the
    # root, created it at the top of the drive, wrote 45 MB into it and exited
    # zero, and the only symptom here was the next line failing to find a file.
    # The installer now refuses arguments it cannot parse whole, so this would be
    # caught rather than obeyed; the call operator is what stops it being sent.
    # `Continue` for the duration of the call, and this is the whole reason
    # `Assert-Refused` works. PowerShell turns a native command's stderr into
    # ErrorRecords when it is redirected inside PowerShell, and under
    # `$ErrorActionPreference = 'Stop'` those are *terminating*. Every refusal
    # this script exists to assert is written to stderr, so each one threw here
    # -- at the redirect, before the exit code could be read -- and the failure
    # surfaced as a `NativeCommandError` naming the refusal text, which reads
    # exactly like the installer being broken rather than like the installer
    # correctly refusing. `Stage-DevRuntime.ps1` carries the same warning about
    # cargo's progress output; the difference is that this script *needs* the
    # output, so it lowers the preference rather than leaving the stream alone.
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $output = & $installer --install --install-root $installRoot 2>&1
    } finally {
        $ErrorActionPreference = $previousPreference
    }
    # Both, always. The exit code proves a refusal happened; only the message
    # says WHICH refusal, and the two have already disagreed here -- a stamp left
    # by an earlier run made a fresh install refuse as a same-version reinstall,
    # which as a bare `exit 1` was indistinguishable from the installer being
    # broken.
    [pscustomobject]@{
        ExitCode = $LASTEXITCODE
        Output   = ($output | ForEach-Object { $_.ToString() }) -join ' '
    }
}

function Assert-Refused {
    param([Parameter(Mandatory)][string]$Scenario)
    $result = Invoke-Installer
    if ($result.ExitCode -eq 0) {
        throw "${Scenario}: installer unexpectedly exited successfully."
    }
    Write-Host "${Scenario}: refused (exit code $($result.ExitCode)) -- $($result.Output)"
}

# Assert what is *in* the Add/Remove Programs entry, not merely that it exists.
#
# This script checked that the key was created and then removed, and nothing
# else -- which is how `DisplayName: SpeakEasy` survived here. Every value below
# is also an assertion about identity: the key is `ai.speakeasy.mini` and was
# `ai.speakeasy.desktop` until 2026-08-18, when setup was registering Mini under
# the *parent* product's entry and Mini's uninstaller then deleted it.
#
# Case-sensitive, including the three paths. The installer writes back the exact
# string it was handed, so a difference in case is a difference in behaviour
# rather than a Windows path equivalence.
function Assert-ArpValues {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$Version
    )
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ai.speakeasy.mini'
    $entry = Get-ItemProperty -Path $key -ErrorAction SilentlyContinue
    if (-not $entry) {
        throw "No Add/Remove Programs entry at ${key}; the install did not register."
    }
    $expected = [ordered]@{
        DisplayName     = 'SpeakEasy Mini'
        DisplayVersion  = $Version
        Publisher       = 'SpeakEasy Mini'
        InstallLocation = $Root
        UninstallString = '"{0}\speakeasy-bootstrapper.exe" --uninstall' -f $Root
        DisplayIcon     = "$Root\ai-speakeasy-mini.exe"
        NoModify        = 1
        NoRepair        = 1
    }
    foreach ($name in $expected.Keys) {
        $actual = $entry.$name
        if ($null -eq $actual) {
            throw "Add/Remove Programs value ${name} is absent."
        }
        if ("$actual" -cne "$($expected[$name])") {
            throw "Add/Remove Programs value ${name} is '${actual}'; expected '$($expected[$name])'."
        }
    }
    Write-Host ("Add/Remove Programs: DisplayName, DisplayVersion, Publisher, InstallLocation, " +
        "UninstallString, DisplayIcon, NoModify and NoRepair all match (root ${Root}).")
}

function Assert-ProcessStopped {
    param([Parameter(Mandatory)][System.Diagnostics.Process]$Process)
    if (-not $Process.HasExited) {
        Stop-Process -Id $Process.Id -Force
        $Process.WaitForExit()
    }
}

Push-Location $repositoryRoot
try {
    $runningApps = @(Get-Process -Name 'ai-speakeasy-mini' -ErrorAction SilentlyContinue)
    if ($runningApps.Count -gt 0) {
        $paths = $runningApps | ForEach-Object { $_.Path } | Where-Object { $_ } | Select-Object -Unique
        throw ('Cannot start the ephemeral installer lifecycle while SpeakEasy is already running. ' +
            'Close SpeakEasy and rerun this script; refusing to terminate a user-owned process. ' +
            "Detected: $($paths -join ', ')")
    }
    # The install root is ephemeral, but the version stamp it writes is not: it
    # is the single HKCU value every install on this machine reads. So a real
    # installation -- or the wreckage of an interrupted run of this script --
    # makes the very first install below refuse as a same-version reinstall,
    # which is a correct refusal of the wrong thing. Checked up front and named,
    # rather than deleted: the stamp may belong to an installation the person
    # running this actually uses, and this script does not get to remove it for
    # the same reason it will not terminate a running SpeakEasy.
    $versionStampKey = 'HKCU:\Software\SpeakEasy Mini\LocalDevelopment'
    $existingStamp = (Get-ItemProperty -Path $versionStampKey -Name Version -ErrorAction SilentlyContinue).Version
    if ($existingStamp) {
        throw ("SpeakEasy $existingStamp is recorded as installed on this machine, so the " +
            'ephemeral install below would be refused as a same-version reinstall. ' +
            'Uninstall SpeakEasy (or run the bootstrapper with --uninstall) and rerun this script.')
    }
    # Keep the broad artifact assertions in the canonical script so the release
    # workflow and local validation cannot drift into two different contracts.
    & (Join-Path $PSScriptRoot 'Test-LocalInstall.ps1') `
        -ArtifactRoot $artifactRoot `
        -RunningAppRefusalEvidence 'passed_ephemeral_lifecycle'
    if ($LASTEXITCODE -ne 0) { throw 'Static local-install checks failed.' }

    New-Item -ItemType Directory -Path $installRoot -Force | Out-Null
    $fresh = Invoke-Installer
    if ($fresh.ExitCode -ne 0) {
        throw "Fresh current-user install failed with exit code $($fresh.ExitCode): $($fresh.Output)"
    }
    $installedDesktop = Join-Path $installRoot 'ai-speakeasy-mini.exe'
    # The bootstrapper installs at the root, not in `repair\`: it is setup as
    # well as repair now, and a user looking for the thing that fixes a broken
    # install should find it beside the app rather than in a subdirectory named
    # after only one of its two jobs.
    $installedBootstrapper = Join-Path $installRoot 'speakeasy-bootstrapper.exe'
    # No separate uninstall.exe: the bootstrapper is the uninstaller, which is
    # what the Add/Remove Programs UninstallString points at.
    foreach ($path in @($installedDesktop, $installedBootstrapper)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Fresh install did not produce the expected file: $path"
        }
    }

    Assert-ArpValues -Root $installRoot -Version $productVersion

    $desktopHashBeforeRefusal = (Get-FileHash -LiteralPath $installedDesktop -Algorithm SHA256).Hash
    $app = Start-Process -FilePath $installedDesktop -PassThru
    Start-Sleep -Seconds 5
    if ($app.HasExited) {
        throw "Installed desktop did not remain running for the running-app refusal check (exit code $($app.ExitCode))."
    }
    Assert-Refused 'running-app install'
    $desktopHashAfterRefusal = (Get-FileHash -LiteralPath $installedDesktop -Algorithm SHA256).Hash
    if ($desktopHashAfterRefusal -ne $desktopHashBeforeRefusal) {
        throw 'Running-app refusal changed the installed desktop executable.'
    }
    Assert-ProcessStopped $app

    Assert-Refused 'same-version install'

    $versionKey = 'HKCU:\Software\SpeakEasy Mini\LocalDevelopment'
    New-Item -Path $versionKey -Force | Out-Null
    $version = [version]$productVersion
    $newerVersion = '{0}.{1}.{2}' -f ($version.Major + 1), $version.Minor, $version.Build
    Set-ItemProperty -Path $versionKey -Name Version -Value $newerVersion
    Assert-Refused 'automatic downgrade'

    $repairData = Join-Path $installRoot 'repair-smoke-data'
    $repairBundle = Join-Path $installRoot 'repair-smoke-bundle'
    New-Item -ItemType Directory -Path (Join-Path $repairData 'settings') -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $repairData 'settings\smoke.txt') -Value 'repair smoke' -Encoding utf8
    $backupOutput = & $installedBootstrapper backup `
        --data-root $repairData `
        --bundle-root $repairBundle `
        --installer $installer `
        --version $productVersion `
        --created-unix-ms 0 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Repair backup failed: $($backupOutput -join ' ')" }
    $backupLine = $backupOutput | Where-Object { $_ -match '^backup_manifest=' } | Select-Object -First 1
    if (-not $backupLine) { throw 'Repair backup did not report its manifest path.' }
    $backupManifest = $backupLine.ToString().Substring('backup_manifest='.Length)
    & $installedBootstrapper verify $backupManifest | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Repair verification failed.' }

    # `/S` is still the spelling, so the instruction this script gives is the one
    # it always gave. `--keep-user-data` is new and is passed on purpose.
    #
    # This comment said the opposite until 2026-08-21: `--remove-all` was
    # "deliberately NOT passed", because the default had to keep user data and
    # asserting that here was what stopped an unattended uninstall from becoming
    # destructive. The owner inverted that default -- an uninstall that leaves
    # 2.14 GB of weights behind has not uninstalled anything -- so the flag this
    # script must pass is the *opposite* one, and for a different reason: not
    # safety, but cost. Without it every lifecycle run re-downloads the weights.
    #
    # The production default is therefore not exercised by *this* uninstall. It is
    # pinned in `apps/bootstrapper`'s own tests
    # (`removing_user_data_leaves_no_profile_directory_and_keeping_it_leaves_all_of_it`),
    # which can stage a profile root rather than deleting this machine's, and it
    # is run for real by the default-install-root scenario at the end of this
    # script, whose `APPDATA` is redirected somewhere it costs nothing to delete.
    $uninstallOutput = & $installedBootstrapper --uninstall /S --keep-user-data 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "Silent uninstall failed with exit code ${LASTEXITCODE}: $($uninstallOutput -join ' ')"
    }
    if (Test-Path -LiteralPath $installedDesktop -PathType Leaf) {
        throw 'Silent uninstall left the installed executable behind.'
    }
    # The uninstaller is the one file Windows will not let itself delete, so it
    # is the one most likely to be quietly left behind -- it was, until it moved
    # its own image out of the install root first. Asserted separately from the
    # desktop executable because the two fail for entirely different reasons.
    if (Test-Path -LiteralPath $installedBootstrapper -PathType Leaf) {
        throw 'Silent uninstall left its own executable behind; the install root cannot be cleaned.'
    }
    # Nothing at all, and that is the change. `proof\` used to survive by design,
    # because it held an on-demand CUDA runtime that cost ~2.97 GB to fetch again
    # -- a download this fork does not have and has not had since the streaming
    # engine left. So the allowed-survivor set is now empty, and the install root
    # itself must be gone: `--keep-user-data` keeps the *profile*, never anything
    # under the program directory.
    #
    # Asserted as "the directory does not exist" rather than "it is empty",
    # because an empty directory left behind is exactly what the previous rule
    # produced on a machine that never fetched a runtime, and it read as clean.
    if (Test-Path -LiteralPath $installRoot) {
        $survivors = @(Get-ChildItem -LiteralPath $installRoot -Force -Recurse -ErrorAction SilentlyContinue |
            ForEach-Object { $_.FullName.Replace("$installRoot\", '') } | Sort-Object)
        throw ("Silent uninstall left the install root behind: " +
            $(if ($survivors.Count -gt 0) { $survivors -join ', ' } else { '(empty directory)' }))
    }
    if (Test-Path -LiteralPath 'HKCU:\Software\SpeakEasy Mini\LocalDevelopment') {
        throw 'Silent uninstall left the version stamp behind; the next install would refuse.'
    }
    $arpKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ai.speakeasy.mini'
    if (Test-Path -LiteralPath $arpKey) {
        throw 'Silent uninstall left the Add/Remove Programs entry behind.'
    }

    # Everything above drove `speakeasy-bootstrapper.exe` with a `payload\`
    # directory beside it, which is the developer's layout. Nobody downloads
    # that. What a user runs is `SpeakEasyMiniSetup.exe`, the same program with
    # the payload appended to its own image -- a different code path to reach
    # the same files, and until this ran, the only untested one in the chain
    # that actually ships.
    #
    # Last rather than first, because it writes the same single HKCU version
    # stamp: run before the sequence above and every install in it refuses as a
    # same-version reinstall. By here the machine is clean again.
    $setupExecutable = Join-Path $artifactRoot 'SpeakEasyMiniSetup.exe'
    if (-not (Test-Path -LiteralPath $setupExecutable -PathType Leaf)) {
        throw "The single-file installer was not found in $artifactRoot."
    }
    $embeddedRoot = Join-Path $lifecycleRoot "$PID-embedded"
    if (Test-Path -LiteralPath $embeddedRoot) {
        Remove-Item -LiteralPath $embeddedRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path $embeddedRoot -Force | Out-Null
    try {
        # `--install-root` with the call operator, never `Start-Process
        # -ArgumentList`: this repository's own path has a space in it, and that
        # joins its array with spaces and quotes nothing.
        $previousPreference = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $embeddedOutput = & $setupExecutable --install --install-root $embeddedRoot 2>&1
        } finally {
            $ErrorActionPreference = $previousPreference
        }
        if ($LASTEXITCODE -ne 0) {
            throw "Single-file install failed with exit code ${LASTEXITCODE}: $($embeddedOutput -join ' ')"
        }
        # Byte-identical to what the sibling-directory install placed, which is
        # the actual claim: the embedded archive is not a second copy of the
        # payload that could differ from the packaged one, it IS the packaged
        # one. A hash comparison says that; "the file exists" does not.
        foreach ($relative in @('ai-speakeasy-mini.exe', 'speakeasy-bootstrapper.exe', 'proof\granite-worker.exe')) {
            $embedded = Join-Path $embeddedRoot $relative
            $packaged = Join-Path $artifactRoot "payload\$relative"
            if (-not (Test-Path -LiteralPath $embedded -PathType Leaf)) {
                throw "Single-file install did not place $relative."
            }
            $embeddedHash = (Get-FileHash -LiteralPath $embedded -Algorithm SHA256).Hash
            $packagedHash = (Get-FileHash -LiteralPath $packaged -Algorithm SHA256).Hash
            if ($embeddedHash -ne $packagedHash) {
                throw "Single-file install placed a different $relative than the packaged payload."
            }
        }
        $embeddedUninstall = & (Join-Path $embeddedRoot 'speakeasy-bootstrapper.exe') --uninstall /S --keep-user-data 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw "Single-file uninstall failed with exit code ${LASTEXITCODE}: $($embeddedUninstall -join ' ')"
        }
        Write-Host 'single-file install: placed and removed the embedded payload'
    } finally {
        if (-not $KeepInstall -and (Test-Path -LiteralPath $embeddedRoot)) {
            Remove-Item -LiteralPath $embeddedRoot -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    # Every install above was handed `--install-root`. The default is the branch a
    # user actually takes -- `probe::install_root`, `%LOCALAPPDATA%\SpeakEasy Mini`
    # -- and it is the branch with the worst history: it returned the *parent*
    # product's directory before the fork was renamed, and `C:\` when
    # `LOCALAPPDATA` was unset, either of which uninstall would then have removed
    # whole. Its unit tests are pure and hand the variable in; nothing exercised
    # the derivation through a real install.
    #
    # `LOCALAPPDATA` and `APPDATA` are redirected for this scenario, not changed
    # on the machine: the first is what the install root is derived from, the
    # second is the profile, the Start Menu folder and everything `--uninstall`
    # removes. Both point under `target\installer-lifecycle`, so the default path
    # is derived and exercised for real while every byte it writes is test-owned.
    # Without the `APPDATA` half this would seed and then delete the profile of
    # whoever is running it.
    #
    # It also uninstalls with the *production* default rather than
    # `--keep-user-data`, which nothing else here can do: the flag is passed
    # everywhere above so a lifecycle run does not re-download 2.14 GB of weights,
    # and that leaves the default -- remove the profile too -- proven only in
    # `apps/bootstrapper`'s own tests. A redirected `APPDATA` holding nothing but
    # what this scenario seeded is the one place it costs nothing to run for real.
    $defaultProfileRoot = Join-Path $lifecycleRoot "$PID-default"
    if (Test-Path -LiteralPath $defaultProfileRoot) {
        Remove-Item -LiteralPath $defaultProfileRoot -Recurse -Force
    }
    $defaultLocalAppData = Join-Path $defaultProfileRoot 'Local'
    $defaultAppData = Join-Path $defaultProfileRoot 'Roaming'
    New-Item -ItemType Directory -Path $defaultLocalAppData -Force | Out-Null
    New-Item -ItemType Directory -Path $defaultAppData -Force | Out-Null
    $defaultInstallRoot = Join-Path $defaultLocalAppData 'SpeakEasy Mini'
    $realLocalAppData = $env:LOCALAPPDATA
    $realAppData = $env:APPDATA
    try {
        $env:LOCALAPPDATA = $defaultLocalAppData
        $env:APPDATA = $defaultAppData
        $previousPreference = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $defaultOutput = & $setupExecutable --install 2>&1
        } finally {
            $ErrorActionPreference = $previousPreference
        }
        if ($LASTEXITCODE -ne 0) {
            throw "Default-root install failed with exit code ${LASTEXITCODE}: $($defaultOutput -join ' ')"
        }
        # The claim is the derivation, so this asserts the directory the installer
        # chose for itself rather than one the script handed it.
        foreach ($relative in @('ai-speakeasy-mini.exe', 'speakeasy-bootstrapper.exe')) {
            $placed = Join-Path $defaultInstallRoot $relative
            if (-not (Test-Path -LiteralPath $placed -PathType Leaf)) {
                $chose = @(Get-ChildItem -LiteralPath $defaultLocalAppData -Force -ErrorAction SilentlyContinue |
                    ForEach-Object { $_.Name } | Sort-Object)
                throw ("Default-root install did not place ${relative} under ${defaultInstallRoot}; " +
                    "LOCALAPPDATA holds: " + $(if ($chose.Count -gt 0) { $chose -join ', ' } else { '(nothing)' }))
            }
        }
        # Registered under the derived root too. `InstallLocation` is what
        # `install::installed_location` reads back, so a right-place install that
        # recorded the wrong root would still uninstall the wrong directory.
        Assert-ArpValues -Root $defaultInstallRoot -Version $productVersion

        $defaultBootstrapper = Join-Path $defaultInstallRoot 'speakeasy-bootstrapper.exe'
        $defaultUninstall = & $defaultBootstrapper --uninstall /S 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw "Default-root uninstall failed with exit code ${LASTEXITCODE}: $($defaultUninstall -join ' ')"
        }
        if (Test-Path -LiteralPath $defaultInstallRoot) {
            $survivors = @(Get-ChildItem -LiteralPath $defaultInstallRoot -Force -Recurse -ErrorAction SilentlyContinue |
                ForEach-Object { $_.FullName.Replace("$defaultInstallRoot\", '') } | Sort-Object)
            throw ("Default-root uninstall left the install root behind: " +
                $(if ($survivors.Count -gt 0) { $survivors -join ', ' } else { '(empty directory)' }))
        }
        $defaultProfile = Join-Path $defaultAppData 'ai.speakeasy.mini'
        if (Test-Path -LiteralPath $defaultProfile) {
            throw "Default-root uninstall kept the profile at ${defaultProfile} without being asked to."
        }
        if (Test-Path -LiteralPath $arpKey) {
            throw 'Default-root uninstall left the Add/Remove Programs entry behind.'
        }
        if (Test-Path -LiteralPath 'HKCU:\Software\SpeakEasy Mini\LocalDevelopment') {
            throw 'Default-root uninstall left the version stamp behind; the next install would refuse.'
        }
        Write-Host ("default install root: derived %LOCALAPPDATA%\SpeakEasy Mini, registered it, " +
            'and removed the program directory and the profile with it')
    } finally {
        $env:LOCALAPPDATA = $realLocalAppData
        $env:APPDATA = $realAppData
        if (-not $KeepInstall -and (Test-Path -LiteralPath $defaultProfileRoot)) {
            Remove-Item -LiteralPath $defaultProfileRoot -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    [pscustomobject]@{
        schema_version = 1
        tested_utc = [DateTime]::UtcNow.ToString('o')
        fresh_install = 'passed'
        single_file_install = 'passed'
        default_install_root = 'passed'
        add_remove_programs_values = 'passed'
        launch = 'passed'
        running_app_install_refusal = 'passed'
        same_version_refusal = 'passed'
        automatic_downgrade_refusal = 'passed'
        repair_backup_and_verify = 'passed'
        uninstall = 'passed'
        checksums_and_manifest = 'passed'
        authenticode = 'not_configured_prerequisite'
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $artifactRoot 'installer-lifecycle-report.json') -Encoding utf8
    Write-Host 'SpeakEasy installer lifecycle checks: passed'
}
finally {
    Pop-Location
    if (-not $KeepInstall -and (Test-Path -LiteralPath $installRoot)) {
        Remove-Item -LiteralPath $installRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
