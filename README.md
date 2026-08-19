# SpeakEasy Mini

A local, private Windows dictation app. Press a global hotkey, speak, press it
again, and the transcript is pasted into whatever field has focus — no cloud
account, no network round-trip, nothing about what you said leaves the machine.

**One engine, one pass.** IBM Granite Speech 4.1 transcribes the recording
after you stop it, and the same pass produces the punctuation and casing.
Nothing appears on screen while you speak, and there is no second engine behind
it: if a transcription fails, the app tells you what went wrong and what to do
about it rather than quietly handing you a worse result.

## How it differs from SpeakEasy

SpeakEasy Mini is a fork that removes two things:

- **No real-time transcription.** The streaming recognizer and everything under
  it — sherpa-onnx, ONNX Runtime, the Nemotron models — is gone. That removes
  about 3 GB of downloads, a second inference runtime, and the class of bug
  where a live guess and a final transcript disagree.
- **No large HUD.** A narrow dock clings to the edge of your screen showing the
  level meter, the elapsed time, and how the last dictation ended. Right-click
  it for settings.

It installs alongside SpeakEasy rather than replacing it: different identity,
separate settings and logs, and `Ctrl+Alt+P` instead of `Ctrl+Alt+L` so the two
never fight over the shortcut.

## Installing

There are two ways in, and they exist for different kinds of trust.

### 1. The installer

Download `SpeakEasyMiniSetup.exe` from
[Releases](https://github.com/kwp490/speakeasy-granite-rust-mini/releases) and
run it — not as Administrator. One file, about 37 MB: it carries the app and the
engine, but no models. It walks eight steps:

1. **Checks this computer** — processor, memory, disk, graphics card — and
   reports what it found. Nothing is installed or downloaded yet.
2. **Asks how it should run.** Today that is the processor on every machine: the
   graphics-card engine is compiled rather than downloaded and has not been
   published, so the option is shown and disabled with that reason rather than
   hidden. See [the CUDA note](#a-note-on-graphics-cards).
3. **Downloads the model** — about 2.2 GB — and verifies every file against a
   SHA-256 digest pinned in
   [`models/trusted-manifest.json`](models/trusted-manifest.json). Interrupted
   downloads resume where they stopped.
4. **Installs**, into `%LOCALAPPDATA%\SpeakEasy Mini`, for your user account
   only. No administrator prompt.
5. **Asks for your shortcut**, and *registers it* to check it is free rather
   than assuming — Windows will not tell you who owns a hotkey, so the only way
   to know is to take it. `Ctrl+Alt+P` by default.
6. **Asks for words to protect** — names and jargon you do not want respelled.
   Optional; Settings has the same list.
7. **Asks what is kept.** Transcripts are discarded when you close the app
   unless you say otherwise, and the diagnostic log is on.
8. **Transcribes a short bundled audio clip and checks the result word for
   word.** This is the step that matters: a speech model with a broken audio
   projector does not fail, it writes fluent text without listening to
   anything. Comparing against known ground truth is the only check that
   catches it.

Then it launches the app, and tells you if it could not.

If any step fails, it says which one and stops rather than installing something
that will not work — with one deliberate exception: a failed engine check offers
Retry and lets you continue anyway, saying what continuing costs.

Windows SmartScreen will warn about the download. The build is not code-signed
and will not be; the release page carries a SHA-256 for the file so you can
check you have the bytes that were published:

```powershell
Get-FileHash .\SpeakEasyMiniSetup.exe -Algorithm SHA256
```

### 2. Have an agent do it, so you can watch

If you would rather read the code than trust a binary — reasonable, for
something that listens to your microphone and types into your windows — give
this repository's URL to Claude Code or a comparable coding agent and ask it to
set the app up for you. Everything the installer does is a documented, ordinary
build:

```
Clone <this repo>, read docs/NEW-MACHINE.md, and set up SpeakEasy Mini
on this machine. Show me each download and its verified hash before
you run anything.
```

Nothing is hidden from that path. The model URLs and their digests are in the
manifest, the build steps are in [docs/NEW-MACHINE.md](docs/NEW-MACHINE.md),
and the checks are one script. You end up with the same app, having seen every
step.

## What it needs

To **run**:

- Windows 11 x64
- The WebView2 runtime — already present on an up-to-date Windows 11. Setup
  **checks for it and does not download it**: Microsoft serves its installer
  from a redirect whose bytes change by design, so it cannot be pinned by
  checksum the way everything else here is. If it is missing, setup says so and
  names it rather than installing something it cannot verify
- About 4 GB free disk space for the Granite weights, downloaded at setup
  rather than bundled
- A microphone
- An NVIDIA card is **optional**. Granite runs on the CPU perfectly well —
  roughly 1.6 s for a short utterance against 0.16 s on an RTX 5090.

### A note on graphics cards

Granite's GPU support is a **build feature**, not a download: llama.cpp's CUDA
backend is compiled into the worker executable rather than loaded beside it. No
CUDA-built worker has been published, so every install today runs on the
processor — including on machines with a card that clears every requirement.

Setup says so rather than hiding it, and the app records which configuration was
installed so that "running on the processor" can be told from "the graphics-card
engine failed to load". If you want the GPU build now,
`scripts\Enable-GraniteCuda.ps1` builds one locally and needs the CUDA Toolkit.

To **build**, additionally:

- Rust (pinned via `rust-toolchain.toml`)
- Node.js and npm (pinned via `.node-version` and `package.json` `engines`;
  `.npmrc` sets `engine-strict=true`, so a mismatched Node fails `npm ci`
  outright rather than warning)
- Visual Studio Build Tools, CMake and LLVM/libclang — `speakeasy-granite`
  compiles llama.cpp from source rather than downloading it
- The CUDA Toolkit, only if you want to build the GPU worker yourself

`.\scripts\Initialize-DevMachine.ps1` reports which of these are present and
stages what it can. [docs/NEW-MACHINE.md](docs/NEW-MACHINE.md) is the full
sequence.

## Building and running

```powershell
npm ci
npm run tauri -- dev
```

The full gate — formatting, clippy, tests, dependency policy, frontend
lint/typecheck/build:

```powershell
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
```

An installer:

```powershell
.\scripts\Build-LocalInstaller.ps1
```

Produces `SpeakEasyMiniSetup.exe` under `target\local-development\<version>\`,
unsigned and current-user, with a manifest and SHA-256 checksums of everything
it packages. Beside it are the same installer without its payload
(`speakeasy-bootstrapper.exe`) and the `payload\` directory it would otherwise
carry, which is what `scripts\Test-InstallerLifecycle.ps1` drives.

## Privacy

Audio and transcripts stay on the machine. The only network traffic is
downloading model weights at setup, from URLs pinned by digest in the manifest.

Transcripts are held in memory and discarded when the app closes, which is the
default. Choosing to keep them writes them to disk and asks you to acknowledge
that first. "Discarded when the app closes" is implemented by never writing
them, not by deleting on exit — a delete-on-exit is a promise a crash cannot
keep.

The diagnostic log records error codes and counters, never transcript text,
device names, or paths. See [docs/PRIVACY.md](docs/PRIVACY.md).

## Repository layout

- `apps/desktop` — the Tauri app (React/TypeScript frontend, Rust backend).
- `apps/bootstrapper` — the installer, which also carries the repair and backup
  verbs. Shipped as the downloadable setup binary.
- `crates/` — the Rust libraries the app is built from: audio capture, the
  worker protocol boundary, delivery/paste safety, personalization transforms,
  storage, model lifecycle, Windows integration.
- `workers/granite-worker` — the supervised child process that runs Granite on
  llama.cpp.
- `docs/` — architecture, user guide, privacy, and security documentation.
- `packaging/` — third-party license notices shipped with the installer.
- `scripts/` — build, packaging, and verification scripts.

All builds, tests, version bumps and installer work are local. GitHub runners
and automation are intentionally not part of this project;
`scripts/Test-LocalOnlyPolicy.ps1` fails if `.github` configuration reappears.

## Licence

MIT — see [LICENSE](LICENSE). Third-party notices are in `packaging/`.
