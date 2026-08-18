# Privacy

## Summary

The base application works offline after you provision a local model.
Network access happens only for model and runtime downloads that you start.
Dictation audio and transcript text stay local. Optional session history and
explicitly enabled diagnostic logging are the only ways transcript content is
persisted, and history is off by default.

"Local" describes inference and data handling, not model or runtime download
calls. Like any Windows app, SpeakEasy Mini can't prevent a
local administrator, malware running as the same user, or Windows's own
crash-reporting/backup infrastructure from observing process memory or
app-owned files.

## What's stored, where, and when it leaves the machine

| Data | Default location / lifetime | Leaves the machine? | Your controls |
|---|---|---|---|
| Live microphone audio | Memory only, for the active capture | Never (local ASR) | Explicit activation, visible recording state, cancel |
| Transcript (draft/final) | Memory, for the active session | Never (local ASR) | Result view, explicit copy, session clear |
| Persisted history (optional) | Off by default; per-user SQLite when enabled | Never | Separate consent, retention period, export, delete-all |
| Clipboard text (explicit copy or auto-paste) | Windows clipboard until overwritten | Visible to other clipboard readers on your machine | Separate auto-copy/auto-paste settings |
| Settings, dictionary, snippets | Per-user app data (plaintext) | Never | Export/reset |
| Installed model files | Per-user app-owned model folder | Downloaded once, verified against a trusted manifest | Install/remove |
| Diagnostic log | Per-user app data when enabled; reason codes, timings, and coarse state | Never | The existing `disk_logging_enabled` setting |

## Network behavior

- No network request happens for dictation itself once a model is
  installed.
- Model and runtime downloads use HTTPS with the standard trust chain.

## Storage, retention, and deletion

- Settings use atomic writes with a backup, so a crash mid-write can't
  corrupt your profile.
- Persisted history (if you enable it) supports configurable retention
  (1–365 days) and an explicit delete-all that removes the database and its
  sidecar files.
- Secure targets (password fields, etc.) are excluded from history even
  when history is enabled.
- Model removal only ever deletes the app-owned, verified copy — never a
  path you pointed it at manually.
- Uninstall gives you separate choices for settings/personalization,
  history, models, and recovery backups. Deletion is best-effort (normal
  filesystem delete, not guaranteed forensic erasure).

## Logging

Transcribed text is never logged to disk. Detailed diagnostic logging remains
available for troubleshooting: stable reason codes, counts, timings,
engine/device state, and coarse state. The existing `disk_logging_enabled`
setting is the single toggle for the sanitized local diagnostic log; it does
not enable transcript or raw-audio logging, and the shipped app has no separate
transcript-content logging mode. Logs and exportable diagnostics never contain
raw audio, credentials, clipboard contents, or full file paths.

## Credentials

The app only reads legacy credential entries to report whether an older
installation has values that may need migration. It does not use them for a
current network feature, return them through the UI, log them, or place them
in a command-line argument or environment variable.

## Residual risks (true of any local Windows app)

- A local administrator, malware in your user session, or a clipboard
  monitor could observe sensitive data while it's in memory or on the
  clipboard.
- Plaintext settings/history are readable to anyone with file access to
  your user profile, unless you add disk encryption yourself.
- Best-effort deletion doesn't guarantee data is unrecoverable from SSD
  storage.
