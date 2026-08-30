# Privacy

## Summary

The base application works offline after you provision a local model.
Network access happens only for model and runtime downloads that you start.
Dictation audio and transcript text stay local. **Optional history is the only
thing that writes transcript text to disk**, and it is off by default; the
diagnostic log never contains transcript text at all. Exports you ask for are
the other way text reaches a file.

"Local" describes inference and data handling, not model or runtime download
calls. Like any Windows app, SpeakEasy Mini can't prevent a
local administrator, malware running as the same user, or Windows's own
crash-reporting/backup infrastructure from observing process memory or
app-owned files.

## What's stored, where, and when it leaves the machine

| Data | Default location / lifetime | Leaves the machine? | Your controls |
|---|---|---|---|
| Live microphone audio | Memory only, discarded when the recording ends | Never (local ASR) | Explicit activation, visible recording state, cancel |
| Transcript log | Memory, newest 50. Seeded at launch from persisted history, so with history enabled it can include transcripts from earlier runs | Never (local ASR) | Read it in Settings → Transcript log, or the pinned log window; copy one entry. Deleting the saved history removes the entries restored from it; the rest dies with the process |
| Persisted history (optional) | Off by default; per-user SQLite when enabled, plaintext | Never | Separate consent, retention period, export, delete-all. Transcripts the app delivered to a password field, the secure desktop, an elevated window, or a target it could not classify are excluded — see the qualification below |
| Clipboard text | Windows clipboard until overwritten | Visible to other clipboard readers on your machine | The Copy button, and the auto-paste setting — which also copies, because pasting is a clipboard operation |
| Settings, dictionary, snippets | Per-user app data (plaintext) | Never | Export/reset |
| Installed model files | Per-user app-owned model folder | Downloaded during setup when no verified copy is present, verified against a trusted manifest | Install/remove |
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
- **Secure targets are excluded from history when SpeakEasy Mini attempted the
  delivery and classified where the text went.** The classification is made by
  the same guard that refuses to paste into them, and the exclusion is applied
  *after* delivery has decided, so a transcript is never written on the
  assumption that its target was safe. A target the app tried to inspect and
  could not is treated as unsafe for this purpose.

  **The qualification matters, and it is where the protection ends.** Two paths
  produce a transcript without any delivery being attempted: dictating with
  auto-paste turned off, and re-transcribing retained audio from Settings. On
  both, no window is inspected, because none received the text — the transcript
  is placed in the transcript log and the result view for you to copy. Those
  transcripts are recorded as *delivery not attempted* and, with history
  enabled, they are retained.

  So if you dictate with auto-paste off and then paste the result into a
  password field yourself, that transcript is in the history database. The app
  cannot prevent this and does not claim to: it has no way to know where you
  paste from the clipboard, and inspecting the foreground window at some later
  moment would be a guess about an action it never observed.

  **The recommendation is therefore to turn persisted history off before
  dictating sensitive material.** That is the only setting that holds on every
  path, and it is the shipped default. Leaving auto-paste on makes the
  classification run and does exclude the cases it covers, but it is not the
  safer configuration in general — it hands your text straight to whatever holds
  the foreground, which on a slow transcription may not be the window you were
  in when you started. Turning history off removes the retention question
  entirely.
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
