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
    storage at `C:\Users\<name>\AppData\Roaming` with nothing inside that view to
    show it: file counts, directory sizes, hashes of files already there,
    reparse-point status and path-string equality all pass under exactly the
    redirection this exists to catch. The only evidence is a write a second,
    independently obtained view of the disk can see, and a write back through that
    second view the first one can see in turn.

    The second view is the drive's administrative share, `\\localhost\<drive>$`,
    which the SMB server resolves against the real volume rather than against the
    calling process's view of the namespace. Elevation is not asserted as a
    separate condition -- if the share opens, it opens; if it does not, that is
    reported as its own failure rather than inferred from a token check.

    - **Both roots, both directions, every run.** `%APPDATA%` and `%LOCALAPPDATA%`
      are probed independently; a pass means each of them carried a token out and
      a different token back.
    - **One file, named once, from a CSPRNG.** Nothing here enumerates either
      directory, reads anything already in it, or creates a product directory.
    - **Cleanup removes an ordinary file and nothing else.** Every probe path is
      classified before and after; only a `File` is deleted, anything else is left
      where it is and reported, and the path has to classify `Missing` afterwards.
      A cleanup that failed fails the whole check rather than being warned about.
    - **Fail closed.** Every refusal names the root and the direction, an entry
      that cannot be classified is a refusal rather than an absence, and there is
      no switch that skips the check -- a caller that could skip it would skip it
      on the machine where it mattered.
#>

function New-PathIdentityProbeName {
    <#
    .DESCRIPTION
        Cryptographically random, so a collision with a product file, with a file
        an earlier run left behind, or with anything an operator has is negligibly
        unlikely rather than impossible. An exact entry that is already there is
        refused without being read, written or removed.
    #>
    $bytes = [byte[]]::new(24)
    [Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    'speakeasy-mini-path-identity-' + [Convert]::ToHexString($bytes).ToLowerInvariant() + '.probe'
}

function Get-ExactEntryKind {
    <#
    .SYNOPSIS
        What is at exactly this path: `Missing`, `File`, `Directory` or
        `ReparsePoint`. Anything unclassifiable throws.

    .DESCRIPTION
        **`File.Exists` is not an absence test.** It answers false for a
        directory, and false again for a path it could not inspect at all, so a
        directory standing where the probe file was -- or a path that has become
        unreadable -- would read as "successfully removed".

        `File.GetAttributes` inspects the one path given and nothing else: no
        parent enumeration, and no following of a reparse point, because
        `GetFileAttributesEx` reports the link's own attributes. `Test-Path` is
        not used because it resolves a link and answers about the *target*, so a
        dangling link would read as Missing -- and Missing is what licenses a
        delete or a "confirmed gone".

        **Only a genuine not-found is `Missing`.** Access denied, a malformed
        path, a provider or I/O failure: those throw, naming the path and the
        exception type. Folding them into `Missing` is the defect this exists to
        remove.

        `ProfileCapture.ps1` carries an equivalent classifier for `config\`, with
        the same reasoning and a different call site; a change to either is worth
        checking against the other.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path)
    try {
        $attributes = [IO.File]::GetAttributes($Path)
    } catch [IO.FileNotFoundException] {
        return 'Missing'
    } catch [IO.DirectoryNotFoundException] {
        # A parent that is not there means this exact path holds nothing, which is
        # the question being asked. It is not an inability to answer it.
        return 'Missing'
    } catch {
        # Unwrapped, because PowerShell wraps a failure thrown by a .NET method in
        # a `MethodInvocationException` and naming that says nothing about what
        # went wrong. The typed catches above match the inner type already.
        $cause = if ($_.Exception -is [Management.Automation.MethodInvocationException] -and
            $null -ne $_.Exception.InnerException) { $_.Exception.InnerException } else { $_.Exception }
        throw ('Cannot classify ' + $Path + ': ' + $cause.GetType().FullName + ' -- ' + $cause.Message)
    }
    # Reparse first: a junction is also a directory, and the attribute is what
    # decides whether following the path leaves the view.
    if ($attributes -band [IO.FileAttributes]::ReparsePoint) { return 'ReparsePoint' }
    if ($attributes -band [IO.FileAttributes]::Directory) { return 'Directory' }
    'File'
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

    .PARAMETER TestOnlyBeforeCleanup
        Runs between the probe and the cleanup, receiving the ordinary probe path.
        Lets the control replace the probe file with a directory at the moment
        cleanup is about to run, which is the only deterministic way to prove that
        an entry of the wrong kind is refused rather than read as absent.

        None of the four reaches the real-host entry point:
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
        [string]$TestOnlySecondDirectionWriteView,
        [scriptblock]$TestOnlyBeforeCleanup
    )

    # The ordinary view has to be a directory: it is the caller's own path, and a
    # missing one is a different fault from a disagreement between views. Through
    # the classifier, so a view that cannot be inspected refuses rather than
    # reading as "not a directory". A reparse point passes, because what a link
    # resolves to is exactly what the token test below decides.
    #
    # The independent view is deliberately *not* pre-checked -- under the
    # redirection this exists to catch, the share-side directory may genuinely not
    # exist, and "the second view never saw the file" is the correct diagnosis for
    # that, not "the share is broken".
    $ordinaryViewKind = Get-ExactEntryKind -Path $OrdinaryView
    if ($ordinaryViewKind -notin @('Directory', 'ReparsePoint')) {
        throw "${Label}: the ordinary path is a $ordinaryViewKind rather than a directory: $OrdinaryView"
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

    # Refused rather than overwritten, whatever kind of entry it is. The name is
    # random, so something already there is either a run that did not clean up or
    # a collision nothing here can explain, and writing over it would destroy the
    # evidence. Classified rather than tested for existence: a path that cannot be
    # inspected must refuse, not read as free.
    foreach ($existing in $allProbePaths) {
        $existingKind = Get-ExactEntryKind -Path $existing
        if ($existingKind -ne 'Missing') {
            throw ("${Label}: the probe name already exists at $existing as a $existingKind. Nothing " +
                'here will overwrite or remove it. Resolve it by hand and rerun.')
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
        if ($TestOnlyBeforeCleanup) {
            try {
                $null = & $TestOnlyBeforeCleanup $ordinaryProbe
            } catch {
                $cleanupFailures.Add("the before-cleanup test seam threw: $($_.Exception.Message)")
            }
        }
        foreach ($path in $allProbePaths) {
            $kind = $null
            try {
                $kind = Get-ExactEntryKind -Path $path
            } catch {
                $cleanupFailures.Add("could not classify $path : $($_.Exception.Message)")
                continue
            }
            if ($kind -eq 'Missing') { continue }
            if ($kind -ne 'File') {
                # A directory, a link, or anything else standing at the probe name
                # is not the file this run wrote, so it is not this run's to
                # remove. Reported and left exactly as found: the probe cannot be
                # accounted for, which is the thing the caller has to know.
                $cleanupFailures.Add("$path is a $kind and was left in place; the probe file this run " +
                    'wrote there cannot be accounted for')
                continue
            }
            try {
                [IO.File]::Delete($path)
            } catch {
                $cleanupFailures.Add("could not remove $path : $($_.Exception.Message)")
                continue
            }
            # Classified again rather than tested for existence. `File.Exists`
            # answers false for a directory and false for a path it could not
            # inspect, so it would report either as removed.
            $after = $null
            try {
                $after = Get-ExactEntryKind -Path $path
            } catch {
                $cleanupFailures.Add("could not confirm $path is gone: $($_.Exception.Message)")
                continue
            }
            if ($after -ne 'Missing') {
                $cleanupFailures.Add("$path classifies as $after after it was removed")
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
