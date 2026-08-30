# Source and build notice

SpeakEasy Mini is built from this Rust/npm workspace with the exact dependency
locks in `Cargo.lock` and `package-lock.json`. It's an independent Rust
implementation and does not reuse the earlier Python (SpeakEasy AI Granite)
source or identity.

The installer payload is three executables: `ai-speakeasy-mini.exe`,
`speakeasy-bootstrapper.exe` — which installs SpeakEasy Mini for the current
user, without elevation, in place of the NSIS installer it replaced — and the
CPU-built `granite-worker.exe`. Those, the notices, checksums and install
manifest are assembled by `scripts/Build-LocalInstaller.ps1`.

It used to carry a streaming inference worker, `onnxruntime.dll` and
sherpa-onnx's native runtime beside them; all three left with the streaming
engine. `speakeasy-granite` compiles llama.cpp directly into the worker, so
`granite-worker.exe` needs no native library beside it on a CPU install.

Granite's GGUF model files are fetched during setup and verified by digest, not
bundled — on a fresh install, or on any install without an already verified copy
on disk. A user who chooses the graphics-card configuration also fetches a
CUDA-built `granite-worker.exe` and the two NVIDIA CUDA redistributables it needs,
likewise verified by digest and likewise not bundled.
`packaging/THIRD-PARTY-NOTICES.txt` lists what is embedded and what is downloaded,
separately.

This artifact is unsigned (no Authenticode code-signing key) and may trigger
Windows SmartScreen. It is publicly distributed, from the project's GitHub
Releases page, with a `SHA256SUMS` file beside it.
