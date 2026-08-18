<#
.SYNOPSIS
Puts Granite's final pass on the GPU in an installed build, by staging a
CUDA-built worker over the CPU one the installer ships.

.DESCRIPTION
**Interim.** This script is the stopgap until setup fetches a published CUDA
worker itself; see `docs/handoff/setup-wizard-redesign.md`. Owner decision
2026-08-14: both engines run on the GPU when the hardware supports it, so
"Granite is CPU-only in shipped builds" is no longer the policy -- it is
just the current state of the packaging, and this script is how you get past it
today.

The installer ships the CPU worker. `Invoke-ProofPackage.ps1` builds
`speakeasy-granite-worker` with default features, because the `cuda` feature
makes the CUDA Toolkit a hard build prerequisite for the whole workspace gate
(see `speakeasy-granite`'s `Cargo.toml`) -- and because the installer is kept
minimal by design, with anything fetchable fetched at setup time instead.

Until that fetch exists, this closes the gap locally, and the sharpest edge is
that it does not survive: reinstalling or upgrading SpeakEasy overwrites
`proof\granite-worker.exe` and puts Granite back on the CPU, **silently**.
Re-run this afterwards. `-Verify` reports which one is currently staged. That
failure mode is the single best argument for finishing the download path, at
which point this script should be retired rather than maintained.

Bundling the CUDA *redistributables* into the installer was considered and
rejected on real evidence: cuBLAS alone is ~492 MB, and the CUDA execution
provider's redistributables at ~2.3 GB already overflow `makensis` (see
`Invoke-ProofPackage.ps1`). Note that this is an argument about the DLLs, not
about the worker -- the CUDA worker measured 54.4 MB for a single architecture.

Measured on this project's own fixture (docs/handoff/transcribe-cpp-benchmark.md,
RTX 5090): Granite Q4 resident run 1,571.9 ms on CPU versus 156.4 ms on CUDA,
RTF 0.158 versus 0.0157. Cold load is slower (5,218 ms versus 2,104 ms) and it
holds ~3.27 GiB of VRAM.

# It also changes what a dev run stages, which is worth knowing

Building with `--features cuda` writes a CUDA worker to
`target\release\speakeasy-granite-worker.exe`, and that is the exact path
`Stage-DevRuntime.ps1` copies into `target\debug\proof\` for `tauri dev`. A
plain `cargo build -p speakeasy-granite-worker --release` writes a CPU worker
back to it. So the dev tree's Granite flavour follows whichever was built last,
and the two scripts will quietly disagree.

Nothing breaks silently -- a CUDA worker with no cudart/cuBLAS beside it fails
to launch outright rather than falling back -- but if Granite stops working
under `tauri dev` after running this, that is why. Copy the three DLLs into
`target\debug\proof\` as well, or rebuild without the feature.

# Verified, not assumed

`granite_final_pass_transcribes_the_fixture_through_the_real_worker_process`
passes with this staged, and the worker it spawns holds a CUDA context in
`nvidia-smi` while it runs (+2.6 GiB VRAM). Check by **PID**, not by process
name: `nvidia-smi` reports `[Insufficient Permissions]` instead of a path for
many processes, so a path match silently finds nothing and reads exactly like
a CPU fallback.

.PARAMETER InstallRoot
The installed app. Defaults to `%LOCALAPPDATA%\SpeakEasy`.

.PARAMETER CudaPath
CUDA Toolkit root. Defaults to the highest version found under
`%ProgramFiles%\NVIDIA GPU Computing Toolkit\CUDA`, or `$env:CUDA_PATH`.

.PARAMETER Architectures
`CMAKE_CUDA_ARCHITECTURES`. Defaults to this machine's own compute capability
read from `nvidia-smi` (12.0 becomes 120). Do not leave this unset by hand:
ggml's CMakeLists then falls back to a broad multi-architecture list meant for
redistributable builds, which is a multi-gigabyte, multi-minute compile.

.PARAMETER SkipBuild
Stage a worker that is already built, without asking cargo.

.PARAMETER Verify
Report which worker is staged and exit. Changes nothing.

.PARAMETER Revert
Restore the CPU worker saved on the first run, and remove the staged CUDA
DLLs.

.EXAMPLE
.\scripts\Enable-GraniteCuda.ps1

.EXAMPLE
.\scripts\Enable-GraniteCuda.ps1 -Verify

.EXAMPLE
.\scripts\Enable-GraniteCuda.ps1 -Revert
#>
[CmdletBinding()]
param(
    [string]$InstallRoot = (Join-Path $env:LOCALAPPDATA 'SpeakEasy'),
    [string]$CudaPath,
    [string]$Architectures,
    [switch]$SkipBuild,
    [switch]$Verify,
    [switch]$Revert
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$proof = Join-Path $InstallRoot 'proof'
$staged = Join-Path $proof 'granite-worker.exe'
# The CPU worker, kept aside on the first run so `-Revert` does not need a
# second build or a reinstall to undo this.
$cpuBackup = Join-Path $proof 'granite-worker.cpu.exe'

# cuBLAS loads cuBLASLt itself at run time, so all three have to be present
# even though only cuBLAS appears in the worker's import table.
$cudaDlls = @('cudart64_13.dll', 'cublas64_13.dll', 'cublasLt64_13.dll')

if (-not (Test-Path -LiteralPath $proof -PathType Container)) {
    throw "No installed SpeakEasy at $InstallRoot. Install one first -- see docs/LOCAL-DEVELOPMENT.md."
}

function Get-StagedFlavour {
    if (-not (Test-Path -LiteralPath $staged -PathType Leaf)) { return 'none' }
    # The CUDA build carries ggml's kernels and is an order of magnitude larger
    # than the CPU one (57 MB versus 4 MB when this was written). Read the
    # marker rather than trusting the size.
    $text = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($staged))
    if ($text.Contains('ggml-cuda')) { return 'cuda' }
    return 'cpu'
}

if ($Verify) {
    $flavour = Get-StagedFlavour
    [pscustomobject]@{
        staged_worker  = $flavour
        worker_path    = $staged
        worker_mb      = if (Test-Path $staged) { [math]::Round((Get-Item $staged).Length / 1MB, 1) } else { $null }
        cuda_dlls      = @($cudaDlls | Where-Object { Test-Path (Join-Path $proof $_) })
        cpu_backup_kept = Test-Path $cpuBackup
    } | Format-List
    exit 0
}

# The app holds its worker open, and a resident one keeps the file locked.
$running = @(Get-Process -Name 'ai-speakeasy-mini', 'SpeakEasy', 'speakeasy-granite-worker' -ErrorAction SilentlyContinue)
if ($running.Count -gt 0) {
    throw ("SpeakEasy is running ($($running.ProcessName -join ', ')). Stop it first: " +
        "the app holds proof\granite-worker.exe open and the copy would fail or be ignored.")
}

if ($Revert) {
    if (-not (Test-Path -LiteralPath $cpuBackup -PathType Leaf)) {
        throw "No CPU worker saved at $cpuBackup. Reinstall SpeakEasy to restore it."
    }
    Move-Item -LiteralPath $cpuBackup -Destination $staged -Force
    foreach ($dll in $cudaDlls) {
        Remove-Item -LiteralPath (Join-Path $proof $dll) -Force -ErrorAction SilentlyContinue
    }
    Write-Host "Granite reverted to the CPU worker."
    exit 0
}

if (-not $CudaPath) {
    $CudaPath = $env:CUDA_PATH
}
if (-not $CudaPath) {
    $toolkits = Join-Path $env:ProgramFiles 'NVIDIA GPU Computing Toolkit\CUDA'
    $CudaPath = Get-ChildItem -LiteralPath $toolkits -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $CudaPath -or -not (Test-Path -LiteralPath (Join-Path $CudaPath 'bin\nvcc.exe'))) {
    throw ("No CUDA Toolkit found. Install one, or pass -CudaPath. Note that CUDA_PATH is " +
        "often not set machine-wide even when the toolkit is installed, which is why " +
        "Enter-DevEnvironment.ps1 can silently skip its CUDA block.")
}

if (-not $Architectures) {
    $capability = (& nvidia-smi --query-gpu=compute_cap --format=csv,noheader 2>$null | Select-Object -First 1)
    if ($capability -match '^\s*(\d+)\.(\d+)\s*$') {
        $Architectures = "$($Matches[1])$($Matches[2])"
    } else {
        throw 'Could not read compute capability from nvidia-smi. Pass -Architectures (Blackwell consumer parts are 120).'
    }
}

$worker = Join-Path $repositoryRoot 'target\release\speakeasy-granite-worker.exe'
if (-not $SkipBuild) {
    Write-Host "Building speakeasy-granite-worker --features cuda (arch $Architectures, toolkit $CudaPath)"
    Push-Location $repositoryRoot
    try {
        $env:CUDA_PATH = $CudaPath
        $env:CMAKE_CUDA_ARCHITECTURES = $Architectures
        . (Join-Path $PSScriptRoot 'Enter-DevEnvironment.ps1') | Out-Null
        # Unredirected on purpose: PowerShell 5.1 turns a native command's
        # stderr into ErrorRecords when redirected inside PowerShell, and with
        # $ErrorActionPreference = 'Stop' cargo's progress output aborts this.
        & cargo build -p speakeasy-granite-worker --features cuda --release --locked
        if ($LASTEXITCODE -ne 0) { throw "cuda build failed ($LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}
if (-not (Test-Path -LiteralPath $worker -PathType Leaf)) {
    throw "No worker at $worker. Run without -SkipBuild."
}

# Keep the CPU worker exactly once. A second run must not overwrite the backup
# with the CUDA worker it staged on the first.
if (-not (Test-Path -LiteralPath $cpuBackup) -and (Get-StagedFlavour) -eq 'cpu') {
    Copy-Item -LiteralPath $staged -Destination $cpuBackup -Force
}

Copy-Item -LiteralPath $worker -Destination $staged -Force

# Beside the worker, not on PATH. Windows searches an executable's own
# directory first, and this project has already been bitten once by a System32
# copy of a native dependency outranking PATH (see the onnxruntime note in
# docs/handoff/). Putting them here makes the resolution unambiguous.
$cudaBin = Join-Path $CudaPath 'bin\x64'
if (-not (Test-Path -LiteralPath $cudaBin)) { $cudaBin = Join-Path $CudaPath 'bin' }
foreach ($dll in $cudaDlls) {
    $source = Join-Path $cudaBin $dll
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Required CUDA runtime DLL not found: $source"
    }
    Copy-Item -LiteralPath $source -Destination (Join-Path $proof $dll) -Force
}

[pscustomobject]@{
    staged_worker = Get-StagedFlavour
    worker_mb     = [math]::Round((Get-Item $staged).Length / 1MB, 1)
    cuda_dlls_mb  = [math]::Round((($cudaDlls | ForEach-Object { (Get-Item (Join-Path $proof $_)).Length }) | Measure-Object -Sum).Sum / 1MB, 1)
    architectures = $Architectures
    toolkit       = $CudaPath
    reverts_on    = 'any reinstall or upgrade -- re-run this script afterwards'
} | Format-List

# Explicit, so a caller chaining on this script gets the staging result rather
# than whatever exit code the last cmdlet happened to leave behind.
exit 0
