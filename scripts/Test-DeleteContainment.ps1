#Requires -Version 7
<#
.SYNOPSIS
    Proves the recursive deletes outside `Invoke-ProofPackage.ps1`'s build root
    refuse an unsafe path before deleting anything.

.DESCRIPTION
    `Test-BuildRootContainment.ps1` covers `-BuildRoot`. Three other recursive
    deletes are reachable in this repository, and each one computes its path:

    - `Build-LocalInstaller.ps1` removes `-ArtifactRoot`, which is caller-supplied
      and nested by design (`local-development\<version>`);
    - `Invoke-ProofPackage.ps1` removes `apps\desktop\src-tauri\proof-stage`,
      which is a fixed path -- and a fixed path is not a safe path, because a
      junction can be created at it or at any directory above it;
    - `Test-CleanupFailureRestoresConfig.ps1` removes its own scratch directory.

    All three now go through `DeleteContainment.ps1`, which walks the root and
    every existing component of the relative path and refuses a reparse point, a
    file where a directory belongs, or the root itself.

    **The red case never reaches outside a test-owned directory.** The canary
    lives under `.tools\`, in a directory this control creates and removes; the
    junction points at it. So a refusal that failed to happen would destroy a
    directory this control owns, and the canary assertion is what reports it --
    rather than the failure being demonstrated on something the workspace does
    not own.

    Two layers are asserted: the shared validator directly, with junctions, and
    the two scripts end to end, so a script that stopped calling the validator is
    caught rather than covered by the validator's own passing tests. A last case
    turns the rule on this script: a pre-existing `proof-stage` junction must make
    it refuse, and must still be there afterwards.

    **This control cleans up by hand and not through the validator.** A red run
    mutates `DeleteContainment.ps1`, and a cleanup that called the mutated code
    would be trusting the thing under test to tidy up after proving it broken.
    Every removal here is an exact path: the canary file by name, then its
    directory non-recursively, which refuses if anything unexpected is in it.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'DeleteContainment.ps1')
$targetRoot = (Resolve-Path -LiteralPath (Join-Path $repositoryRoot 'target')).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
$stagePath = Join-Path $repositoryRoot 'apps\desktop\src-tauri\proof-stage'

$run = [Guid]::NewGuid().ToString('N')
$toolsRoot = Join-Path $repositoryRoot '.tools'
$canaryRoot = Join-Path $toolsRoot "delete-containment-canary-$run"
$canaryFile = Join-Path $canaryRoot 'do-not-delete-me.txt'
$junction = Join-Path $targetRoot "delete-containment-junction-$run"
$fileWhereDirectoryBelongs = Join-Path $targetRoot "delete-containment-file-$run"
$createdCanary = $false
$createdToolsRoot = $false
$createdJunction = $false
$createdFile = $false
$createdStageJunction = $false
$createdPreexistingStageJunction = $false
$passed = 0

# The same rule this control exists to enforce, applied to its own scratch space
# and written out here rather than called from `DeleteContainment.ps1`: a red run
# mutates that file, and a control must not depend on the code it is testing.
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
        [Parameter(Mandatory)][scriptblock]$Action
    )
    $threw = $false
    $message = $null
    try { & $Action } catch { $threw = $true; $message = $_.Exception.Message }
    if (-not $threw) {
        throw "${Scenario}: the delete was accepted instead of refused."
    }
    # The refusal has to be containment, not something incidental like a missing
    # toolchain, or this proves nothing about the delete.
    if ($message -notmatch 'reparse point|must resolve beneath|must not be|must be exactly one|file where a directory') {
        throw "${Scenario}: refused for an unrelated reason, so containment is unproved: $message"
    }
    if (-not (Test-Path -LiteralPath $script:canaryFile -PathType Leaf)) {
        throw "${Scenario}: the canary is gone; the delete ran before the refusal."
    }
    Write-Host "  ${Scenario}: refused, canary intact -- $message"
    $script:passed++
}

Push-Location $repositoryRoot
try {
    if (Test-Path -LiteralPath $stagePath) {
        throw ("$stagePath already exists; this control creates a junction there and will not " +
            'disturb whatever that is. Remove it by hand and rerun.')
    }

    # `.tools\` is checked before anything is put in it, and the GUID directory is
    # refused rather than reused: the cleanup below removes it, and a directory
    # this run did not create is not this run's to remove. Whether this run
    # created `.tools\` itself decides the same question for it -- a pre-existing
    # one holds the staged toolchain and is never this control's.
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
    [IO.File]::WriteAllText($canaryFile, 'Test-owned, outside target\, and it must survive every case.')
    $canaryHash = (Get-FileHash -LiteralPath $canaryFile -Algorithm SHA256).Hash
    Write-Host "canary: $canaryFile"
    Write-Host "  sha256: $canaryHash"

    New-Item -ItemType Junction -Path $junction -Target $canaryRoot | Out-Null
    $createdJunction = $true
    $junctionItem = Get-Item -LiteralPath $junction -Force
    if (-not ($junctionItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'The junction has no reparse-point attribute; this control cannot prove anything.'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $junction 'do-not-delete-me.txt') -PathType Leaf)) {
        throw 'The junction does not resolve to the canary directory; this control cannot prove anything.'
    }
    Write-Host "junction: $junction -> $canaryRoot"

    Set-Content -LiteralPath $fileWhereDirectoryBelongs -Value 'a file, where a directory is expected' -Encoding ascii
    $createdFile = $true

    # --- The shared validator ------------------------------------------------
    Write-Host ''
    Write-Host 'Resolve-ContainedDirectory, driven directly'
    Assert-Refused 'the path is a junction' {
        Remove-ContainedDirectory -Path $junction -Root $targetRoot -Label 'Test path'
    }
    Assert-Refused 'a junction on an ancestor' {
        Remove-ContainedDirectory -Path (Join-Path $junction 'nested\deeper') -Root $targetRoot -Label 'Test path'
    }
    Assert-Refused 'the root itself' {
        Remove-ContainedDirectory -Path $targetRoot -Root $targetRoot -Label 'Test path'
    }
    Assert-Refused 'outside the root' {
        Remove-ContainedDirectory -Path $canaryRoot -Root $targetRoot -Label 'Test path'
    }
    Assert-Refused 'escaping with ..' {
        Remove-ContainedDirectory -Path (Join-Path $targetRoot '..\..\escaped') -Root $targetRoot -Label 'Test path'
    }
    Assert-Refused 'a file where a directory must be' {
        Remove-ContainedDirectory -Path (Join-Path $fileWhereDirectoryBelongs 'below') -Root $targetRoot -Label 'Test path'
    }
    Assert-Refused 'nested where only a direct child is allowed' {
        Remove-ContainedDirectory -Path (Join-Path $targetRoot 'a\b') -Root $targetRoot -Label 'Test path' -DirectChildOnly
    }
    # And it accepts the ordinary case, so the refusals above are not a validator
    # that refuses everything.
    $ordinary = Join-Path $targetRoot "delete-containment-ordinary-$run"
    New-Item -ItemType Directory -Path (Join-Path $ordinary 'nested') -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $ordinary 'nested\file.txt') -Value 'removable' -Encoding ascii
    [void](Remove-ContainedDirectory -Path $ordinary -Root $targetRoot -Label 'Test path')
    if (Test-Path -LiteralPath $ordinary) { throw 'The validator accepted an ordinary directory and did not remove it.' }
    Write-Host '  an ordinary nested directory under target: accepted and removed'
    $passed++

    # --- Build-LocalInstaller.ps1, end to end --------------------------------
    # Its validation runs before the build, so these refuse in milliseconds.
    Write-Host ''
    Write-Host 'Build-LocalInstaller.ps1 -ArtifactRoot'
    $build = Join-Path $PSScriptRoot 'Build-LocalInstaller.ps1'
    Assert-Refused 'ArtifactRoot is a junction' { & $build -ArtifactRoot $junction }
    Assert-Refused 'ArtifactRoot has a junction on an ancestor' {
        & $build -ArtifactRoot (Join-Path $junction 'local-development\9.9.9')
    }
    Assert-Refused 'ArtifactRoot is the target root' { & $build -ArtifactRoot $targetRoot }
    Assert-Refused 'ArtifactRoot is outside the workspace' { & $build -ArtifactRoot $canaryRoot }

    # --- Invoke-ProofPackage.ps1's proof stage, end to end -------------------
    Write-Host ''
    Write-Host 'Invoke-ProofPackage.ps1, proof-stage'
    New-Item -ItemType Junction -Path $stagePath -Target $canaryRoot | Out-Null
    $createdStageJunction = $true
    Write-Host "junction: $stagePath -> $canaryRoot"
    # A valid BuildRoot, so the only thing left to refuse is the stage. The stage
    # check runs before Cargo, so this costs nothing.
    Assert-Refused 'proof-stage is a junction' {
        & (Join-Path $PSScriptRoot 'Invoke-ProofPackage.ps1') -BuildRoot (Join-Path $targetRoot "delete-containment-build-$run")
    }
    [IO.Directory]::Delete($stagePath, $false)
    $createdStageJunction = $false
    Write-Host '  removed the junction; the canary directory is still there: ' -NoNewline
    Write-Host ([IO.Directory]::Exists($canaryRoot))

    # --- This control's own rule, applied to this control ---------------------
    # It refuses to start when something is already at `proof-stage`, and that
    # refusal has to leave the something alone. The failure it guards against is
    # a `finally` that deletes any reparse point it finds there, which removes a
    # link the run never created.
    Write-Host ''
    Write-Host 'a pre-existing proof-stage is refused and survives'
    New-Item -ItemType Junction -Path $stagePath -Target $canaryRoot | Out-Null
    $createdPreexistingStageJunction = $true
    $threw = $false
    $message = $null
    # The script drives itself. The child refuses at its first statement, so it
    # creates nothing, recurses no further, and its own `finally` is exactly what
    # is under test here.
    try { & $PSCommandPath } catch { $threw = $true; $message = $_.Exception.Message }
    if (-not $threw) {
        throw 'a pre-existing proof-stage: the control started anyway instead of refusing.'
    }
    if ($message -notlike '*already exists*') {
        throw "a pre-existing proof-stage: refused for an unrelated reason: $message"
    }
    Write-Host "  refused: $message"
    if (-not (Test-Path -LiteralPath $stagePath)) {
        throw 'a pre-existing proof-stage: the refusing run deleted the junction it did not create.'
    }
    $stillLinked = (Get-Item -LiteralPath $stagePath -Force).Attributes -band [IO.FileAttributes]::ReparsePoint
    if (-not $stillLinked) {
        throw 'a pre-existing proof-stage: what survived is no longer the junction that was planted.'
    }
    Write-Host '  the planted junction is still there, and still a junction'
    $passed++
    # Only what this run planted.
    [IO.Directory]::Delete($stagePath, $false)
    $createdPreexistingStageJunction = $false
    Write-Host '  removed the junction this control planted'

    $finalHash = (Get-FileHash -LiteralPath $canaryFile -Algorithm SHA256).Hash
    if ($finalHash -ne $canaryHash) { throw "The canary changed: $finalHash, was $canaryHash." }
    Write-Host ''
    Write-Host "canary still $finalHash after every refusal"
    Write-Host "delete containment: passed ($passed assertions)"
}
finally {
    Pop-Location
    # **Each removal is gated on this run having created that exact thing.** The
    # previous shape deleted any reparse point found at `proof-stage`, which is
    # how a control that refuses to disturb a pre-existing junction deletes one
    # anyway on its way out.
    #
    # Links go first, and with the .NET call rather than `Remove-Item`: deleting a
    # link must not recurse into what it points at.
    if ($createdJunction -and (Test-Path -LiteralPath $junction)) {
        [IO.Directory]::Delete($junction, $false)
    }
    if (($createdStageJunction -or $createdPreexistingStageJunction) -and (Test-Path -LiteralPath $stagePath)) {
        [IO.Directory]::Delete($stagePath, $false)
    }
    if ($createdFile -and (Test-Path -LiteralPath $fileWhereDirectoryBelongs -PathType Leaf)) {
        Remove-Item -LiteralPath $fileWhereDirectoryBelongs -Force
    }
    # The canary file by name, then its directory non-recursively. A recursive
    # delete here would be this control breaking the rule it enforces, and
    # `Directory.Delete($path, $false)` refuses a directory holding anything
    # unexpected rather than taking it with the rest.
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

# Explicit: `Build-LocalInstaller.ps1` is invoked above and a script's exit code
# otherwise trails `$LASTEXITCODE`.
exit 0
