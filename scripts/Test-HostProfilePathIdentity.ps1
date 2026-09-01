#Requires -Version 7
<#
.SYNOPSIS
    Real-host preflight: proves this shell's `%APPDATA%` and `%LOCALAPPDATA%`
    reach ordinary host storage.

.DESCRIPTION
    Run this before any proof that installs SpeakEasy Mini for real. Those proofs
    call the same assertion themselves, so this is not a substitute for them --
    it is the cheap way to find out whether the shell you are in can run them at
    all, without installing anything first.

    A pass means each profile root carried a token out to the drive's
    administrative share and a different token back, and that the probe file was
    removed and every probe path then classified as absent. A failure means this
    shell's profile paths are redirected, or that host identity cannot be proved
    from here; either way the live-profile proofs must not be run in it.

    `HostProfilePathIdentity.ps1` carries the algorithm and the reasoning.
    `Test-HostProfilePathIdentityIsScoped.ps1` proves the behaviour against
    repository-local directories, with no profile and no share.

    This entry point takes no parameters. The test seams the checker exposes for
    that control are deliberately unreachable from here.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'HostProfilePathIdentity.ps1')

Write-Host 'host profile path identity preflight'
$results = @(Assert-HostProfilePathIdentity -Context 'Test-HostProfilePathIdentity.ps1')

if ($results.Count -ne 2) {
    throw "Expected both profile roots to be proved; got $($results.Count)."
}

Write-Host ''
foreach ($result in $results) {
    Write-Host "$($result.Label)"
    Write-Host "  ordinary   : $($result.OrdinaryView)"
    Write-Host "  independent: $($result.IndependentView)"
    Write-Host "  directions : ordinary->independent $($result.FirstDirection), independent->ordinary $($result.SecondDirection)"
    Write-Host "  probe      : $($result.ProbeName) removed, and every probe path classified absent"
}

Write-Host ''
Write-Host 'host profile path identity: passed (both roots, both directions, probe accounted for)'
