# Changelog

## Unreleased

The first version of SpeakEasy Mini, forked from SpeakEasy and reduced to one
transcription engine and one HUD.

- **IBM Granite Speech is the only engine.** It runs once, after the recording
  stops, and the same pass produces the punctuation and casing. The streaming
  recognizer and everything under it — sherpa-onnx, ONNX Runtime, the Nemotron
  models, the `transcribe.cpp` canary, the live HUD text and the stabilizer —
  is removed, along with roughly 3 GB of downloads and a second native runtime.
- **No fallback.** A pass that fails, returns nothing, or fails the
  reference-free plausibility gate ends the dictation with a named reason
  instead of quietly substituting a weaker transcript.
- **The side dock is the only HUD.** The 420x280 transcriber window is gone, as
  is the setting that switched between the two presentations.
- **The transcript log is its own settings page**, and can be pinned into a
  small always-on-top window of its own. It is the only place a delivered
  transcript can be read back.
- **Transcripts are discarded when the app closes by default**, implemented by
  never writing them rather than deleting on exit. Choosing to keep them writes
  them to disk behind the existing plaintext disclosure.
- **The installer is the only setup path.** The in-app seven-step wizard is
  removed; setup probes the hardware, downloads, verifies, and proves the engine
  transcribes before the app launches.
- **Its own identity** — `SpeakEasy Mini` / `ai.speakeasy.mini`, `Ctrl+Alt+P` —
  so it installs and runs alongside SpeakEasy without sharing settings, logs, a
  single-instance lock, or a global shortcut.

- **A failed dictation says what to do about it.** Every failure code carries a
  one-line reason and real guidance, shown in the dock's action row and in a
  panel at the top of Settings → Transcription. A Windows toast was specified
  and refused: it needs an AppUserModelID from an installed Start Menu shortcut
  and otherwise displays nothing while reporting success.
- **ONNX Runtime's CUDA staging is gone** — a 2.97 GB on-demand fetch of cuDNN
  and cuFFT, its coordinator, three commands and a settings panel. llama.cpp
  needs cudart and cuBLAS, which the manifest keeps, and setup will fetch them
  with the GPU worker as the single artifact they physically are.

Removed with their subjects: the provider-override control (Granite's GPU
support is a build feature, and no setting can conjure a worker binary), the
GPU "qualified" promotion (nothing can execute on the GPU to earn it until a
CUDA worker ships), and the `immediate_repetitions` / `self_corrections`
cleanup rules (they were already disabled for Granite, which now produces every
transcript).
