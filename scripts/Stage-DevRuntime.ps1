<#
.SYNOPSIS
Stages the native runtime into the dev target directory, so a `tauri dev` build
can actually transcribe.

.DESCRIPTION
Nothing else does this. `proof/` is produced only when an installer is built --
`Invoke-ProofPackage.ps1` stages it and `src-tauri/tauri.proof.conf.json` maps
it into the bundle -- and `src-tauri/tauri.conf.json`, the config `tauri dev`
uses, declares no `resources` at all. So `resource_dir()` in dev
(`target/<profile>`) had no `proof/`, `RuntimeWizardCoordinator::paths()`
returned `runtime_resources_unavailable`, and every dictation failed.

That failure was expensive out of proportion to its cause, which is why this
script exists rather than a line in a README. The app starts normally, the UI
works, capture runs, the level meter moves -- and then finalization fails with
the transcriber's *generic* error text. It is indistinguishable from a real
transcription regression until someone reads
`%APPDATA%\ai.speakeasy.mini\logs\speakeasy.log`, and it cost a debugging
round on 2026-08-10 doing exactly that.

`tauri.conf.json`'s `beforeDevCommand` runs this, so `npm run tauri -- dev`
stages before it launches and the trap cannot be stepped in again by
forgetting a step. Plain `npm run dev` (Vite alone, for frontend-only work)
does not, and does not need to.

.PARAMETER TargetProfile
Which cargo profile directory to stage into. `tauri dev` builds the desktop app
with the debug profile, so its resource root is `target/debug` -- that is the
default and normally the only one wanted.

Note this is the profile of the *app*, not of the workers. The workers are
always built `--release`: `speakeasy-granite-worker` links llama.cpp, and a
debug build of it is slow enough to misrepresent how the app behaves. Dev
should disagree with the installed build about as little as possible, and
inference speed is not a good thing to disagree about.

.PARAMETER SkipBuild
Stage from whatever worker binaries are already in `target/release`, without
asking cargo. For a fast relaunch when only frontend code changed. Fails if
they are not there yet, rather than staging a partial `proof/`.

.PARAMETER AllowMissingRuntime
Stage what is available and warn about the rest instead of failing. For
frontend-only work on a clone that has not built the worker yet, where
paying for a llama.cpp compile to look at a stylesheet is disproportionate.

Dictation will fail under this flag. That is the whole point of the default
being a hard failure -- the alternative is the silent version of this problem,
which is where we came in.

.EXAMPLE
.\scripts\Stage-DevRuntime.ps1

.EXAMPLE
.\scripts\Stage-DevRuntime.ps1 -SkipBuild
#>
[CmdletBinding()]
param(
    # Not `-Profile`: `$PROFILE` is a PowerShell automatic variable, and a
    # parameter of that name shadows it inside the script.
    [ValidateSet('debug', 'release')]
    [string]$TargetProfile = 'debug',

    [switch]$SkipBuild,

    [switch]$AllowMissingRuntime
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot

# An explicit, named way out, so that skipping this is a decision someone made
# rather than a step they forgot. Wired here rather than only as a switch
# because `beforeDevCommand` is a fixed string -- an environment variable is
# the only lever a developer has over it without editing tracked config.
if ($env:SPEAKEASY_SKIP_DEV_STAGING) {
    Write-Host 'Stage-DevRuntime: skipped (SPEAKEASY_SKIP_DEV_STAGING is set).'
    Write-Host '  Dictation will fail with runtime_resources_unavailable.'
    exit 0
}

# The installed layout is the specification, and it is already written down in
# the bundle config. Reading it here rather than restating the file list means
# a resource added to the installer is staged for dev too, instead of the two
# drifting until someone notices in the way described above.
$proofConfigPath = Join-Path $repositoryRoot 'apps\desktop\src-tauri\tauri.proof.conf.json'
$proofConfig = Get-Content -LiteralPath $proofConfigPath -Raw | ConvertFrom-Json
$resources = $proofConfig.bundle.resources
if (-not $resources) {
    throw "No bundle.resources in $proofConfigPath -- this script reads the installed layout from there."
}

# Only the `proof/` half. `speakeasy-bootstrapper.exe` and `notices/*` are
# also mapped, but nothing in the desktop crate resolves them at run time, so
# staging them would slow every dev launch to satisfy no code path.
$wanted = @()
foreach ($property in $resources.PSObject.Properties) {
    if ($property.Value -like 'proof/*') {
        $wanted += [pscustomobject]@{
            Name  = Split-Path -Leaf $property.Value
            Stage = $property.Name
        }
    }
}
if ($wanted.Count -eq 0) {
    throw "No proof/ resources in $proofConfigPath -- has the bundle layout changed?"
}

# The one `proof/` entry cargo produces. Keyed by the *installed* name, which
# is not the cargo target name.
#
# This map used to carry `inference-worker.exe` as well, and everything else in
# `proof/` came out of a fetched sherpa-onnx archive. The fork removed both, and
# left this script pointing at a package that no longer exists: every
# `npm run tauri -- dev` ran `cargo build -p speakeasy-inference-worker`, which
# fails, so `beforeDevCommand` threw and the app could not be launched at all.
# Nothing caught it, because no test runs the dev launcher.
$workerCrates = @{
    'granite-worker.exe' = @{ Package = 'speakeasy-granite-worker'; Output = 'speakeasy-granite-worker.exe' }
}

$destination = Join-Path $repositoryRoot "target\$TargetProfile\proof"
$workerBin = Join-Path $repositoryRoot 'target\release'

$missing = [System.Collections.Generic.List[string]]::new()

# Build first, and only the packages that are actually staged -- if the bundle
# layout ever stops shipping one of them, this stops building it. Cargo decides
# whether there is anything to do; an up-to-date tree costs about a second.
$packages = @($wanted | Where-Object { $workerCrates.ContainsKey($_.Name) } |
    ForEach-Object { $workerCrates[$_.Name].Package })
if ($packages.Count -gt 0 -and -not $SkipBuild) {
    Write-Host "Stage-DevRuntime: building $($packages -join ', ') (release)"
    Push-Location $repositoryRoot
    try {
        # cargo and rustc are not on PATH in a fresh shell; this sets
        # CARGO_HOME/RUSTUP_HOME to the workspace-local toolchain.
        . (Join-Path $PSScriptRoot 'Enter-DevEnvironment.ps1') | Out-Null
        $arguments = @('build', '--release', '--locked')
        foreach ($package in $packages) { $arguments += @('-p', $package) }
        # Deliberately unredirected. PowerShell 5.1 turns a native command's
        # stderr into ErrorRecords when it is redirected inside PowerShell, and
        # with $ErrorActionPreference = 'Stop' cargo's ordinary progress output
        # would abort this script.
        & cargo @arguments
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed for: $($packages -join ', ')" }
    } finally {
        Pop-Location
    }
}

New-Item -ItemType Directory -Path $destination -Force | Out-Null

# Copy only what differs, so a relaunch that changed nothing does not rewrite
# 19 MB and does not churn timestamps other tooling might be watching.
function Copy-IfChanged {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Target
    )
    $from = Get-Item -LiteralPath $Source
    $to = Get-Item -LiteralPath $Target -ErrorAction SilentlyContinue
    if ($to -and $to.Length -eq $from.Length -and $to.LastWriteTimeUtc -eq $from.LastWriteTimeUtc) {
        return $false
    }
    Copy-Item -LiteralPath $Source -Destination $Target -Force
    # Carry the source's timestamp over, or the check above never matches and
    # every launch copies again.
    (Get-Item -LiteralPath $Target).LastWriteTimeUtc = $from.LastWriteTimeUtc
    return $true
}

$copied = 0
$unchanged = 0
foreach ($resource in $wanted) {
    if ($workerCrates.ContainsKey($resource.Name)) {
        $source = Join-Path $workerBin $workerCrates[$resource.Name].Output
        $hint = if ($SkipBuild) {
            "run without -SkipBuild, or: cargo build --release -p $($workerCrates[$resource.Name].Package)"
        } else {
            "cargo reported success but did not produce it -- check the build output above"
        }
    } else {
        # Every `proof/` entry is now a cargo output. The other branch used to
        # source native libraries from a fetched sherpa-onnx archive; with that
        # engine gone, a `proof/` name this script cannot build is a mismatch
        # between the payload manifest and this map, not a missing download --
        # so say that, rather than pointing at a fetch script the fork deleted.
        throw ("Stage-DevRuntime: proof/$($resource.Name) is declared in " +
            "tauri.proof.conf.json but no crate in `$workerCrates builds it. " +
            'Add it there, or drop it from the payload manifest.')
    }

    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        # Named as the app will look for it, sourced as this script looked for
        # it, and paired with the command that fixes it. A bare "not found"
        # here would reproduce the diagnosis problem one layer up.
        $missing.Add("  proof/$($resource.Name)  <- $source`n      $hint")
        continue
    }

    if (Copy-IfChanged -Source $source -Target (Join-Path $destination $resource.Name)) {
        $copied += 1
    } else {
        $unchanged += 1
    }
}

if ($missing.Count -gt 0) {
    $report = "Stage-DevRuntime: $($missing.Count) runtime file(s) missing:`n" + ($missing -join "`n")
    if (-not $AllowMissingRuntime) {
        throw ($report + "`n`n" +
            "Dictation cannot work without these -- it fails at finalization with`n" +
            "runtime_resources_unavailable, which the transcriber reports in the same`n" +
            "words as any other failure. Failing here instead, where the cause is legible.`n`n" +
            "For frontend-only work, re-run with -AllowMissingRuntime, or set`n" +
            "SPEAKEASY_SKIP_DEV_STAGING=1 to skip staging entirely.")
    }
    Write-Warning ($report + "`n  Continuing: -AllowMissingRuntime was passed. Dictation will fail.")
}

Write-Host "Stage-DevRuntime: $destination -- $copied copied, $unchanged already current."
exit 0
