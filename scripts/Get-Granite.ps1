<#
.SYNOPSIS
Fetches and verifies Granite Speech's GGUF files against the trusted manifest.

.DESCRIPTION
Hugging Face serves `ibm-granite/granite-speech-4.1-2b-GGUF` as loose files, not
one archive, so there is no single digest to check the way
`scripts/Get-GpuRuntime.ps1` checks sherpa's and NVIDIA's `.tar.bz2`/`.zip`
archives. This script downloads each required file on its own and verifies it
by length and SHA-256 against `models/trusted-manifest.json` — the same
discipline, applied per file instead of per archive.

Every byte this script will accept is pinned in the manifest and read from
there, not restated here. A mismatch means the file changed under a fixed URL,
which is a supply-chain event rather than something to retry through, so it
throws.

.PARAMETER PackId
Which Granite pack to fetch. Defaults to whichever `final-asr` pack the
manifest marks `install_eligible` — the Q4_K_M pack since 2026-08-04, when
measurement replaced Q8_0 with it (docs/handoff/granite-final-pass.md, Phase
9). Resolved from the manifest rather than hardcoded, so the default follows a
future swap instead of silently fetching the pack the app no longer loads.
Pass an explicit id to fetch a recorded alternative such as
`granite-speech-4.1-2b-q8_0-cpu`; it is not install-eligible, so nothing in
the app will select it on its own.

.PARAMETER Destination
Where to place the verified files. Defaults to `.tools/granite-speech-4.1-2b`,
the layout `speakeasy-granite`'s own hardware proofs and
`workers/granite-worker` already expect.

.PARAMETER SkipDownload
Verify what is already on disk; fail rather than fetch anything.

.EXAMPLE
.\scripts\Get-Granite.ps1
.EXAMPLE
.\scripts\Get-Granite.ps1 -PackId granite-speech-4.1-2b-q8_0-cpu
.EXAMPLE
.\scripts\Get-Granite.ps1 -SkipDownload
#>
[CmdletBinding()]
param(
    [string]$PackId,
    [string]$Destination,
    [switch]$SkipDownload
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repositoryRoot = Split-Path -Parent $PSScriptRoot
if (-not $Destination) {
    $Destination = Join-Path $repositoryRoot '.tools/granite-speech-4.1-2b'
}

$manifestPath = Join-Path $repositoryRoot 'models/trusted-manifest.json'
$manifest = Get-Content -Raw -Encoding utf8 $manifestPath | ConvertFrom-Json

if ($PackId) {
    $pack = $manifest.packs | Where-Object { $_.id -eq $PackId }
    if (-not $pack) {
        throw "trusted-manifest.json has no pack with id '$PackId'"
    }
} else {
    # The same admission the app's own resolver applies: exactly one
    # `final-asr` pack is install-eligible, and it is the one a dictation will
    # load. Refusing on a tie rather than picking the first keeps this honest
    # if a future edit flips both packs on -- the app would report `Ambiguous`
    # in that state too, and a script that quietly downloaded one of them
    # would hide it.
    $eligible = @($manifest.packs | Where-Object {
        $_.role -eq 'final-asr' -and $_.install_eligible
    })
    if ($eligible.Count -ne 1) {
        throw "expected exactly one install-eligible final-asr pack in trusted-manifest.json, found $($eligible.Count); pass -PackId explicitly"
    }
    $pack = $eligible[0]
}
if ($pack.archive) {
    throw "$PackId carries an archive; this script is for the archive-less, loose-file shape only"
}

Write-Host "pack: $($pack.id)@$($pack.revision) ($($pack.display_name))"
New-Item -ItemType Directory -Force -Path $Destination | Out-Null

foreach ($file in $pack.required_files) {
    $path = Join-Path $Destination $file.path

    if (-not (Test-Path $path)) {
        if ($SkipDownload) {
            throw "$($file.path) is not on disk and -SkipDownload was given: $path"
        }
        if (-not $file.url) {
            throw "$($file.path) has no URL in the manifest and is not already on disk"
        }
        Write-Host "downloading $($file.path) ..."
        Invoke-WebRequest -Uri $file.url -OutFile $path
    }

    $actualBytes = (Get-Item $path).Length
    if ($actualBytes -ne $file.bytes) {
        throw "$($file.path): expected $($file.bytes) bytes, got $actualBytes"
    }
    $actualHash = (Get-FileHash -Algorithm SHA256 $path).Hash.ToLower()
    if ($actualHash -ne $file.sha256) {
        throw "$($file.path): SHA-256 mismatch. expected $($file.sha256), got $actualHash"
    }
    Write-Host ("verified {0,-40} {1,12:N0} bytes" -f $file.path, $actualBytes)
}

Write-Host ''
Write-Host "Granite files verified at $Destination"
Write-Host 'Prove the engine actually runs on them:'
Write-Host '  cargo test -p speakeasy-granite granite --locked -- --ignored --nocapture'
