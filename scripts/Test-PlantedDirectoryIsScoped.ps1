#Requires -Version 7
<#
.SYNOPSIS
    Proves `PlantedDirectory.ps1` against directories under `target\`, with no
    profile, no share, and nothing installed.

.DESCRIPTION
    `Test-PreflightRefusalIsInert.ps1` uses both of these against the operator's
    real `%LOCALAPPDATA%`, and takes a mandatory `-ArtifactRoot` and asserts host
    identity as it loads -- so nothing in it can be exercised without installing
    for real. This control runs anywhere.

    What it holds to:

    - `Get-DirectoryState` answers `$null` for a directory that is not there and
      an **empty array** for one that is there and holds no files. Those are
      different answers, and the postcondition that reads them reports the
      machine unchanged when it gets the wrong one;
    - a directory holding only an empty subdirectory is still present;
    - `Remove-PlantedDirectoryEntries` removes exactly the named files and the
      named directories, deepest first, when the marker holds this run's value;
    - it removes nothing at all when the marker is missing or holds another
      value, and refuses a directory holding anything it was not given.

    Only uniquely named paths under `target\` are touched, and a canary beside
    them is hashed before and after every refusal.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'PlantedDirectory.ps1')
. (Join-Path $PSScriptRoot 'DeleteContainment.ps1')

$targetRoot = (Resolve-Path -LiteralPath (Join-Path $repositoryRoot 'target')).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
# A GUID rather than the process id: a pid is reused, and the `finally` removes
# this directory recursively.
$scratch = Join-Path $targetRoot ('planted-directory-control-' + [Guid]::NewGuid().ToString('N'))
$markerName = '.speakeasy-preflight-control-marker'
$canaryName = 'operator-would-notice-this.txt'
$canary = Join-Path $scratch $canaryName
$canaryHash = $null
$passed = 0
$scratchCreated = $false

function Assert-Refused {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][scriptblock]$Action,
        [string[]]$MessageContains = @()
    )
    $message = $null
    try {
        & $Action | Out-Null
    } catch {
        $message = $_.Exception.Message
    }
    if ($null -eq $message) {
        throw "CASE '$Name': expected a refusal, and the check returned successfully."
    }
    foreach ($needle in $MessageContains) {
        if ($message -notlike "*$needle*") {
            throw "CASE '$Name': the refusal does not mention '$needle'. It said: $message"
        }
    }
    $message
}

function Assert-CanaryIntact {
    param([Parameter(Mandatory)][string]$Stage)
    $now = (Get-FileHash -LiteralPath $canary -Algorithm SHA256).Hash
    if ($now -ne $canaryHash) { throw "${Stage}: the canary changed; it was $canaryHash and is $now" }
}

# One planted set, built exactly the way the proof builds it: `@(...)`, so the
# arrays are `object[]` holding PSObject slots. A `string[]` parameter would
# convert at bind time and the hazard would never reach the function.
function New-PlantedSet {
    param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][string]$MarkerValue)
    $directories = @($Root, (Join-Path $Root 'proof'))
    $files = @(
        (Join-Path $Root $markerName)
        (Join-Path $Root 'ai-speakeasy-mini.exe')
        (Join-Path $Root 'proof\granite-worker.exe')
    )
    foreach ($d in $directories) { [void][IO.Directory]::CreateDirectory($d) }
    Set-Content -LiteralPath $files[0] -Value $MarkerValue -NoNewline -Encoding utf8
    Set-Content -LiteralPath $files[1] -Value 'not a real executable' -Encoding utf8
    Set-Content -LiteralPath $files[2] -Value 'not a real worker' -Encoding utf8
    @{ Directories = $directories; Files = $files }
}

try {
    [void][IO.Directory]::CreateDirectory($scratch)
    $scratchCreated = $true
    Set-Content -LiteralPath $canary -Value 'a file this control must never touch' -Encoding utf8
    $canaryHash = (Get-FileHash -LiteralPath $canary -Algorithm SHA256).Hash
    Write-Host "scratch: $scratch"

    Write-Host ''
    Write-Host 'CASE: Get-DirectoryState tells absent from present-but-empty'
    $absent = Join-Path $scratch 'no-such-directory'
    if ($null -ne (Get-DirectoryState -Root $absent)) {
        throw 'CASE absent: a directory that is not there did not answer $null.'
    }
    Write-Host '  a directory that is not there  : $null'

    # The exact shape the crashed proof left behind: the root survived holding one
    # empty subdirectory, and reading that as absent is what let the postcondition
    # report the machine unchanged.
    $empty = Join-Path $scratch 'present-but-empty'
    [void][IO.Directory]::CreateDirectory((Join-Path $empty 'proof'))
    $emptyState = Get-DirectoryState -Root $empty
    if ($null -eq $emptyState) {
        throw 'CASE present-but-empty: a directory holding an empty subdirectory answered $null, so it reads as absent.'
    }
    if ($emptyState.Count -ne 0) {
        throw "CASE present-but-empty: expected no files, got $($emptyState.Count)."
    }
    Write-Host '  present, holds no files        : empty array, not $null'

    $withFiles = Join-Path $scratch 'holds-files'
    [void][IO.Directory]::CreateDirectory((Join-Path $withFiles 'nested'))
    Set-Content -LiteralPath (Join-Path $withFiles 'one.txt') -Value 'one' -Encoding utf8
    Set-Content -LiteralPath (Join-Path $withFiles 'nested\two.txt') -Value 'two' -Encoding utf8
    $withFilesState = Get-DirectoryState -Root $withFiles
    if ($withFilesState.Count -ne 2) {
        throw "CASE holds-files: expected 2 entries, got $($withFilesState.Count)."
    }
    foreach ($entry in $withFilesState) {
        if ($entry -notmatch '^[^=]+=[0-9A-F]{64}$') { throw "CASE holds-files: malformed entry '$entry'." }
    }
    Write-Host "  present, holds 2 files         : $($withFilesState -join ', ')"
    $passed++

    Write-Host ''
    Write-Host 'CASE: the planted set is removed deepest first when the marker holds this run''s value'
    $goodRoot = Join-Path $scratch 'planted-good'
    $goodValue = [Guid]::NewGuid().ToString('N')
    $good = New-PlantedSet -Root $goodRoot -MarkerValue $goodValue
    # The regression: built with `@(...)`, these are `object[]` of PSObject slots.
    # `[array]::Copy` into a `string[]` throws on them, after the marker file has
    # already been deleted -- so the directories stay and no second attempt is
    # authorised. Removal has to survive being handed exactly this array.
    if ($good.Directories -isnot [object[]]) {
        throw 'CASE planted-good: the directory list is not an object[], so it would not reproduce the cast.'
    }
    Remove-PlantedDirectoryEntries -Root $goodRoot -MarkerName $markerName -MarkerValue $goodValue `
        -Files $good.Files -Directories $good.Directories
    foreach ($path in ($good.Files + $good.Directories)) {
        if (Test-Path -LiteralPath $path) { throw "CASE planted-good: $path survived removal." }
    }
    Write-Host "  removed 3 file(s) and 2 directory(ies); $goodRoot is gone"
    Assert-CanaryIntact -Stage 'CASE planted-good'
    $passed++

    Write-Host ''
    Write-Host 'CASE: no marker, and a wrong marker, both remove nothing'
    $noMarkerRoot = Join-Path $scratch 'planted-no-marker'
    $noMarkerValue = [Guid]::NewGuid().ToString('N')
    $noMarker = New-PlantedSet -Root $noMarkerRoot -MarkerValue $noMarkerValue
    Remove-Item -LiteralPath $noMarker.Files[0] -Force
    $message = Assert-Refused -Name 'no marker' -MessageContains @('no control marker', 'removing nothing') -Action {
        Remove-PlantedDirectoryEntries -Root $noMarkerRoot -MarkerName $markerName -MarkerValue $noMarkerValue `
            -Files $noMarker.Files -Directories $noMarker.Directories
    }
    foreach ($path in @($noMarker.Files[1], $noMarker.Files[2]) + $noMarker.Directories) {
        if (-not (Test-Path -LiteralPath $path)) { throw "CASE no-marker: $path was removed by a refusal." }
    }
    Write-Host "  no marker   : $message"

    $wrongRoot = Join-Path $scratch 'planted-wrong-marker'
    $wrong = New-PlantedSet -Root $wrongRoot -MarkerValue ([Guid]::NewGuid().ToString('N'))
    $message = Assert-Refused -Name 'wrong marker' -MessageContains @('not this run''s', 'removing nothing') -Action {
        Remove-PlantedDirectoryEntries -Root $wrongRoot -MarkerName $markerName `
            -MarkerValue ([Guid]::NewGuid().ToString('N')) -Files $wrong.Files -Directories $wrong.Directories
    }
    foreach ($path in $wrong.Files + $wrong.Directories) {
        if (-not (Test-Path -LiteralPath $path)) { throw "CASE wrong-marker: $path was removed by a refusal." }
    }
    Write-Host "  wrong value : $message"
    Assert-CanaryIntact -Stage 'CASE markers'
    $passed++

    Write-Host ''
    Write-Host 'CASE: a directory holding something it was not given stays, and is named'
    $extraRoot = Join-Path $scratch 'planted-extra'
    $extraValue = [Guid]::NewGuid().ToString('N')
    $extra = New-PlantedSet -Root $extraRoot -MarkerValue $extraValue
    $stranger = Join-Path $extraRoot 'proof\someone-elses-file.txt'
    Set-Content -LiteralPath $stranger -Value 'not this control''s' -Encoding utf8
    $message = Assert-Refused -Name 'extra content' -MessageContains @('still holds', 'someone-elses-file.txt') -Action {
        Remove-PlantedDirectoryEntries -Root $extraRoot -MarkerName $markerName -MarkerValue $extraValue `
            -Files $extra.Files -Directories $extra.Directories
    }
    if (-not (Test-Path -LiteralPath $stranger -PathType Leaf)) {
        throw 'CASE extra-content: the file this control did not create was removed.'
    }
    Write-Host "  refused: $message"
    Assert-CanaryIntact -Stage 'CASE extra-content'
    $passed++

    Write-Host ''
    Write-Host "planted directory state is scoped: passed ($passed cases)"
} finally {
    if ($scratchCreated) {
        [void](Remove-ContainedDirectory -Path $scratch -Root $targetRoot -Label "This control's scratch directory")
    }
}
