<#
.SYNOPSIS
Reports what a from-scratch build needs, and stages the parts that can be staged
without elevation.

.DESCRIPTION
docs/NEW-MACHINE.md is the authoritative sequence, and it is prose: an agent or a
new developer has to read it, interpret it, and check each step by hand. That is
the longest and most trap-prone part of setting this project up, and two of its
traps are silent — winget's `--override` for the C++ workload can no-op without
failing, and a Node that does not match the pin makes `npm ci` fail outright
rather than warn, because `.npmrc` sets `engine-strict=true`.

This script is the checkable version of that document. It does not replace it: it
reports every prerequisite as PRESENT or MISSING with the reason, stages what it
can, and exits non-zero if something a build genuinely needs is absent. Run it,
read the summary, then read NEW-MACHINE.md for anything it reports missing.

**What it deliberately does not do.** It never installs Visual Studio Build
Tools, LLVM, or the CUDA Toolkit. Each needs an interactive UAC prompt that
cannot be driven unattended -- NEW-MACHINE.md records the CUDA installer refusing
an unattended session specifically -- and a script that appeared to install them
and silently did not would be worse than one that reports the truth. It also
never touches a system Node: the pinned version is staged under gitignored
`.tools` and only prepended to PATH inside this repository's shells, so whatever
Node the machine has for other work is left alone.

.PARAMETER SkipNode
Do not download or stage the pinned Node. Use when a matching Node is already on
PATH, or when there is no network.

.PARAMETER SkipRuntime
Do not stage the native sherpa/ONNX runtime. Skips a multi-gigabyte download on a
machine that only needs to compile.

.EXAMPLE
./scripts/Initialize-DevMachine.ps1

.EXAMPLE
./scripts/Initialize-DevMachine.ps1 -SkipRuntime
#>
[CmdletBinding()]
param(
    [switch]$SkipNode,
    [switch]$SkipRuntime
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$toolsRoot = Join-Path $repositoryRoot '.tools'

# Collected rather than printed as they happen, so the summary is one block a
# reader (or an agent) can act on instead of a scroll of interleaved output.
$findings = [Collections.Generic.List[psobject]]::new()
function Add-Finding {
    param(
        [Parameter(Mandatory)][string]$Item,
        [Parameter(Mandatory)][ValidateSet('present', 'staged', 'missing', 'optional')][string]$State,
        [Parameter(Mandatory)][string]$Detail,
        [switch]$Required
    )
    $findings.Add([pscustomobject]@{
            Item     = $Item
            State    = $State
            Detail   = $Detail
            Required = [bool]$Required
        })
}

Write-Host '--- SpeakEasy dev machine check ---'
Write-Host ''

# Every check below resolves tools the way Enter-DevEnvironment.ps1 does -- the
# workspace-local copy first, then the user profile, then PATH -- rather than
# asking PATH alone.
#
# That is not defensiveness. `~/.cargo/bin` is on the *user* PATH and CMake on the
# *machine* PATH, so a shell that inherited a reduced environment (a service, a
# sandboxed agent, a fresh non-login shell) has neither, and a PATH-only check
# then reports "install rustup" on a machine where rustup is already installed.
# A false MISSING is worse than no check: it sends the reader to reinstall
# something they have, and this script exists to be trusted by someone who does
# not yet know the project.
function Resolve-Tool {
    param(
        [Parameter(Mandatory)][string]$Command,
        [string[]]$Candidates = @()
    )
    $found = Get-Command $Command -ErrorAction SilentlyContinue
    if ($found) { return $found.Source }
    foreach ($candidate in $Candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    return $null
}

# Rust. `rust-toolchain.toml` pins the channel, so rustup installs the right one
# on demand; what matters is that rustup exists at all.
$expectedRust = ([regex]::Match((Get-Content -Raw (Join-Path $repositoryRoot 'rust-toolchain.toml')), 'channel\s*=\s*"([^"]+)"')).Groups[1].Value
$rustupPath = Resolve-Tool -Command 'rustup' -Candidates @(
    (Join-Path $toolsRoot 'cargo\bin\rustup.exe'),
    (Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe')
)
$cargoPath = Resolve-Tool -Command 'cargo' -Candidates @(
    (Join-Path $toolsRoot 'cargo\bin\cargo.exe'),
    (Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe')
)
if ($rustupPath -and $cargoPath) {
    & $rustupPath toolchain install $expectedRust --profile minimal --component clippy --component rustfmt *>&1 | Out-Null
    Push-Location $repositoryRoot
    try { $cargoVersion = (& $cargoPath --version) -join '' } finally { Pop-Location }
    if ($cargoVersion -match [regex]::Escape($expectedRust)) {
        Add-Finding -Item 'Rust' -State 'present' -Detail "$cargoVersion ($cargoPath)" -Required
    } else {
        Add-Finding -Item 'Rust' -State 'missing' -Required `
            -Detail "cargo reports '$cargoVersion'; this workspace pins $expectedRust. rust-toolchain.toml should select it automatically inside the repo."
    }
} else {
    Add-Finding -Item 'Rust' -State 'missing' -Required `
        -Detail 'rustup/cargo not found on PATH, in .tools\cargo\bin, or in %USERPROFILE%\.cargo\bin. Install from https://rustup.rs, then re-run; the pinned toolchain installs itself from rust-toolchain.toml.'
}

# Node. Staged under .tools rather than installed, so a machine with a different
# system Node keeps it -- see Enter-DevEnvironment.ps1 for the PATH ordering.
$expectedNode = (Get-Content -Raw (Join-Path $repositoryRoot '.node-version')).Trim()
$stagedNode = Join-Path $toolsRoot "node-v$expectedNode-win-x64"
if (Test-Path (Join-Path $stagedNode 'node.exe')) {
    Add-Finding -Item 'Node (pinned)' -State 'present' -Detail "$expectedNode already staged at .tools\node-v$expectedNode-win-x64" -Required
} elseif ($SkipNode) {
    $systemNode = if (Get-Command node -ErrorAction SilentlyContinue) { (& node --version).Trim() } else { '(none)' }
    $state = if ($systemNode -eq "v$expectedNode") { 'present' } else { 'missing' }
    Add-Finding -Item 'Node (pinned)' -State $state -Required `
        -Detail "-SkipNode given; system node is $systemNode and the pin is v$expectedNode. engine-strict=true means npm ci fails on any mismatch."
} else {
    try {
        $downloads = Join-Path $toolsRoot 'downloads'
        New-Item -ItemType Directory -Path $downloads -Force | Out-Null
        $archive = Join-Path $downloads "node-v$expectedNode-win-x64.zip"
        if (-not (Test-Path $archive)) {
            Write-Host "staging Node $expectedNode ..."
            Invoke-WebRequest -Uri "https://nodejs.org/dist/v$expectedNode/node-v$expectedNode-win-x64.zip" -OutFile $archive -UseBasicParsing
        }
        # Verified against nodejs.org's own checksum list before extraction. A
        # runtime this project then executes is not something to take on trust
        # from a transfer that happened to complete.
        $sums = (Invoke-WebRequest -Uri "https://nodejs.org/dist/v$expectedNode/SHASUMS256.txt" -UseBasicParsing).Content
        $expectedHash = ((($sums -split "`n") | Where-Object { $_ -match "node-v$expectedNode-win-x64\.zip" } | Select-Object -First 1) -split '\s+')[0]
        $actualHash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($expectedHash -ne $actualHash) {
            throw "checksum mismatch (expected $expectedHash, got $actualHash); refusing to extract"
        }
        Expand-Archive -LiteralPath $archive -DestinationPath $toolsRoot -Force
        Add-Finding -Item 'Node (pinned)' -State 'staged' -Detail "$expectedNode staged and checksum-verified into .tools" -Required
    } catch {
        Add-Finding -Item 'Node (pinned)' -State 'missing' -Required `
            -Detail "could not stage Node $expectedNode : $($_.Exception.Message)"
    }
}

# CMake and libclang: needed only by the crates that compile llama.cpp, but that
# includes the shipped Granite worker, so a build of the product needs both.
$cmakeStaged = Get-ChildItem -Path $toolsRoot -Directory -Filter 'cmake-*-windows-x86_64' -ErrorAction SilentlyContinue |
    Where-Object { Test-Path (Join-Path $_.FullName 'bin\cmake.exe') } | Select-Object -First 1
if ($cmakeStaged) {
    Add-Finding -Item 'CMake' -State 'present' -Detail "staged at .tools\$($cmakeStaged.Name)" -Required
} else {
    # The default installer location is checked explicitly: CMake adds itself to
    # the *machine* PATH, which a reduced shell does not inherit.
    $cmakePath = Resolve-Tool -Command 'cmake' -Candidates @(
        (Join-Path $env:ProgramFiles 'CMake\bin\cmake.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'CMake\bin\cmake.exe')
    )
    if ($cmakePath) {
        Add-Finding -Item 'CMake' -State 'present' -Detail (((& $cmakePath --version) | Select-Object -First 1) + " ($cmakePath)") -Required
    } else {
        Add-Finding -Item 'CMake' -State 'missing' -Required `
            -Detail 'not found on PATH, in Program Files, or staged. Install it, or unpack it as .tools\cmake-<version>-windows-x86_64. Needed by llama-cpp-sys-2.'
    }
}

$libclang = & (Join-Path $PSScriptRoot 'Resolve-Libclang.ps1')
if ($null -ne $libclang) {
    Add-Finding -Item 'libclang' -State 'present' -Detail $libclang -Required
} else {
    Add-Finding -Item 'libclang' -State 'missing' -Required `
        -Detail 'not found. Install LLVM or set LIBCLANG_PATH. bindgen needs it, via llama-cpp-sys-2.'
}

# The C++ toolset cannot be checked by asking for a command, because the compiler
# is not on PATH until a developer prompt is entered. vswhere is the documented
# way, and NEW-MACHINE.md records why: winget's --override for this workload can
# report success and install nothing.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path -LiteralPath $vswhere) {
    $vcInstall = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
    if ($vcInstall) {
        Add-Finding -Item 'MSVC C++ toolset' -State 'present' -Detail ($vcInstall | Select-Object -First 1) -Required
    } else {
        Add-Finding -Item 'MSVC C++ toolset' -State 'missing' -Required `
            -Detail 'vswhere found no VC.Tools component. Re-run the Build Tools installer with setup.exe modify --add Microsoft.VisualStudio.Workload.VCTools; a winget --override can silently no-op.'
    }
} else {
    Add-Finding -Item 'MSVC C++ toolset' -State 'missing' -Required `
        -Detail 'vswhere.exe not present, so Visual Studio Build Tools is very likely not installed. Needs an interactive installer; this script will not attempt it.'
}

# Optional from here down: the app runs CPU-only without any of it.
if (Get-Command nvidia-smi -ErrorAction SilentlyContinue) {
    Add-Finding -Item 'NVIDIA GPU' -State 'optional' -Detail 'nvidia-smi present; the CUDA packs are selectable. CPU remains a full fallback.'
} else {
    Add-Finding -Item 'NVIDIA GPU' -State 'optional' -Detail 'no nvidia-smi. CPU-only, which is a supported configuration.'
}

if ($SkipRuntime) {
    Add-Finding -Item 'Native runtime' -State 'optional' -Detail '-SkipRuntime given; run Get-GpuRuntime.ps1 before `npm run tauri -- dev` or any test that loads a model.'
} elseif (Test-Path (Join-Path $toolsRoot 'sherpa-onnx\current\lib')) {
    & (Join-Path $PSScriptRoot 'Get-GpuRuntime.ps1') -LinkOnly *>&1 | Out-Null
    Add-Finding -Item 'Native runtime' -State 'present' -Detail 'sherpa/ONNX staged; re-linked into target/debug by Get-GpuRuntime.ps1 -LinkOnly'
} else {
    Add-Finding -Item 'Native runtime' -State 'missing' `
        -Detail 'not staged. Run .\scripts\Get-GpuRuntime.ps1 (downloads and hash-verifies several GB). Required before running the app or any model test, not to compile.'
}

Write-Host ''
Write-Host '--- summary ---'
$findings |
    Select-Object @{n = 'Item'; e = { $_.Item } },
        @{n = 'State'; e = { $_.State.ToUpperInvariant() } },
        @{n = 'Detail'; e = { $_.Detail } } |
    Format-Table -AutoSize -Wrap |
    Out-String -Width 160 |
    Write-Host

$blocking = @($findings | Where-Object { $_.Required -and $_.State -eq 'missing' })
if ($blocking.Count -gt 0) {
    Write-Host "$($blocking.Count) required item(s) missing. See docs/NEW-MACHINE.md for each."
    Write-Host 'Nothing above needs elevation except the C++ toolset, LLVM and the CUDA Toolkit.'
    exit 1
}

Write-Host 'All required prerequisites are present.'
Write-Host 'Next: . .\scripts\Enter-DevEnvironment.ps1   then   npm ci   then   npm run tauri -- dev'
Write-Host 'Full gate: .\scripts\Invoke-ScaffoldChecks.ps1'
exit 0
