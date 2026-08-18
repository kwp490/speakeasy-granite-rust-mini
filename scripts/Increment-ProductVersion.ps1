[CmdletBinding()]
param(
    [ValidateSet('Major', 'Minor', 'Patch')]
    [string]$Bump = 'Minor'
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repositoryRoot 'Cargo.toml'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "Workspace Cargo.toml is missing at $manifestPath."
}

$lines = [IO.File]::ReadAllLines($manifestPath)
$inWorkspacePackage = $false
$versionIndex = -1
$currentVersion = $null
for ($index = 0; $index -lt $lines.Count; $index++) {
    $trimmed = $lines[$index].Trim()
    if ($trimmed -match '^\[(.+)\]$') {
        $inWorkspacePackage = ($Matches[1] -eq 'workspace.package')
        continue
    }
    if ($inWorkspacePackage -and $trimmed -match '^version\s*=\s*"([^"]+)"') {
        $versionIndex = $index
        $currentVersion = [version]$Matches[1]
        break
    }
}
if ($versionIndex -lt 0 -or $null -eq $currentVersion) {
    throw "No [workspace.package] version found in $manifestPath."
}

switch ($Bump) {
    'Major' { $nextVersion = [version]::new($currentVersion.Major + 1, 0, 0) }
    'Minor' { $nextVersion = [version]::new($currentVersion.Major, $currentVersion.Minor + 1, 0) }
    'Patch' { $nextVersion = [version]::new($currentVersion.Major, $currentVersion.Minor, $currentVersion.Build + 1) }
}

# Keep the Cargo manifest as the only product-version input. The frontend reads
# it during its Vite build, while installers and Rust read CARGO_PKG_VERSION.
$lines[$versionIndex] = "version = `"$nextVersion`""
$utf8 = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllLines($manifestPath, $lines, $utf8)

Write-Host "Product version bumped from $currentVersion to $nextVersion ($Bump)."

# Refresh Cargo.lock in the same breath, because a bump that does not is not
# finished. Every workspace member's version is recorded in the lockfile, so
# rewriting the manifest alone leaves the two disagreeing -- and the whole gate
# builds with `--locked`, which refuses to update the lockfile and fails with
# "cannot update the lock file ... because --locked was passed". That reads as a
# broken checkout rather than as an unfinished bump, and nothing in the bump
# announced it as a step.
#
# `--offline` on purpose: this only needs to re-resolve path members whose version
# just changed, and a bump must not become the moment third-party crates get
# re-resolved from the network.
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$devEnvironment = Join-Path $PSScriptRoot 'Enter-DevEnvironment.ps1'
if (Test-Path -LiteralPath $devEnvironment) {
    . $devEnvironment *>$null
}
if (Get-Command cargo -ErrorAction SilentlyContinue) {
    Push-Location $repositoryRoot
    try {
        & cargo update --workspace --offline
        if ($LASTEXITCODE -ne 0) {
            throw ("cargo update --workspace --offline failed after the version bump. " +
                'Cargo.lock still records the old version; run it by hand before building.')
        }
        Write-Host "Cargo.lock refreshed to $nextVersion."
    } finally {
        Pop-Location
    }
} else {
    Write-Warning ('Cargo is not available, so Cargo.lock still records ' +
        "$currentVersion. Run `cargo update --workspace --offline` before any " +
        '--locked build, or the gate will fail on the lockfile rather than on the bump.')
}

Write-Host 'Run the local quality gate and installer lifecycle checks before committing the bump.'
