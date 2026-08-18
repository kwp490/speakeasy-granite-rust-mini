# Source and build notice

SpeakEasy is built from this Rust/npm workspace with the exact dependency
locks in `Cargo.lock` and `package-lock.json`. It's an independent Rust
implementation and does not reuse the earlier Python (SpeakEasy AI Granite)
source or identity.

`speakeasy-bootstrapper.exe` — which installs SpeakEasy for the current user,
without elevation, in place of the NSIS installer it replaced — the inference
worker,
Granite worker (CPU build only), `onnxruntime.dll` and sherpa-onnx's native
runtime, notices, checksums, and install manifest are assembled by
`scripts/Build-LocalInstaller.ps1`. ASR model packs (including Granite's
GGUF files) and the CUDA runtime are fetched on demand after install, not
bundled.

This artifact is unsigned (no Authenticode code-signing key) and not
distributed publicly.
