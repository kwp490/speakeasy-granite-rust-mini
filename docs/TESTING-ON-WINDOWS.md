# Testing SpeakEasy Mini on another Windows computer

The current test build is unsigned and installs for the current Windows user.
It is not a release-certified build. Do not run the installer as Administrator.

Installers are **built locally** — no GitHub runner ever produces one, and
`scripts/Test-LocalOnlyPolicy.ps1` fails if CI configuration reappears. A build
is afterwards *attached* to a GitHub release by hand. The repository is
**public** and so are the releases: the asset downloads anonymously, and that is
the intended transfer route.

Two earlier revisions of this paragraph were wrong in opposite directions. One
claimed the project "does not publish installers … on GitHub", which stopped
being true at `v1.1.0-test.1`. The next said the repository was private and the
download needed a credential, which sent readers to build from source for a file
they could have clicked.

## Fastest path: copy the installer to the target machine

The target machine needs **no build toolchain at all** — no Rust, Node, CMake,
LLVM or Visual Studio Build Tools. Those are only for building. It needs
Windows 11 x64, the WebView2 runtime, and disk space for the model.

**WebView2 is detected, never fetched.** Setup checks for it and stops with an
instruction if it is missing; it will not download a Microsoft installer on your
behalf. An up-to-date Windows 11 already has it.

**Sizes, and which kind of byte each one is.** The model is **2,298,601,952
bytes** — 2.30 GB decimal, 2.14 GiB — and that is both the download and what it
occupies once installed. Choosing the graphics card adds **438.5 MB** of
transfer (the CUDA worker plus two NVIDIA redistributables), which unpacks to
**585.3 MB**. So the transfer is ~2.30 GB on the processor path and ~2.74 GB on
the graphics-card path. Allow **6 GB free** to be comfortable: the installed
files plus staging headroom, since each archive is written before it is
unpacked.

**Setup downloads the model, not the first launch.** The app does not fetch
anything at startup; by the time it opens, everything it needs is on disk and
verified.

On the machine that has this repository:

```powershell
.\scripts\Build-LocalInstaller.ps1
$version = & .\scripts\Get-ProductVersion.ps1
Get-ChildItem "target\local-development\$version"
```

Copy these two files to the target machine by any means — USB, network share,
cloud drive:

- `SpeakEasyMiniSetup.exe`
- `SHA256SUMS`

On the target machine, **verify the checksum before running it**, because an
unsigned installer that arrived over a copy is exactly the case a checksum is
for:

```powershell
Get-FileHash .\SpeakEasyMiniSetup.exe -Algorithm SHA256
Get-Content .\SHA256SUMS | Select-String 'setup.exe'
```

Those two hashes must match. Then run the installer normally — not as
Administrator. Because it is unsigned, Windows SmartScreen may warn; that is
expected for a locally built artifact.

To install unattended to the default per-user location:

```powershell
.\SpeakEasyMiniSetup.exe --install --install-root "$env:LOCALAPPDATA\SpeakEasy Mini"
```

Both the file name and the flags changed when NSIS was replaced, and this
document carried the old ones until 2026-08-21. `/S "/D=..."` is NSIS syntax:
the current installer does not recognise it, falls through to the repair verb
parser, prints its usage and exits 1 — measured, not assumed. Use the call
operator as written above rather than `Start-Process -ArgumentList`, which
quotes nothing and would hand `SpeakEasy` and `Mini` over as two arguments.

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
- Rustup with the pinned Rust 1.98.0 toolchain;
- Node.js 22.16.0 and npm 10.9.2;
- Visual Studio Build Tools with the Desktop development with C++ workload;
- CMake and LLVM/libclang for the Granite worker build;
- PowerShell 7 or Windows PowerShell 5.1.

From PowerShell:

```powershell
Set-Location C:\Coding_Projects\speakeasy-granite-rust-mini
git switch main
rustup toolchain install 1.98.0
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

The hardware-gated proofs are `#[ignore]`d and need a real NVIDIA machine, a
staged worker and the model files; ordinary green unit tests do not prove that
CUDA executed. They live in `apps/desktop/src-tauri/src/granite_engine.rs` — the
CUDA execution-provider proof this paragraph used to name was ONNX Runtime's and
left with the streaming engine.

```powershell
cargo test -p speakeasy-desktop --lib a_cuda_worker_reports -- --ignored
```

That one needs a CUDA-built worker staged at `target\debug\proof\`; it asserts
the `ggml-cuda` marker first and fails loudly rather than passing vacuously on a
processor build.

## What to report

Please record the Windows version, GPU model and driver version, the **device**
Settings reports dictation running on (and the provider line beneath it, if one
appears), whether model verification completed, whether a short dictation
produced text, and any error code shown in Settings. The device rather than the
pack: there is one Granite model file and a graphics-card worker offloads that
same file, so the pack reads `cpu` even on a machine holding the card. Exported
diagnostics are sanitized and should not contain audio, transcripts, tokens, or
credentials.
