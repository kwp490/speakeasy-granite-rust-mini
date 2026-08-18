[CmdletBinding()]
param(
    [switch]$SkipNpmInstall,

    # Exclude speakeasy-granite (and speakeasy-granite-worker, which links it)
    # from clippy and test.
    #
    # Those are the only two crates that compile llama.cpp, which means they are
    # the only ones needing CMake, libclang and a C++ toolchain, and a cold build
    # of it costs minutes that cache poorly. Splitting them into a separate
    # local invocation keeps a slow C++ build from starving the gate that
    # catches everything else, and makes the cost attributable in the log.
    #
    # There is no way to lint either crate without compiling llama.cpp — `-p
    # speakeasy-granite` and `-p speakeasy-granite-worker` build the sys crate
    # either way — so this is a scheduling split, not a way to avoid the build.
    # Both stay workspace members and stay linted; see -GraniteOnly for the
    # other half.
    [switch]$SkipGranite,

    # Run only the granite half: clippy and test for those two crates, nothing
    # else. The counterpart to -SkipGranite; together they cover the workspace.
    [switch]$GraniteOnly
)

if ($SkipGranite -and $GraniteOnly) {
    throw '-SkipGranite and -GraniteOnly are mutually exclusive.'
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'Test-LocalOnlyPolicy.ps1')
$toolsRoot = Join-Path $repositoryRoot '.tools'
$cargoHome = Join-Path $toolsRoot 'cargo'
$rustupHome = Join-Path $toolsRoot 'rustup'

$expectedToolchain = '1.97.1'

# The workspace-local toolchain under .tools is how a developer machine is set
# up (see Enter-DevEnvironment.ps1), but .tools is gitignored, so CI installs
# the toolchain the ordinary way instead. Requiring .tools here made this script
# — and therefore the whole gate — unrunnable anywhere but one machine.
#
# Pointing CARGO_HOME/RUSTUP_HOME at .tools has to be conditional for the same
# reason: setting them to paths that do not exist breaks a correctly installed
# toolchain rather than falling back to it.
$workspaceCargoExe = Join-Path $cargoHome 'bin/cargo.exe'
$workspaceCargoReady = Test-Path -LiteralPath $workspaceCargoExe -PathType Leaf
if ($workspaceCargoReady) {
    $env:CARGO_HOME = $cargoHome
    $env:RUSTUP_HOME = $rustupHome
    $env:PATH = (Join-Path $cargoHome 'bin') + ';' + $env:PATH
} elseif (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "No Cargo found. Expected either the workspace-local toolchain under .tools/cargo (run rustup-init from .tools/downloads) or cargo $expectedToolchain on PATH."
}

# Assert the version whichever toolchain we landed on. The pin is the point: a
# gate that passes on a different compiler than the one this project ships is
# not the gate anyone thinks it is.
$cargoVersion = (& cargo --version) -join ''
if ($cargoVersion -notmatch [regex]::Escape($expectedToolchain)) {
    throw "Cargo $expectedToolchain is required; found '$cargoVersion'."
}

# Native build prerequisites for `llama-cpp-sys-2`, which compiles llama.cpp
# instead of downloading a prebuilt library. This script does not source
# Enter-DevEnvironment.ps1, so it has to resolve them itself.
#
# These are asserted, not warned about, because this script runs the gate:
# `cargo clippy --workspace` and `cargo test --workspace` both build
# speakeasy-granite, so a missing prerequisite here is a failed gate several
# minutes into a C++ build with an opaque panic. Say so up front instead.
if (-not $SkipGranite) {
    $cmakeBin = Join-Path $toolsRoot 'cmake-4.4.0-windows-x86_64\bin'
    if (Test-Path (Join-Path $cmakeBin 'cmake.exe')) {
        $env:PATH = $cmakeBin + ';' + $env:PATH
    } elseif (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
        throw 'CMake is required to build llama.cpp (speakeasy-granite -> llama-cpp-sys-2). Stage it under .tools/cmake-4.4.0-windows-x86_64 or install it and put it on PATH.'
    }

    $libclangDirectory = & (Join-Path $PSScriptRoot 'Resolve-Libclang.ps1')
    if ($null -eq $libclangDirectory) {
        throw 'libclang is required by bindgen (speakeasy-granite -> llama-cpp-sys-2). Install LLVM or set LIBCLANG_PATH to a directory containing libclang.dll.'
    }
    $env:LIBCLANG_PATH = $libclangDirectory
}

# `sherpa-onnx-sys` links through SHERPA_ONNX_LIB_DIR from .cargo/config.toml.
# Assert the selected, version-stamped runtime before Cargo starts so a missing
# native directory is reported as a staging problem rather than as LNK1181.
$nativeRuntimeVersion = & (Join-Path $PSScriptRoot 'Get-NativeRuntimeVersion.ps1')
$nativeRuntimeRoot = Join-Path $toolsRoot 'sherpa-onnx/current'
$nativeRuntimeLibDir = Join-Path $nativeRuntimeRoot 'lib'
$nativeRuntimeStamp = Join-Path $nativeRuntimeRoot 'version.txt'
if (-not (Test-Path -LiteralPath $nativeRuntimeLibDir -PathType Container)) {
    throw "The sherpa native runtime for version $nativeRuntimeVersion is not staged at $nativeRuntimeLibDir. Run .\scripts\Get-GpuRuntime.ps1 -LinkOnly first."
}
if (-not (Test-Path -LiteralPath $nativeRuntimeStamp -PathType Leaf)) {
    throw "The sherpa native runtime stamp is missing at $nativeRuntimeStamp. Re-run .\scripts\Get-GpuRuntime.ps1 -LinkOnly."
}
$stagedNativeRuntimeVersion = (Get-Content -LiteralPath $nativeRuntimeStamp -Raw).Trim()
if ($stagedNativeRuntimeVersion -ne $nativeRuntimeVersion) {
    throw "The staged sherpa runtime is version $stagedNativeRuntimeVersion but the workspace requires $nativeRuntimeVersion. Re-run .\scripts\Get-GpuRuntime.ps1 -LinkOnly."
}

Push-Location $repositoryRoot
try {
    # The granite-only job is the C++ half and nothing else: no npm, no
    # dependency policy, no advisory scan. Those are workspace-wide concerns and
    # running them twice would double the slowest parts of the gate for no
    # additional coverage.
    if ($GraniteOnly) {
        & cargo clippy -p speakeasy-granite -p speakeasy-granite-worker --all-targets --locked -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw 'cargo clippy (speakeasy-granite, speakeasy-granite-worker) failed.' }
        & cargo test -p speakeasy-granite -p speakeasy-granite-worker --locked
        if ($LASTEXITCODE -ne 0) { throw 'cargo test (speakeasy-granite, speakeasy-granite-worker) failed.' }
        Write-Host 'granite checks passed.'
        Write-Host ''
        Write-Host 'Note: the GGUF tests are #[ignore]d hardware proofs and did NOT run here.'
        Write-Host 'Run them where the model files are:'
        Write-Host '  cargo test -p speakeasy-granite granite -- --ignored --nocapture'
        Write-Host '  cargo test -p speakeasy-granite-worker granite -- --ignored --nocapture'
        return
    }

    if (-not $SkipNpmInstall) {
        & npm ci
        if ($LASTEXITCODE -ne 0) { throw 'npm ci failed.' }
    }

    # `cargo fmt` reads sources; it never builds, so it always covers everything.
    & cargo fmt --check
    if ($LASTEXITCODE -ne 0) { throw 'cargo fmt --check failed.' }

    $cargoScope = @('--workspace')
    if ($SkipGranite) {
        $cargoScope += @('--exclude', 'speakeasy-granite', '--exclude', 'speakeasy-granite-worker')
    }

    & cargo clippy @cargoScope --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'cargo clippy failed.' }
    & cargo test @cargoScope --locked
    if ($LASTEXITCODE -ne 0) { throw 'cargo test failed.' }
    # ripgrep and gitleaks are pinned in dependency-policy/tools.json alongside the
    # three cargo tools. They were not, and this gate's own gitleaks error told the
    # reader to "install the pinned version recorded in the local tool setup" when no
    # such record existed anywhere -- an instruction that could not be followed.
    # Asserted here rather than in Test-DependencyPolicy.ps1 so each check sits next
    # to the use it protects.
    $gateTools = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot 'dependency-policy/tools.json') |
        ConvertFrom-Json
    if ($null -eq (Get-Command rg -ErrorAction SilentlyContinue)) {
        throw ("ripgrep is required by the local quality gate; install version " +
            "$($gateTools.ripgrep), pinned in dependency-policy/tools.json.")
    }
    $ripgrepVersion = ((& rg --version) | Select-Object -First 1).Trim()
    if ($ripgrepVersion -ne "ripgrep $($gateTools.ripgrep)") {
        throw "Expected ripgrep $($gateTools.ripgrep); found '$ripgrepVersion'."
    }
    $workerClientDefinitions = @(rg -l 'pub struct ProcessWorkerClient' apps crates workers --glob '*.rs' 2>$null)
    if ($workerClientDefinitions.Count -ne 1 -or (Test-Path -LiteralPath (Join-Path $repositoryRoot 'apps\asr-server'))) {
        throw ("Expected exactly one supervised worker client and no retired asr-server; found: " +
            ($workerClientDefinitions -join ', '))
    }
    Write-Host ("Supervised worker client: {0}" -f $workerClientDefinitions[0])
    if ($null -eq (Get-Command cargo-llvm-cov -ErrorAction SilentlyContinue)) {
        throw 'cargo-llvm-cov is required by the local quality gate; install the pinned version from dependency-policy/tools.json.'
    }
    $coverageRoot = Join-Path $repositoryRoot 'target\coverage'
    New-Item -ItemType Directory -Path $coverageRoot -Force | Out-Null
    $coveragePath = Join-Path $coverageRoot 'cargo-llvm-cov.json'
    & cargo llvm-cov @cargoScope --locked --no-clean --summary-only --json --output-path $coveragePath
    if ($LASTEXITCODE -ne 0) { throw 'cargo-llvm-cov failed.' }
    $coverage = Get-Content -Raw -LiteralPath $coveragePath | ConvertFrom-Json
    $totals = $coverage.data[0].totals
    Write-Host ("Rust coverage (no threshold): lines {0:N2}% ({1}/{2}), functions {3:N2}% ({4}/{5}), regions {6:N2}% ({7}/{8})" -f `
        $totals.lines.percent, $totals.lines.covered, $totals.lines.count, `
        $totals.functions.percent, $totals.functions.covered, $totals.functions.count, `
        $totals.regions.percent, $totals.regions.covered, $totals.regions.count)
    & npm run lint
    if ($LASTEXITCODE -ne 0) { throw 'frontend lint failed.' }
    & npm run typecheck
    if ($LASTEXITCODE -ne 0) { throw 'frontend typecheck failed.' }
    & npm test
    if ($LASTEXITCODE -ne 0) { throw 'frontend tests failed.' }
    & npm run build
    if ($LASTEXITCODE -ne 0) { throw 'frontend build failed.' }
    & (Join-Path $PSScriptRoot 'Test-DependencyPolicy.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'dependency-policy check failed.' }
    & cargo deny check
    if ($LASTEXITCODE -ne 0) { throw 'cargo deny failed.' }
    & cargo audit
    if ($LASTEXITCODE -ne 0) { throw 'cargo audit failed.' }
    & npm audit
    if ($LASTEXITCODE -ne 0) { throw 'npm audit failed.' }

    $gitleaks = Get-Command gitleaks -ErrorAction SilentlyContinue
    if ($null -eq $gitleaks) {
        throw ('gitleaks is required by the local quality gate; install version ' +
            "$($gateTools.gitleaks), pinned in dependency-policy/tools.json.")
    }
    $gitleaksVersion = ((& gitleaks version) | Select-Object -First 1).Trim()
    if ($gitleaksVersion -ne $gateTools.gitleaks) {
        throw "Expected gitleaks $($gateTools.gitleaks); found '$gitleaksVersion'."
    }
    & gitleaks detect --source . --no-banner --redact --exit-code 1
    if ($LASTEXITCODE -ne 0) { throw 'gitleaks found a potential secret.' }
} finally {
    Pop-Location
}
