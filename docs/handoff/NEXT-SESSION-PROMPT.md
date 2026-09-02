# Prompt for the next session

Copy everything below the line into a new Codex session in
`C:\Coding Projects\speakeasy-granite-rust-mini`.

Keep this file to the work that is actually next. It points into
`docs/handoff/CURRENT.md`; it does not replace it.

---

Continue work on SpeakEasy Mini, the local-only Windows dictation app in
`C:\Coding Projects\speakeasy-granite-rust-mini`.

Read these completely before changing anything:

1. `CLAUDE.md`
2. `docs/handoff/CURRENT.md`
3. `docs/UI-GUIDE.md` for any interface work

Then establish the live state instead of trusting this snapshot:

```powershell
git status -sb
git fetch origin
git log --oneline --decorate -5
git log --oneline origin/main..HEAD
& .\scripts\Get-ProductVersion.ps1
```

At the 2026-09-02 handoff, the tree is expected to be clean and pushed, and the
workspace version is `1.9.0`; verify both. The approved Settings workspace
redesign is production code (`9683d54`), and the live contrast proof fix is
`a761bc3`. A later commit may contain only this handoff refresh.

SpeakEasy Mini 1.9.0 is installed for the current user at
`C:\Users\User\AppData\Local\SpeakEasy Mini` and was left running normally.
The installed bootstrapper re-proved the processor engine, and the dock reported
`CPU / ready`. The user's existing personalization and provider files survived
the uninstall/reinstall byte-identically. Do not uninstall, reset, or replace
that installation unless the user asks.

The local installer is
`target\local-development\1.9.0\SpeakEasyMiniSetup.exe` with SHA-256
`4fc0d2f84b190aaeebd4d0f5b65118228250b67120c325d6022a6f6277589928`.
It was built from an empty fresh-build root. The full repository gate and the
installer lifecycle proof passed. The installed settings UI passed all six pages
at 720, 880 and 1200 CSS px without horizontal overflow or nested scroll regions,
and the dock/settings contrast proof passed 14 light/dark surfaces.

One release proof remains: `Test-SetupWizard.ps1` stopped at its mandatory
host-identity preflight because this shell cannot see the profile through
`\\localhost\C$`. That refusal happened before mutation. Never bypass the guard.
If the next task is to publish v1.9.0, first use a shell where this passes:

```powershell
.\scripts\Test-HostProfilePathIdentity.ps1
```

Only after that succeeds should the guarded wizard proof be run:

```powershell
.\scripts\Test-SetupWizard.ps1 -ArtifactRoot 'target\local-development\1.9.0'
```

No v1.9.0 tag or GitHub Release exists yet. Do not create either unless the user
explicitly asks to publish the release. If source or packaged inputs change,
rebuild the installer and repeat its proofs; do not reuse the recorded digest.

On this host, the full gate needed two environment accommodations:

- set `TEMP` and `TMP` to a repository-local directory such as
  `target\gate-temp`, because the ambient `C:\WINDOWS\TEMP` allowed the gate to
  create its temporary dependency-policy script but refused cleanup;
- the pinned gitleaks 8.30.1 binary is installed under
  `%LOCALAPPDATA%\Microsoft\WinGet\Packages\Gitleaks.Gitleaks_Microsoft.Winget.Source_8wekyb3d8bbwe`
  but its WindowsApps shim is absent, so prepend that directory to `PATH`.

For ordinary development, dot-source the environment in every new shell:

```powershell
. .\scripts\Enter-DevEnvironment.ps1
```

Use `Invoke-ScaffoldChecks.ps1 -SkipNpmInstall` as the full gate. It is not green
until it exits 0 and ends `no leaks found`. Preserve unrelated user changes, and
ask git—not this handoff—what has changed since the snapshot above.
