#Requires -Version 7
<#
.SYNOPSIS
    Capture and restore SpeakEasy Mini's `config\` around a proof that installs
    for real.

.DESCRIPTION
    Dot-sourced by `Test-SetupWizard.ps1` and `Test-PreflightRefusalIsInert.ps1`,
    and driven against a scratch directory by `Test-ProfileCaptureIsScoped.ps1`,
    so the contract is provable without a profile. Captured rather than
    redirected because `APPDATA` moves setup but not the installed app, which
    resolves its own directory through `SHGetKnownFolderPath`.

    - Only ordinary files are read, written or removed; the config root and every
      captured or tracked name is classified at capture and again at restore.
    - A reparse point is refused, never followed.
    - `Get-TrackedConfigFileNames` lists every name a run can create, so one the
      run produced is removed again. A tracked name that turns out to be a
      directory or a link is reported, never treated as absent or as restored.
    - Unknown entries of any kind are left as found and reported. Deleting an
      unrecognised entry is how a tidy-up removes the operator's data.
    - Recognised files are restored before anything is reported.

    `logs\speakeasy.log` and the SQLite side files are out of scope: the run
    appends to the log on purpose.
#>

function Get-TrackedConfigFileNames {
    # Every name a wizard run can create: the five seeds setup writes and the four
    # profile files the app writes on its first start.
    @(
        'install-hotkey.txt'
        'install-logging.txt'
        'install-provider.txt'
        'install-retention.txt'
        'install-vocabulary.txt'
        'personalization.json'
        'personalization.json.bak'
        'settings.json'
        'settings.json.bak'
    )
}

function Get-InstallSeedFileNames {
    # The subset setup writes and the app consumes.
    @(
        'install-hotkey.txt'
        'install-logging.txt'
        'install-provider.txt'
        'install-retention.txt'
        'install-vocabulary.txt'
    )
}

function Get-OrphanEntryKind {
    # A dangling link is still an entry in its parent's listing even when
    # `Get-Item` will not resolve it. Returns a kind, or `$null` when the parent
    # really has no such name.
    param([Parameter(Mandatory)][string]$Path)
    $full = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($full)
    $leaf = [IO.Path]::GetFileName($full)
    if ([string]::IsNullOrEmpty($parent) -or [string]::IsNullOrEmpty($leaf)) { return $null }
    if (-not [IO.Directory]::Exists($parent)) { return $null }
    foreach ($entry in [IO.Directory]::EnumerateFileSystemEntries($parent, $leaf)) {
        $attributes = [IO.File]::GetAttributes($entry)
        if ($attributes -band [IO.FileAttributes]::ReparsePoint) { return 'ReparsePoint' }
        if ($attributes -band [IO.FileAttributes]::Directory) { return 'Directory' }
        return 'File'
    }
    $null
}

function Get-ConfigEntryKind {
    param([Parameter(Mandatory)][string]$Path)
    # `Get-Item` before `Test-Path`: `Test-Path` resolves a link and answers about
    # the *target*, so a dangling link would read as Missing -- and Missing is
    # what licenses a restore to write there, through the link.
    #
    # **Only a genuinely absent item is Missing.** Access denied, an invalid path,
    # a provider failure: those throw, naming the path. Folding them into Missing
    # would license that same write on a path nothing could classify.
    $item = $null
    try {
        $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    } catch [Management.Automation.ItemNotFoundException] {
        $orphan = Get-OrphanEntryKind -Path $Path
        if ($null -eq $orphan) { return 'Missing' }
        return $orphan
    } catch {
        throw ('Cannot classify ' + $Path + ': ' + $_.Exception.GetType().FullName + ' -- ' +
            $_.Exception.Message)
    }
    # Reparse first: a junction is also a `DirectoryInfo`, and it is the attribute
    # that decides whether following the path leaves the directory.
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { return 'ReparsePoint' }
    if ($item -is [IO.DirectoryInfo]) { return 'Directory' }
    if ($item -is [IO.FileInfo]) { return 'File' }
    'Other'
}

function Get-ConfigPathChain {
    # Root first, one entry per component, with the filesystem root preserved
    # exactly.
    #
    # `DirectoryInfo.Parent` rather than string trimming: trimming the separator
    # turns `C:\` into `C:`, which is drive-*relative*, and `Get-Item C:` then
    # answers about whatever directory the process last used on that drive -- a
    # refusal naming a path the caller never gave. The parent walk also ends at a
    # UNC share root without inventing partial paths.
    param([Parameter(Mandatory)][string]$Path)
    $chain = [Collections.Generic.List[string]]::new()
    $node = [IO.DirectoryInfo]::new([IO.Path]::GetFullPath($Path))
    while ($null -ne $node) {
        $chain.Insert(0, $node.FullName)
        $node = $node.Parent
    }
    $chain.ToArray()
}

function Assert-OrdinaryConfigRoot {
    # **Every existing component of the absolute path, not just the leaf.** A
    # junction on an ancestor sends every read and write below it somewhere else
    # while the path string still reads as though it were inside.
    #
    # This finds reparse points, and only reparse points. An alias the filesystem
    # does not represent as one -- a container redirecting `%APPDATA%`, say --
    # is invisible here and is covered only by the host write probe.
    param([Parameter(Mandatory)][string]$ConfigRoot, [Parameter(Mandatory)][string]$Stage)
    $chain = @(Get-ConfigPathChain -Path $ConfigRoot)
    $full = $chain[-1]

    if ($chain.Count -gt 1) {
        foreach ($ancestor in $chain[0..($chain.Count - 2)]) {
            switch (Get-ConfigEntryKind -Path $ancestor) {
                'Missing' { }
                'Directory' { }
                'ReparsePoint' {
                    throw ("${Stage}: $ancestor is a reparse point on the way to the config root, " +
                        'so nothing here will read or write through it.')
                }
                default {
                    throw ("${Stage}: $ancestor is a $_ where a directory must be, on the way to " +
                        'the config root.')
                }
            }
        }
    }

    # Absent is fine -- a profile that has never been written.
    switch (Get-ConfigEntryKind -Path $full) {
        'Missing' { return }
        'Directory' { return }
        default {
            throw ("${Stage}: the config root is a $_ rather than an ordinary directory, so nothing " +
                "here will read or write through it: $full")
        }
    }
}

function Get-ConfigCapture {
    param(
        [Parameter(Mandatory)][string]$ConfigRoot,
        [Parameter(Mandatory)][string[]]$AlsoTrack
    )
    Assert-OrdinaryConfigRoot -ConfigRoot $ConfigRoot -Stage 'Config capture'

    $names = [Collections.Generic.List[string]]::new()
    if (Test-Path -LiteralPath $ConfigRoot -PathType Container) {
        # Every entry, not only files: a directory or junction sitting in `config\`
        # has to be seen now so it can be left alone later rather than discovered
        # by a delete.
        foreach ($entry in (Get-ChildItem -LiteralPath $ConfigRoot -Force | Sort-Object Name)) {
            $names.Add($entry.Name)
        }
    }
    foreach ($name in $AlsoTrack) {
        if (-not $names.Contains($name)) { $names.Add($name) }
    }

    $capture = [ordered]@{}
    $refused = @()
    foreach ($name in ($names | Sort-Object)) {
        $path = Join-Path $ConfigRoot $name
        $kind = Get-ConfigEntryKind -Path $path
        $tracked = $AlsoTrack -contains $name
        switch ($kind) {
            'File' {
                # One read, and the digest computed from those same bytes. Reading
                # twice -- once for the content, once for `Get-FileHash` -- lets a
                # write between them produce a capture whose digest describes a
                # file the bytes do not. In memory, not a copy on disk: a sibling
                # file would be one more thing an aborted run could leave behind.
                $bytes = [IO.File]::ReadAllBytes($path)
                $capture[$name] = [pscustomobject]@{
                    Existed = $true
                    Kind = 'File'
                    Bytes = $bytes
                    Sha256 = [BitConverter]::ToString(
                        [Security.Cryptography.SHA256]::HashData($bytes)).Replace('-', '')
                }
            }
            'Missing' {
                $capture[$name] = [pscustomobject]@{
                    Existed = $false; Kind = 'Missing'; Bytes = $null; Sha256 = $null
                }
            }
            default {
                # A tracked name that is not a file cannot be captured, and must
                # not be recorded as absent -- "absent" is what licenses the
                # restore to delete it. Refuse the run instead.
                if ($tracked) { $refused += "$name is a $kind" }
                # An untracked one is simply not captured. It is reported at
                # restore, when it is clear whether it was there all along.
            }
        }
    }
    if ($refused.Count -gt 0) {
        throw ('Config capture refused: ' + ($refused -join '; ') +
            ". A name a run can create must be an ordinary file or absent; nothing here will " +
            "read or write through it. Resolve it by hand in $ConfigRoot and rerun.")
    }
    $capture
}

function Restore-ConfigCapture {
    param(
        [Parameter(Mandatory)][string]$ConfigRoot,
        [Parameter(Mandatory)]$Capture
    )
    # Revalidated: the capture describes the disk as it was before the run, and
    # the run is what may have replaced the directory with something else.
    Assert-OrdinaryConfigRoot -ConfigRoot $ConfigRoot -Stage 'Config restore'

    $changed = @()
    $blocked = @()
    $blockedNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($name in $Capture.Keys) {
        $path = Join-Path $ConfigRoot $name
        $before = $Capture[$name]
        $kind = Get-ConfigEntryKind -Path $path
        if ($kind -notin 'File', 'Missing') {
            # Neither written through nor removed. Recorded so the postcondition
            # below does not read this as restored.
            $blocked += "$name is now a $kind"
            [void]$blockedNames.Add($name)
            continue
        }
        if ($before.Existed) {
            $currentHash = if ($kind -eq 'File') {
                (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
            } else { $null }
            if ($currentHash -ne $before.Sha256) {
                if (-not (Test-Path -LiteralPath $ConfigRoot -PathType Container)) {
                    New-Item -ItemType Directory -Path $ConfigRoot -Force | Out-Null
                }
                [IO.File]::WriteAllBytes($path, $before.Bytes)
                $changed += $name
            }
        } elseif ($kind -eq 'File') {
            Remove-Item -LiteralPath $path -Force
            $changed += "$name (created by the run, removed)"
        }
    }
    if ($changed.Count -gt 0) {
        Write-Host "  config: restored $($changed -join '; ')"
    } else {
        Write-Host '  config: the run changed none of it'
    }

    # Asserted, because a restore that silently failed would look like a clean run.
    $wrong = @()
    foreach ($name in $Capture.Keys) {
        if ($blockedNames.Contains($name)) { continue }
        $path = Join-Path $ConfigRoot $name
        $before = $Capture[$name]
        $exists = Test-Path -LiteralPath $path -PathType Leaf
        if ($before.Existed) {
            if (-not $exists) { $wrong += "$name is missing" }
            else {
                $after = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
                if ($after -ne $before.Sha256) { $wrong += "$name is $after, captured $($before.Sha256)" }
            }
        } elseif ($exists) {
            $wrong += "$name exists and did not before the run"
        }
    }
    if ($wrong.Count -gt 0) {
        throw ('Config restore failed: ' + ($wrong -join '; ') + '.')
    }

    # Last, and reported rather than acted on -- of every kind. The capture covers
    # every entry that was there and every name a run creates, so anything here is
    # unanticipated, which is exactly when guessing is worst.
    $unknown = @()
    if (Test-Path -LiteralPath $ConfigRoot -PathType Container) {
        foreach ($entry in (Get-ChildItem -LiteralPath $ConfigRoot -Force | Sort-Object Name)) {
            if (-not $Capture.Contains($entry.Name)) {
                $unknown += "$($entry.Name) [$(Get-ConfigEntryKind -Path $entry.FullName)]"
            }
        }
    }
    if ($blocked.Count -gt 0 -or $unknown.Count -gt 0) {
        $parts = @()
        if ($blocked.Count -gt 0) { $parts += 'tracked names that are no longer files: ' + ($blocked -join '; ') }
        if ($unknown.Count -gt 0) { $parts += 'unrecognised entries: ' + ($unknown -join '; ') }
        throw ('Config restore left entries in place, deliberately -- ' + ($parts -join ' | ') +
            ". Nothing here was followed, written through or deleted. Check them by hand in " +
            "$ConfigRoot, and add a name to Get-TrackedConfigFileNames if a run is meant to " +
            'produce it.')
    }
    Write-Host '  config: every captured file byte-identical to the pre-run capture'
}
