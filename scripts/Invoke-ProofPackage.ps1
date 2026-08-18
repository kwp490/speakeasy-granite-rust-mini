[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$desktopRoot = Join-Path $repositoryRoot 'apps\desktop'
$stage = Join-Path $desktopRoot 'src-tauri\proof-stage'
$buildRoot = Join-Path $repositoryRoot 'target\installer-build'
$worker = Join-Path $buildRoot 'release\speakeasy-inference-worker.exe'
$bootstrapper = Join-Path $buildRoot 'release\speakeasy-bootstrapper.exe'
$graniteWorker = Join-Path $buildRoot 'release\speakeasy-granite-worker.exe'

# Where `.\scripts\Get-GpuRuntime.ps1` extracts the runtime archives it fetches
# and hash-verifies. This script does not re-verify them: that already
# happened once, against `models/trusted-manifest.json`, when they were
# downloaded, and re-hashing multi-gigabyte CUDA archives on every packaging
# run would cost minutes for no new information.
$sherpaCudaLib = Join-Path $repositoryRoot '.tools\sherpa-onnx\current\lib'

# sherpa's own DLLs from the CUDA archive's `lib/` -- these alone are enough to
# run the CPU provider. `onnxruntime_providers_tensorrt.dll` is dropped since
# TensorRT is not used (see the GPU migration handoff).
#
# The CUDA execution provider and its cuBLAS/cuFFT/cuDNN dependencies are
# NOT bundled here. Their combined size (~2.3 GB across 14 files, several individually
# in the hundreds of megabytes) exceeds what the NSIS compiler this project uses can
# link into one installer -- discovered as `makensis`'s "Internal compiler error
# ... mmapping file ... is out of range" once this migration actually tried
# bundling them, not anticipated in the migration plan. Until CUDA runtime
# distribution has its own download-on-demand mechanism (the "CUDA core +
# cuDNN add-on" split the migration handoff's Decisions table calls for, not
# yet built), an installer built from this script is CPU-only:
# `RuntimeWizardCoordinator::paths()` treats the CUDA provider DLL as optional
# for exactly this reason.
$sherpaDlls = @(
    'cargs.dll',
    'onnxruntime.dll',
    'onnxruntime_providers_shared.dll',
    'sherpa-onnx-c-api.dll',
    'sherpa-onnx-cxx-api.dll'
)

function Copy-RuntimeDll {
    param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][string]$Destination)
    $source = Get-ChildItem -Path $sherpaCudaLib -Filter $Name -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if (-not $source) {
        throw "Required runtime DLL not found -- run .\scripts\Get-GpuRuntime.ps1 first: $Name"
    }
    Copy-Item -LiteralPath $source.FullName -Destination $Destination -Force
}

Push-Location $repositoryRoot
$previousCargoTarget = $env:CARGO_TARGET_DIR
try {
    $env:CARGO_TARGET_DIR = $buildRoot
    . .\scripts\Enter-DevEnvironment.ps1
    # `speakeasy-granite-worker` builds with its default (CPU) features, so this
    # produces the CPU worker regardless of what GPU hardware the packaging
    # machine happens to have.
    #
    # That is a *distribution* decision now, not a capability one. It used to
    # mean "the CUDA path is not something a user can reach"; it no longer does
    # (owner decision 2026-08-14 -- both engines run on the GPU when the
    # hardware supports it). What it means today is that the installer is kept
    # deliberately minimal: anything that can be fetched at setup time is
    # fetched, and only what cannot be is bundled. The CUDA worker can be
    # published and fetched, so it is not bundled here.
    #
    # The CPU worker IS bundled, because it is the one thing that must exist
    # before any download succeeds -- a machine with no network, or a user who
    # declines the GPU, still has to be able to transcribe.
    #
    # See docs/handoff/setup-wizard-redesign.md for the fetch-first policy and
    # for where a published CUDA worker is expected to live.
    & cargo build -p speakeasy-inference-worker -p speakeasy-bootstrapper -p speakeasy-granite-worker --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Release worker/bootstrapper/granite-worker build failed.' }

    Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    Copy-Item -LiteralPath $worker -Destination (Join-Path $stage 'inference-worker.exe')
    Copy-Item -LiteralPath $bootstrapper -Destination (Join-Path $stage 'speakeasy-bootstrapper.exe')
    Copy-Item -LiteralPath $graniteWorker -Destination (Join-Path $stage 'granite-worker.exe')
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'packaging\THIRD-PARTY-NOTICES.txt') $stage
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'packaging\MODEL-NOTICES.md') $stage
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'packaging\SOURCE-NOTICE.md') $stage
    foreach ($dll in $sherpaDlls) { Copy-RuntimeDll -Name $dll -Destination $stage }

    # Stale-artifact guard, kept from the NSIS era and still earning its place.
    # The hazard was never specific to installers: a build that fails silently
    # leaves a byte-for-byte plausible artifact with the *same file name*
    # whenever the version has not changed, so "newest wins" hands back the
    # previous build as though it were this one. That happened once with a
    # setup.exe from an earlier session. The app executable has exactly the same
    # property, so it is cleared before building rather than trusted afterwards.
    $desktopExe = Join-Path $buildRoot 'release\ai-speakeasy-desktop.exe'
    if (Test-Path -LiteralPath $desktopExe) {
        Write-Host 'removing previous ai-speakeasy-desktop.exe'
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
        throw 'Tauri completed without producing ai-speakeasy-desktop.exe.'
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
