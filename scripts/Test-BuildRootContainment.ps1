#Requires -Version 7
<#
.SYNOPSIS
    Proves `Invoke-ProofPackage.ps1` refuses an unsafe build root before deleting
    anything.

.DESCRIPTION
    `-FreshBuild` removes and recreates its build root, so what bounds that
    recursive delete is the validation in front of it. Checking only whether the
    final directory is a reparse point is not enough: a nested path can have a
    junction on an *ancestor*, and the delete would then follow it out of the
    workspace while every path string still reads as though it were inside.

    The build root is therefore required to be exactly one directory beneath the
    resolved `target\`, with no reparse point on either end.

    This proves the refusals with a real junction, and proves them **safely**: a
    canary file is placed outside `target\`, a junction inside `target\` points at
    its directory, and each refused input is passed to `-FreshBuild`. Every case
    must throw, and the canary must still be there afterwards -- so the test can
    only pass if the refusal happened *before* the delete rather than after it.

    No case is expected to delete anything, so nothing is deleted through the
    junction at any point.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$targetRoot = (Resolve-Path -LiteralPath (Join-Path $repositoryRoot 'target')).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
$proofPackage = Join-Path $PSScriptRoot 'Invoke-ProofPackage.ps1'

# Outside `target\`, so following the junction would leave the directory the
# delete was bounded to -- which is the whole hazard. Under `.tools\`, which is
# git-ignored and on the project drive, rather than under `%TEMP%`: a proof that
# needs the profile cannot be run from an environment whose profile is redirected,
# and this one does not need it. Test-owned either way, so even the failure mode
# stays inside a directory this control created.
$toolsRoot = Join-Path $repositoryRoot '.tools'
$canaryRoot = Join-Path $toolsRoot ('containment-canary-' + [Guid]::NewGuid().ToString('N'))
$canaryFile = Join-Path $canaryRoot 'do-not-delete-me.txt'
$junction = Join-Path $targetRoot ('containment-junction-' + $PID)
$createdCanary = $false
$createdToolsRoot = $false
$createdJunction = $false

# The rule this control enforces, applied to its own scratch space, and written
# out here rather than called from `DeleteContainment.ps1`: a control must not
# depend on the code it is testing to clean up after it.
function Assert-OrdinaryDirectoryUnderRepository {
    param([Parameter(Mandatory)][string]$Path)
    $full = [IO.Path]::GetFullPath($Path).TrimEnd([IO.Path]::DirectorySeparatorChar)
    $rootFull = [IO.Path]::GetFullPath($script:repositoryRoot).TrimEnd([IO.Path]::DirectorySeparatorChar)
    if (-not $full.StartsWith($rootFull + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$full is not beneath $rootFull."
    }
    $item = Get-Item -LiteralPath $full -Force
    if ($item -isnot [IO.DirectoryInfo]) { throw "$full is not a directory." }
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "$full is a reparse point; this control will not create anything under it."
    }
}

function Assert-Refused {
    param(
        [Parameter(Mandatory)][string]$Scenario,
        [Parameter(Mandatory)][string]$BuildRoot,
        [Parameter(Mandatory)][string]$CanaryFile
    )
    $threw = $false
    $message = $null
    try {
        & $proofPackage -FreshBuild -BuildRoot $BuildRoot
    } catch {
        $threw = $true
        $message = $_.Exception.Message
    }
    if (-not $threw) {
        throw "${Scenario}: -FreshBuild accepted '$BuildRoot' instead of refusing it."
    }
    # The refusal has to be the containment check, not something incidental like a
    # missing toolchain, or this proves nothing about the delete.
    if ($message -notmatch 'BuildRoot|reparse point|directory') {
        throw "${Scenario}: refused for an unrelated reason, so containment is unproved: $message"
    }
    if (-not (Test-Path -LiteralPath $CanaryFile -PathType Leaf)) {
        throw "${Scenario}: the canary outside target\ is gone; the delete ran before the refusal."
    }
    Write-Host "  ${Scenario}: refused, canary intact -- $message"
}

Push-Location $repositoryRoot
try {
    # Whether this run created `.tools\` decides whether it may remove it. A
    # pre-existing one holds the staged toolchain and is never this control's.
    if (-not (Test-Path -LiteralPath $toolsRoot -PathType Container)) {
        New-Item -ItemType Directory -Path $toolsRoot -Force | Out-Null
        $createdToolsRoot = $true
    }
    Assert-OrdinaryDirectoryUnderRepository -Path $toolsRoot
    if (Test-Path -LiteralPath $canaryRoot) {
        throw "The canary directory already exists, which a fresh GUID makes impossible: $canaryRoot"
    }
    [IO.Directory]::CreateDirectory($canaryRoot) | Out-Null
    $createdCanary = $true
    [IO.File]::WriteAllText($canaryFile, 'This file is outside target\ and must survive every case.')
    $canaryHash = (Get-FileHash -LiteralPath $canaryFile -Algorithm SHA256).Hash
    Write-Host "canary: $canaryFile"
    Write-Host "  sha256: $canaryHash"

    if (Test-Path -LiteralPath $junction) {
        throw "A previous run left $junction behind; remove it by hand and rerun."
    }
    New-Item -ItemType Junction -Path $junction -Target $canaryRoot | Out-Null
    $createdJunction = $true
    Write-Host "junction: $junction -> $canaryRoot"
    $junctionItem = Get-Item -LiteralPath $junction -Force
    if (-not ($junctionItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'The junction was created without a reparse-point attribute; this test cannot prove anything.'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $junction 'do-not-delete-me.txt') -PathType Leaf)) {
        throw 'The junction does not resolve to the canary directory; this test cannot prove anything.'
    }
    Write-Host '  the junction resolves outside target\, as the hazard requires'

    Write-Host ''
    # The leaf itself is a reparse point.
    Assert-Refused -Scenario 'the build root is a junction' -BuildRoot $junction -CanaryFile $canaryFile
    # Nested one level under the junction: the leaf is not a reparse point, its
    # ancestor is. This is the case a leaf-only check would let through.
    Assert-Refused -Scenario 'a junction on an ancestor' -BuildRoot (Join-Path $junction 'nested') -CanaryFile $canaryFile
    # Nested without any junction: still refused, because depth is what allows the
    # ancestor case to exist at all.
    Assert-Refused -Scenario 'nested two levels under target' -BuildRoot (Join-Path $targetRoot 'a\b') -CanaryFile $canaryFile
    # The target root itself, which would take every other build with it.
    Assert-Refused -Scenario 'the target root itself' -BuildRoot $targetRoot -CanaryFile $canaryFile
    # Outside the workspace entirely.
    Assert-Refused -Scenario 'outside the workspace' -BuildRoot $canaryRoot -CanaryFile $canaryFile
    # An escaping relative path, which `GetFullPath` collapses before the check.
    Assert-Refused -Scenario 'escaping with ..' -BuildRoot (Join-Path $targetRoot '..\..\escaped') -CanaryFile $canaryFile

    $finalHash = (Get-FileHash -LiteralPath $canaryFile -Algorithm SHA256).Hash
    if ($finalHash -ne $canaryHash) {
        throw "The canary changed: $finalHash, was $canaryHash."
    }
    Write-Host ''
    Write-Host "canary still $finalHash after every refusal"
    Write-Host 'build-root containment: passed'
}
finally {
    Pop-Location
    # The junction goes first, and with the .NET call rather than `Remove-Item`:
    # deleting the link must not recurse into the directory it points at.
    if ($createdJunction -and (Test-Path -LiteralPath $junction)) {
        [IO.Directory]::Delete($junction, $false)
        Write-Host "removed the junction; canary directory still present: $([IO.Directory]::Exists($canaryRoot))"
    }
    # The canary file by name, then its directory non-recursively. A control that
    # recursively deletes its own scratch space is not obeying the rule it exists
    # to enforce, and `Directory.Delete($path, $false)` refuses a directory that
    # holds anything unexpected rather than taking it along.
    if ($createdCanary) {
        if (Test-Path -LiteralPath $canaryFile -PathType Leaf) {
            Remove-Item -LiteralPath $canaryFile -Force
        }
        if ([IO.Directory]::Exists($canaryRoot)) {
            $left = @(Get-ChildItem -LiteralPath $canaryRoot -Force)
            if ($left.Count -gt 0) {
                Write-Warning ("$canaryRoot holds $($left.Count) unexpected entr(ies) and stays: " +
                    ((@($left | ForEach-Object { $_.Name })) -join ', ')) -WarningAction Continue
            } else {
                [IO.Directory]::Delete($canaryRoot, $false)
            }
        }
    }
    # `.tools\` only if this run made it, only non-recursively, and only while it
    # is still empty. A pre-existing one holds the staged toolchain.
    if ($createdToolsRoot -and [IO.Directory]::Exists($toolsRoot)) {
        $leftInTools = @(Get-ChildItem -LiteralPath $toolsRoot -Force)
        if ($leftInTools.Count -eq 0) {
            [IO.Directory]::Delete($toolsRoot, $false)
            Write-Host 'removed the empty .tools directory this run created'
        } else {
            Write-Host ".tools holds $($leftInTools.Count) entr(ies) and stays"
        }
    }
}
