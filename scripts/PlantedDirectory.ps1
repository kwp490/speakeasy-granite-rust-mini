#Requires -Version 7
<#
.SYNOPSIS
    The two state helpers a live-profile control needs to plant a directory and
    take it back again: hashing a tree's files, and removing only what the run
    provably created.

.DESCRIPTION
    Extracted from `Test-PreflightRefusalIsInert.ps1` so both can be proved
    against `target\` with no profile, no share and nothing installed;
    `Test-PlantedDirectoryIsScoped.ps1` is that control. The proof script has a
    mandatory `-ArtifactRoot` and asserts host identity as it loads, so nothing
    defined inside it can be exercised without installing for real.

    Two hazards are load-bearing here, and each one produced a defect:

    - **An empty array returned from a function becomes `$null`.** PowerShell
      unrolls a returned collection, so `@()` and "no such directory" arrive at
      the caller identically. `Get-DirectoryState` uses `$null` to mean absent,
      so a root that exists and holds no files read as absent, and a
      postcondition reported the machine unchanged over directories still on
      disk.
    - **`@(...)` is an `object[]` whose slots hold PSObject wrappers.** They
      enumerate as `String`, so the array looks well-typed, but
      `[array]::Copy` into a `string[]` casts slot by slot and throws
      `InvalidCastException`. A PowerShell cast unwraps them instead.
#>

function Get-DirectoryState {
    <#
    .DESCRIPTION
        `$null` means no such directory. An empty array means the directory is
        there and holds no files -- **not the same answer**, and the difference is
        what tells a postcondition that a planted root is still on disk. Every
        file below the root, as `relative-path=SHA-256`, sorted.
    #>
    param([Parameter(Mandatory)][string]$Root)
    if (-not (Test-Path -LiteralPath $Root -PathType Container)) { return $null }
    # The comma keeps an empty result an array rather than letting it unroll to
    # `$null`, which is this function's answer for a directory that is not there.
    ,@(Get-ChildItem -LiteralPath $Root -Recurse -File -Force |
        ForEach-Object { '{0}={1}' -f $_.FullName.Substring($Root.Length + 1), (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash } |
        Sort-Object)
}

function Remove-PlantedDirectoryEntries {
    <#
    .DESCRIPTION
        Refuses unless the caller's own marker file is still there holding this
        run's value, then removes exactly the files named and the directories
        named, deepest first and non-recursively. A directory holding anything
        else refuses to go and names what is in it.

        Keyed on an explicit list and a marker rather than on a "this control
        created it" flag: a recursive delete of an install root on that belief
        removes whatever else arrived while the run was in progress, and the
        point of the case being proved is that an unregistered directory belongs
        to nobody provable.

        `$Directories` is deliberately `object[]`: callers build it with `@(...)`
        and the normalising cast belongs here, once, rather than at each call.
    #>
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$MarkerName,
        [Parameter(Mandatory)][string]$MarkerValue,
        [Parameter(Mandatory)][object[]]$Files,
        [Parameter(Mandatory)][object[]]$Directories
    )
    $markerPath = Join-Path $Root $MarkerName
    if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
        throw "There is no control marker at $markerPath, so nothing here is provably this control's; removing nothing."
    }
    $found = (Get-Content -LiteralPath $markerPath -Raw).Trim()
    if ($found -ne $MarkerValue) {
        throw "The marker at $markerPath reads '$found', not this run's '$MarkerValue'; removing nothing."
    }
    foreach ($file in [string[]]$Files) {
        if (Test-Path -LiteralPath $file -PathType Leaf) { Remove-Item -LiteralPath $file -Force }
    }
    # A PowerShell cast, not `[array]::Copy` into a `string[]`: see the PSObject
    # hazard above. The failure lands after the marker has been deleted, leaving
    # the directories behind and no marker to authorise a second attempt.
    $deepestFirst = [string[]]$Directories
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
