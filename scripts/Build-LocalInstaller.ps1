[CmdletBinding()]
param(
    [string]$ArtifactRoot
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$workspaceCargoHome = Join-Path $repositoryRoot '.tools\cargo'
$workspaceRustupHome = Join-Path $repositoryRoot '.tools\rustup'
$workspaceCargoExe = Join-Path $workspaceCargoHome 'bin\cargo.exe'
$workspaceCargoReady = (Test-Path -LiteralPath $workspaceCargoExe -PathType Leaf) -and
    ((Get-Item -LiteralPath $workspaceCargoExe).Length -gt 0)
if ($workspaceCargoReady) {
    $env:CARGO_HOME = $workspaceCargoHome
    $env:RUSTUP_HOME = $workspaceRustupHome
    $env:PATH = (Join-Path $workspaceCargoHome 'bin') + ';' + $env:PATH
}
$targetRoot = (Resolve-Path (Join-Path $repositoryRoot 'target')).Path
$productVersion = & (Join-Path $PSScriptRoot 'Get-ProductVersion.ps1')
if (-not $ArtifactRoot) {
    # `<version>`, matching what README.md and docs\STREAMING-OBJECTIVE.md already
    # document as `target\local-development\<version>\`. The scripts were the ones
    # that had drifted from the docs, not the other way round.
    $ArtifactRoot = Join-Path $targetRoot "local-development\$productVersion"
}
$artifactFull = [IO.Path]::GetFullPath($ArtifactRoot)
if (-not $artifactFull.StartsWith($targetRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'ArtifactRoot must remain under the workspace target directory.'
}
if (Test-Path -LiteralPath $artifactFull) {
    Remove-Item -LiteralPath $artifactFull -Recurse -Force
}

# The Tauri NSIS bundle-marker rewrite that used to live here is gone with NSIS.
# It existed because the bundler patched a marker byte-range into the executable
# it embedded, so the file on disk after installation did not hash the same as
# the one in `target/release`. The bootstrapper copies the executable unchanged,
# so the installed file and the built file are now the same bytes and a plain
# hash is correct.

& (Join-Path $PSScriptRoot 'Invoke-ProofPackage.ps1')
if ($LASTEXITCODE -ne 0) { throw 'Canonical proof build failed.' }

$installerBuild = Join-Path $targetRoot 'installer-build'
$bootstrapperSource = Join-Path $installerBuild 'release\speakeasy-bootstrapper.exe'
if (-not (Test-Path -LiteralPath $bootstrapperSource -PathType Leaf)) {
    throw "The bootstrapper was not produced: $bootstrapperSource"
}

New-Item -ItemType Directory -Path $artifactFull -Force | Out-Null
Copy-Item -LiteralPath $bootstrapperSource -Destination $artifactFull

# The payload the bootstrapper installs is assembled below, from `$payloadSpecs`
# — the same list the install manifest is built from. Deliberately not from
# `src-tauri/proof-stage`: `Invoke-ProofPackage.ps1` deletes that directory in its
# `finally`, so reading it here found an empty tree and failed at the first file.
# One list feeding both the copy and the manifest also means the payload cannot
# contain something the manifest does not describe.
$payloadRoot = Join-Path $artifactFull 'payload'
New-Item -ItemType Directory -Path $payloadRoot -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $repositoryRoot 'packaging\THIRD-PARTY-NOTICES.txt') -Destination $artifactFull
Copy-Item -LiteralPath (Join-Path $repositoryRoot 'packaging\MODEL-NOTICES.md') -Destination $artifactFull
Copy-Item -LiteralPath (Join-Path $repositoryRoot 'packaging\SOURCE-NOTICE.md') -Destination $artifactFull
Copy-Item -LiteralPath (Join-Path $repositoryRoot 'CHANGELOG.md') -Destination $artifactFull
Copy-Item -LiteralPath (Join-Path $repositoryRoot 'docs\PRIVACY.md') -Destination $artifactFull
Copy-Item -LiteralPath (Join-Path $repositoryRoot 'docs\SECURITY.md') -Destination $artifactFull

$signature = Get-AuthenticodeSignature -LiteralPath (Join-Path $artifactFull 'speakeasy-bootstrapper.exe')
$webViewVersion = $null
foreach ($key in @(
    'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F1E7E6A8-9D45-4B4C-BD24-3E7E5E7E4B2F}',
    'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F1E7E6A8-9D45-4B4C-BD24-3E7E5E7E4B2F}'
)) {
    if (Test-Path $key) {
        $webViewVersion = (Get-ItemProperty $key -ErrorAction SilentlyContinue).pv
        if ($webViewVersion) { break }
    }
}
if (-not $webViewVersion) {
    $webViewRoot = Join-Path ${env:ProgramFiles(x86)} 'Microsoft\EdgeWebView\Application'
    if (Test-Path -LiteralPath $webViewRoot) {
        $webViewVersion = Get-ChildItem -LiteralPath $webViewRoot -Directory |
            Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
            Sort-Object { [version]$_.Name } -Descending |
            Select-Object -First 1 -ExpandProperty Name
    }
}
$defender = try {
    $status = Get-MpComputerStatus -ErrorAction Stop
    [ordered]@{
        available = $true
        antivirus_enabled = [bool]$status.AntivirusEnabled
        realtime_enabled = [bool]$status.RealTimeProtectionEnabled
        signature_age_days = $status.AntivirusSignatureAge
    }
} catch {
    [ordered]@{ available = $false }
}
$components = Get-ChildItem -LiteralPath $artifactFull -File |
    Sort-Object Name |
    ForEach-Object {
        [ordered]@{
            name = $_.Name
            bytes = $_.Length
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
$sherpaCudaLib = Join-Path $repositoryRoot '.tools\sherpa-onnx\current\lib'
function Resolve-RuntimeDll {
    param([Parameter(Mandatory)][string]$Name)
    $source = Get-ChildItem -Path $sherpaCudaLib -Filter $Name -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if (-not $source) {
        throw "Required runtime DLL not found -- run .\scripts\Get-GpuRuntime.ps1 first: $Name"
    }
    $source.FullName
}
# Same sherpa CPU-capable set `Invoke-ProofPackage.ps1` stages into `proof/`.
# The CUDA execution provider and its cuBLAS/cuFFT/cuDNN dependencies are
# excluded, and the reason has changed: it used to be that their combined size
# overflowed the NSIS compiler, which no longer exists here. It is now the
# fetch-first policy -- ~2.97 GB that `gpu_runtime.rs` downloads on demand,
# pinned in `models/trusted-manifest.json`, only on machines that want it.
$runtimeDllNames = @(
    'cargs.dll', 'onnxruntime.dll',
    'onnxruntime_providers_shared.dll', 'sherpa-onnx-c-api.dll', 'sherpa-onnx-cxx-api.dll'
)
$payloadSpecs = @(
    @((Join-Path $installerBuild 'release\ai-speakeasy-desktop.exe'), 'ai-speakeasy-desktop.exe', 'desktop'),
    @((Join-Path $installerBuild 'release\speakeasy-bootstrapper.exe'), 'speakeasy-bootstrapper.exe', 'bootstrapper'),
    @((Join-Path $installerBuild 'release\speakeasy-inference-worker.exe'), 'proof/inference-worker.exe', 'worker'),
    # CPU-only, built with `speakeasy-granite-worker`'s default features -- see
    # Invoke-ProofPackage.ps1. Its GGUF model files are not bundled here, same
    # as every other ASR pack: they are fetched on demand by Get-Granite.ps1
    # after install, verified against models/trusted-manifest.json.
    @((Join-Path $installerBuild 'release\speakeasy-granite-worker.exe'), 'proof/granite-worker.exe', 'granite-worker')
) + @($runtimeDllNames | ForEach-Object {
    ,@((Resolve-RuntimeDll -Name $_), "proof/$_", 'native-dll')
})
$installedPayload = foreach ($spec in $payloadSpecs) {
    $source = $spec[0]
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Installed payload source is missing: $source"
    }
    $item = Get-Item -LiteralPath $source
    # Copy into the payload as the same pass that describes it, so the described
    # layout and the copied one cannot disagree.
    $payloadDestination = Join-Path $payloadRoot $spec[1].Replace('/', '\')
    $payloadParent = Split-Path -Parent $payloadDestination
    if ($payloadParent -and -not (Test-Path -LiteralPath $payloadParent)) {
        New-Item -ItemType Directory -Path $payloadParent -Force | Out-Null
    }
    Copy-Item -LiteralPath $source -Destination $payloadDestination
    [ordered]@{
        role = $spec[2]
        installed_relative_path = $spec[1]
        bytes = $item.Length
        # A plain hash for every role now. The desktop executable used to need a
        # rewritten one because the NSIS bundler patched a marker into the copy
        # it embedded; the bootstrapper copies it unchanged.
        sha256 = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
$manifest = [ordered]@{
    schema_version = 1
    product = 'SpeakEasy'
    # The version that was actually built. As a literal this field could claim
    # 1.0.0 while shipping something else, and `Test-LocalInstall.ps1` reads this
    # manifest — so the lie would have been the thing under test.
    version = $productVersion
    generated_utc = [DateTime]::UtcNow.ToString('o')
    canonical_installer = 'speakeasy-bootstrapper'
    install_mode = 'currentUser'
    elevation_required = $false
    # Detected and reported, never fetched: the Evergreen Bootstrapper is served
    # from a redirect whose bytes change by design and so cannot be pinned, and
    # this project downloads nothing it cannot pin (owner decision 2026-08-15).
    webview2_variant = 'detectOnly'
    webview2_detected_version = $webViewVersion
    architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    os_description = [Runtime.InteropServices.RuntimeInformation]::OSDescription
    os_version = [Environment]::OSVersion.VersionString
    signing = [ordered]@{
        authenticode_status = $signature.Status.ToString()
        signed = ($signature.Status -eq 'Valid')
        # Never signed, by owner decision (2026-08-14). Recorded as a decision so
        # the field does not read as a prerequisite somebody forgot to configure.
        signing_planned = $false
        credentials_provisioned = $false
    }
    updater = [ordered]@{
        ui_enabled = $false
        endpoint_provisioned = $false
        keys_provisioned = $false
    }
    qualification = [ordered]@{
        local_development = $true
        smart_screen_reputation_claim = $false
        clean_vm_certified = $false
        public_notices_reviewed = $false
        winget = $false
    }
    defender = $defender
    installed_payload = $installedPayload
    components = $components
}
$manifestPath = Join-Path $artifactFull 'current-host-install-manifest.json'
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding utf8

$checksumFiles = Get-ChildItem -LiteralPath $artifactFull -File |
    Where-Object Name -ne 'SHA256SUMS' |
    Sort-Object Name
$checksumLines = foreach ($file in $checksumFiles) {
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash *$($file.Name)"
}
$checksumLines | Set-Content -LiteralPath (Join-Path $artifactFull 'SHA256SUMS') -Encoding ascii

$bootstrapperCopy = Join-Path $artifactFull 'speakeasy-bootstrapper.exe'
$payloadFiles = @(Get-ChildItem -LiteralPath $payloadRoot -Recurse -File)
[pscustomobject]@{
    artifact_root = $artifactFull
    bootstrapper = $bootstrapperCopy
    bootstrapper_bytes = (Get-Item -LiteralPath $bootstrapperCopy).Length
    bootstrapper_sha256 = (Get-FileHash -LiteralPath $bootstrapperCopy -Algorithm SHA256).Hash.ToLowerInvariant()
    payload_files = $payloadFiles.Count
    payload_bytes = ($payloadFiles | Measure-Object -Sum Length).Sum
    signed = $false
    local_development = $true
} | Format-List
