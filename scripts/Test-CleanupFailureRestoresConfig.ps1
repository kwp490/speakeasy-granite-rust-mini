#Requires -Version 7
<#
.SYNOPSIS
    Proves the wizard proof restores the operator's configuration even when its
    own cleanup fails.

.DESCRIPTION
    `Test-SetupWizard.ps1` installs for real and rewrites `config\` on the way
    through, so the restore in its `finally` is the only thing standing between a
    proof run and the operator's dictionary. Cleanup does three other things
    first -- stop the application, stop the setup program, uninstall -- and all
    three can fail. Ordered as plain statements, the first failure ends the block
    and the restore never runs.

    The ordering lives in `WizardCleanup.ps1` so it can be proved here without an
    installation. Every failure mode is driven against `Invoke-WizardCleanup`
    directly, against a scratch configuration directory under `target\`:

    - stopping the installed application throws
    - stopping the setup program throws
    - the uninstaller is missing
    - the uninstaller returns a nonzero exit code (a real child process)
    - invoking the uninstaller throws
    - all three steps fail at once
    - a step fails *and* the restore fails, which must report both
    - a step fails under `$WarningPreference = 'Stop'`, which a helper that wrote
      its failures to the warning stream would not survive

    Each case asserts the failure was reported, that the restore still ran, and --
    independently, by re-reading presence and SHA-256 from disk -- that the
    scratch configuration is byte-identical. The last of those is the point: a
    restore that returned without writing satisfies the first two and not the
    third.

    With `-ArtifactRoot`, one end-to-end case runs the real wizard proof with
    `-InjectCleanupFailure StopApp`. That is what ties the ordering to the script
    that uses it: the helper could be correct and unwired. The injection throws
    *after* the application has really been stopped, so the uninstall still runs
    and the machine is not left holding an installation.

.PARAMETER ArtifactRoot
    Where `Build-LocalInstaller.ps1` put `SpeakEasyMiniSetup.exe`. Optional; when
    it is absent only the helper-level cases run, and the script says so.
#>
[CmdletBinding()]
param(
    [string]$ArtifactRoot
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'WizardCleanup.ps1')
. (Join-Path $PSScriptRoot 'DeleteContainment.ps1')
# Defines the guard; it is called only in the end-to-end case below, which is the
# only part of this control that touches the operator's profile.
. (Join-Path $PSScriptRoot 'HostProfilePathIdentity.ps1')

$targetRoot = (Resolve-Path -LiteralPath (Join-Path $repositoryRoot 'target')).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
# A GUID rather than the process id. A pid is reused, so the name could already
# name something on disk -- and the `finally` removes this directory recursively.
$scratch = Join-Path $targetRoot ('cleanup-failure-control-' + [Guid]::NewGuid().ToString('N'))
$scratchConfig = Join-Path $scratch 'config'
$fakeUninstallerDirectory = Join-Path $scratch 'fake-install'
$scratchCreated = $false

# Presence and SHA-256, re-read from disk. Deliberately not the code that
# restores: the postcondition has to be able to catch a restore that did nothing.
function Get-ScratchState {
    param([Parameter(Mandatory)][string]$Root)
    $state = [ordered]@{}
    foreach ($file in (Get-ChildItem -LiteralPath $Root -File -Force | Sort-Object Name)) {
        $state[$file.Name] = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
    }
    $state
}

function Format-State {
    param([Parameter(Mandatory)]$State)
    (($State.Keys | ForEach-Object { "$_=$($State[$_])" }) -join '|')
}

$passed = 0

function Invoke-Case {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][scriptblock]$StopInstalledApplication,
        [Parameter(Mandatory)][scriptblock]$StopSetupProgram,
        [Parameter(Mandatory)][scriptblock]$Uninstall,
        [Parameter(Mandatory)][scriptblock]$RestoreConfiguration,
        [string[]]$ExpectFailureContaining = @(),
        [switch]$ExpectRestoreFailed,
        [switch]$ExpectConfigurationDamaged,
        # Make any warning the helper writes fatal. `Write-Warning` throws under
        # `$WarningPreference = 'Stop'`, so a helper that writes one escapes its
        # no-throw contract without any supplied action having misbehaved --
        # and the wizard proof calls it from a `finally`, where a throw replaces
        # the failure the run itself hit.
        [switch]$ReportingIsFatal
    )
    Write-Host ''
    Write-Host "CASE: $Name"
    # Scrambled first, so "byte-identical afterwards" means the restore put it
    # back rather than that nothing ever changed it.
    foreach ($file in Get-ChildItem -LiteralPath $script:scratchConfig -File -Force) {
        Set-Content -LiteralPath $file.FullName -Value "rewritten by the proof: $Name" -Encoding utf8
    }
    $scrambled = Format-State -State (Get-ScratchState -Root $script:scratchConfig)
    if ($scrambled -eq $script:pristineState) {
        throw "${Name}: scrambling the scratch configuration did not change it, so this case is vacuous."
    }

    # Preference variables are dynamically scoped, so setting this here is what
    # `Invoke-WizardCleanup` sees.
    $previousWarningPreference = $WarningPreference
    if ($ReportingIsFatal) { $WarningPreference = 'Stop' }
    $result = $null
    try {
        $result = Invoke-WizardCleanup `
            -StopInstalledApplication $StopInstalledApplication `
            -StopSetupProgram $StopSetupProgram `
            -Uninstall $Uninstall `
            -RestoreConfiguration $RestoreConfiguration
    } catch {
        throw ("${Name}: Invoke-WizardCleanup threw instead of returning its failures. " +
            "It is called from the wizard proof's finally, where a throw replaces the failure " +
            "the run itself hit. Escaped: $($_.Exception.Message)")
    } finally {
        $WarningPreference = $previousWarningPreference
    }

    # **One object, always.** Asserted on every case rather than only the noisy
    # one: this is what the caller indexes into, and an array makes the shape
    # depend on what a step happened to print. Member-access enumeration papers
    # over it for a single result element and aggregates across any others, so
    # the failure mode is an ambiguous answer rather than a reliably absent one.
    $returned = @($result)
    if ($returned.Count -ne 1) {
        $kinds = (@($returned | ForEach-Object { $_.GetType().Name }) -join ', ')
        throw ("${Name}: Invoke-WizardCleanup returned $($returned.Count) objects ($kinds) instead of " +
            'one result object. A supplied action emitted to the success stream and it was relayed.')
    }
    $result = $returned[0]
    foreach ($property in 'Failures', 'ConfigurationRestored') {
        if ($null -eq $result.PSObject.Properties[$property]) {
            throw "${Name}: the returned object has no $property property; it is not the result object."
        }
    }

    $reported = $result.Failures -join ' || '
    Write-Host "  reported $($result.Failures.Count) failure(s): $reported"
    foreach ($needle in $ExpectFailureContaining) {
        if ($reported -notlike "*$needle*") {
            throw "${Name}: no reported failure mentions '$needle'. Reported: $reported"
        }
    }
    if ($ExpectFailureContaining.Count -eq 0 -and $result.Failures.Count -ne 0) {
        throw "${Name}: expected no failures and got: $reported"
    }

    if ($ExpectRestoreFailed) {
        if ($result.ConfigurationRestored) {
            throw "${Name}: the restore threw and the result still says the configuration was restored."
        }
    } elseif (-not $result.ConfigurationRestored) {
        throw "${Name}: the restore did not run, so a cleanup failure cost the operator their configuration."
    }

    $after = Format-State -State (Get-ScratchState -Root $script:scratchConfig)
    if ($ExpectConfigurationDamaged) {
        if ($after -eq $script:pristineState) {
            throw "${Name}: the restore was supposed to fail and the configuration came back anyway."
        }
        Write-Host '  the configuration is still scrambled, as this case requires'
    } else {
        if ($after -ne $script:pristineState) {
            throw "${Name}: the configuration is not byte-identical. Now $after, was $($script:pristineState)"
        }
        Write-Host '  every scratch config file is byte-identical to the capture'
    }
    $script:passed++
}

try {
    # Refused rather than cleared. A directory this run did not create is a
    # directory it cannot delete recursively, whatever its name suggests -- and
    # with a fresh GUID, finding one means something is wrong rather than stale.
    if (Test-Path -LiteralPath $scratch) {
        throw "The scratch directory already exists, which a fresh GUID makes impossible: $scratch"
    }
    [void](Resolve-ContainedDirectory -Path $scratch -Root $targetRoot `
        -Label 'The scratch directory' -DirectChildOnly)
    New-Item -ItemType Directory -Path $scratchConfig -Force | Out-Null
    New-Item -ItemType Directory -Path $fakeUninstallerDirectory -Force | Out-Null
    $scratchCreated = $true
    Set-Content -LiteralPath (Join-Path $scratchConfig 'personalization.json') `
        -Value '{"dictionary":[{"source":"LogicMonitor","origin":"user_entry"}]}' -Encoding utf8
    Set-Content -LiteralPath (Join-Path $scratchConfig 'settings.json') `
        -Value '{"privacy":{"persisted_history_enabled":true}}' -Encoding utf8
    $capture = @{}
    foreach ($file in Get-ChildItem -LiteralPath $scratchConfig -File -Force) {
        $capture[$file.Name] = [IO.File]::ReadAllBytes($file.FullName)
    }
    $pristineState = Format-State -State (Get-ScratchState -Root $scratchConfig)
    Write-Host "scratch configuration: $scratchConfig"
    Write-Host "  $pristineState"

    # The stand-in for `Restore-ConfigCapture`. It writes bytes; the postcondition
    # reads hashes off disk, so the two cannot agree by sharing a bug.
    $restore = {
        foreach ($name in $capture.Keys) {
            [IO.File]::WriteAllBytes((Join-Path $scratchConfig $name), $capture[$name])
        }
        Write-Host '  restore: wrote the captured bytes back'
    }
    $restoreThatFails = { throw 'the profile directory is not writable' }
    $succeeds = { }

    # A real child process with a nonzero exit code, so the exit-code branch is
    # driven by `$LASTEXITCODE` rather than by a value the test set.
    $failingUninstaller = Join-Path $fakeUninstallerDirectory 'speakeasy-bootstrapper.cmd'
    Set-Content -LiteralPath $failingUninstaller -Value '@exit /b 3' -Encoding ascii
    $uninstallerNonZero = {
        & $failingUninstaller --uninstall /S --keep-user-data | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "the uninstaller exited $LASTEXITCODE rather than 0" }
    }
    $missingPath = Join-Path $fakeUninstallerDirectory 'speakeasy-bootstrapper.exe'
    $uninstallerMissing = {
        if (-not (Test-Path -LiteralPath $missingPath -PathType Leaf)) {
            throw "the uninstaller is not at $missingPath"
        }
        & $missingPath --uninstall /S --keep-user-data | Out-Null
    }
    # No presence guard, so the invocation itself is what fails.
    $uninstallerInvocationThrows = {
        & (Join-Path $fakeUninstallerDirectory 'no-such-uninstaller.exe') --uninstall /S | Out-Null
    }
    $stopThrows = { throw 'Access is denied' }

    Invoke-Case -Name 'nothing fails' `
        -StopInstalledApplication $succeeds -StopSetupProgram $succeeds `
        -Uninstall $succeeds -RestoreConfiguration $restore

    Invoke-Case -Name 'stopping the installed application throws' `
        -StopInstalledApplication $stopThrows -StopSetupProgram $succeeds `
        -Uninstall $succeeds -RestoreConfiguration $restore `
        -ExpectFailureContaining 'stopping the installed application'

    Invoke-Case -Name 'stopping the setup program throws' `
        -StopInstalledApplication $succeeds -StopSetupProgram $stopThrows `
        -Uninstall $succeeds -RestoreConfiguration $restore `
        -ExpectFailureContaining 'stopping the setup program'

    Invoke-Case -Name 'the uninstaller is missing' `
        -StopInstalledApplication $succeeds -StopSetupProgram $succeeds `
        -Uninstall $uninstallerMissing -RestoreConfiguration $restore `
        -ExpectFailureContaining 'uninstalling', 'is not at'

    Invoke-Case -Name 'the uninstaller exits nonzero' `
        -StopInstalledApplication $succeeds -StopSetupProgram $succeeds `
        -Uninstall $uninstallerNonZero -RestoreConfiguration $restore `
        -ExpectFailureContaining 'uninstalling', 'exited 3'

    Invoke-Case -Name 'invoking the uninstaller throws' `
        -StopInstalledApplication $succeeds -StopSetupProgram $succeeds `
        -Uninstall $uninstallerInvocationThrows -RestoreConfiguration $restore `
        -ExpectFailureContaining 'uninstalling'

    Invoke-Case -Name 'all three steps fail' `
        -StopInstalledApplication $stopThrows -StopSetupProgram $stopThrows `
        -Uninstall $uninstallerNonZero -RestoreConfiguration $restore `
        -ExpectFailureContaining 'stopping the installed application', 'stopping the setup program', 'uninstalling'

    # The restore failure must be reported *beside* the cleanup failure, not
    # instead of it. Reporting only one of two is how the worse of the two --
    # the operator's configuration left rewritten -- becomes invisible.
    Invoke-Case -Name 'a cleanup step fails and so does the restore' `
        -StopInstalledApplication $stopThrows -StopSetupProgram $succeeds `
        -Uninstall $succeeds -RestoreConfiguration $restoreThatFails `
        -ExpectFailureContaining 'stopping the installed application', "restoring the operator's configuration" `
        -ExpectRestoreFailed -ExpectConfigurationDamaged

    # --- The helper's own code, not a supplied action -------------------------
    # Everything above fails inside a scriptblock the caller handed over. These
    # two make the *helper's* reporting fatal: it used to write each failure to
    # the warning stream, and `Write-Warning` is a throw under
    # `$WarningPreference = 'Stop'`, which escapes a function whose whole contract
    # is that it does not throw. They pass because the helper writes no warnings
    # at all, and they are the guard against one being reintroduced.
    #
    # They do **not** exercise the helper's orchestration `catch`. Nothing left
    # inside its `try` has a failure a test can manufacture, so that branch is a
    # backstop and is uncovered; this control does not pretend otherwise.
    # --- Every action emits to the success stream ------------------------------
    # `Stop-Process -PassThru`, an unsuppressed `New-Item`, a bare expression: all
    # ordinary things for a cleanup step to do, and all of them used to be relayed
    # into this function's own output. What this proves is the return *shape* --
    # one object, with both properties -- not that any particular caller was
    # misreading it.
    $noisyStopApp = { 'the app was stopped'; [pscustomobject]@{ Stopped = $true } }
    $noisyStopSetup = { 1..3 }
    $noisyUninstall = { Get-Item -LiteralPath $scratchConfig }
    $noisyRestore = {
        foreach ($name in $capture.Keys) {
            [IO.File]::WriteAllBytes((Join-Path $scratchConfig $name), $capture[$name])
        }
        'the restore wrote its files'
        Get-ChildItem -LiteralPath $scratchConfig -File
    }
    Invoke-Case -Name 'every supplied action emits output' `
        -StopInstalledApplication $noisyStopApp -StopSetupProgram $noisyStopSetup `
        -Uninstall $noisyUninstall -RestoreConfiguration $noisyRestore

    Invoke-Case -Name 'every supplied action emits output and one of them fails' `
        -StopInstalledApplication $noisyStopApp -StopSetupProgram $stopThrows `
        -Uninstall $noisyUninstall -RestoreConfiguration $noisyRestore `
        -ExpectFailureContaining 'stopping the setup program'

    Invoke-Case -Name 'a warning from the helper would be fatal' `
        -StopInstalledApplication $stopThrows -StopSetupProgram $succeeds `
        -Uninstall $succeeds -RestoreConfiguration $restore `
        -ExpectFailureContaining 'stopping the installed application' `
        -ReportingIsFatal

    Invoke-Case -Name 'a warning would be fatal and the restore fails too' `
        -StopInstalledApplication $stopThrows -StopSetupProgram $succeeds `
        -Uninstall $succeeds -RestoreConfiguration $restoreThatFails `
        -ExpectFailureContaining 'stopping the installed application', "restoring the operator's configuration" `
        -ReportingIsFatal -ExpectRestoreFailed -ExpectConfigurationDamaged

    Write-Host ''
    Write-Host "$passed helper-level case(s) passed"

    # --- End to end, so the ordering above is the ordering that runs -----------
    if (-not $ArtifactRoot) {
        Write-Host ''
        Write-Host ('END-TO-END CASE NOT RUN: pass -ArtifactRoot to drive the real wizard proof with ' +
            '-InjectCleanupFailure and prove Test-SetupWizard.ps1 uses this ordering.') -ForegroundColor Yellow
    } else {
        $artifactFull = if ([IO.Path]::IsPathRooted($ArtifactRoot)) {
            [IO.Path]::GetFullPath($ArtifactRoot)
        } else {
            [IO.Path]::GetFullPath((Join-Path $repositoryRoot $ArtifactRoot))
        }
        # Here and not at the top of the file: everything above runs against a
        # scratch directory under `target\` and must stay runnable where the
        # profile is redirected. From this line on the case drives a real install
        # against the operator's own profile, so the two views have to be proved
        # to be one before the first path is derived from them.
        Assert-HostProfilePathIdentity -Context 'Test-CleanupFailureRestoresConfig.ps1 (end-to-end case)'

        $installRoot = Join-Path $env:LOCALAPPDATA 'SpeakEasy Mini'
        $configRoot = Join-Path (Join-Path $env:APPDATA 'ai.speakeasy.mini') 'config'
        $seedFiles = @(
            'install-hotkey.txt'
            'install-logging.txt'
            'install-retention.txt'
            'install-vocabulary.txt'
            'install-provider.txt'
        )
        function Get-OperatorConfigState {
            $names = [Collections.Generic.List[string]]::new()
            if (Test-Path -LiteralPath $configRoot -PathType Container) {
                foreach ($file in Get-ChildItem -LiteralPath $configRoot -File -Force) { $names.Add($file.Name) }
            }
            foreach ($name in $seedFiles) { if (-not $names.Contains($name)) { $names.Add($name) } }
            $state = [ordered]@{}
            foreach ($name in ($names | Sort-Object)) {
                $path = Join-Path $configRoot $name
                $state[$name] = if (Test-Path -LiteralPath $path -PathType Leaf) {
                    (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
                } else { '(absent)' }
            }
            $state
        }

        Write-Host ''
        Write-Host 'END-TO-END CASE: the real wizard proof, with its first cleanup step failing'
        $operatorBefore = Get-OperatorConfigState
        foreach ($name in $operatorBefore.Keys) { Write-Host "  $name = $($operatorBefore[$name])" }

        $threw = $false
        $message = $null
        try {
            & (Join-Path $PSScriptRoot 'Test-SetupWizard.ps1') -ArtifactRoot $artifactFull -InjectCleanupFailure StopApp
        } catch {
            $threw = $true
            $message = $_.Exception.Message
        }
        if (-not $threw) {
            throw 'The wizard proof reported success with a failing cleanup step; the failure was swallowed.'
        }
        if ($message -notlike '*Cleanup after the wizard proof failed*' -or
            $message -notlike '*stopping the installed application*') {
            throw ('The wizard proof failed for something other than the injected cleanup step, so this ' +
                "case is inconclusive: $message")
        }
        Write-Host "  reported: $message"

        $operatorAfter = Get-OperatorConfigState
        $differences = @()
        foreach ($name in $operatorBefore.Keys) {
            $afterValue = if ($operatorAfter.Contains($name)) { $operatorAfter[$name] } else { '(absent)' }
            if ($operatorBefore[$name] -ne $afterValue) {
                $differences += "$name was $($operatorBefore[$name]) and is $afterValue"
            }
        }
        foreach ($name in $operatorAfter.Keys) {
            if (-not $operatorBefore.Contains($name)) { $differences += "$name appeared" }
        }
        if ($differences.Count -gt 0) {
            throw ("The cleanup failure cost the operator their configuration: " + ($differences -join '; '))
        }
        Write-Host "  all $($operatorBefore.Count) config entries byte-identical after the failed cleanup"

        # The steps after the failing one still ran. Without that, "the restore
        # happened" would be true of a run that also left an installation behind.
        if (Test-Path -LiteralPath $installRoot) {
            throw 'The cleanup failure stopped the uninstall; the installation is still there.'
        }
        $stamp = (Get-ItemProperty -Path 'HKCU:\Software\SpeakEasy Mini\LocalDevelopment' -Name Version -ErrorAction SilentlyContinue).Version
        if ($stamp) { throw "The cleanup failure left the version stamp behind: $stamp" }
        Write-Host '  the uninstall ran anyway: no install root and no version stamp'
    }

    Write-Host ''
    Write-Host 'cleanup failure still restores the configuration: passed'
}
finally {
    # Only a directory this run created, and only after the exact path is
    # re-validated: a direct child of `target\`, an ordinary directory, no
    # reparse point on it or on `target\`. Reported rather than thrown, because
    # this `finally` may already be carrying a case failure and a staging
    # directory left behind damages nothing.
    if ($scratchCreated) {
        try {
            [void](Remove-ContainedDirectory -Path $scratch -Root $targetRoot `
                -Label 'The scratch directory' -DirectChildOnly)
        } catch {
            Write-Warning ('The scratch directory was left in place: ' + $_.Exception.Message) -WarningAction Continue
        }
    }
}

# Explicit, because the cases above run a child process that exits 3 on purpose
# and a script's exit code otherwise trails `$LASTEXITCODE`. A passing control
# reported failure to any caller that reads the code rather than the exception.
exit 0
