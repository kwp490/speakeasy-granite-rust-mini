# Changelog

## Unreleased

Work that landed after 1.4.2 was tagged. The version number has deliberately not
moved: it is what the binary reports, what the registry stamp records and what
the install manifest carries, and `scripts/Increment-ProductVersion.ps1` moves
all three together when a release is actually cut.

### Setup can no longer record a configuration it did not prove — 2026-08-20

- **The provider is proved, never chosen.** A support log read
  `engine=cpu_gpu_runtime_missing device=cpu installed=cuda`: three correct
  fields, an impossible combination, and nothing anywhere that compared them.
  The installed-configuration record came from the provider page's radio button,
  three pages before anything ran. It is now written from the engine check's
  verdict and nowhere else, and requires all three of a published CUDA worker, a
  complete payload on disk, and NVML placing that worker's own process on a
  device.
- **The graphics-card option is disabled when it cannot be installed**, with the
  reason named. `UI-GUIDE.md` had said so since the page was designed; nothing
  ever did it.
- **The app compares the record against reality at every warm** and says so when
  they disagree, rather than reporting the installation it was told about.
  Dictation is never refused for it — the same model file produces the same
  transcript on either device, so refusing would cost the user their dictation to
  make a point about provisioning.

### The graphics-card path, proved on hardware — 2026-08-21

- **CUDA moved to 13.x.** The catalog pinned 12.9 while every machine that can
  build a CUDA worker ships 13.x. Harmless while nothing read the list, and a
  refusal the day something did. Note that CUDA 13 moved its libraries from
  `bin/` to `bin/x64/`, so this was not the old paths with a digit changed.
  `scripts/Get-CudaRuntime.ps1` now produces the pinned entries and proves they
  are byte-identical to the installed toolkit.
- **`--verify-provider`** re-runs the engine check against an installed build and
  rewrites the recorded configuration from its verdict, so re-proving no longer
  costs a reinstall. The app's own diagnostic copy now points at it instead of
  recommending a reinstall.
- **All five provider states have been produced on real hardware** (RTX 4070
  Laptop GPU), including the two that a working card cannot be asked to produce
  and so are driven from a staged driver probe.
- Measured on that card, on a 6.42 s clip: resident pass 2,928 ms on the
  processor against 361 ms on CUDA. The transcript is byte-identical on both,
  which matters more than the speed — the installer's engine check compares a
  whole transcript against one pinned ground truth.
- **Nothing is published.** A graphics-card worker is still not in the trusted
  manifest, so a release still installs the processor configuration on every
  machine and says so.

### Uninstall leaves nothing — 2026-08-21

- **An uninstall now removes everything by default**: the program directory
  whole, and the settings, transcript history, downloaded models, recovery
  backups and diagnostic log with it — directories included, not just their
  contents. It kept all of that by default before, inherited from the NSIS
  installer it replaced, so a user who asked the product to go was left with
  2.14 GB of models and a settings tree and told it had been removed.
- **It asks once first**, with every category named and with anything in the
  program folder that setup did not place there listed separately. The focused
  button is No.
- **`--keep-user-data` is the opt-out**, and is a testing affordance for repeated
  install/uninstall cycles rather than a user-facing choice. `--remove-all` is
  refused rather than silently accepted, because it named the behaviour that is
  now the default.
- **Files in the program folder that setup did not install are no longer spared.**
  They were, on the reasoning that an unrecognised file there was probably a
  large on-demand download — a download this product has not had since the
  streaming engine was removed.

## 1.4.2 — 2026-08-19

The first release of SpeakEasy Mini, forked from SpeakEasy and reduced to one
transcription engine and one HUD.

The version number is inherited from the parent product rather than restarted,
because it is what the binary reports, what the registry stamp records and what
the install manifest carries. A release tag that disagreed with all three would
be the more confusing of the two options.

### Setup, completed

Everything below landed on 2026-08-19 and is what makes a fresh machine a
working one.

- **Setup is one downloadable file.** `SpeakEasyMiniSetup.exe` carries the app
  and the engine appended to its own image, past the end of the PE data Windows'
  loader reads. It used to need a `payload\` directory beside it, which is fine
  for a developer and installs nothing for anybody else. Every appended file is
  checked against a SHA-256 before it is written — not against tampering, but
  because a truncated download still *runs*, since the missing part is not part
  of the program.
- **Setup finishes by starting the app.** It previously closed its own window,
  while the README described a step that launched it.
- **Three placeholder steps are built.** Choosing the configuration, choosing
  the shortcut, and adding protected words all rendered "this step is not built
  yet". A fourth step, the retention question, is new.
- **The answers now reach the app.** The desktop side had read
  `install-hotkey.txt` and `install-logging.txt` since before the fork and
  nothing had ever written them, so the shortcut and the logging choice were
  collected and discarded. One writer now records the shortcut, the logging
  choice, the retention answer, the protected words and the installed
  configuration, and the app consumes all five.
- **The shortcut is verified by taking it.** Setup registers the chosen
  combination and releases it, because Windows will not say who owns a global
  hotkey and the alternative is the user discovering the conflict by pressing
  the key and watching nothing happen.
- **The installed configuration is recorded**, so the app can tell "running on
  the processor because that is what was installed" from "running on the
  processor because the graphics-card engine will not load". Those were the same
  silent state; the distinction now appears as `installed=` in the
  `granite_warm` log line.
- **The single-file installer is proven, not assumed.**
  `Test-InstallerLifecycle.ps1` installs from the embedded payload into its own
  ephemeral root and compares every placed file against the packaged one by
  hash. The path that ships was the only one in the chain nothing exercised.

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
  a pinned SHA-256, installs, and then transcribes a bundled clip through the
  real engine and compares the result word for word before it launches the app.
  That last step is the one that matters: a speech model whose audio projector
  failed to attach does not error, it writes fluent text without listening to
  anything, so only matching content is evidence.

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
