# Contributing

This repository is maintained as a personal project. Changes should preserve
the local-first, fail-closed behavior documented in `docs/ARCHITECTURE.md` and
`docs/PRIVACY.md`.

## Local-only development

Builds, tests, version bumps, installer creation, and installation happen on
the local Windows machine. GitHub Actions, hosted runners, Dependabot, and
other `.github` automation are intentionally prohibited. The quality gate
enforces that policy; see [docs/LOCAL-DEVELOPMENT.md](docs/LOCAL-DEVELOPMENT.md).

## Prerequisites

- Windows 11 x64 with Visual Studio Build Tools and the C++ workload;
- Rust toolchain 1.97.1 from `rust-toolchain.toml`;
- Node.js 22.16.0 and npm 10.9.2, pinned by `.node-version` and
  `package.json`;
- CMake 4.4.0 and LLVM/libclang for the Granite worker;
- PowerShell and Git.

Run `.scripts\Get-GpuRuntime.ps1 -LinkOnly` to stage the verified native
import libraries before compiling. A full local runtime can be staged with
`.scripts\Get-GpuRuntime.ps1` when CUDA hardware qualification is required.

## Required gate

After a change, run the repository gate from PowerShell:

```powershell
.\scripts\Invoke-ScaffoldChecks.ps1
```

The non-Granite and Granite portions can be run locally with
`.scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall -SkipGranite` and
`.scripts\Invoke-ScaffoldChecks.ps1 -GraniteOnly`. The gate runs formatting,
clippy, Rust tests, frontend lint/typecheck/tests/build, dependency policy,
`cargo deny`, `cargo audit`, npm audit, and gitleaks. Hardware and interactive
proofs are explicitly marked ignored and must not be described as having run.

Increment the product version with `.\scripts\Increment-ProductVersion.ps1`.
It defaults to a minor release; pass `-Bump Major` or `-Bump Patch` when the
release policy calls for a different semantic-version increment.

## Dependency policy

`scripts/Test-DependencyPolicy.ps1` enforces the following:

- Cargo and npm dependencies use exact versions and committed lockfiles;
- workspace path dependencies must be workspace members and must be listed in
  the direct-dependency allowlist;
- build scripts may not perform implicit network downloads;
- advisory exceptions are explicit and expire on a dated review schedule;
- patched third-party sources are revision-pinned and recorded in the policy;
- Installed audit/tooling versions remain pinned in `dependency-policy/tools.json`.

Update the relevant manifest, lockfile, policy entry, and documentation
together. Do not commit model weights, native archives, credentials, or
generated local runtime directories.
