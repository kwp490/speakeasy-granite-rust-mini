# SpeakEasy Mini runbook

This is the first-response guide for the local Windows build. Keep raw audio,
transcripts, credentials, and full private paths out of issue reports and
diagnostic exports.

## First response

1. Stop and retry a short dictation once. Check that the microphone is present
   in Windows privacy settings and that the intended model is installed and
   verified.
2. Read the result view and Settings diagnostics. Record the stable error or
   degradation code, model/source/revision, provider, and whether the result
   was retained. Do not infer success from a green hardware-independent test.
3. If the worker is quarantined, use the explicit recovery action, close and
   reopen SpeakEasy Mini, and retry after the machine is idle. Do not delete the
   profile or model directory before preserving the diagnostic code.
4. If the problem began after an installer change, use the Repair shortcut and
   the recovery steps below. Same-version reinstall and automatic downgrade
   are intentionally refused.

## Failure codes and first action

| Stable code | Meaning | First action |
| --- | --- | --- |
| `degradation.microphone_denied` | Windows denied microphone access | Re-enable microphone permission, then retry |
| `degradation.microphone_unavailable` | No supported capture device is available | Reconnect/select a microphone |
| `degradation.audio_overflow` | Capture audio exceeded the bounded queue | Stop, retry a shorter dictation, and check device load |
| `degradation.final_engine_failed` | Final worker timed out, crashed, or ran out of memory | Use the retained result/retry path and inspect the worker code |
| `degradation.worker_quarantined` | Repeated worker failures paused automatic restarts | Use explicit worker recovery, then retry once |
| `degradation.accelerator_lost` | GPU/provider disappeared during the operation | Restart the app; if it persists, revert to the CPU worker |
| `degradation.model_unavailable` | Model is missing, corrupt, tampered, or mid-update | Re-verify or reinstall the selected model |
| `degradation.target_changed` / `degradation.target_refused` | The focused target changed or is unsafe | Use the result view and explicit Copy |
| `degradation.clipboard_unavailable` | Clipboard was busy or changed | Retry Copy after closing clipboard managers |
| `degradation.optional_network_unavailable` | Optional network capability is unavailable | Continue with the local result; no network is needed for local ASR |
| `degradation.optional_storage_unavailable` | History/settings storage is unavailable or too new | Continue without history and repair the profile |
| `degradation.lifecycle_interrupted` | Sleep/resume interrupted the operation | Retry after the machine is awake and stable |
| `degradation.shutdown_in_progress` | Shutdown interrupted the operation | Reopen SpeakEasy Mini after startup completes |
| `degradation.protocol_too_new` | An optional worker protocol is newer than this app | Install matching app/worker artifacts |
| `degradation.disk_full` | The operation needs more disk space | Free space, then retry the download or dictation |
| `degradation.dictation_busy` | Another dictation owns the operation | Wait for it to finish or cancel it explicitly |

The app also reports more specific boundary codes such as
`capture_device_unavailable`, `capture_failed`, `capture_drain_timeout`,
`profile_recovery_required`, `history_recovery_required`, and
`pending_update_health_check_failed`. Preserve the exact code when reporting
the issue; it is more useful than a paraphrase.

## Diagnostics

In Settings, enable **Keep a local diagnostic log** when troubleshooting. This
uses the existing `disk_logging_enabled` setting and writes sanitized event
names, reason codes, timings, counts, engine/device state, and coarse state to
the per-user `logs\speakeasy.log`. The log rotates at 5 MiB and keeps one
previous generation.

The shipped app never writes transcript text or raw audio to this log, even
when the setting is enabled. Exported diagnostics likewise omit transcripts,
audio, credentials, tokens, and full private paths. Turn logging back off after
the reproduction if the issue no longer needs it.

## Repair and recovery

`apps/bootstrapper` is the tool bundled with the installer, at
`speakeasy-bootstrapper.exe` in the install directory. It has no Tauri or
inference dependency. Run with no arguments it is SpeakEasy Mini's setup wizard; run
with any of the verbs below it is the repair and recovery tool it absorbed, and
those verbs are unchanged from the `speakeasy-repair` they came from. It can:

- verify a retained recovery manifest and every recorded SHA-256/byte count;
- create a backup of the settings/history/model data selected by the caller,
  plus the installer and optional attached-file integrity record;
- restore only into a new or empty absolute destination;
- inspect/update the pending-update health marker; and
- launch a verified installer only after an explicit `reinstall` command.

The tool requires the recovery manifest and paths supplied by the caller. A
required `--signature` file is copied and checked as an integrity record, not
as a publisher identity or Authenticode signature; this local build is
unsigned. Never use `--allow-unsigned-local-development` for an artifact whose
provenance you have not independently checked.

Typical commands are shown by running `speakeasy-bootstrapper --help`. Running
it with **no** arguments opens the setup wizard instead, which is what a
double-click does:

```text
speakeasy-bootstrapper verify <backup-manifest.json>
speakeasy-bootstrapper restore --manifest <backup-manifest.json> --destination <new-or-empty-path>
speakeasy-bootstrapper status --data-root <app-data-root>
```

The repair verbs refuse non-empty restore destinations and do not silently
overwrite data. Settings and history are plaintext on disk; a backup is a
recovery copy, not secure erasure or encryption.

## Returning to a previous installer

Use an installer retained in a verified recovery bundle or a checksum-verified
GitHub release asset. First quit SpeakEasy Mini for every local Windows session.
Run `verify`, review the reported product version and unsigned status, then
use the explicit `reinstall` command (adding
`--allow-unsigned-local-development` only for a known local development build).
The installer lifecycle intentionally blocks same-version reinstall and
automatic downgrade; the Repair shortcut is the supported explicit route back
to a previous version.
