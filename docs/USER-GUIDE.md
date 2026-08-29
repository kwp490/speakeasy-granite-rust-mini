# SpeakEasy Mini User Guide

This build is unsigned and installs for the current Windows user only. It
targets Windows 11 x64.

If you have SpeakEasy installed, this installer is independent of it: it does
not replace, stop, start, repair, or uninstall that product, installs into its
own location, keeps its own settings, and uses a different keyboard shortcut so
the two never fight over one.

## Install and first dictation

1. Download `SpeakEasyMiniSetup.exe` from
   [Releases](https://github.com/kwp490/speakeasy-granite-rust-mini/releases) and
   run it as your normal Windows user. Do not elevate it. Windows SmartScreen
   will warn about it, because the build is not signed; the release page carries
   a SHA-256 you can check the download against.
2. It checks your hardware, downloads the transcription model (2.30 GB, or
   about 2.74 GB if you pick the graphics card) and verifies every file against
   a checksum fixed in advance.
3. It asks four things: which configuration to run, your dictation shortcut, any
   words you want protected from being respelled, and whether to keep your
   transcripts after you close the app. The last one defaults to **no** —
   transcripts are held in memory and never written to disk unless you say
   otherwise.
4. Then it **tests that transcription actually works**, by transcribing a short
   built-in clip and checking the result word for word. This is the step worth
   waiting for: a speech model that has stopped listening does not report an
   error, it writes fluent text about nothing.
5. When it finishes, the app opens. There is no second setup to walk through,
   and the answers you gave are already applied.
6. Allow Windows microphone access if prompted, and pick your microphone from
   **Settings → Audio** if the default is not the one you want.
7. Click into a text field somewhere, press `Ctrl+Alt+P`, say a sentence, and
   press it again. The text appears where your cursor was.

No transcript text appears while you speak, and that is expected: transcription
happens after you stop, and takes a few seconds. The dock does show that it is
recording, with an input-level meter and the elapsed time.

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
- **The dock** — the narrow strip SpeakEasy Mini keeps on screen. Its record
  button starts and stops a dictation exactly as the hotkey does, and delivery
  is the same: the transcript goes to whatever had focus. Use **Copy** to place
  it on the clipboard instead. The dock never takes keyboard focus, so it is
  deliberately not keyboard-operable — every action it offers has a keyboard
  path through the hotkey or the settings window. Right-click it for settings.

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

Uninstall removes everything: the program, your settings and personalization,
your transcript history, the downloaded speech models, recovery backups, and the
diagnostic log. It asks once first, listing all of it, and the answer it starts
on is **No** — so nothing is deleted by pressing Enter. Anything in the program
folder that setup did not put there is listed separately in that question,
because it is the part you cannot predict.

Deletion is best-effort: a file Windows will not release is reported by name
rather than passed over in silence. Uninstall never touches the separate Python
product's data, models, or credentials.
