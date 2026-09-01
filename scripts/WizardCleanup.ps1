#Requires -Version 7
<#
.SYNOPSIS
    The cleanup order `Test-SetupWizard.ps1` runs after it owns the machine.

.DESCRIPTION
    Dot-sourced rather than inlined so the ordering can be proved without an
    installation, by `Test-CleanupFailureRestoresConfig.ps1`.

    - **The restore runs even when every earlier step fails.** Its `finally` is
      outermost and encloses the step list's construction as well as the
      enumeration; inside it, one `catch` per step so later steps still run, and
      one around the sequence for anything this function's own code raises.
    - **Exactly one object comes back.** Supplied-action output is discarded, not
      relayed, so the return shape does not depend on what a scriptblock printed.
    - **Nothing throws and nothing writes to the warning stream.** The caller is a
      `finally` that may already be carrying the run's own failure, and
      `Write-Warning` throws under `$WarningPreference = 'Stop'`. Reporting is the
      caller's job.

    The orchestration `catch` is a backstop: no case reaches it, because nothing
    left inside its `try` can fail in a way a test can manufacture. Uncovered, and
    this is not a claim otherwise.
#>

function Invoke-WizardCleanup {
    <#
    .PARAMETER StopInstalledApplication
        Stops the application this run launched. First, because the app holds
        `settings.json` and would write its own copy back over a restore.

    .PARAMETER StopSetupProgram
        Stops the wizard, which an aborted run leaves on screen waiting for a
        click nobody is going to make.

    .PARAMETER Uninstall
        Removes the installation this run performed. Must check the uninstaller's
        native exit code itself: a nonzero exit is a cleanup failure, and
        `& $uninstaller | Out-Null` reports it in nothing else.

    .PARAMETER RestoreConfiguration
        Puts the operator's configuration back. Runs after the uninstall, which
        `--keep-user-data` leaves the profile in place for.

    .OUTPUTS
        `Failures`, one string per step that failed in the order they were
        attempted, then an orchestration failure if there was one, then a restore
        failure last; and `ConfigurationRestored`, which is `$true` only when the
        restore ran and did not throw. The caller decides whether to throw or
        warn.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][scriptblock]$StopInstalledApplication,
        [Parameter(Mandatory)][scriptblock]$StopSetupProgram,
        [Parameter(Mandatory)][scriptblock]$Uninstall,
        [Parameter(Mandatory)][scriptblock]$RestoreConfiguration
    )

    # The collection first, so both the `try` and the `finally` below can write to
    # it whatever happens between them.
    $failures = [Collections.Generic.List[string]]::new()
    $restored = $false

    # Outermost: everything that can fail is inside this `try`, and the `finally`
    # that restores the operator's configuration is therefore reachable from all
    # of it -- the step list's construction included.
    try {
        try {
            $steps = @(
                [pscustomobject]@{ Name = 'stopping the installed application'; Action = $StopInstalledApplication }
                [pscustomobject]@{ Name = 'stopping the setup program'; Action = $StopSetupProgram }
                [pscustomobject]@{ Name = 'uninstalling'; Action = $Uninstall }
            )
            foreach ($step in $steps) {
                try {
                    # Discarded, not relayed. A scriptblock that emits anything to
                    # the success stream -- `Stop-Process -PassThru`, an
                    # unsuppressed cmdlet, a bare expression -- would otherwise
                    # join this function's output and make the return an array.
                    $null = & $step.Action
                } catch {
                    $failures.Add("$($step.Name): $($_.Exception.Message)")
                }
            }
        } catch {
            # This function's own code rather than a caller's action. A backstop,
            # not a covered branch -- see the note in the header.
            $failures.Add("cleanup orchestration: $($_.Exception.Message)")
        }
    } finally {
        try {
            $null = & $RestoreConfiguration
            $restored = $true
        } catch {
            # Added rather than substituted. A restore failure is the worst
            # outcome here and must not be reported in place of the cleanup
            # failure that may have caused it.
            $failures.Add("restoring the operator's configuration: $($_.Exception.Message)")
        }
    }

    [pscustomobject]@{
        Failures = $failures.ToArray()
        ConfigurationRestored = $restored
    }
}
