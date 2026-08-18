# Testing SpeakEasy Mini on another Windows computer

The current test build is unsigned and installs for the current Windows user.
It is not a release-certified build. Do not run the installer as Administrator.

Installers are **built locally** — no GitHub runner ever produces one, and
`scripts/Test-LocalOnlyPolicy.ps1` fails if CI configuration reappears. A build
may afterwards be *attached* to a GitHub release by hand as an archive of what
was shipped; the repository is private, so downloading that asset needs a
credential and it is not the intended transfer route.

An earlier revision of this file claimed the project "does not publish
installers … on GitHub". That was wrong once `v1.1.0-test.1` existed, and it is
the sentence a reader lands on first, so it sent people to build from source
when they did not need to.

## Fastest path: copy the installer to the target machine

The target machine needs **no build toolchain at all** — no Rust, Node, CMake,
LLVM or Visual Studio Build Tools. Those are only for building. It needs
Windows 11 x64, the WebView2 runtime (already present on an up-to-date Windows
11; the installer fetches it otherwise), and roughly 8 GB free for the model
packs it downloads on first launch.

On the machine that has this repository:

```powershell
.\scripts\Build-LocalInstaller.ps1
$version = & .\scripts\Get-ProductVersion.ps1
Get-ChildItem "target\local-development\$version"
```

Copy these two files to the target machine by any means — USB, network share,
cloud drive:

- `SpeakEasy Mini_<version>_x64-setup.exe`
- `SHA256SUMS`

On the target machine, **verify the checksum before running it**, because an
unsigned installer that arrived over a copy is exactly the case a checksum is
for:

```powershell
Get-FileHash .\SpeakEasy Mini_<version>_x64-setup.exe -Algorithm SHA256
Get-Content .\SHA256SUMS | Select-String 'setup.exe'
```

Those two hashes must match. Then run the installer normally — not as
Administrator. Because it is unsigned, Windows SmartScreen may warn; that is
expected for a locally built artifact.

To install unattended to the default per-user location:

```powershell
.\SpeakEasy Mini_<version>_x64-setup.exe /S "/D=$env:LOCALAPPDATA\SpeakEasy Mini"
```

The installer refuses to replace a running app, to reinstall the same version,
or to downgrade. Quit SpeakEasy Mini from its tray icon before upgrading.

The installer does not bundle model weights, and it does the whole of first-run
setup itself — hardware check, download, verification, and an engine smoke test
that transcribes a bundled clip and compares the result word for word. By the
time the app launches, there is nothing left to configure.

To confirm an install:

1. Grant microphone access when Windows asks.
2. Press the shortcut (`Ctrl+Alt+P`), speak a sentence, press it again.
3. The text should appear in whatever had focus. Nothing appears while you
   speak — that is expected, not a fault.
4. Open **Settings → Transcription** and confirm the engine and provider it
   reports match what setup installed. On a machine set up for the GPU, a CPU
   reading here is a fault and the app says so.

The model weights are downloaded from IBM's public
[`ibm-granite/granite-speech-4.1-2b-GGUF`](https://huggingface.co/ibm-granite/granite-speech-4.1-2b-GGUF)
repository, and the GPU worker from a repository pinned in the manifest. No
Hugging Face account or token is required, and every file is verified by length
and SHA-256 before activation.

## Build and test from source

Use this route when testing a code change rather than the published installer.
Install these prerequisites first:

- Windows 11 x64;
- Rustup with the pinned Rust 1.97.1 toolchain;
- Node.js 22.16.0 and npm 10.9.2;
- Visual Studio Build Tools with the Desktop development with C++ workload;
- CMake and LLVM/libclang for the Granite worker build;
- PowerShell 7 or Windows PowerShell 5.1.

From PowerShell:

```powershell
Set-Location C:\Coding_Projects\speakeasy-granite-rust-mini
git switch main
rustup toolchain install 1.97.1
npm ci
.\scripts\Enter-DevEnvironment.ps1
npm run tauri -- dev
```

There is no native runtime to stage: the streaming engine that needed one is
gone, and llama.cpp is compiled from source by the toolchain. Models are
downloaded and verified by setup, or by `scripts\Get-Granite.ps1` for a dev
checkout. Do not copy model files into the repository or commit them to Git.

For the non-hardware workspace checks:

```powershell
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall -SkipGranite
```

The ignored CUDA execution-provider proof requires a real supported NVIDIA
machine and the full runtime/model setup; ordinary green unit tests do not
prove that CUDA executed.

## What to report

Please record the Windows version, GPU model and driver version, whether the
GPU or CPU pack was selected, whether model verification completed, whether a
short dictation produced text, and any error code shown in Settings. Exported
diagnostics are sanitized and should not contain audio, transcripts, tokens, or
credentials.
