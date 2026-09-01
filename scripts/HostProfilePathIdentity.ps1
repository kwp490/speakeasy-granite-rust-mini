#Requires -Version 7
<#
.SYNOPSIS
    Proves `%APPDATA%` and `%LOCALAPPDATA%` name ordinary host storage, before a
    proof inspects or mutates the machine through them.

.DESCRIPTION
    Dot-sourced by the proofs that install for real and by
    `Test-HostProfilePathIdentity.ps1`, and driven against repository-local
    directories by `Test-HostProfilePathIdentityIsScoped.ps1`, so the contract is
    provable without touching a profile.

    **A path string is not a storage identity.** A container can present its own
    storage at `C:\Users\<name>\AppData\Roaming` while every environment variable,
    every path comparison and every reparse-point check agrees the path is
    ordinary. Nothing readable from inside that view distinguishes it, so the only
    evidence is a write that a second, independently obtained view of the disk can
    see -- and a write through that second view that the first one can see back.

    So this refuses to accept anything cheaper. Not file counts, not directory
    sizes, not hashes of files that are already there, not reparse-point status,
    not path-string equality: those all pass under exactly the redirection this
    exists to catch.

    The second view is the drive's administrative share, `\\localhost\<drive>$`,
    which the SMB server resolves against the real volume rather than against the
    calling process's view of the namespace. Elevation is not asserted as a
    separate condition -- if the share opens, it opens; if it does not, that is
    reported as its own failure rather than inferred from a token check.

    - **Both roots, both directions, every run.** `%APPDATA%` and `%LOCALAPPDATA%`
      are probed independently; a pass means each of them carried a token out and
      a different token back.
    - **One file, named once, cryptographically.** Nothing here enumerates either
      directory, reads anything already in it, or creates a product directory.
    - **Cleanup is part of the result.** The probe name is removed through every
      view it could have reached and its absence confirmed; a cleanup that failed
      fails the whole check rather than being warned about.
    - **Fail closed.** Every refusal names the root and the direction. There is no
      switch that skips the check, because a caller that could skip it would skip
      it on the machine where it mattered.
#>

function New-PathIdentityProbeName {
    <#
    .DESCRIPTION
        Cryptographically random, so it cannot collide with a product file, with
        a file another run of this left behind, or with anything an operator has.
        `Guid::NewGuid` would very probably do; a CSPRNG removes the "probably".
    #>
    $bytes = [byte[]]::new(24)
    [Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    'speakeasy-mini-path-identity-' + [Convert]::ToHexString($bytes).ToLowerInvariant() + '.probe'
}

function New-PathIdentityToken {
    $bytes = [byte[]]::new(32)
    [Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    [Convert]::ToHexString($bytes).ToLowerInvariant()
}

function Get-AdminSharePath {
    <#
    .SYNOPSIS
        `C:\some\path` -> `\\localhost\C$\some\path`.

    .DESCRIPTION
        Separated from the bidirectional check so each is provable on its own:
        this one is pure string mapping and needs no share, and the checker takes
        two directory views and needs no profile.

        The drive is read from the path, never assumed to be C. Anything that
        cannot be mapped unambiguously is refused rather than guessed at: a
        relative or drive-relative path (which resolves against a working
        directory this has no business consulting), a path that is already UNC or
        a `\\?\` device path (which has no administrative share to derive), and
        any root that is not a single drive letter.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw 'Cannot derive an administrative-share path: the path is empty.'
    }
    # Before `GetFullPath`, which would happily normalise a device path into
    # something that still has no administrative share behind it.
    if ($Path.StartsWith('\\') -or $Path.StartsWith('//')) {
        throw ("Cannot derive an administrative-share path from a UNC or device path, because it " +
            "names no local drive: $Path")
    }
    if (-not [IO.Path]::IsPathFullyQualified($Path)) {
        throw ("Cannot derive an administrative-share path from a path that is not fully qualified, " +
            "because it would resolve against a working directory: $Path")
    }
    $full = [IO.Path]::GetFullPath($Path)
    $root = [IO.Path]::GetPathRoot($full)
    if ($root -notmatch '^[A-Za-z]:\\$') {
        throw "Cannot derive an administrative-share path: $Path does not sit on a local drive letter."
    }
    $drive = $root.Substring(0, 1).ToUpperInvariant()
    $remainder = $full.Substring($root.Length).TrimEnd([IO.Path]::DirectorySeparatorChar)
    '\\localhost\' + $drive + '$\' + $remainder
}

function Get-AdminShareRoot {
    # The share itself, for reporting "the share is unavailable" separately from
    # "the two views disagree". They call for different actions by the operator.
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path)
    $root = [IO.Path]::GetPathRoot([IO.Path]::GetFullPath($Path))
    '\\localhost\' + $root.Substring(0, 1).ToUpperInvariant() + '$\'
}

function Test-DirectoryViewIdentity {
    <#
    .SYNOPSIS
        Proves two directory paths address the same storage, in both directions.

    .DESCRIPTION
        Takes both views explicitly rather than deriving one, so the contract can
        be driven against two repository-local directories with no share and no
        profile involved. `Assert-HostProfilePathIdentity` is what pairs a real
        profile root with its administrative share.

        Nothing is enumerated. Every operation names exactly one file.

    .PARAMETER TestOnlyProbeName
        Fixes the probe filename. For the repository-local control only, so it can
        plant a collision and assert an exact absence afterwards.

    .PARAMETER TestOnlyFirstDirectionWriteView
        Sends the first token's write to a third directory, which is the only way
        to build a one-way mirror: a pair of views where the first direction fails
        and the second would still succeed. Without it, a defect that dropped the
        first comparison could not be shown red, because the second comparison
        catches every failure two ordinary directories can produce.

    .PARAMETER TestOnlySecondDirectionWriteView
        The mirror image, for the same reason.

        None of the three reaches the real-host entry point:
        `Assert-HostProfilePathIdentity` does not accept them and
        `Test-HostProfilePathIdentity.ps1` takes no parameters at all.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Label,
        [Parameter(Mandatory)][string]$OrdinaryView,
        [Parameter(Mandatory)][string]$IndependentView,
        [string]$TestOnlyProbeName,
        [string]$TestOnlyFirstDirectionWriteView,
        [string]$TestOnlySecondDirectionWriteView
    )

    # The ordinary view has to be a directory: it is the caller's own path, and a
    # missing one is a different fault from a disagreement between views. The
    # independent view is deliberately *not* pre-checked -- under the redirection
    # this exists to catch, the share-side directory may genuinely not exist, and
    # "the second view never saw the file" is the correct diagnosis for that, not
    # "the share is broken".
    if (-not [IO.Directory]::Exists($OrdinaryView)) {
        throw "${Label}: the ordinary path is not a directory: $OrdinaryView"
    }

    $probeName = if ($TestOnlyProbeName) { $TestOnlyProbeName } else { New-PathIdentityProbeName }
    $firstWriteView = if ($TestOnlyFirstDirectionWriteView) { $TestOnlyFirstDirectionWriteView } else { $OrdinaryView }
    $secondWriteView = if ($TestOnlySecondDirectionWriteView) { $TestOnlySecondDirectionWriteView } else { $IndependentView }

    $ordinaryProbe = Join-Path $OrdinaryView $probeName
    $independentProbe = Join-Path $IndependentView $probeName
    $firstWriteProbe = Join-Path $firstWriteView $probeName
    $secondWriteProbe = Join-Path $secondWriteView $probeName
    # Every name this run could put the probe under, so cleanup covers all of them
    # and never has to enumerate a directory to find out.
    $allProbePaths = @($ordinaryProbe, $independentProbe, $firstWriteProbe, $secondWriteProbe) |
        Select-Object -Unique

    # Refused rather than overwritten. The name is random, so something already
    # there is either a previous run that did not clean up or a collision nothing
    # here can explain -- and writing over it would destroy the evidence.
    foreach ($existing in $allProbePaths) {
        if ([IO.File]::Exists($existing) -or [IO.Directory]::Exists($existing)) {
            throw ("${Label}: the probe name already exists at $existing. Nothing here will overwrite " +
                'it. Remove it by hand and rerun.')
        }
    }

    $tokenA = New-PathIdentityToken
    $tokenB = New-PathIdentityToken
    $probeFailure = $null
    $cleanupFailures = [Collections.Generic.List[string]]::new()

    # Outermost, and it encloses the whole probe. The inner `catch` records the
    # probe's failure instead of letting it propagate, so the combined refusal
    # below is always reached and a cleanup failure can never be lost behind it --
    # and, equally, can never replace it.
    try {
        try {
            [IO.File]::WriteAllText($firstWriteProbe, $tokenA)

            # First direction: the ordinary view wrote it, the independent view
            # must see those exact bytes.
            if (-not [IO.File]::Exists($independentProbe)) {
                throw ("first direction failed: the file written through $firstWriteProbe is not " +
                    "visible at $independentProbe, so these two paths do not address the same storage")
            }
            $readA = [IO.File]::ReadAllText($independentProbe)
            if ($readA -ne $tokenA) {
                throw ("first direction failed: $independentProbe holds different bytes from the ones " +
                    "written through $firstWriteProbe")
            }

            # Second direction, with a different token: a view that only mirrors
            # one way passes the first check and fails here.
            [IO.File]::WriteAllText($secondWriteProbe, $tokenB)
            if (-not [IO.File]::Exists($ordinaryProbe)) {
                throw ("second direction failed: the file written through $secondWriteProbe is not " +
                    "visible at $ordinaryProbe, so these two paths do not address the same storage")
            }
            $readB = [IO.File]::ReadAllText($ordinaryProbe)
            if ($readB -ne $tokenB) {
                throw ("second direction failed: $ordinaryProbe did not take up the bytes written " +
                    "through $secondWriteProbe")
            }
        } catch {
            $probeFailure = $_.Exception.Message
        }
    } finally {
        # Only this one filename, through each view it could have reached. Never a
        # recursive delete, never an enumeration: a cleanup that removed whatever
        # it found in a profile root is a worse failure than the one it is
        # cleaning up after.
        foreach ($path in $allProbePaths) {
            try {
                if ([IO.File]::Exists($path)) { [IO.File]::Delete($path) }
            } catch {
                $cleanupFailures.Add("could not remove $path : $($_.Exception.Message)")
            }
        }
        foreach ($path in $allProbePaths) {
            try {
                if ([IO.File]::Exists($path)) { $cleanupFailures.Add("$path is still present after cleanup") }
            } catch {
                $cleanupFailures.Add("could not confirm $path is gone: $($_.Exception.Message)")
            }
        }
    }

    if ($probeFailure -or $cleanupFailures.Count -gt 0) {
        $parts = @()
        if ($probeFailure) { $parts += $probeFailure }
        # Reported with the probe failure, never instead of it. A probe that
        # refused and then could not clean up is two problems.
        if ($cleanupFailures.Count -gt 0) { $parts += 'cleanup: ' + ($cleanupFailures -join '; ') }
        throw ("${Label}: " + ($parts -join ' | '))
    }

    [pscustomobject]@{
        Label = $Label
        OrdinaryView = $OrdinaryView
        IndependentView = $IndependentView
        ProbeName = $probeName
        FirstDirection = $true
        SecondDirection = $true
        ProbeRemoved = $true
    }
}

function Assert-HostProfilePathIdentity {
    <#
    .SYNOPSIS
        The guard. Both profile roots, both directions, or the caller stops.

    .DESCRIPTION
        Called before the first live-profile or machine-state access in every
        proof that installs for real. It takes no switch that skips it: a proof
        that could be told to trust the environment would be told exactly that on
        the machine where the environment was lying.

    .PARAMETER Context
        Who is asking, so a refusal says which proof stopped.
    #>
    [CmdletBinding()]
    param([string]$Context)

    $prefix = if ($Context) { "$Context : " } else { '' }
    $roots = [ordered]@{
        'APPDATA' = $env:APPDATA
        'LOCALAPPDATA' = $env:LOCALAPPDATA
    }
    $results = [Collections.Generic.List[object]]::new()

    foreach ($name in $roots.Keys) {
        $value = $roots[$name]
        if ([string]::IsNullOrWhiteSpace($value)) {
            throw "$prefix%$name% is not set, so host path identity cannot be established."
        }

        try {
            $share = Get-AdminSharePath -Path $value
            $shareRoot = Get-AdminShareRoot -Path $value
        } catch {
            throw "$prefix%$name% cannot be mapped to an administrative share -- $($_.Exception.Message)"
        }

        # Distinguished from a disagreement between the views, because the two
        # need different things from the operator: this one needs a shell that can
        # open the share, the other needs a shell whose profile is not redirected.
        if (-not [IO.Directory]::Exists($shareRoot)) {
            throw ("$prefix the administrative share $shareRoot is not available, so host path " +
                "identity for %$name% cannot be proved independently. Run this from an ordinary " +
                'Windows Terminal on the host, elevated if the share requires it.')
        }

        try {
            $results.Add((Test-DirectoryViewIdentity -Label "%$name%" -OrdinaryView $value -IndependentView $share))
        } catch {
            throw ("$prefix$($_.Exception.Message) -- this shell's %$name% does not reach ordinary " +
                'host storage, or host identity cannot be proved from it. Do not run live-profile ' +
                'proofs here.')
        }
    }

    foreach ($result in $results) {
        Write-Host ("  host identity: $($result.Label) proved both directions against " +
            "$($result.IndependentView), probe removed")
    }
    $results.ToArray()
}
