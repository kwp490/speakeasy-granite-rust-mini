# Local development policy

SpeakEasy Mini is a local-only project. Builds, tests, version bumps, installer
creation, and installation happen on the Windows development machine through
the checked-in PowerShell scripts. GitHub-hosted runners, GitHub Actions,
Dependabot, and other `.github` automation are intentionally not used.

The local quality gate enforces this policy with
`scripts/Test-LocalOnlyPolicy.ps1`; it fails if any `.github` configuration is
reintroduced. Run the gate directly from PowerShell:

```powershell
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall -SkipGranite
```

To increment the product version, use the local versioning script. It defaults
to a minor release and also supports major and patch releases:

```powershell
.\scripts\Increment-ProductVersion.ps1
.\scripts\Increment-ProductVersion.ps1 -Bump Major
.\scripts\Increment-ProductVersion.ps1 -Bump Patch
```

The root `[workspace.package] version` in `Cargo.toml` remains the single
product-version source. The frontend, Rust binaries, installer, recovery
checks, and local artifact manifest derive their version from it.

Build and install the current-user test artifact locally:

```powershell
.\scripts\Build-LocalInstaller.ps1
$version = & .\scripts\Get-ProductVersion.ps1
$installer = Join-Path (Get-Location) "target\local-development\$version\SpeakEasyMiniSetup.exe"
$installRoot = Join-Path $env:LOCALAPPDATA 'SpeakEasy Mini'
& $installer --install --install-root $installRoot
```

Both the artifact name and the flags here were NSIS's until 2026-08-21 —
`SpeakEasyMini_<version>_x64-setup.exe` with `/S "/D=..."` — and neither had
existed since the bootstrapper replaced it. The old command does not fail
visibly: it falls through to the repair verb parser, prints usage and exits 1.

Use the call operator, not `Start-Process -ArgumentList`, which joins its array
with spaces and quotes nothing — and this repository's own path has a space in
it. Read `$LASTEXITCODE`; the installer reports refusals that way.

Stop SpeakEasy Mini before installing an upgrade. The installer deliberately refuses
to replace a running app, reinstall the same version, or downgrade an installed
version — so an install/uninstall cycle at the same version needs the uninstall
first:

```powershell
& (Join-Path $installRoot 'speakeasy-bootstrapper.exe') --uninstall /S --keep-user-data
```

`--keep-user-data` is what makes that cycle cheap: without it the uninstall also
removes the ~2.14 GB of model weights, which is correct for a user and expensive
for a developer doing it repeatedly. It is a testing flag and nothing else.
