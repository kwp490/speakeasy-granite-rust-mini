# Security

This build is unsigned and may trigger Windows SmartScreen. It is nonetheless
publicly distributed, from the project's GitHub Releases page, so the release
carries a `SHA256SUMS` file: an unsigned download a stranger can verify by digest
is the only assurance on offer, and signing is a settled decision not to, rather
than something pending.

## Process and file integrity

- The desktop resolves the Granite worker, the model root, and — on a
  graphics-card installation — the CUDA runtime files beside the worker, only
  from canonical, app-owned absolute paths under `%LOCALAPPDATA%\SpeakEasy Mini`
  and `%APPDATA%\ai.speakeasy.mini`. It never searches the current directory or
  `PATH` for a helper binary.
- Model files are hashed against the trusted manifest's SHA-256 digests
  immediately before the worker is started. **What that protects against:** a
  partial or interrupted download, a corrupted file, a mismatched or substituted
  model sitting on disk at the moment the check runs, and a manifest entry that
  does not match the bytes it names. Any of those refuses the warm and reports
  `granite_model_files_unverified` rather than loading.

  **What it does not protect against, stated because the gap is real and open:**
  the hash is computed in the desktop process, and the worker is then handed a
  *path* and reopens the files itself, checking presence rather than digests. So
  anything able to write to the model directory as the user — malware in the
  session, another process running as you, a local administrator — can replace a
  file in the window between the check and the worker's open, and nothing would
  notice. Closing it means verifying inside the worker or handing it
  already-verified handles; it is an open finding in `docs/handoff/CURRENT.md`
  and needs a threat-model decision rather than more code. Note that an attacker
  with that access already has the user's files, so this raises the bar on what a
  local attacker can do rather than being the only thing standing between them
  and your data.
- The Granite worker runs as a supervised child process (Windows Job object,
  deadlines, crash-loop quarantine after repeated failures) so a native crash
  can't take down the desktop app or leave orphaned processes.

## Installer behavior

- Setup refuses to install or uninstall while SpeakEasy Mini is running under any
  local user session. (NSIS was named here until 2026-08-21; it was replaced
  entirely by `apps/bootstrapper` and the refusal is now that program's.)
- Same-version reinstalls and automatic downgrades are refused; use the
  Repair shortcut for explicit recovery instead.
- An uninstall removes the program and all of its user data by default, asks
  once with the whole scope named before doing so, and reports by name anything
  it could not remove. `--keep-user-data` is a testing flag, not a user option.
- The repair tool requires explicit artifact selection and confirmation,
  and refuses to restore into a non-empty destination.

## Update mechanism

There is no auto-update mechanism wired up in this build — no updater UI,
key, or endpoint is active.

## Reporting a vulnerability

Report suspected vulnerabilities privately to [kwp490 through GitHub's private
vulnerability reporting](https://github.com/kwp490/speakeasy-granite-rust-mini/security/advisories/new).
This is the project's security-report address; there is no backup maintainer.
Include the version, Windows version, impact, and the smallest safe
reproduction steps. Don't attach real recordings, transcripts, credentials,
tokens, or another person's data to a report.

This project doesn't currently have code signing, a public disclosure
process, or third-party security certification — treat it accordingly for
your own threat model.
