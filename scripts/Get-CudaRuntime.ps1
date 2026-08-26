<#
.SYNOPSIS
Fetch NVIDIA's CUDA redistributables and print the `models/trusted-manifest.json`
entries that pin them.

.DESCRIPTION
The catalog pins the two CUDA libraries a CUDA-built Granite worker loads --
`cudart` and `cuBLAS` -- by archive digest *and* by the digest of each file
inside. `speakeasy_models::required_cuda_runtime_files` reads the file names back
out of those `proof_files`, so the pin is not documentation: it is the enforced
requirement list, and `inspect_gpu_payload` refuses a payload missing any of
them. See `docs/ARCHITECTURE.md` under "Which provider runs, and how you find
out".

This script is how those entries are produced, and it exists because the
alternative is a set of numbers nobody can re-derive. Its predecessor
(`Get-GpuRuntime.ps1`) was deleted with the streaming engine, and the 12.9
entries it had produced then sat in the catalog for months as unreproducible
constants -- which is exactly the state that let a `cudart64_13.dll` in one
script coexist with a `cudart64_12.dll` pinned in the catalog.

Nothing here is a decision about providers. It downloads, verifies, and prints
JSON; pasting the result into the catalog is a human edit, deliberately.

# Two digests, checked against two independent sources

The archive's length and SHA-256 come from NVIDIA's own `redistrib_<version>.json`
-- fetched, never typed -- and are then re-checked against the bytes that
actually arrived. That is the pair that matters: the manifest says what NVIDIA
published, and the download says what this machine received. A truncated or
substituted transfer fails the second check while passing nothing.

The per-file digests cannot come from NVIDIA, who publish none, so they come from
the verified archive's own members. That is why the archive check has to happen
first: a per-file digest taken from an unverified download would look identical
to a real pin.

# The toolkit comparison, and why it is here

`Enable-GraniteCuda.ps1` staged the libraries from the *installed CUDA Toolkit*
rather than from these archives, because a machine building a CUDA worker already
has them on disk. That was only sound if the toolkit's bytes were the bytes this
catalog pins, and "the version numbers look the same" is not that claim. So this
script compares them by digest and says so per file.

That script was retired on 2026-08-26, when setup began fetching these archives
itself, so the comparison no longer guards a staging path. **It is kept because
it is now the cheaper claim to make**: it says the archive a user downloads
contains the same code as a real NVIDIA toolkit installation, which nothing else
here checks and which no amount of digest-pinning the archive can establish on
its own. A dev staging a worker by hand still copies from the toolkit, and this
is still how they know those bytes match. A mismatch is not
necessarily wrong -- it means the toolkit is a different component version than
the one being pinned -- but it does mean the staged libraries are not the pinned
ones, and someone has to choose which to move.

.PARAMETER RedistributableVersion
Which `redistrib_<version>.json` to pin from. The default matches the CUDA
Toolkit this workspace builds against; `-ListVersions` shows what NVIDIA offers.

The DLL soname digit follows this version's major component -- CUDA 13 ships
`cudart64_13.dll` -- so a major bump changes the required file names, which is
the whole reason they are pinned rather than pattern-matched.

.PARAMETER CacheRoot
Where archives and extracted files are kept. Defaults to `.tools\cuda-runtime`,
which is gitignored. Downloads are reused when their digest already matches, so a
re-run after a catalog edit costs nothing.

.PARAMETER ToolkitPath
CUDA Toolkit root to compare against. Defaults to `$env:CUDA_PATH`, then the
highest version under `%ProgramFiles%\NVIDIA GPU Computing Toolkit\CUDA`. Skipped
with a note when there is none, since this script is useful on a machine with no
toolkit at all.

.PARAMETER ListVersions
Print the `redistrib_13.*.json` manifests NVIDIA currently serves, and exit.

.PARAMETER Force
Re-download even when a cached archive already matches its published digest.

.EXAMPLE
.\scripts\Get-CudaRuntime.ps1

.EXAMPLE
.\scripts\Get-CudaRuntime.ps1 -ListVersions

.EXAMPLE
.\scripts\Get-CudaRuntime.ps1 -RedistributableVersion 13.3.0
#>
[CmdletBinding()]
param(
    [string]$RedistributableVersion = '13.3.1',
    [string]$CacheRoot,
    [string]$ToolkitPath,
    [switch]$ListVersions,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot

# Resolved against the repository root rather than the current location:
# `Set-Location` does not move the process working directory, so
# `GetFullPath('.tools\...')` would resolve against wherever the shell started.
if (-not $CacheRoot) {
    $CacheRoot = Join-Path $repositoryRoot '.tools\cuda-runtime'
}

$redistBase = 'https://developer.download.nvidia.com/compute/cuda/redist'

if ($ListVersions) {
    $index = (Invoke-WebRequest -Uri "$redistBase/" -UseBasicParsing).Content
    [regex]::Matches($index, 'redistrib_13\.[0-9.]+\.json') |
        ForEach-Object { $_.Value } |
        Sort-Object -Unique
    exit 0
}

# Which libraries each component contributes to `proof_files`, by **file name**.
# Declared here rather than discovered, because "every DLL we found" is not a
# requirement -- it is whatever NVIDIA happened to ship. cuBLAS contributes two
# because cuBLAS loads cuBLASLt itself at run time, so a payload carrying only
# the import-table dependency still fails to start.
#
# Names, not paths. Where inside the archive NVIDIA puts a library is a property
# of the archive rather than of our requirements -- the same distinction
# `archive_prefix` draws in `models/trusted-manifest.json` -- and CUDA 13 moved
# them: 12.9 shipped `bin/cudart64_12.dll` and 13.3 ships
# `bin/x64/cudart64_13.dll`. Hardcoding the directory would have made a layout
# change read as a missing file. The name is still exact, and an archive with no
# match, or more than one, is refused rather than guessed at.
#
# The soname digit follows CUDA's major version, so it is derived from the
# version being pinned instead of written twice.
$major = ($RedistributableVersion -split '\.')[0]
$components = @(
    [pscustomobject]@{
        Component  = 'cuda_cudart'
        IdPrefix   = 'nvidia-cuda-cudart-windows-x64'
        ProofFiles = @("cudart64_$major.dll")
    }
    [pscustomobject]@{
        Component  = 'libcublas'
        IdPrefix   = 'nvidia-libcublas-windows-x64'
        ProofFiles = @("cublas64_$major.dll", "cublasLt64_$major.dll")
    }
)

$downloadRoot = Join-Path $CacheRoot 'download'
$extractRoot = Join-Path $CacheRoot 'extract'
foreach ($directory in @($downloadRoot, $extractRoot)) {
    if (-not (Test-Path -LiteralPath $directory -PathType Container)) {
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
    }
}

Add-Type -AssemblyName System.IO.Compression.FileSystem

function Get-Sha256 {
    param([Parameter(Mandatory)][string]$Path)
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$manifestUrl = "$redistBase/redistrib_$RedistributableVersion.json"
Write-Host "Reading $manifestUrl"
$published = (Invoke-WebRequest -Uri $manifestUrl -UseBasicParsing).Content | ConvertFrom-Json

$artifacts = @()
$comparisons = @()

foreach ($component in $components) {
    $entry = $published.($component.Component)
    if (-not $entry) {
        throw "redistrib_$RedistributableVersion.json publishes no $($component.Component)."
    }
    $platform = $entry.'windows-x86_64'
    if (-not $platform) {
        throw "$($component.Component) $($entry.version) publishes no windows-x86_64 archive."
    }

    $url = "$redistBase/$($platform.relative_path)"
    $archiveName = Split-Path -Leaf $platform.relative_path
    $archivePath = Join-Path $downloadRoot $archiveName
    # NVIDIA wraps every redistributable in `<name>-<version>-archive/`, and the
    # archive name is that directory plus `.zip`. Read from the published path
    # rather than composed from the version, so a republish that changes the
    # naming moves the prefix with the URL it came from.
    $archivePrefix = [IO.Path]::GetFileNameWithoutExtension($archiveName)

    $needsDownload = $true
    if (-not $Force -and (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
        if ((Get-Sha256 $archivePath) -eq $platform.sha256.ToLowerInvariant()) {
            Write-Host "Cached $archiveName already matches its published digest."
            $needsDownload = $false
        }
    }
    if ($needsDownload) {
        Write-Host "Downloading $archiveName ($([math]::Round($platform.size / 1MB, 1)) MB)"
        # Not `Invoke-WebRequest`: under PowerShell 5.1 it buffers the whole
        # response in memory before writing, and these archives run to hundreds
        # of megabytes. WebClient streams straight to the file.
        $client = New-Object System.Net.WebClient
        try {
            $client.DownloadFile($url, $archivePath)
        } finally {
            $client.Dispose()
        }
    }

    # Length before digest, because a truncated transfer is the likely failure
    # and its message should say so rather than report a hash nobody can act on.
    $actualBytes = (Get-Item -LiteralPath $archivePath).Length
    if ($actualBytes -ne $platform.size) {
        throw ("$archiveName is $actualBytes bytes; NVIDIA publishes $($platform.size). " +
            'Delete it from the cache and re-run.')
    }
    $actualDigest = Get-Sha256 $archivePath
    if ($actualDigest -ne $platform.sha256.ToLowerInvariant()) {
        throw ("$archiveName hashes to $actualDigest; NVIDIA publishes $($platform.sha256). " +
            'These bytes are not the published archive.')
    }
    Write-Host "Verified $archiveName against redistrib_$RedistributableVersion.json."

    # The whole archive's uncompressed size, read from the central directory
    # rather than by extracting it. `extracted_bytes` is what the manifest
    # validator compares the `proof_files` total against, and extracting cuBLAS
    # in full writes well over a gigabyte to learn one number.
    $zip = [IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        # Cast, because `Measure-Object -Sum` returns a double and
        # `ConvertTo-Json` then writes `9570537.0` -- which the manifest's `u64`
        # refuses, at parse time, in a file nothing else would explain.
        $extractedBytes = [uint64](($zip.Entries | Measure-Object -Property Length -Sum).Sum)
        $proofFiles = @()
        foreach ($wanted in $component.ProofFiles) {
            # Exactly one, by name. Two entries with the same file name in one
            # archive would mean NVIDIA ships per-architecture copies, and
            # picking either silently would pin a library the loader might not be
            # the one to resolve.
            $candidates = @($zip.Entries | Where-Object { $_.Name -eq $wanted })
            if ($candidates.Count -eq 0) {
                throw ("$archiveName contains no $wanted. The soname digit follows CUDA's " +
                    "major version -- check that $RedistributableVersion is the major this " +
                    'catalog means to pin.')
            }
            if ($candidates.Count -gt 1) {
                throw ("$archiveName contains $($candidates.Count) copies of $wanted " +
                    "($($candidates.FullName -join ', ')). Which one the worker loads is not " +
                    'this script''s call to make.')
            }
            $found = $candidates[0]
            # The path recorded in the manifest, with NVIDIA's wrapping directory
            # stripped, because that prefix is pinned separately.
            $wantedPath = $found.FullName.Substring($archivePrefix.Length + 1)
            $staging = Join-Path $extractRoot (Join-Path $archivePrefix $wantedPath)
            $stagingParent = Split-Path -Parent $staging
            if (-not (Test-Path -LiteralPath $stagingParent -PathType Container)) {
                New-Item -ItemType Directory -Path $stagingParent -Force | Out-Null
            }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($found, $staging, $true)
            $digest = Get-Sha256 $staging
            $proofFiles += [ordered]@{
                path   = $wantedPath
                bytes  = $found.Length
                sha256 = $digest
            }
            $comparisons += [pscustomobject]@{
                File   = $wanted
                Pinned = $digest
                Bytes  = $found.Length
            }
        }
    } finally {
        $zip.Dispose()
    }

    $artifacts += [ordered]@{
        id             = "$($component.IdPrefix)-$($entry.version)"
        kind           = 'native-runtime'
        version        = $entry.version
        url            = $url
        archive_bytes  = $actualBytes
        archive_sha256 = $actualDigest
        archive_prefix = $archivePrefix
        extracted_bytes = $extractedBytes
        # Not NVIDIA's own `license` field, which reads "CUDA Toolkit" -- the
        # name of the product rather than of the licence. The catalog records
        # what the terms actually are, and has since the 12.9 entries.
        licenses       = @('NVIDIA CUDA Toolkit EULA')
        proof_files    = $proofFiles
        proof_status   = 'hash-verified-against-nvidia-redistrib-manifest-and-loaded-by-cuda-provider'
    }
}

# The toolkit comparison. Skipped rather than fatal when there is no toolkit:
# this script's job is producing the pin, and a machine that only pins does not
# need one installed.
if (-not $ToolkitPath) { $ToolkitPath = $env:CUDA_PATH }
if (-not $ToolkitPath) {
    $toolkits = Join-Path $env:ProgramFiles 'NVIDIA GPU Computing Toolkit\CUDA'
    $ToolkitPath = Get-ChildItem -LiteralPath $toolkits -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if ($ToolkitPath) {
    $toolkitBin = Join-Path $ToolkitPath 'bin\x64'
    if (-not (Test-Path -LiteralPath $toolkitBin -PathType Container)) {
        $toolkitBin = Join-Path $ToolkitPath 'bin'
    }
    Write-Host ''
    Write-Host "Comparing the pinned files against $toolkitBin"
    $report = foreach ($comparison in $comparisons) {
        $installed = Join-Path $toolkitBin $comparison.File
        if (Test-Path -LiteralPath $installed -PathType Leaf) {
            $installedDigest = Get-Sha256 $installed
            [pscustomobject]@{
                file    = $comparison.File
                toolkit = if ($installedDigest -eq $comparison.Pinned) { 'same bytes as pinned' } else { "DIFFERS ($installedDigest)" }
            }
        } else {
            [pscustomobject]@{ file = $comparison.File; toolkit = 'not installed' }
        }
    }
    $report | Format-Table -AutoSize
} else {
    Write-Host ''
    Write-Host 'No CUDA Toolkit found, so the staged-versus-pinned comparison was skipped.'
}

$output = Join-Path $CacheRoot 'manifest-artifacts.json'
$json = ConvertTo-Json -InputObject $artifacts -Depth 6
Set-Content -LiteralPath $output -Value $json -Encoding UTF8
Write-Host ''
Write-Host "Paste these into models/trusted-manifest.json's `"artifacts`" (also written to $output):"
Write-Host ''
Write-Output $json

exit 0
