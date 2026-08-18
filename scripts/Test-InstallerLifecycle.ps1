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
    # it always gave. `--remove-all` is deliberately NOT passed: the default must
    # keep user data, and asserting that here is what stops a future change from
    # making an unattended uninstall destructive.
    $uninstallOutput = & $installedBootstrapper --uninstall /S 2>&1
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
    # What an ordinary uninstall is allowed to spare, and nothing else. `proof\`
    # survives by design: it holds the on-demand CUDA runtime that costs ~2.97 GB
    # to fetch again. Pinned as an exact set so that a future change which starts
    # sparing something else has to say so here.
    $survivors = @(Get-ChildItem -LiteralPath $installRoot -Force -ErrorAction SilentlyContinue |
        ForEach-Object { $_.Name } | Sort-Object)
    $allowedSurvivors = @('proof')
    $unexpected = @($survivors | Where-Object { $_ -notin $allowedSurvivors })
    if ($unexpected.Count -gt 0) {
        throw "Silent uninstall left unexpected files in the install root: $($unexpected -join ', ')"
    }
    if (Test-Path -LiteralPath 'HKCU:\Software\SpeakEasy Mini\LocalDevelopment') {
        throw 'Silent uninstall left the version stamp behind; the next install would refuse.'
    }
    $arpKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ai.speakeasy.mini'
    if (Test-Path -LiteralPath $arpKey) {
        throw 'Silent uninstall left the Add/Remove Programs entry behind.'
    }

    [pscustomobject]@{
        schema_version = 1
        tested_utc = [DateTime]::UtcNow.ToString('o')
        fresh_install = 'passed'
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
