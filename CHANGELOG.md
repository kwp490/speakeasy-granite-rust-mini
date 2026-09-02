# Changelog

## 1.9.0 — 2026-09-02

Settings now uses a restrained desktop workspace across all six pages: a compact
product header, a stable navigation rail, page introductions, predictable group
spacing, full labeled choice rows, and readable definition lists in place of the
Advanced tile wall. Runtime summaries use human-facing names by default while
the exact identifiers remain available under **Show raw values**.

Below 760 px the rail becomes a fixed 3-by-2 tab grid, and below 520 px it becomes
2-by-3. The same vertical-tab keyboard behavior, one-panel rendering,
mount-on-activation rules, backend commands, privacy behavior, and failure copy
remain unchanged. Layout and contrast proofs now cover Transcript log as well as
the other five pages.

## 1.8.1 — 2026-08-30

With persisted history enabled, a transcript delivered into a password field was
stored anyway — the exclusion that was supposed to stop it could never fire. A
history that failed to save also took the transcript down with it. Plus an
engine that could warm up as the wrong model, a launch that read 6.90 GB before
the app was usable, and a notice that announced a delivery before the
transcription had started.

### A transcript delivered to a protected target was stored — 2026-08-29

**If you have persisted history enabled, review it.** The rule that keeps a
password field's transcript out of history was never able to refuse: the row was
written with the protected-target flag set to a fixed `false`, and it was written
*before* anything had looked at where the text went. The guard behind it was
real, and nothing had ever reached it.

The order is now a data dependency rather than a convention — the history row
cannot be built until the delivery has been classified — so the flag describes
the delivery it belongs to. Persisted history remains **off by default**, and
that remains the only setting that holds on every path.

### A failed save no longer costs you the transcript — 2026-08-29

Writing the history row was also the first thing that happened, ahead of the
transcript reaching the screen, so any database error discarded a good
transcript and left the dock stuck on *finalizing* for the rest of the session.
The transcript is published first now and a storage failure is reported without
touching it. Deleting saved history likewise removes only what came from that
history and leaves what the current run produced.

### The engine can no longer warm up as the wrong model — 2026-08-29

Several faults in how the app decides a model is ready and which one is loaded.
A warm-up that had finished could still report itself as verifying; a second
warm-up could settle using the first one's verdict; and a model already in
memory could be handed back while a *different* pack was marked verified on
digests nobody had taken for it. A model in memory that does not match the one
being asked for is now refused with a named reason instead of transcribing with
the wrong adapter.

### The shortcut refuses what the dock refuses — 2026-08-29

The dock's Start button and the keyboard shortcut disagreed about when a
dictation could begin, and the shortcut was the permissive one — so a machine
below the 8 GB memory floor had a disabled button and a working hotkey, and
would record for two minutes before reporting that the engine could not start.
Both go through one rule now, applied before any audio is captured.

### Startup stopped reading the model three times — 2026-08-29

A configured launch verified the same 2.30 GB of weights on three separate
paths — about 6.90 GB of reading before the app was usable, on a product whose
advertised floor is 8 GB of memory. It is read once now, immediately before the
worker is handed it.

### The recording-limit notice no longer claims a delivery — 2026-08-29

When a dictation hit the length ceiling, the notice said the transcript had been
delivered — and it said so *before* transcription began. The pass that followed
could still find no speech, time out, or be refused. On a processor install that
claim could arrive up to 44 seconds before any text landed.

### Settings tells you when an action was refused — 2026-08-29

Ten more Settings controls can report a rejection, four of which previously had
no failure handling at all: the control would look as though the change had been
made. Every control now updates only after the change actually succeeded, and
refuses a second submission while one is still in flight.

### The transcript log updates when something changes — 2026-08-29

The pinned log polled on a timer; it is driven by the change itself now, which
is quicker and quieter. The recent-transcripts list also stops describing itself
as covering only the current session — with history enabled it spans earlier
runs, which is what it always did.

### The notices describe what is actually shipped — 2026-08-30

The privacy, security, distribution and third-party notices were checked against
the real payload and the real behaviour. The distribution notes described a
product that had not been distributed, and the privacy note promised more about
delivery than the app can observe: a transcript is excluded from history only
when a delivery was attempted and classified, so with automatic pasting turned
off there is no target to judge and the transcript may be retained. That
limitation is now stated beside the protection instead of left out.

## 1.8.0 — 2026-08-28

The dock can start a dictation, not only end one, and it has a settings button.

### The button is always there, and it starts as well as stops — 2026-08-28

The dock's button used to appear only while you were already recording, so the
one surface whose whole point is staying reachable while you work in another
window had no way to *begin* anything. It is now present in every state and its
label is the state: **Ready**, **Stop**, or three working dots while the
transcript is being made.

It says `Ready` only when it really is. While the model is loading, or setup is
unfinished, or a transcript is still being produced, the dots show and the button
is inert — a permanently visible button is a louder place to claim readiness than
the small chip above it, and claiming it early was a defect fixed one row up in
1.7.0.

### A settings button on the dock — 2026-08-28

A gear beside the close button opens Settings. It was already on the dock's
right-click menu, which is only findable by someone who thought to try
right-clicking a window that never takes keyboard focus.

### A taller dock, and the waveform got the space — 2026-08-28

The window went from 360 to 400 to pay for the permanent button. The waveform is
the only row that stretches, so it took all 40 pixels and went from 112 to 152 —
larger than it was before the device indicator was added at all.

### The device indicator moved below the waveform — 2026-08-28

Reported from use. Between the wordmark and the waveform it cut across the
column, the dotted meter read as hanging off it, and the brightest thing on the
dock sat in the middle while the bottom third was empty. It now sits under the
waveform with the elapsed time and the button, so the top of the dock is just
the name and the bottom is what is happening.

The elapsed time and the "how it ended" mark now share one row, since the button
occupies the row the mark used to sit in. Neither is ever wanted at the same
moment as the other.

## 1.7.1 — 2026-08-28

Settings told graphics-card users they were running on the processor. Plus three
things the dock and setup were getting wrong, and the words you protect now
survive being misheard.

> **1.7.0 was never released.** It was built and installed here, the install was
> tested, and the testing found the Settings defects below — so the number moved
> rather than shipping two different binaries as one version. Everything 1.7.0
> was going to carry is in this release.

### Settings said "Processor (CPU)" while dictation ran on the card — 2026-08-28

Reported from use, on a machine that really was using its graphics card. Two
separate fields were wrong and one sentence could never be right.

**Advanced → PROVIDER** showed the processor on every machine. It was reporting
which *model file* was selected rather than which device was running it, and
there is only one model file — it is named for the processor and the graphics card
runs that same file. It reports the real device now.

**Advanced was also frozen at the moment the window was created**, which on a
graphics-card machine meant it reported the engine as unavailable for as long as
the app stayed open. The page now reads fresh figures each time you open it, so
the speed measurements on it stop being stuck too.

**"The graphics-card engine … has not passed its local execution check yet"** is
gone. That sentence could only ever say "not yet", however well the card was
working, and it sat directly beneath a line saying dictation was running on the
card. The two lines above it answer the same question and are checked against the
graphics driver, so nothing was lost. The **Re-test graphics-card engine** button
stays and still does what it says.

Nothing about dictation, speed or transcripts changed here — only what the app
told you about itself. If you looked at Settings on a graphics-card machine and
concluded you were on the processor, that was this.

### The dock tells you the truth about being ready — 2026-08-28

It said **ready** for the whole two-gigabyte model load. Pressing the hotkey in
that window did nothing useful, and there was no way to tell that from a dock
that had genuinely finished warming up. It now reports loading while it loads.

### The dock says which device dictation runs on — 2026-08-28

Processor or graphics card, and it never guesses: a graphics-card engine whose
context could not be confirmed says so rather than claiming the card. Until now
the only place this appeared was the diagnostic log.

### "Return to default HUD" now does something — 2026-08-28

The dock's right-click menu carried the item since this fork began and clicking it
did nothing at all — silently, because the id it was built with was not the id
anything handled.

### Protected words survive being misheard — 2026-08-28

A protected compound like `LogicMonitor` is now also corrected from the two-word
form the recogniser actually produces, and two mishearings that no rule predicts
are corrected by name. Three more were **refused** on purpose, because they
rewrite words somebody might legitimately say.

One defect went with it: a term that can be misheard two different ways generated
the same internal id twice, and a duplicate id rejected the **whole** batch — so
a single collision cost you every protected word, not one.

### Setup's words page arrives filled in — 2026-08-28

It was an empty box, so the feature reached only people who already knew what it
was for. An install that skips the wizard entirely seeds the same defaults, and
neither path ever replaces words you have already saved.

### Also

A wizard-less install no longer inherits the previous install's record of which
engine was installed — that record is now written only from a check that actually
ran. Nothing else here changes dictation.

## 1.6.1 — 2026-08-26

One report from use, about the first thing anybody sees.

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

Nothing about dictation changed. **1.6.0 is otherwise identical**, so the only
reason to take this one is that you have to read setup to install it.

## 1.6.0 — 2026-08-26

Dictation can run on the graphics card, which is the feature this fork has been
building toward since it started, and one data-loss defect found while releasing
it.

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
