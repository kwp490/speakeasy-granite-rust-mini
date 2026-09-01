#Requires -Version 7
<#
.SYNOPSIS
    Proves a stale bootstrapper cannot survive the canonical packaging command.

.DESCRIPTION
    Cargo decides what to rebuild from source timestamps. A source file restored
    from a safe copy carries the copy's timestamp, so it is *older* than the
    artifact compiled from the version that replaced it, and Cargo reports
    nothing to do -- leaving the defect in the packaged binary while the source on
    disk looks clean and `git diff` is empty.

    Planting a stale *executable* does not reproduce that: the output is then
    older than its own fingerprint and Cargo rebuilds. So this reproduces the real
    sequence.

    1. A marker is compiled into `probe.rs`'s `PRODUCT` constant.
    2. An incremental build puts it in the bootstrapper. The marker must be
       present, or the plant failed and everything below is vacuous.
    3. `probe.rs` is restored -- content and original timestamp both -- which
       leaves the source older than the binary built from it.
    4. `Build-LocalInstaller.ps1` runs, the canonical release command.
    5. The marker must appear in nothing the release would ship.

    Step 5 is what fails without `-FreshBuild`: Cargo sees a source older than
    its fingerprint, skips the crate, and packages the marker.

    `probe.rs` is restored in the `finally` whatever happens, asserted by SHA-256
    against the pre-edit capture.

.PARAMETER ArtifactRoot
    Where to package. Defaults to a test-owned directory under
    `target\local-development\` so a run cannot overwrite an artifact that has
    already been proved. Validated before the recursive delete in the `finally`.

.PARAMETER KeepArtifacts
    Leave the packaged artifact behind, for a lifecycle or wizard proof to reuse.
#>
[CmdletBinding()]
param(
    [string]$ArtifactRoot,
    [switch]$KeepArtifacts
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$targetRoot = (Resolve-Path -LiteralPath (Join-Path $repositoryRoot 'target')).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
$buildRoot = Join-Path $targetRoot 'installer-build'
$builtBootstrapper = Join-Path $buildRoot 'release\speakeasy-bootstrapper.exe'
$markedSource = Join-Path $repositoryRoot 'apps\bootstrapper\src\probe.rs'

if (-not $ArtifactRoot) {
    $ArtifactRoot = Join-Path $targetRoot "local-development\stale-artifact-control-$PID"
}
$artifactFull = [IO.Path]::GetFullPath($ArtifactRoot).TrimEnd([IO.Path]::DirectorySeparatorChar)
# The `finally` deletes this recursively, so it is contained the same way
# `Invoke-ProofPackage.ps1` contains its build root. This one is nested by design
# (`local-development\<name>`), so instead of refusing depth, every component from
# the target root down is checked for a reparse point -- a junction on an ancestor
# would send the delete outside the workspace while the path string still reads as
# though it were inside.
$separator = [IO.Path]::DirectorySeparatorChar
if (-not $artifactFull.StartsWith($targetRoot + $separator, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ArtifactRoot must remain under $targetRoot; got $artifactFull."
}
$walked = $targetRoot
foreach ($segment in $artifactFull.Substring($targetRoot.Length + 1).Split($separator)) {
    $walked = Join-Path $walked $segment
    if (-not (Test-Path -LiteralPath $walked)) { continue }
    $walkedItem = Get-Item -LiteralPath $walked -Force
    if ($walkedItem -isnot [IO.DirectoryInfo]) {
        throw "ArtifactRoot has a file where a directory must be: $walked."
    }
    if ($walkedItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "ArtifactRoot passes through a reparse point and may resolve outside the workspace: $walked."
    }
}

# Distinctive enough that a hit is unambiguous, and long enough that it cannot
# occur by accident in a compiled binary.
$marker = 'SPEAKEASY-STALE-ARTIFACT-CONTROL-b7f3c1d2-DO-NOT-SHIP'
$markerBytes = [Text.Encoding]::ASCII.GetBytes($marker)

function Test-FileContainsMarker {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][byte[]]$Needle)
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt $Needle.Length) { return $false }
    $first = $Needle[0]
    $last = $bytes.Length - $Needle.Length
    for ($i = 0; $i -le $last; $i++) {
        if ($bytes[$i] -ne $first) { continue }
        $hit = $true
        for ($j = 1; $j -lt $Needle.Length; $j++) {
            if ($bytes[$i + $j] -ne $Needle[$j]) { $hit = $false; break }
        }
        if ($hit) { return $true }
    }
    $false
}

# Bytes, hash and the original timestamp. The timestamp matters: a file put back
# with its old mtime is what makes Cargo skip the rebuild.
$sourceBytes = [IO.File]::ReadAllBytes($markedSource)
$sourceHash = (Get-FileHash -LiteralPath $markedSource -Algorithm SHA256).Hash
$sourceWritten = (Get-Item -LiteralPath $markedSource).LastWriteTime
$sourceRestored = $false

Push-Location $repositoryRoot
try {
    . (Join-Path $PSScriptRoot 'Enter-DevEnvironment.ps1')

    Write-Host "capturing $markedSource"
    Write-Host "  sha256 : $sourceHash"
    Write-Host "  written: $sourceWritten"

    # A live constant, not a new static: an unreferenced `#[used]` static survives
    # rustc and is then dropped by the linker's `/OPT:REF`, so the marker never
    # reaches the binary. `PRODUCT` is read by the Add/Remove Programs writer and
    # the install-root derivation, so it is unarguably compiled in.
    #
    # The marked binary is never installed -- this packages and searches it -- so a
    # `PRODUCT` naming a directory nobody wants has nowhere to take effect.
    $sourceText = [Text.Encoding]::UTF8.GetString($sourceBytes)
    $originalDeclaration = 'pub const PRODUCT: &str = "SpeakEasy Mini";'
    if (-not $sourceText.Contains($originalDeclaration)) {
        throw "Could not find PRODUCT's declaration in $markedSource; the plant would be silent."
    }
    $markedText = $sourceText.Replace(
        $originalDeclaration,
        "pub const PRODUCT: &str = `"SpeakEasy Mini $marker`";")
    [IO.File]::WriteAllText($markedSource, $markedText)
    Write-Host 'appended the marker to PRODUCT'

    # Incremental on purpose: this is the setup, not the boundary under test, and
    # it only has to get the marker into the bootstrapper.
    #
    # **The package set must match `Invoke-ProofPackage.ps1`'s exactly.** Building
    # `speakeasy-bootstrapper` alone unifies features differently from building it
    # beside `speakeasy-granite-worker`, so the fingerprints differ and the
    # packaging build recompiles the crate whatever the timestamps say -- which
    # makes this control pass with no fix in place.
    $previousCargoTarget = $env:CARGO_TARGET_DIR
    try {
        $env:CARGO_TARGET_DIR = $buildRoot
        & cargo build -p speakeasy-bootstrapper -p speakeasy-granite-worker --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'The marker build failed.' }
    } finally {
        $env:CARGO_TARGET_DIR = $previousCargoTarget
    }
    if (-not (Test-Path -LiteralPath $builtBootstrapper -PathType Leaf)) {
        throw "The marker build produced no bootstrapper at $builtBootstrapper."
    }
    if (-not (Test-FileContainsMarker -Path $builtBootstrapper -Needle $markerBytes)) {
        throw ('The marker is not in the bootstrapper the marker build produced, so this ' +
            'control would pass without proving anything.')
    }
    $markedHash = (Get-FileHash -LiteralPath $builtBootstrapper -Algorithm SHA256).Hash
    $markedBinaryWritten = (Get-Item -LiteralPath $builtBootstrapper).LastWriteTime
    Write-Host "the marker is compiled into $builtBootstrapper"
    Write-Host "  sha256 : $markedHash"
    Write-Host "  written: $markedBinaryWritten"

    # Content back and timestamp back: the timestamp is the defect, not tidiness.
    [IO.File]::WriteAllBytes($markedSource, $sourceBytes)
    (Get-Item -LiteralPath $markedSource).LastWriteTime = $sourceWritten
    $sourceRestored = $true
    $restoredHash = (Get-FileHash -LiteralPath $markedSource -Algorithm SHA256).Hash
    if ($restoredHash -ne $sourceHash) {
        throw "Restoring $markedSource produced $restoredHash, not $sourceHash."
    }
    Write-Host ''
    Write-Host "restored $markedSource byte-identically, timestamp $((Get-Item -LiteralPath $markedSource).LastWriteTime)"
    if ((Get-Item -LiteralPath $markedSource).LastWriteTime -ge $markedBinaryWritten) {
        throw ('The restored source is not older than the binary built from the marker, so ' +
            'Cargo would rebuild it anyway and this control cannot fail.')
    }
    Write-Host '  the source is now older than the binary built from it, which is the defect'

    Write-Host ''
    Write-Host "running the canonical packaging command into $artifactFull"
    & (Join-Path $PSScriptRoot 'Build-LocalInstaller.ps1') -ArtifactRoot $artifactFull
    if ($LASTEXITCODE -ne 0) { throw 'Build-LocalInstaller.ps1 failed.' }

    Write-Host ''
    if (-not (Test-Path -LiteralPath $builtBootstrapper -PathType Leaf)) {
        throw "The bootstrapper is missing after packaging: $builtBootstrapper"
    }
    if (Test-FileContainsMarker -Path $builtBootstrapper -Needle $markerBytes) {
        throw ("The marker survived into the built bootstrapper: $builtBootstrapper. " +
            'The packaging command reused a binary compiled from source that no longer exists.')
    }
    $rebuiltHash = (Get-FileHash -LiteralPath $builtBootstrapper -Algorithm SHA256).Hash
    Write-Host "the packaged bootstrapper hashes $rebuiltHash, not the marked $markedHash"

    # Every file the release would hand a user. The check above is about one
    # binary; this covers the payload and the packed single-file installer, which
    # is where a survivor would actually reach somebody.
    $inspected = 0
    $survivors = @()
    foreach ($file in Get-ChildItem -LiteralPath $artifactFull -Recurse -File) {
        $inspected++
        if (Test-FileContainsMarker -Path $file.FullName -Needle $markerBytes) {
            $survivors += $file.FullName
        }
    }
    if ($survivors.Count -gt 0) {
        throw ('The marker reached the packaged artifact: ' + ($survivors -join ', '))
    }
    Write-Host "searched $inspected packaged files; the marker is in none of them"

    $packageManifest = Get-Content -LiteralPath (Join-Path $buildRoot 'proof-package.json') -Raw | ConvertFrom-Json
    if (-not $packageManifest.fresh_build) {
        throw 'The packaging command reported fresh_build=false.'
    }
    Write-Host 'the proof build reported fresh_build=true'

    # Into the build root, not the artifact root: `Build-LocalInstaller.ps1`
    # generates `SHA256SUMS` over the artifact directory, so a file added here
    # afterwards would be an artifact nothing had hashed.
    $reportPath = Join-Path $buildRoot 'stale-artifact-control-report.json'
    [pscustomobject]@{
        schema_version = 1
        marked_sha256 = $markedHash.ToLowerInvariant()
        rebuilt_sha256 = $rebuiltHash.ToLowerInvariant()
        packaged_files_searched = $inspected
        marker_survivors = $survivors.Count
        fresh_build = [bool]$packageManifest.fresh_build
    } | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $reportPath -Encoding utf8
    Write-Host "report: $reportPath"

    Write-Host ''
    Write-Host 'stale-artifact refusal: passed'
}
finally {
    Pop-Location
    # Unconditional. A throw between the edit and the restore would otherwise
    # leave a marker static in a tracked source file.
    if (-not $sourceRestored) {
        [IO.File]::WriteAllBytes($markedSource, $sourceBytes)
        (Get-Item -LiteralPath $markedSource).LastWriteTime = $sourceWritten
        $emergencyHash = (Get-FileHash -LiteralPath $markedSource -Algorithm SHA256).Hash
        if ($emergencyHash -ne $sourceHash) {
            Write-Warning "RESTORE FAILED: $markedSource hashes $emergencyHash, captured $sourceHash."
        } else {
            Write-Host "restored $markedSource after a failure; sha256 matches the capture"
        }
    }
    if (-not $KeepArtifacts -and (Test-Path -LiteralPath $artifactFull)) {
        Remove-Item -LiteralPath $artifactFull -Recurse -Force -ErrorAction SilentlyContinue
    }
}
