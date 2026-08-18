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
  removed; setup probes the hardware, downloads and verifies every file against
  a pinned SHA-256, and installs. The engine smoke test that would prove the
  model actually transcribes before launch is specified but **not yet built** —
  its clip and verified ground truth are committed, the runner is not.

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

### Fixed after the fork, once anything was actually run

The fork updated every path it executed and left every path it did not. Nothing
below was covered by a test, because these *are* the tests.

- **The app could not be launched, and no dictation could have succeeded.** The
  dev staging script built a crate the fork deleted, and
  `RuntimeWizardCoordinator::paths()` required three deleted binaries before the
  Granite worker — so it failed on every call and every dictation would have
  ended in `GraniteUnavailable`.
- **Setup would have installed over SpeakEasy and, on uninstall, deleted it.**
  The install root, Start Menu folder, `%APPDATA%` identifier, Add/Remove
  Programs key and version stamp were all inherited from the parent product.
- **Add/Remove Programs showed the wrong product**, and 22 of 23 user-facing
  strings in the setup wizard named SpeakEasy rather than SpeakEasy Mini.
- **The quality gate had not been runnable since the fork**, so every "green"
  claim came from running its sub-commands by hand; the installer could not be
  built at all; and the installer lifecycle proof's refusal assertions could
  never have passed, because PowerShell turned each refusal's stderr into a
  terminating error before its exit code could be read.
- **A machine with 4–8 GiB could start a dictation that could not finish.** The
  dictation floor is now Granite's floor, so it is refused before recording
  rather than after the user has spoken.
