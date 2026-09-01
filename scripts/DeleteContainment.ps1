#Requires -Version 7
<#
.SYNOPSIS
    What has to be true of a directory before anything deletes it recursively.

.DESCRIPTION
    Dot-sourced by the recursive deletes that have been migrated to it:
    `Build-LocalInstaller.ps1`'s artifact root, `Invoke-ProofPackage.ps1`'s proof
    stage, and `Test-CleanupFailureRestoresConfig.ps1`'s scratch directory. It is
    **not** yet used everywhere. `Invoke-ProofPackage.ps1`'s `-FreshBuild` build
    root carries an equivalent check inline; `Test-InstallerLifecycle.ps1` and
    `Test-StaleArtifactRefusal.ps1` have not been migrated. New or changed
    recursive deletes come here.

    A textual prefix check is not containment: `GetFullPath` collapses `..` but
    says nothing about reparse points, and a junction on an *ancestor* sends the
    delete out of the workspace while every path string still reads as though it
    were inside.

    So the whole chain is walked -- the root, then each component of the relative
    path -- and any existing component that is a reparse point, or a file where a
    directory belongs, refuses the whole operation. Equality with the root refuses
    too: removing the root would take every sibling with it.

    **Refuse rather than follow.** These functions throw; they never fall back to
    deleting something narrower, because a caller that asked to remove the wrong
    directory has a bug the delete would hide.
#>

function Resolve-ContainedDirectory {
    <#
    .PARAMETER Path
        The directory a caller intends to remove or overwrite. Must be absolute:
        `GetFullPath` resolves a relative path against the *process* working
        directory, which `Set-Location` does not move.

    .PARAMETER Root
        The directory the path must stay strictly beneath.

    .PARAMETER Label
        How the path is named in a refusal, so the message says which argument
        was wrong rather than only which path.

    .PARAMETER DirectChildOnly
        Require exactly one level. Used where nesting buys nothing, because a
        path with no ancestors between it and the root has no ancestor a junction
        can sit on.

    .OUTPUTS
        The resolved, validated full path.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$Label,
        [switch]$DirectChildOnly
    )
    $separator = [IO.Path]::DirectorySeparatorChar
    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd($separator)
    $full = [IO.Path]::GetFullPath($Path).TrimEnd($separator)

    if (-not (Test-Path -LiteralPath $rootFull -PathType Container)) {
        throw ($Label + ' cannot be contained because its root is not a directory: ' + $rootFull + '.')
    }
    if ($full -eq $rootFull) {
        throw ($Label + ' must not be ' + $rootFull +
            ' itself; removing it would take everything beside it.')
    }
    if (-not $full.StartsWith($rootFull + $separator, [StringComparison]::OrdinalIgnoreCase)) {
        throw ($Label + ' must resolve beneath ' + $rootFull + '; got ' + $full + '.')
    }

    # `[char[]]` is load-bearing. An `@(...)` of chars binds to the
    # `Split(string, StringSplitOptions)` overload, which looks for the two
    # characters as one literal sequence, finds nothing, and returns the whole
    # relative path as a single segment -- so every intermediate directory goes
    # unchecked and a junction on an ancestor is accepted.
    $segments = $full.Substring($rootFull.Length + 1).Split(
        [char[]]@($separator, [IO.Path]::AltDirectorySeparatorChar), [StringSplitOptions]::None)
    if ($DirectChildOnly -and $segments.Count -ne 1) {
        throw ($Label + ' must be exactly one directory beneath ' + $rootFull +
            ' so no junction can sit on an ancestor of it; got ' + $full + '.')
    }

    # The root is checked as well as the components below it. A junction at the
    # root resolves the whole chain somewhere else, and every component under it
    # would still look ordinary.
    $chain = [Collections.Generic.List[string]]::new()
    $chain.Add($rootFull)
    $walked = $rootFull
    foreach ($segment in $segments) {
        if ([string]::IsNullOrWhiteSpace($segment)) {
            throw ($Label + ' has an empty path component: ' + $full + '.')
        }
        $walked = Join-Path $walked $segment
        $chain.Add($walked)
    }
    foreach ($component in $chain) {
        if (-not (Test-Path -LiteralPath $component)) { continue }
        $item = Get-Item -LiteralPath $component -Force
        if ($item -isnot [IO.DirectoryInfo]) {
            throw ($Label + ' passes through a file where a directory must be: ' + $component + '.')
        }
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw ($Label + ' passes through a reparse point and may resolve outside ' +
                $rootFull + ': ' + $component + '.')
        }
    }
    $full
}

function Remove-ContainedDirectory {
    <#
    .DESCRIPTION
        Validate, then remove. Absent is success: the caller wanted the directory
        gone and it is.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$Label,
        [switch]$DirectChildOnly
    )
    $full = Resolve-ContainedDirectory -Path $Path -Root $Root -Label $Label -DirectChildOnly:$DirectChildOnly
    if (Test-Path -LiteralPath $full) {
        Remove-Item -LiteralPath $full -Recurse -Force
    }
    $full
}
