# SpeakEasy Mini User Guide

This build is unsigned and installs for the current Windows user only. It
targets Windows 11 x64.

If you have SpeakEasy installed, this installer is independent of it: it does
not replace, stop, start, repair, or uninstall that product, installs into its
own location, keeps its own settings, and uses a different keyboard shortcut so
the two never fight over one.

## Install and first dictation

1. Run the installer as your normal Windows user. Do not elevate it.
2. It checks your hardware, downloads the transcription model, verifies every
   file, and then **tests that transcription actually works** by transcribing a
   short built-in clip and checking the result. It asks one question — whether
   to keep your transcripts between sessions — and the default is no.
3. When it finishes, the app opens. There is no second setup to walk through.
4. Allow Windows microphone access if prompted, and pick your microphone from
   **Settings → Audio** if the default is not the one you want.
5. Click into a text field somewhere, press `Ctrl+Alt+P`, say a sentence, and
   press it again. The text appears where your cursor was.

Nothing appears on screen while you speak, and that is expected: transcription
happens after you stop, and takes a few seconds.

## Dictating

**Put your cursor in the field you want the text to land in, first.** SpeakEasy Mini
pastes into whatever has focus when the dictation finishes, so the text field has
to be the thing you were last working in. Launching SpeakEasy Mini and pressing the
hotkey without clicking into a document first is not a way to use it — there is no
target to paste into at that point, and the transcript goes to the clipboard and
SpeakEasy Mini's own result view rather than into your work. SpeakEasy Mini never types into
its own windows.

Nothing about a dictation starts by itself. It begins only when you press the
hotkey or the record button, never on launch, never when Windows starts SpeakEasy Mini
for you, and never in the background.

There are two ways to dictate:

- **Global hotkey** (`Ctrl+Alt+P` by default) — works from any app, even
  when the SpeakEasy Mini window is closed. Click into your text field, press once
  to start recording, press again to stop; SpeakEasy Mini transcribes locally and
  pastes the final text into that field. You can change the binding or turn the
  hotkey off in Settings → Activation.
- **Capture and Transcribe** (in the SpeakEasy Mini window) — select **Start
  capture**, speak, select **Stop**, then **Transcribe locally**. The result
  appears in SpeakEasy Mini's own result view; use **Copy** to place it on the
  clipboard.

Automatic paste is refused — leaving the text in the result view instead —
whenever the focused target is a password field, read-only, an elevated or
unrecognized window, the secure desktop, or a target SpeakEasy Mini can't safely
observe. A short Windows sound can mark recording start/stop if you enable
it in Settings; the visual recording indicator is always the source of
truth.

Push-to-talk and hands-free activation modes are selectable but not fully
exercised yet — hands-free in particular has no automatic stop (use the
Stop button or the hotkey again). Toggle mode is the well-tested default.

## Models, languages, and network use

The supported path today is English transcription with IBM Granite Speech 4.1,
on CPU or — where a CUDA-built worker is installed — on the GPU. It is the only
engine: transcription happens once, after you stop the recording, and the same
pass adds the punctuation and capitalisation. No other language or translation
pair is available. Limited `en-US` sentence-case formatting may be applied;
other locales are left unchanged (not implemented yet).

Nothing appears on screen while you speak. If a transcription fails, you get an
explanation and no text, rather than a lower-quality transcript — there is no
second engine to fall back on.

Model download uses the network; once installed and verified, dictation
works fully offline. SpeakEasy Mini never silently substitutes a different
model. Remote ASR, cloud text polish, and application auto-update are not
enabled in this build.

## Privacy and local data

Audio stays in memory for the active result/retry and isn't written to disk
by default. Transcript history is off (session-only) unless you opt in; if
enabled, it's stored in plaintext in your per-user app data, with retention,
export, and delete controls, and secure targets are always excluded from it.
Explicit Copy places text on the Windows clipboard, where other clipboard
tools can read it until it's overwritten.

Diagnostics you can export are sanitized and never include transcript text,
audio, credentials, tokens, or full private file paths. Diagnostic WAV
export is not implemented. See [PRIVACY.md](PRIVACY.md) and
[SECURITY.md](SECURITY.md) for the complete picture.

## Troubleshooting

- **No microphone appears** — check Windows microphone privacy permission,
  reconnect the device, and retry.
- **The model is unavailable or fails verification** — use the Models
  screen to reinstall it; partial or hash-mismatched downloads are never
  activated.
- **Transcription failed** — the captured audio remains available for
  Retry. Repeated worker failures pause automatic restarts (quarantine)
  until you use the manual recovery action.
- **Install says SpeakEasy Mini is running** — quit SpeakEasy Mini from its tray icon
  first.
- **Same version or downgrade refused during install** — use the Repair
  shortcut, or explicitly choose a different installer.

## Repair, backup, and uninstall

The Repair shortcut verifies file checksums, backs up your settings/history
before touching anything, and restores only into a new or empty
destination.

Uninstall offers separate choices for settings/personalization, transcript
history, installed models, and recovery backups — review each one; deletion
is best-effort. Uninstall never touches the separate Python product's data,
models, or credentials.
