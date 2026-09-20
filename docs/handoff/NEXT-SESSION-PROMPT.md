# Prompt for the next session

Copy everything below the line into a new session in
`C:\Coding_Projects\speakeasy-granite-rust-mini`.

Keep this file to the work that is actually next. It points into
`docs/handoff/CURRENT.md`; it does not replace it.

---

Continue work on SpeakEasy Mini, the local-only Windows dictation app in
`C:\Coding_Projects\speakeasy-granite-rust-mini`.

Read these completely before changing anything:

1. `CLAUDE.md`
2. `docs/AI-WORKING-AGREEMENT.md`
3. `docs/handoff/CURRENT.md`
4. `docs/UI-GUIDE.md` for any interface work

Then establish the live state instead of trusting this snapshot:

```powershell
git status -sb
git log --oneline --decorate -5
git remote -v
& .\scripts\Get-ProductVersion.ps1
```

Once a remote exists, this is what lists commits that are in no release:

```powershell
git log --oneline origin/main..HEAD
```

**This checkout has no git remote and only a short history.** At the 2026-09-20
handoff it was on `master` with two commits: a baseline snapshot of the tree as
received, and the review-fix batch on top of it. It is not a clone of
`kwp490/speakeasy-granite-rust-mini`, so it has none of that repository's
ancestry. Do not add a remote and push without the owner deciding how the two
histories reconcile — a force-push from here would discard the published
history and the v1.8.1 release's ancestry. Ask before touching remote state.

The workspace version is `1.9.1`; verify it. An independent review raised eight
findings and all eight are fixed, each with a regression test proved by a red
control. The same batch took RUSTSEC-2026-0285 by moving `rustls` to 0.23.45.
`docs/handoff/CURRENT.md` has the detail under "Prepared v1.9.1 state".

SpeakEasy Mini 1.9.1 is installed for the current user at
`%LOCALAPPDATA%\SpeakEasy Mini` and was left running normally. The install was a
direct `Decision::Upgrade` from 1.9.0 rather than an uninstall and reinstall, so
the existing personalization and settings files were never touched. Do not
uninstall, reset, or replace that installation unless the user asks.

The local installer is
`target\local-development\1.9.1\SpeakEasyMiniSetup.exe`, 38,249,187 bytes, with
SHA-256 `5c3f960b711836c27217730bceac6fe5f4ecd361a099a6d89b536e361ab75cb3`.
It was built from an empty fresh-build root and the full repository gate passed,
exiting 0 and ending `no leaks found`.

**Both installer proofs are outstanding, and neither was skipped by choice.**

- `Test-InstallerLifecycle.ps1` refuses while any SpeakEasy Mini version stamp
  exists under `HKCU:\Software\SpeakEasy Mini\LocalDevelopment`. The real
  installation owns that stamp, and the script will not delete a stamp that may
  belong to an installation someone uses. It needs a host with SpeakEasy Mini
  uninstalled.
- `Test-SetupWizard.ps1` refuses at its mandatory host-identity preflight,
  because this shell cannot prove the profile through `\\localhost\C$`.

Never bypass either guard. Before any release is cut from 1.9.1, run:

```powershell
.\scripts\Test-HostProfilePathIdentity.ps1
```

and only once it succeeds, on a host with the app uninstalled:

```powershell
.\scripts\Test-InstallerLifecycle.ps1 -ArtifactRoot 'target\local-development\1.9.1'
.\scripts\Test-SetupWizard.ps1 -ArtifactRoot 'target\local-development\1.9.1'
```

No `v1.9.1` tag or GitHub Release exists. Do not create either unless the user
explicitly asks to publish. If source or packaged inputs change, rebuild the
installer and repeat its proofs; do not reuse the recorded digest.

## Environment notes for this host

All five pinned gate tools are installed and match
`dependency-policy/tools.json`: `cargo-llvm-cov`, `cargo-deny` and `cargo-audit`
under `~/.cargo/bin`, with `ripgrep 15.2.0` and `gitleaks 8.30.1` beside them.
The gate's ripgrep check compares the version line exactly, so ripgrep must be
the `cargo install` build — an official release binary reports a `(rev ...)`
suffix and fails that check.

Use PowerShell 7. Windows PowerShell 5.1 turns ordinary cargo progress output
into a terminating `NativeCommandError` under these scripts.

Two hazards cost real time in the last session:

- **Quoted heredocs in the Bash tool strip one level of backslashes**, which
  silently turns `\\localhost\C$` into `\localhost\C$` and breaks regex escapes
  written the same way. Read the bytes back after writing any backslash.
- **This machine's antivirus quarantines freshly built unsigned binaries by
  heuristic.** It removed `rustdoc.exe` from the Rust toolchain mid-gate, which
  surfaced as an opaque "could not execute process ... (os error 2)" doctest
  failure. If a build artifact vanishes, check quarantine before debugging the
  build, and never change an antivirus setting.

For ordinary development, dot-source the environment in every new shell:

```powershell
. .\scripts\Enter-DevEnvironment.ps1
```

Use `Invoke-ScaffoldChecks.ps1 -SkipNpmInstall` as the full gate. It is not
green until it exits 0 and ends `no leaks found`. Preserve unrelated user
changes, and ask git — not this handoff — what has changed since the snapshot
above.
