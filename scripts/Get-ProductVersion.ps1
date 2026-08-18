<#
.SYNOPSIS
The product version, read from the one place that defines it.

.DESCRIPTION
`[workspace.package] version` in the root `Cargo.toml` is the single source of
truth. Everything else derives from it:

- every crate inherits it through `version.workspace = true`;
- `CARGO_PKG_VERSION` comes from it, and the recovery contract compares the
  running version against a pending update's `target_version`
  (`crates\speakeasy-storage\src\recovery.rs`), so the version the code sees has
  to be the version that shipped;
- Tauri derives the bundle and installer version from it, because
  `apps\desktop\src-tauri\tauri.conf.json` deliberately has no `version` key.
  Its own schema says so: "If removed the version number from `Cargo.toml` is
  used." Tauri recommends the opposite direction, but that would leave the
  bundle version and `CARGO_PKG_VERSION` free to disagree, and the recovery
  check above is the thing that would quietly break.

Scripts call this instead of repeating a literal. Fifteen places used to spell
the version out, and a bump had to hit all of them in lockstep or the build, the
installer, or the emitted install manifest would disagree about what was
shipping — silently, in the manifest's case.

.EXAMPLE
$version = & ./scripts/Get-ProductVersion.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$manifestPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'Cargo.toml'
if (-not (Test-Path -LiteralPath $manifestPath)) {
    throw "Workspace Cargo.toml is missing at $manifestPath."
}

# Scoped to the `[workspace.package]` table on purpose. A bare search for
# `version = "..."` would match the first pinned dependency in
# `[workspace.dependencies]` instead and report a third party's version as the
# product's.
$inWorkspacePackage = $false
foreach ($line in Get-Content -LiteralPath $manifestPath) {
    $trimmed = $line.Trim()
    if ($trimmed -match '^\[(.+)\]$') {
        $inWorkspacePackage = ($Matches[1] -eq 'workspace.package')
        continue
    }
    if (-not $inWorkspacePackage) { continue }
    if ($trimmed -match '^version\s*=\s*"([^"]+)"') {
        return $Matches[1]
    }
}

throw "No `[workspace.package] version` found in $manifestPath."
