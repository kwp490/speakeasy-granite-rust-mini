# Changelog

## 1.6.0 — 2026-08-26

Dictation can run on the graphics card, which is the feature this fork has been
building toward since it started, one data-loss defect found while releasing it,
and readable text in setup.

### Setup and the uninstaller are readable — 2026-08-26

Reported from use: the text in both windows was small and hard to read. It was —
every word of it was drawn at the size Windows uses for **menu bars**, on lines
of about 105 characters, with the page heading, the step counter and the body all
the same size as each other.

Body text is now a third larger and the heading is larger again and semibold, so
a page has a shape before it has been read. That applies to everything with words
in it, buttons and check-box labels included, and both windows grew to hold it.

Two things worth saying about how it is sized. It is a **ratio** of your own
Windows text size rather than a fixed point size, so raising Windows'
Accessibility → Text size raises this too instead of overriding it. And it
inherits the face your system actually uses, so a Windows installed in a language
that ships a different UI font gets that font, larger.

### Dictation on the graphics card — 2026-08-26

Setup can install the CUDA build of the speech engine. On a machine with a
supported NVIDIA card it is the offered default; on one without, the option is
shown disabled with the reason rather than hidden, which it has always been.

**About nine times faster.** Measured on an RTX 4070 Laptop GPU over a real
105-second dictation: 4.2 s from finishing speaking to the text arriving, against
44.5 s on the processor for a comparable clip. The transcript is byte-identical
on both devices, which matters more than the speed — setup's engine check
compares a whole transcript against one pinned ground truth, so a graphics-card
path that moved a single punctuation mark would have failed on every card.

- **Choosing the processor is honoured.** Nothing is downloaded for a
  configuration you did not ask for. This was not true when the work started: the
  download plan read the machine's *capability* rather than the answer, which is
  the same value for exactly as long as the option stays disabled — so publishing
  the worker would have turned the provider page into a control that decided
  nothing.
- **A graphics-card install downloads 2.5 GB** rather than 2.1 GB, as setup
  itself reports it. The
  extra is the CUDA worker and the two NVIDIA libraries it loads; the model file
  is the same either way, because the CUDA worker offloads that same file. Every
  file is verified against a SHA-256 digest pinned in the repository, and an
  interrupted download resumes rather than restarting.
- **The engine is published with its notices.** It carries llama.cpp/ggml and a
  statically linked CUDA runtime, and the MIT texts travel with it.
- **The installed configuration is still recorded from proof.** It takes a
  complete payload, a worker that reported a CUDA backend at start-up, and the
  driver placing that worker's own process on a device. Asking for the graphics
  card is not evidence of getting it; a machine whose driver refuses records the
  processor and says so.
- **An upgrade no longer silently reverts it.** Placing the payload overwrites
  the worker with the processor build, so the graphics-card engine is re-staged
  afterwards, every time. Staging it by hand previously did not survive a
  reinstall and nothing said so.

### `--keep-user-data` deleted user data — 2026-08-26

`speakeasy-bootstrapper --uninstall --keep-user-data`, run **without**
`--silent`, presented the confirmation page with every box already ticked. A
command whose name says keep the profile opened on the answer that deletes it,
and accepting that page removed the settings, the vocabulary, the transcript
history and the downloaded models.

The flag was computed and then discarded on the interactive path: the page took
no argument and hardcoded every box checked. It was only ever honoured alongside
`--silent`, which is the combination this project's own test scripts pass, so
nothing had exercised the other one. The page now opens on what the caller asked
for, and still decides — any box can be ticked back on.

It cost a real profile before it was found: 4.28 GB of models, a settings tree
and a vocabulary, none of it recoverable. **If you are on 1.5.x and intend to
uninstall, upgrade to 1.6.0 first** and uninstall from this build.

## 1.5.1 — 2026-08-25

A single defect, reported from use, and it was the worst kind this product can
have: it destroyed the user's words and told them nothing. 1.5.0 ships it, so
anyone dictating for two minutes on that build loses the recording.

### A two-minute dictation was destroyed every time — 2026-08-25

Reported from use: a long dictation errored, a short one immediately after
worked. It was two defects that only showed together.

- **The safety ceiling worked perfectly and the recording was thrown away
  afterwards.** Capture stopped at 120.2 s, exactly as designed. What followed
  discarded it: a finished capture reported six possible conditions and all six
  were treated as failures, when only one — "no audio frames at all" — actually
  means there is nothing to transcribe. The other five describe audio that
  exists and transcribes fine. They are now delivered with a warning.
- **The audio buffer's byte limit ran out before the ceiling did.** Each second
  of retained audio costs more than the raw samples, and the limit worked out at
  116.5 s against a 120 s ceiling — so every maximum-length recording overran
  its buffer, lost its last few seconds, and (per the defect above) was
  destroyed. That is why long recordings failed *every* time while short ones
  never did. The limit is now comfortably clear of the ceiling, and a
  full-length recording is captured whole.
- **The error said nothing.** Four of the five conditions had no message at all
  and displayed as "The operation stopped safely". The diagnostic log recorded
  only `result=no_audio`, without the code it already had in hand. All five now
  have copy that says the transcript was delivered and what may be missing, and
  the log names the condition.
- **Reaching the limit now tells you.** The stop cue sounds — it previously
  stayed silent on exactly the ending the user did not ask for — and a notice
  appears saying the recording stopped at the two-minute maximum, that the
  transcript was delivered, and that anything said afterwards was not recorded.

### Toolchain — 2026-08-25

- **Rust 1.98.0.** Moved from the pinned 1.97.1 after Malwarebytes began
  quarantining that toolchain's `clippy-driver.exe` and `rustdoc.exe` as a
  generic AI detection, which silently broke the quality gate. Byte-identical
  copies of the same files in another toolchain were untouched, and the 1.98.0
  build is unaffected. One new lint (`chunks_exact_to_as_chunks`) applied across
  four PCM decoders.

## 1.5.0 — 2026-08-21

Three sessions of work behind 1.4.2, and one theme runs through all of it: the
product had several places where it stated something its evidence did not
support, and every one of them read as a fact. A record written from a radio
button, a sentence about a device assembled from a fact about a model, a driver
query that failed being reported as a driver that said no, and an uninstall that
announced a removal while leaving 2.14 GB behind.

Still processor-only on every machine: no graphics-card worker is published, so
setup installs the processor configuration and says so.

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
- **It asks once first, on one page**, with a check box per category, every box
  checked, and anything in the program folder that setup did not place there
  listed separately. The page is the confirmation — nothing follows it — and its
  Remove button is the focused one. The downloaded models are the one entry that
  names how much space they are using, measured rather than written down.
- **`--keep-user-data` is the opt-out**, and is a testing affordance for repeated
  install/uninstall cycles rather than a user-facing choice. `--remove-all` is
  refused rather than silently accepted, because it named the behaviour that is
  now the default.
- **An uninstall no longer leaves an empty registry key with the product's name
  on it.** Found by running the production default against a real installation
  for the first time rather than a staged one: everything else was gone and
  `Software\SpeakEasy Mini` was still there, because the version stamp lives in a
  subkey. The parent is now removed too, and only when it is empty.
- **Files in the program folder that setup did not install are no longer spared.**
  They were, on the reasoning that an unrecognised file there was probably a
  large on-demand download — a download this product has not had since the
  streaming engine was removed.

### Two claims the app could not support — 2026-08-21

- **The engine line no longer contradicts itself.** On a machine with a
  graphics-card engine, Settings read `Dictation runs on: Graphics card (GPU) —
  ... so the processor model is being used.` The device was right and the reason
  was right; the sentence was false. The reason is now its own sentence and every
  one of them describes what the installation includes rather than what is
  running, so the two halves can no longer disagree.
- **A driver that will not answer is no longer reported as a fault.** Where the
  installation was recorded as using the graphics card and this run could not be
  checked, the app said dictation was "running on the processor instead" — a
  claim about a device on evidence that establishes none. That case now has its
  own disclosure saying what is actually known, and only a definitive negative is
  reported as something to act on.

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
