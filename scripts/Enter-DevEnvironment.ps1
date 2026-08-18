[CmdletBinding()]
param()

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$toolsRoot = Join-Path $repositoryRoot '.tools'
$cargoHome = Join-Path $toolsRoot 'cargo'
$rustupHome = Join-Path $toolsRoot 'rustup'
$expectedToolchain = '1.97.1'
$workspaceCargoBin = Join-Path $cargoHome 'bin'
$userCargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
$workspaceCargoExe = Join-Path $workspaceCargoBin 'cargo.exe'
$workspaceCargoReady = (Test-Path -LiteralPath $workspaceCargoExe -PathType Leaf) -and
    ((Get-Item -LiteralPath $workspaceCargoExe).Length -gt 0)

if ($workspaceCargoReady) {
    $env:CARGO_HOME = $cargoHome
    $env:RUSTUP_HOME = $rustupHome
    $env:PATH = $workspaceCargoBin + ';' + $env:PATH
    $rustSource = 'workspace-local'
} else {
    # A normal rustup install updates the user PATH, but an already-open
    # PowerShell session does not see that update. Add the conventional bin
    # directory explicitly so this helper works in that session too.
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue) -and
        (Test-Path -LiteralPath (Join-Path $userCargoBin 'cargo.exe') -PathType Leaf)) {
        $env:PATH = $userCargoBin + ';' + $env:PATH
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "No Cargo found. Expected either the workspace-local toolchain under .tools/cargo or a normal Rustup installation."
    }
    $rustSource = 'user/PATH'
}

$cargoVersion = (& cargo --version) -join ''
if ($cargoVersion -notmatch [regex]::Escape($expectedToolchain)) {
    throw "Cargo $expectedToolchain is required; found '$cargoVersion'."
}

# Native build prerequisites for `llama-cpp-sys-2`, which compiles llama.cpp
# rather than downloading it. Both of these were undeclared until 2026-08-04,
# and the cost was not theoretical: the Granite crate built on 2026-08-03 and
# failed in the same checkout the next day, because both were reaching whatever
# happened to be on PATH in one shell. Wiring them here makes the C++ build a
# stated prerequisite instead of ambient luck.
#
# Neither is needed by the rest of the workspace, so a miss is a warning rather
# than a throw — only crates that bind native libraries care.

$cmakeBin = Join-Path $toolsRoot 'cmake-4.4.0-windows-x86_64\bin'
if (Test-Path (Join-Path $cmakeBin 'cmake.exe')) {
    $env:PATH = $cmakeBin + ';' + $env:PATH
} elseif (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    Write-Warning 'CMake not found. Crates that compile llama.cpp (speakeasy-granite) will fail with "is `cmake` not installed?". Stage it under .tools or install it.'
}

# `.npmrc` sets `engine-strict=true` and `package.json` pins node and npm to
# exact versions, so `npm ci` fails outright — not warns — on any other pair.
# A machine whose system Node is newer therefore cannot run the frontend half of
# the gate at all, which is how the full gate stayed unrunnable here: everything
# else was present and only `npm ci` refused.
#
# A staged Node under .tools takes precedence when present, so the pinned pair
# wins over whatever the system has without uninstalling it. docs/NEW-MACHINE.md
# recommends nvm-windows over a second standalone install, and its objection is
# specifically to a standalone *MSI* — which cannot be removed without a UAC
# prompt. A portable zip staged here has neither problem: `nvm use` needs
# elevation, and this does not.
$stagedNode = Join-Path $toolsRoot ('node-v' + (Get-Content -Raw -LiteralPath (Join-Path $repositoryRoot '.node-version')).Trim() + '-win-x64')
if (Test-Path (Join-Path $stagedNode 'node.exe')) {
    $env:PATH = $stagedNode + ';' + $env:PATH
}

$libclangDirectory = & (Join-Path $PSScriptRoot 'Resolve-Libclang.ps1')
if ($null -ne $libclangDirectory) {
    $env:LIBCLANG_PATH = $libclangDirectory
} else {
    Write-Warning 'libclang not found. Crates that run bindgen (speakeasy-granite, via llama-cpp-sys-2) will fail with "Unable to find libclang". Install LLVM or set LIBCLANG_PATH.'
}

# CUDA Toolkit is optional for the CPU/default workflow. When installed, make
# its compiler and versioned MSBuild property visible to CUDA-enabled crates.
$cudaRoot = $env:CUDA_PATH
if ($cudaRoot -and (Test-Path -LiteralPath (Join-Path $cudaRoot 'bin\nvcc.exe'))) {
    $env:CUDAToolkit_ROOT = $cudaRoot
    $cudaLeaf = Split-Path -Path $cudaRoot -Leaf
    if ($cudaLeaf -match '^v(?<major>\d+)\.(?<minor>\d+)$') {
        Set-Item -Path "Env:CUDA_PATH_V$($Matches.major)_$($Matches.minor)" -Value $cudaRoot
    }
    $env:PATH = (Join-Path $cudaRoot 'bin') + ';' + $env:PATH
}

Write-Host "SpeakEasy dev environment ready ($rustSource Rust toolchain)."
