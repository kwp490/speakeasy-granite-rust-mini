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
# Resolved against the repository root when it is relative. `Set-Location` does
# not move the process working directory `GetFullPath` reads, so a relative
# argument otherwise resolves against wherever PowerShell was started.
if (-not [IO.Path]::IsPathRooted($ArtifactRoot)) {
    $ArtifactRoot = Join-Path $repositoryRoot $ArtifactRoot
}
# **Validated here, not only in whatever called this.** `ArtifactRoot` is
# caller-controlled and the next statement removes it recursively. A prefix
# comparison was the whole guard, and a prefix says nothing about a junction on
# `local-development\` sending that delete out of the workspace. `local-development
# \<version>` is nested by design, so every existing component is walked rather
# than depth being refused.
. (Join-Path $PSScriptRoot 'DeleteContainment.ps1')
$artifactFull = Remove-ContainedDirectory -Path $ArtifactRoot -Root $targetRoot -Label 'ArtifactRoot'

# The Tauri NSIS bundle-marker rewrite that used to live here is gone with NSIS.
# It existed because the bundler patched a marker byte-range into the executable
# it embedded, so the file on disk after installation did not hash the same as
# the one in `target/release`. The bootstrapper copies the executable unchanged,
# so the installed file and the built file are now the same bytes and a plain
# hash is correct.

# **`-FreshBuild` is not optional here.** Cargo decides what to rebuild from
# source timestamps, so a source restored from a copy is older than the artifact
# built from the version that replaced it and the next package carries the old
# code. Every release therefore pays a cold build.
$installerBuild = Join-Path $targetRoot 'installer-build'
& (Join-Path $PSScriptRoot 'Invoke-ProofPackage.ps1') -FreshBuild -BuildRoot $installerBuild
if ($LASTEXITCODE -ne 0) { throw 'Canonical proof build failed.' }

# Read back rather than recomputed. The paths below are the ones the build says
# it produced, so a build that wrote somewhere else cannot be packaged from the
# directory this script guessed at.
$packageManifestPath = Join-Path $installerBuild 'proof-package.json'
if (-not (Test-Path -LiteralPath $packageManifestPath -PathType Leaf)) {
    throw "The proof build left no package manifest at $packageManifestPath."
}
$proofPackage = Get-Content -LiteralPath $packageManifestPath -Raw | ConvertFrom-Json
# The build root was recreated before Cargo ran, so the manifest's presence proves
# this run wrote it. The mode is asserted too, because a future edit could drop
# the switch and nothing else would notice.
if (-not $proofPackage.fresh_build) {
    throw 'The proof build reported fresh_build=false; a release must not be packaged from an incremental target.'
}
if ($proofPackage.build_root -ne [IO.Path]::GetFullPath($installerBuild).TrimEnd([IO.Path]::DirectorySeparatorChar)) {
    throw "The proof build reported build_root $($proofPackage.build_root), not $installerBuild."
}
$bootstrapperSource = $proofPackage.executables.bootstrapper.path
$desktopSource = $proofPackage.executables.desktop.path
$graniteWorkerSource = $proofPackage.executables.granite_worker.path
$packer = $proofPackage.executables.payload_packer.path
foreach ($required in @($bootstrapperSource, $desktopSource, $graniteWorkerSource, $packer)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "The proof build named an executable that is not there: $required"
    }
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
# No native DLLs are staged beside the workers. This carried a
# `Resolve-RuntimeDll` helper and the same five sherpa/ONNX names
# `Invoke-ProofPackage.ps1` did, both fetched from `Get-GpuRuntime.ps1` -- a
# script the fork deleted with the engine that needed it. `speakeasy-granite`
# compiles llama.cpp into the worker, so `granite-worker.exe` stands alone on a
# CPU install.

$payloadSpecs = @(
    @($desktopSource, 'ai-speakeasy-mini.exe', 'desktop'),
    @($bootstrapperSource, 'speakeasy-bootstrapper.exe', 'bootstrapper'),
    # CPU-only, built with `speakeasy-granite-worker`'s default features -- see
    # Invoke-ProofPackage.ps1. Its GGUF model files are not bundled here, same
    # as every other ASR pack: setup fetches them when no verified copy is
    # already on disk, and verifies each against models/trusted-manifest.json.
    # `Get-Granite.ps1` fetches the same files by the same pins, but it is a
    # development convenience for staging a dev tree -- production setup uses
    # the bootstrapper's own download path and never runs it.
    @($graniteWorkerSource, 'proof/granite-worker.exe', 'granite-worker')
)
# What the payload actually carries, checked rather than assumed.
#
# A payload may not claim a graphics-card worker without the libraries that
# worker loads: Windows cannot resolve the imports and the engine never starts,
# which is a failure that names no file the user can act on. `provider` below is
# read out of the binary, so the description cannot describe a build somebody
# replaced between two runs.
#
# It is still not a claim that the graphics card *works*. Setup's engine check
# proves that, at install time, by running the worker and asking NVML whether
# that process is on a device -- and only then does anything write `cuda` into
# `install-provider.txt`.
. (Join-Path $PSScriptRoot 'GraniteWorkerProvider.ps1')
$graniteWorkerProvider = Assert-GraniteWorkerPayloadIsCoherent `
    -WorkerPath $graniteWorkerSource `
    -RepositoryRoot $repositoryRoot `
    -StagedDirectory (Join-Path $payloadRoot 'proof')

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
        # Only the Granite worker has a provider to declare. The others run the
        # same way whatever the machine has.
        provider = if ($spec[2] -eq 'granite-worker') { $graniteWorkerProvider } else { $null }
        bytes = $item.Length
        # A plain hash for every role now. The desktop executable used to need a
        # rewritten one because the NSIS bundler patched a marker into the copy
        # it embedded; the bootstrapper copies it unchanged.
        sha256 = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
# The one file a user downloads.
#
# `speakeasy-bootstrapper.exe` and `payload\` both stay beside it: the
# bootstrapper reads an embedded payload in preference to a sibling directory,
# so the pair is still exactly what `Test-InstallerLifecycle.ps1` drives, and
# keeping it means the lifecycle proof exercises the same install code the setup
# file does rather than a second path.
#
# Packed by a Rust binary from `apps/bootstrapper`, not here. The format has to
# have one implementation: a writer in PowerShell and a reader in the installer
# agree until somebody edits one of them, and the disagreement lands on a user
# as "this download is damaged" for a file that downloaded perfectly.
$setupExecutable = Join-Path $artifactFull 'SpeakEasyMiniSetup.exe'
& $packer $payloadRoot $bootstrapperSource $setupExecutable
if ($LASTEXITCODE -ne 0) { throw 'Packing SpeakEasyMiniSetup.exe failed.' }

# Computed after the packing, so the manifest describes the file a user actually
# downloads. It used to run before the payload was even assembled, which was
# harmless while the artifact root held only documents and is not now.
$components = Get-ChildItem -LiteralPath $artifactFull -File |
    Sort-Object Name |
    ForEach-Object {
        [ordered]@{
            name = $_.Name
            bytes = $_.Length
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
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
    # First, because it is the thing that gets published and the thing a user
    # downloads. The bootstrapper below it is the same program without its
    # payload, kept for the lifecycle proof.
    setup = $setupExecutable
    setup_bytes = (Get-Item -LiteralPath $setupExecutable).Length
    setup_sha256 = (Get-FileHash -LiteralPath $setupExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    bootstrapper = $bootstrapperCopy
    bootstrapper_bytes = (Get-Item -LiteralPath $bootstrapperCopy).Length
    bootstrapper_sha256 = (Get-FileHash -LiteralPath $bootstrapperCopy -Algorithm SHA256).Hash.ToLowerInvariant()
    payload_files = $payloadFiles.Count
    payload_bytes = ($payloadFiles | Measure-Object -Sum Length).Sum
    signed = $false
    local_development = $true
} | Format-List
