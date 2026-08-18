<#
.SYNOPSIS
Locates the directory containing libclang.dll, for bindgen.

.DESCRIPTION
`llama-cpp-sys-2` build-depends on bindgen, which loads libclang at build time
and panics with "Unable to find libclang" when it cannot. Nothing in this
workspace declared where that comes from, so whether the Granite crate built at
all depended on what happened to be on PATH in a given shell — it succeeded on
2026-08-03 and failed on 2026-08-04 in the same checkout, on the same machine,
for exactly that reason.

This exists so the answer is discovered once, in one place, and reported when it
is missing rather than surfacing as a panic several minutes into a C++ build.

Returns the directory path, or $null when libclang cannot be found. Callers
decide whether a miss is fatal: it only blocks crates that bind native
libraries, so the rest of the workspace must still build without it.

.NOTES
Prefers an existing $env:LIBCLANG_PATH so an explicit choice always wins over
discovery — including on CI, which sets it directly.
#>
[CmdletBinding()]
param()

function Test-LibclangDirectory {
    param([string]$Directory)

    if ([string]::IsNullOrWhiteSpace($Directory)) { return $false }
    if (-not (Test-Path -LiteralPath $Directory)) { return $false }

    foreach ($name in @('libclang.dll', 'clang.dll')) {
        $candidate = Join-Path $Directory $name
        if (Test-Path -LiteralPath $candidate) {
            # A OneDrive-backed file can be a cloud-only placeholder whose
            # reparse point resolves but whose bytes are absent. bindgen fails
            # opaquely on those, so require real content rather than presence.
            $file = Get-Item -LiteralPath $candidate -Force -ErrorAction SilentlyContinue
            if ($null -ne $file -and $file.Length -gt 0) { return $true }
        }
    }

    return $false
}

if (Test-LibclangDirectory -Directory $env:LIBCLANG_PATH) {
    return (Resolve-Path -LiteralPath $env:LIBCLANG_PATH).Path
}

$candidates = New-Object System.Collections.Generic.List[string]

# clang.exe on PATH implies libclang.dll beside it.
$clangCommand = Get-Command clang -ErrorAction SilentlyContinue
if ($null -ne $clangCommand) {
    $candidates.Add((Split-Path -Parent $clangCommand.Source))
}

# Visual Studio's optional "C++ Clang tools for Windows" component, via the
# installer's own locator rather than a guessed year/edition path.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path -LiteralPath $vswhere) {
    $installations = & $vswhere -all -products '*' -property installationPath
    foreach ($installation in $installations) {
        if (-not [string]::IsNullOrWhiteSpace($installation)) {
            $candidates.Add((Join-Path $installation 'VC\Tools\Llvm\x64\bin'))
            $candidates.Add((Join-Path $installation 'VC\Tools\Llvm\bin'))
        }
    }
}

# Standalone LLVM installs, in rough order of how deliberate they are. The
# OneDrive entry is not hypothetical: it is where this machine's LLVM 22.1.8
# lives, and finding it by accident is what made the build look reproducible
# when it was not.
$candidates.Add((Join-Path $env:ProgramFiles 'LLVM\bin'))
$candidates.Add((Join-Path ${env:ProgramFiles(x86)} 'LLVM\bin'))
$candidates.Add((Join-Path $env:LOCALAPPDATA 'Programs\LLVM\bin'))

# Documents, including OneDrive-redirected Documents. There is more than one
# OneDrive root when a machine is signed into both a personal and a work
# tenant: $env:OneDrive points at whichever is primary — on this machine the
# Harvard tenant — while the LLVM install sits under the personal one. Checking
# only $env:OneDrive is why this resolver missed a libclang that was present.
$documentsRoots = New-Object System.Collections.Generic.List[string]
$documentsRoots.Add($env:USERPROFILE)
foreach ($oneDriveVariable in @($env:OneDrive, $env:OneDriveConsumer, $env:OneDriveCommercial)) {
    if (-not [string]::IsNullOrWhiteSpace($oneDriveVariable)) {
        $documentsRoots.Add($oneDriveVariable)
    }
}
# A personal OneDrive is not always exported as a variable at all.
$documentsRoots.Add((Join-Path $env:USERPROFILE 'OneDrive'))
foreach ($documentsRoot in $documentsRoots) {
    $candidates.Add((Join-Path $documentsRoot 'Documents\LLVM\bin'))
}

$candidates.Add((Join-Path $env:USERPROFILE 'scoop\apps\llvm\current\bin'))
$candidates.Add('C:\tools\LLVM\bin')

foreach ($candidate in $candidates) {
    if (Test-LibclangDirectory -Directory $candidate) {
        return (Resolve-Path -LiteralPath $candidate).Path
    }
}

return $null
