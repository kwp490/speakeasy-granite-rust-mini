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

Granite's GGUF model files and the CUDA runtime are fetched on demand after
install and verified by digest, not bundled.

This artifact is unsigned (no Authenticode code-signing key) and not
distributed publicly.
