# Security

This build is unsigned and has no SmartScreen reputation — it's built for
local, personal use, not public distribution.

## Process and file integrity

- The desktop resolves its worker, native bridge, and ONNX Runtime only from
  canonical, app-owned absolute paths — it never searches the current
  directory or `PATH` for a helper binary.
- The inference worker verifies model files against the trusted manifest's
  checksums before use; a partial or mismatched download is never activated.
- The inference worker runs as a supervised child process (Windows Job
  object, deadlines, crash-loop quarantine after repeated failures) so a
  native crash can't take down the desktop app or leave orphaned processes.

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
