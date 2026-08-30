[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$desktopRoot = Join-Path $repositoryRoot 'apps\desktop'
$stage = Join-Path $desktopRoot 'src-tauri\proof-stage'
$buildRoot = Join-Path $repositoryRoot 'target\installer-build'
$bootstrapper = Join-Path $buildRoot 'release\speakeasy-bootstrapper.exe'
$graniteWorker = Join-Path $buildRoot 'release\speakeasy-granite-worker.exe'

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

    Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
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

    # Stale-artifact guard, kept from the NSIS era and still earning its place.
    # The hazard was never specific to installers: a build that fails silently
    # leaves a byte-for-byte plausible artifact with the *same file name*
    # whenever the version has not changed, so "newest wins" hands back the
    # previous build as though it were this one. That happened once with a
    # setup.exe from an earlier session. The app executable has exactly the same
    # property, so it is cleared before building rather than trusted afterwards.
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
    [pscustomobject]@{
        desktop = $desktopExe
        desktop_bytes = (Get-Item -LiteralPath $desktopExe).Length
        desktop_sha256 = (Get-FileHash $desktopExe -Algorithm SHA256).Hash.ToLowerInvariant()
        install_mode = 'currentUser'
        # Never, by owner decision (2026-08-14) -- not a missing prerequisite.
        signed = $false
    } | Format-List
} finally {
    $env:CARGO_TARGET_DIR = $previousCargoTarget
    Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
    Pop-Location
}

exit 0
