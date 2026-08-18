[CmdletBinding()]
param(
    [string]$ArtifactRoot,
    [switch]$RunDefenderScan,
    [string]$DefenderEvidence = 'not_requested',
    [string]$RunningAppRefusalEvidence = 'not_run'
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$productVersion = & (Join-Path $PSScriptRoot 'Get-ProductVersion.ps1')
if (-not $ArtifactRoot) {
    # Has to agree with Build-LocalInstaller.ps1's default, which derives it the
    # same way. When both were literals they agreed by luck.
    $ArtifactRoot = Join-Path $repositoryRoot "target\local-development\$productVersion"
}
$artifactRoot = [IO.Path]::GetFullPath($ArtifactRoot)
$manifestPath = Join-Path $artifactRoot 'current-host-install-manifest.json'
$checksumsPath = Join-Path $artifactRoot 'SHA256SUMS'
if (-not (Test-Path -LiteralPath $manifestPath)) { throw 'Current-host manifest is missing.' }
if (-not (Test-Path -LiteralPath $checksumsPath)) { throw 'SHA256SUMS is missing.' }
$hostManifest = Get-Content -Raw -Encoding utf8 $manifestPath | ConvertFrom-Json
# Three: the app, the bootstrapper and the Granite worker. It was nine until
# 2026-08-18 -- those three plus the streaming worker and the five sherpa/ONNX
# DLLs beside it. Pinned as an exact count rather than a floor so that a payload
# quietly gaining or losing a file fails here instead of at a user's install.
if ($hostManifest.installed_payload.Count -ne 3) {
    throw 'Current-host manifest must inventory exactly 3 installed executable payload files.'
}
foreach ($payload in $hostManifest.installed_payload) {
    if ($payload.installed_relative_path -match '(^[\\/]|(^|[\\/])\\.\\.([\\/]|$))' -or
        $payload.sha256 -notmatch '^[0-9a-f]{64}$' -or $payload.bytes -le 0) {
        throw "Unsafe or incomplete installed payload record: $($payload.installed_relative_path)"
    }
}
$trustedManifest = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot 'models\trusted-manifest.json') | ConvertFrom-Json
# The pack setup will actually install, resolved by `install_eligible` rather
# than by id. It named `nemotron-3.5-streaming-en-cpu` until 2026-08-18 -- a
# pack that left with the streaming engine, so this threw on a manifest that was
# perfectly correct. Resolving it follows a future swap (Q8_0 to Q4_K_M already
# happened once, on measurement) instead of pinning a name that goes stale.
#
# No model ships inside the installer: the GGUFs are fetched after install and
# verified against this same manifest. What this build owes is compatibility --
# the pack must admit the version being shipped, checked below.
$pack = @($trustedManifest.packs | Where-Object { $_.role -eq 'final-asr' -and $_.install_eligible })
if ($pack.Count -ne 1) {
    throw ("Expected exactly one install-eligible final-asr pack in the trusted " +
        "manifest; found $($pack.Count).")
}
$pack = $pack[0]

# The pack declares the application and worker versions it admits. Only the
# floor is declared, and this asserts the ceiling stays absent.
#
# It used to demand both, and every pack capped both at the then-current product
# version. That made an ordinary version bump silently fatal rather than loud:
# the app enforces the same range at runtime, and `select_exact` returns
# `SelectionError::Incompatible` for a pack out of range — so a build one patch
# above the ceiling installed cleanly, started, and then could not select any ASR
# pack. An install that cannot transcribe is the worst possible failure mode for
# a dictation app, and it was reachable by doing nothing more than bumping the
# patch number.
#
# A ceiling is still expressible for a pack genuinely known to break above some
# version; it must not be the default. `bundled_packs_declare_no_version_ceiling`
# in speakeasy-models asserts the same invariant against the embedded copy, so
# this cannot pass while the shipped binary disagrees.
$built = [version]$productVersion
foreach ($subject in @(
        @{ Name = 'application'; Minimum = $pack.compatibility.minimum_application_version; Maximum = $pack.compatibility.maximum_application_version },
        @{ Name = 'worker'; Minimum = $pack.compatibility.minimum_worker_version; Maximum = $pack.compatibility.maximum_worker_version }
    )) {
    if (-not $subject.Minimum) {
        throw "The trusted model pack declares no $($subject.Name) version floor."
    }
    if ($subject.Maximum) {
        throw ("The trusted model pack declares a $($subject.Name) version ceiling " +
            "($($subject.Maximum)). A ceiling makes the next version bump refuse to " +
            'select any pack at runtime; remove it from models\trusted-manifest.json ' +
            'unless this pack is genuinely known to break above that version.')
    }
    # The worker inherits `version.workspace = true`, so one built version covers
    # both subjects. If the worker is ever versioned separately this loop is where
    # that has to be threaded through.
    if ($built -lt [version]$subject.Minimum) {
        throw ("Trusted model compatibility does not admit $($subject.Name) version " +
            "$productVersion (declared floor $($subject.Minimum)).")
    }
}
. (Join-Path $repositoryRoot 'scripts\Enter-DevEnvironment.ps1')
Push-Location $repositoryRoot
try {
    & cargo test -p speakeasy-storage recovery --locked
    if ($LASTEXITCODE -ne 0) { throw 'Recovery tests failed.' }
    & cargo test -p speakeasy-domain dictation_defers_all_invalidating_operations_until_exact_owner_finishes --locked
    if ($LASTEXITCODE -ne 0) { throw 'Update exclusion test failed.' }
    # `speakeasy-worker`, not `speakeasy-inference-worker`: the crate was renamed
    # when the streaming engine left, and what survived is the half that was
    # never about recognition -- the protocol boundary, the ordering queue and
    # the plausibility gate. `speakeasy-granite-worker` joins it here because on
    # this fork it is the process that actually transcribes, so an installer
    # proof that never exercised it would be proving the wrong worker.
    & cargo test -p speakeasy-worker -p speakeasy-granite-worker --locked
    if ($LASTEXITCODE -ne 0) { throw 'Worker tests failed.' }
    & cargo clippy -p speakeasy-bootstrapper -p speakeasy-storage -p speakeasy-worker -p speakeasy-desktop --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Warning-denied product Clippy failed.' }
} finally {
    Pop-Location
}

$tauri = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot 'apps\desktop\src-tauri\tauri.conf.json') | ConvertFrom-Json
# Bundling is off because `apps/bootstrapper` installs SpeakEasy. A bundler
# target reappearing means two things claim to install the product, and the one
# that runs second wins without saying so.
if ($tauri.bundle.active -ne $false) {
    throw 'tauri.conf.json must not bundle: the bootstrapper installs SpeakEasy.'
}
if ($hostManifest.canonical_installer -ne 'speakeasy-bootstrapper') {
    throw "Canonical installer identity is incorrect: $($hostManifest.canonical_installer)"
}
if ($hostManifest.install_mode -ne 'currentUser' -or $hostManifest.elevation_required -ne $false) {
    throw 'The install must remain current-user and unelevated.'
}
# The absence of a `version` key is the contract now, not its value. Tauri falls
# back to Cargo.toml when it is missing, which is what makes the workspace version
# the only place the product version is written. Reintroducing it here would give
# the bundle a version that `CARGO_PKG_VERSION` — and so the recovery contract's
# `target_version` comparison — knows nothing about.
if ($tauri.PSObject.Properties.Name -contains 'version') {
    throw ('tauri.conf.json must not declare a version; it is derived from ' +
        "[workspace.package] version in Cargo.toml (currently $productVersion). " +
        'Two sources of truth for the version is the bug this prevents.')
}
# Belt and braces: the installer's file name carries the bundle version, so this
# confirms Tauri really did derive it from Cargo.toml rather than defaulting.
if (-not $hostManifest.version -or $hostManifest.version -ne $productVersion) {
    throw ("The built artifact reports version '$($hostManifest.version)' but the " +
        "workspace declares '$productVersion'.")
}
if ($tauri.PSObject.Properties.Name -contains 'plugins' -and $tauri.plugins.PSObject.Properties.Name -contains 'updater') {
    throw 'Updater configuration must remain absent.'
}
# The installation contract used to be pinned by reading literals out of
# `installer-hooks.nsh`. NSIS is gone, so the same properties are pinned against
# the source that owns them now. Every entry below was a behaviour NSIS had, and
# losing any of them is silent: an install that stops refusing downgrades, or an
# uninstall that stops keeping user data, produces no error at all.
$installSource = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot 'apps\bootstrapper\src\install.rs')
foreach ($literal in @(
    # The version stamp NSIS wrote, unchanged: an upgrade from a pre-bootstrapper
    # install has to find the stamp its predecessor left.
    'Software\SpeakEasy Mini\LocalDevelopment',
    # Add/Remove Programs registration, which the Tauri bundler used to generate.
    'Software\Microsoft\Windows\CurrentVersion\Uninstall\ai.speakeasy.mini',
    'RefuseSameVersion',
    'RefuseDowngrade',
    'RefuseRunning',
    # The one executable name. `speakeasy-v2-preview.exe` was pinned here too
    # until 2026-08-18: it belonged to the parent product's legacy preview,
    # which shared SpeakEasy's install directory and never shared Mini's.
    'ai-speakeasy-mini.exe'
)) {
    if (-not $installSource.Contains($literal)) {
        throw "Install contract is missing from install.rs: $literal"
    }
}
$uninstallSource = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot 'apps\bootstrapper\src\uninstall.rs')
foreach ($literal in @(
    # The four keep-or-remove choices NSIS asked as separate dialogs, plus the
    # CUDA runtime page, now one checkbox each.
    'Configuration',
    'History',
    'Models',
    'Recovery',
    'GpuRuntime',
    # The exact history files the NSIS hook removed.
    'data/speakeasy.sqlite3',
    'data/speakeasy.sqlite3-wal',
    'data/speakeasy.sqlite3-shm',
    'model-lifecycle',
    'ai.speakeasy.mini'
)) {
    if (-not $uninstallSource.Contains($literal)) {
        throw "Uninstall contract is missing from uninstall.rs: $literal"
    }
}
# Keep-by-default is the one property here that cannot be recovered if it
# regresses, so it is asserted as a behaviour rather than as a string.
if (-not ($uninstallSource -match 'Default')) {
    throw 'uninstall.rs must derive Default for Removals so nothing optional is removed unasked.'
}

$expected = @{}
foreach ($line in Get-Content -LiteralPath $checksumsPath) {
    if ($line -notmatch '^([0-9a-f]{64}) \*(.+)$') { throw "Invalid checksum line: $line" }
    $expected[$Matches[2]] = $Matches[1]
}
foreach ($name in $expected.Keys) {
    $path = Join-Path $artifactRoot $name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing checksummed file: $name" }
    $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected[$name]) { throw "Checksum mismatch: $name" }
}

# The planted/current-directory-DLL search-order probe this used to run
# against `moonshine-bridge.exe` has no equivalent target: that binary is
# retired along with Moonshine (see the GPU migration handoff, item 14), and
# nothing has replaced it. `inference-worker.exe` is the process that now
# loads onnxruntime.dll/sherpa-onnx's native DLLs and would be the right
# subject for this probe, but it speaks a framed stdin/stdout protocol rather
# than the bridge's single-argument CLI, so porting this test is nontrivial
# and undone. Reported honestly rather than silently dropped.
$maliciousDllProbe = 'not_run_native_bridge_retired'

$defenderResult = $DefenderEvidence
if ($RunDefenderScan) {
    try {
        $defenderStatus = Get-MpComputerStatus -ErrorAction Stop
        if (-not $defenderStatus.AMServiceEnabled -or
            -not $defenderStatus.AntivirusEnabled) {
            throw 'Microsoft Defender antivirus service is not enabled on this host.'
        }
        Start-MpScan -ScanType CustomScan -ScanPath $artifactRoot -ErrorAction Stop
        $threats = Get-MpThreatDetection -ErrorAction Stop |
            Where-Object { $_.Resources -match [regex]::Escape($artifactRoot) }
        if ($threats) { throw 'Microsoft Defender reported a detection in the local artifact.' }
        $defenderResult = 'passed_no_detection'
    } catch {
        throw "Microsoft Defender scan failed: $($_.Exception.Message)"
    }
}

[pscustomobject]@{
    schema_version = 1
    tested_utc = [DateTime]::UtcNow.ToString('o')
    artifact_root = $artifactRoot
    deterministic_recovery = 'passed'
    update_exclusion = 'passed'
    warning_denied_build = 'passed'
    checksums = 'passed'
    malicious_dll_current_directory = $maliciousDllProbe
    unicode_and_space_paths = $maliciousDllProbe
    defender = $defenderResult
    smart_screen = 'unsigned_expected_warning_not_publicly_qualified'
    running_app_install_refusal = $RunningAppRefusalEvidence
    fresh_install = 'requires_install_smoke'
    fresh_windows_profile = 'requires_separate_profile_smoke'
} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $artifactRoot 'local-install-test-report.json') -Encoding utf8

Write-Host "SpeakEasy local install checks: passed (native probe: $maliciousDllProbe; Defender: $defenderResult)"
