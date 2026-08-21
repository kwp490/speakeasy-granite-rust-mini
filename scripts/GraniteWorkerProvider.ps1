#Requires -Version 7
<#
.SYNOPSIS
    What a built `speakeasy-granite-worker.exe` can run on, and what it needs
    beside it to do so.

.DESCRIPTION
    Dot-sourced by the two scripts that assemble a payload --
    `Build-LocalInstaller.ps1` and `Invoke-ProofPackage.ps1` -- and by
    `Enable-GraniteCuda.ps1`, which stages the same libraries into an installed
    build. It exists as one file rather than as a function in each because they
    would drift, and the drift lands as a payload that claims a provider it does
    not carry, or as a staging script that copies libraries under names nothing
    is looking for.

    **The question it answers is "packaged", not "works".** A worker with its
    libraries beside it can still run on the processor -- a refusing driver, a
    claimed card, exhausted VRAM -- so nothing here is proof of operation.
    `apps/bootstrapper`'s engine check is what proves that, at install time, by
    running the worker and asking NVML whether that process is on a device.
    These two functions only stop a payload from being *assembled* with a
    graphics-card worker and no way to load it, which is the failure that does
    not degrade: Windows cannot resolve the imports and the worker never starts.

    Measured 2026-08-20: the shipped payload is CPU-only, so
    `Get-GraniteWorkerProvider` returns `cpu` and the packaging check never
    fires. It was written before it was needed because the moment it is needed is
    the moment a CUDA worker is first packaged, and that is the worst moment to be
    writing it. `Get-RequiredCudaRuntimeFile` stopped being unused on 2026-08-21,
    when `Enable-GraniteCuda.ps1` began staging from it.
#>

Set-StrictMode -Version Latest

function Get-GraniteWorkerProvider {
    <#
    .SYNOPSIS
        `cuda` if this worker binary has llama.cpp's CUDA backend compiled in,
        otherwise `cpu`.

    .DESCRIPTION
        Read out of the binary rather than inferred from how it was built, because
        the build flag and the file that ends up in the payload are two different
        things -- `Enable-GraniteCuda.ps1` copies one over the other on purpose,
        and a `--features cuda` build left in `target\release` outlives the
        command that made it.

        The marker is `ggml-cuda`, which is what `Enable-GraniteCuda.ps1` already
        uses to identify a staged worker. Size would also separate them today
        (57 MB against 4 MB) and is exactly the kind of threshold that stops
        being true.
    #>
    param([Parameter(Mandatory)][string]$WorkerPath)

    if (-not (Test-Path -LiteralPath $WorkerPath -PathType Leaf)) {
        throw "No Granite worker to inspect at $WorkerPath"
    }
    $bytes = [IO.File]::ReadAllBytes($WorkerPath)
    $text = [Text.Encoding]::ASCII.GetString($bytes)
    if ($text.Contains('ggml-cuda')) { return 'cuda' }
    return 'cpu'
}

function Get-RequiredCudaRuntimeFile {
    <#
    .SYNOPSIS
        The file names a CUDA Granite worker needs in its own directory.

    .DESCRIPTION
        Read out of `models/trusted-manifest.json`'s pinned `proof_files`, which
        is the same place `speakeasy_models::required_cuda_runtime_files` reads
        them and the same digests the downloader verifies. A hand-written list
        here is how this workspace came to name `cudart64_13.dll` in one script
        and pin `cudart64_12.dll` in the catalog -- and that mismatch outlived
        the comment describing it, because until the list became enforced
        nothing read either side.
    #>
    param([Parameter(Mandatory)][string]$RepositoryRoot)

    $manifestPath = Join-Path $RepositoryRoot 'models\trusted-manifest.json'
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    # The ids `granite_gpu.rs` names. Kept in step by
    # `the_packager_and_the_models_crate_require_the_same_cuda_libraries`.
    $ids = @(
        'nvidia-cuda-cudart-windows-x64-13.3.29',
        'nvidia-libcublas-windows-x64-13.6.0.2'
    )
    $files = foreach ($artifact in $manifest.artifacts) {
        if ($artifact.id -notin $ids) { continue }
        foreach ($proof in $artifact.proof_files) { Split-Path -Leaf $proof.path }
    }
    @($files | Sort-Object -Unique)
}

function Assert-GraniteWorkerPayloadIsCoherent {
    <#
    .SYNOPSIS
        Refuse to package a graphics-card worker without the libraries it loads.

    .DESCRIPTION
        The whole point of this file. A CUDA worker with no `cudart` beside it is
        not a slower install -- it is an install whose engine cannot start, and
        the error Windows gives for an unresolved import names nothing a user can
        act on.

        Returns the provider the payload actually carries, so the caller can
        record it rather than assume it.
    #>
    param(
        [Parameter(Mandatory)][string]$WorkerPath,
        [Parameter(Mandatory)][string]$RepositoryRoot,
        # Where the worker will sit once installed, and therefore where its
        # libraries have to be. A separate parameter because at packaging time
        # the worker is still in `target\release` and the payload directory is
        # somewhere else entirely.
        [Parameter(Mandatory)][string]$StagedDirectory
    )

    $provider = Get-GraniteWorkerProvider -WorkerPath $WorkerPath
    if ($provider -ne 'cuda') { return $provider }

    $required = Get-RequiredCudaRuntimeFile -RepositoryRoot $RepositoryRoot
    $missing = @($required | Where-Object { -not (Test-Path -LiteralPath (Join-Path $StagedDirectory $_) -PathType Leaf) })
    if ($missing.Count -gt 0) {
        throw ("This payload carries a graphics-card Granite worker and not the libraries it " +
            "loads, so its engine could not start once installed. Missing from " +
            "${StagedDirectory}: $($missing -join ', '). Stage them beside the worker, or " +
            'package a processor build.')
    }
    return $provider
}
