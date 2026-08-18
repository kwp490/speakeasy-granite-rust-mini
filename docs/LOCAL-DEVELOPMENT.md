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
$installer = Join-Path (Get-Location) "target\local-development\$version\SpeakEasyMini_${version}_x64-setup.exe"
$installRoot = Join-Path $env:LOCALAPPDATA 'SpeakEasy Mini'
& $installer /S "/D=$installRoot"
```

Stop SpeakEasy Mini before installing an upgrade. The installer deliberately refuses
to replace a running app, reinstall the same version, or downgrade an installed
version.
