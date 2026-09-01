[CmdletBinding()]
param(
    # Build into a target directory created empty for this run.
    #
    # Cargo decides what to rebuild from source timestamps, so a source file put
    # back from a copy is older than the artifact built from the version that
    # replaced it, and Cargo reports nothing to do. Deleting one named output
    # cannot cover that. `Build-LocalInstaller.ps1` requires this switch.
    [switch]$FreshBuild,
    # Overridable so a proof can package without touching the canonical root.
    # Validated below before anything is deleted.
    [string]$BuildRoot
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$desktopRoot = Join-Path $repositoryRoot 'apps\desktop'
$stage = Join-Path $desktopRoot 'src-tauri\proof-stage'
. (Join-Path $PSScriptRoot 'DeleteContainment.ps1')

# **A fixed path is not a safe path.** `proof-stage` is removed recursively twice
# below, and being spelled out in this file says nothing about what is on disk:
# a junction at `proof-stage`, or at any directory above it, sends the delete
# wherever it points. Checked here so the run refuses before the build rather
# than after eight minutes of it, and checked again at each delete.
$stage = Resolve-ContainedDirectory -Path $stage -Root $repositoryRoot -Label 'The proof stage'

# Resolved and validated before `Remove-Item` is anywhere near it. `target\`
# itself is created if absent so the comparison below has something to resolve
# against on a clean checkout.
$targetRoot = Join-Path $repositoryRoot 'target'
if (-not (Test-Path -LiteralPath $targetRoot -PathType Container)) {
    New-Item -ItemType Directory -Path $targetRoot -Force | Out-Null
}
$targetRoot = (Resolve-Path -LiteralPath $targetRoot).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
if (-not $BuildRoot) {
    $BuildRoot = Join-Path $targetRoot 'installer-build'
}
$buildRoot = [IO.Path]::GetFullPath($BuildRoot).TrimEnd([IO.Path]::DirectorySeparatorChar)
$separator = [IO.Path]::DirectorySeparatorChar

# **Exactly one directory beneath the resolved target root, and no reparse point
# on either end.** This is what bounds the recursive delete below.
#
# Depth is refused rather than inspected: a nested path can have a junction on an
# *ancestor*, so checking only the leaf would let the delete follow one out of the
# workspace. The canonical build root needs no nesting. `GetFullPath` has already
# collapsed any `..`, and equality with the target root would delete every other
# build in the tree.
if (-not $buildRoot.StartsWith($targetRoot + $separator, [StringComparison]::OrdinalIgnoreCase)) {
    throw "BuildRoot must be a directory directly under $targetRoot; got $buildRoot."
}
$relativeBuildRoot = $buildRoot.Substring($targetRoot.Length + 1)
if ([string]::IsNullOrWhiteSpace($relativeBuildRoot)) {
    throw "BuildRoot must name a directory below $targetRoot, not the directory itself."
}
if ($relativeBuildRoot.IndexOf($separator) -ge 0 -or
    $relativeBuildRoot.IndexOf([IO.Path]::AltDirectorySeparatorChar) -ge 0) {
    throw ("BuildRoot must be exactly one directory beneath $targetRoot so no junction can " +
        "sit on an ancestor of it; got $buildRoot.")
}
foreach ($guarded in @($targetRoot, $buildRoot)) {
    if (-not (Test-Path -LiteralPath $guarded)) { continue }
    $guardedItem = Get-Item -LiteralPath $guarded -Force
    if ($guardedItem -isnot [IO.DirectoryInfo]) {
        throw "Expected a directory and found a file: $guarded."
    }
    if ($guardedItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "$guarded is a reparse point and may resolve outside the workspace."
    }
}

if ($FreshBuild) {
    if (Test-Path -LiteralPath $buildRoot) {
        Write-Host "fresh build: removing $buildRoot"
        Remove-Item -LiteralPath $buildRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path $buildRoot -Force | Out-Null
    Write-Host "fresh build: $buildRoot created empty"
} else {
    Write-Host "incremental build into $buildRoot (no -FreshBuild)"
}

$releaseDirectory = Join-Path $buildRoot 'release'
$bootstrapper = Join-Path $releaseDirectory 'speakeasy-bootstrapper.exe'
$graniteWorker = Join-Path $releaseDirectory 'speakeasy-granite-worker.exe'
$payloadPacker = Join-Path $releaseDirectory 'pack-payload.exe'
# Read by `Build-LocalInstaller.ps1` instead of it recomputing these paths. After
# a fresh build the directory was empty, so this file existing is the evidence
# that this run produced what it names.
$packageManifestPath = Join-Path $buildRoot 'proof-package.json'
if (Test-Path -LiteralPath $packageManifestPath) {
    Remove-Item -LiteralPath $packageManifestPath -Force
}

# There is no runtime to stage beside the workers any more. This file used to
# carry a `$sherpaCudaLib` root, a five-DLL list and a `Copy-RuntimeDll` helper
# that fetched from `Get-GpuRuntime.ps1` -- all of which left with the streaming
# engine, except the code that named them. `speakeasy-granite` compiles
# llama.cpp straight into the worker, so `granite-worker.exe` needs nothing
# beside it on a CPU install.
#
# The CUDA worker stays unbundled: it is published, and setup fetches and stages
# it when the user chooses the graphics card. What the payload carries is the
# processor worker, and the reason is the build below -- it requests default
# features, which do not include CUDA.
#
# Bundling it is not what makes downloading possible. The model weights are
# fetched during setup too, so a machine with no network cannot transcribe
# whether or not this binary is present.

Push-Location $repositoryRoot
$previousCargoTarget = $env:CARGO_TARGET_DIR
try {
    $env:CARGO_TARGET_DIR = $buildRoot
    . .\scripts\Enter-DevEnvironment.ps1
    # `speakeasy-granite-worker` builds with its default (CPU) features, so this
    # produces the CPU worker regardless of what GPU hardware the packaging
    # machine happens to have.
    #
    # That is a *distribution* decision, not a capability one: both providers
    # are reachable by a user (owner decision 2026-08-14), and which worker the
    # payload carries decides only what arrives without a download. The CUDA
    # worker is published, so setup fetches it and stages it over this one when
    # the user asks for the graphics card.
    & cargo build -p speakeasy-bootstrapper -p speakeasy-granite-worker --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Release worker/bootstrapper/granite-worker build failed.' }

    [void](Remove-ContainedDirectory -Path $stage -Root $repositoryRoot -Label 'The proof stage')
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    Copy-Item -LiteralPath $bootstrapper -Destination (Join-Path $stage 'speakeasy-bootstrapper.exe')
    Copy-Item -LiteralPath $graniteWorker -Destination (Join-Path $stage 'granite-worker.exe')
    # Refuse to stage a graphics-card worker with no libraries beside it. A CUDA
    # build whose imports Windows cannot resolve does not run slower, it does not
    # start -- and the error for that names no file anyone can act on. `cargo
    # build` above requests default features, so this normally reports `cpu`; the
    # check is for the run where somebody left a `--features cuda` binary in the
    # target directory.
    . (Join-Path $repositoryRoot 'scripts\GraniteWorkerProvider.ps1')
    $stagedWorkerProvider = Assert-GraniteWorkerPayloadIsCoherent `
        -WorkerPath $graniteWorker `
        -RepositoryRoot $repositoryRoot `
        -StagedDirectory $stage
    Write-Host "  granite worker provider: $stagedWorkerProvider"
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'packaging\THIRD-PARTY-NOTICES.txt') $stage
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'packaging\MODEL-NOTICES.md') $stage
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'packaging\SOURCE-NOTICE.md') $stage

    # Stale-artifact guard for the incremental path. A no-op under `-FreshBuild`,
    # which covers every output rather than this one.
    $desktopExe = Join-Path $buildRoot 'release\ai-speakeasy-mini.exe'
    if (Test-Path -LiteralPath $desktopExe) {
        Write-Host 'removing previous ai-speakeasy-mini.exe'
        Remove-Item -LiteralPath $desktopExe -Force
    }

    # No `--bundles`: `tauri.proof.conf.json` sets `bundle.active` to false
    # because `apps/bootstrapper` installs SpeakEasy now. This build produces the
    # executable, and `Build-LocalInstaller.ps1` assembles the payload from the
    # `bundle.resources` map, which is read as a payload manifest.
    Push-Location $desktopRoot
    try {
        & (Join-Path $repositoryRoot 'node_modules\.bin\tauri.cmd') build --config src-tauri/tauri.proof.conf.json
        if ($LASTEXITCODE -ne 0) { throw 'Proof build failed.' }
    } finally {
        Pop-Location
    }

    if (-not (Test-Path -LiteralPath $desktopExe)) {
        throw 'Tauri completed without producing ai-speakeasy-mini.exe.'
    }
    # Every executable the packaging step consumes, named and hashed here rather
    # than rediscovered by path in the caller. `pack-payload.exe` is a bin of the
    # bootstrapper crate, so it comes from this same build.
    $producedExecutables = [ordered]@{
        desktop = $desktopExe
        bootstrapper = $bootstrapper
        granite_worker = $graniteWorker
        payload_packer = $payloadPacker
    }
    foreach ($role in $producedExecutables.Keys) {
        $path = $producedExecutables[$role]
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "The build did not produce the $role executable: $path"
        }
    }
    $packageManifest = [ordered]@{
        schema_version = 1
        # The caller requires this to be true. A manifest left by an incremental
        # run cannot be mistaken for a fresh one on the strength of its presence.
        fresh_build = [bool]$FreshBuild
        build_root = $buildRoot
        release_directory = $releaseDirectory
        install_mode = 'currentUser'
        # Never, by owner decision (2026-08-14) -- not a missing prerequisite.
        signed = $false
        executables = [ordered]@{}
    }
    foreach ($role in $producedExecutables.Keys) {
        $path = $producedExecutables[$role]
        $packageManifest.executables[$role] = [ordered]@{
            path = $path
            bytes = (Get-Item -LiteralPath $path).Length
            sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    $packageManifest | ConvertTo-Json -Depth 5 |
        Set-Content -LiteralPath $packageManifestPath -Encoding utf8
    # To the host, not the output stream: cargo and tauri write to the success
    # stream, so a capturing caller would get their build logs mixed in. The
    # manifest file is the machine-readable half.
    Write-Host ''
    Write-Host "fresh_build     : $([bool]$FreshBuild)"
    Write-Host "build_root      : $buildRoot"
    foreach ($role in $producedExecutables.Keys) {
        $entry = $packageManifest.executables[$role]
        Write-Host ("{0,-16}: {1} bytes {2}" -f $role, $entry.bytes, $entry.sha256)
    }
    Write-Host "package_manifest: $packageManifestPath"
} finally {
    $env:CARGO_TARGET_DIR = $previousCargoTarget
    # Re-validated, because the check at the top describes the disk as it was
    # before the build. Reported rather than thrown: this `finally` may already be
    # carrying the build's own failure, and a refusal to delete leaves a staging
    # directory behind rather than damaging anything.
    try {
        [void](Remove-ContainedDirectory -Path $stage -Root $repositoryRoot -Label 'The proof stage')
    } catch {
        Write-Warning ("The proof stage was left in place: " + $_.Exception.Message) -WarningAction Continue
    }
    Pop-Location
}

exit 0
