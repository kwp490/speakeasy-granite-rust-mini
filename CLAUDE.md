# SpeakEasy Mini — working notes for Claude

A local-only Windows dictation app: Rust + Tauri 2, React frontend, all
inference on-device. Press a hotkey, speak, press again, the transcript is
pasted into whatever has focus.

This file is the orientation a new session needs before touching anything. It
is deliberately short and points at the living docs rather than restating
them. What it *does* carry are the things that are expensive to rediscover:
traps that fail silently, decisions that look wrong until you know why, and
the handful of measurements that settle recurring arguments.

## What this is, and what it is not

SpeakEasy Mini is a fork of SpeakEasy that removes two things and keeps
everything else:

- **No real-time transcription.** The streaming engine — sherpa-onnx, ONNX
  Runtime, the Nemotron models, the `transcribe.cpp` canary, the live HUD text
  and the whole stabilizer — is gone. Nothing appears on screen while you
  speak. **IBM Granite Speech is the only engine**, it runs once after the
  recording stops, and that single pass produces the punctuation and casing
  too. There is no second pass and no fallback.
- **No large HUD.** The 420×280 transcriber window is gone. The narrow side
  dock is the only HUD, and every setting lives in the settings window reached
  by right-clicking it.

It ships under its own identity — `SpeakEasy Mini` / `ai.speakeasy.mini`, with
`Ctrl+Alt+P` — so it installs and runs alongside SpeakEasy without sharing
settings, logs, a single-instance lock, or a global shortcut.

## Read first

| Doc | What it is |
| --- | --- |
| `docs/ARCHITECTURE.md` | How the pieces fit, and what is implemented vs not |
| `docs/UI-GUIDE.md` | **The living UI spec.** Code must match it; amend it in the same change |
| `docs/LOCAL-DEVELOPMENT.md` | Version bumps, installer, the local-only policy |
| `docs/NEW-MACHINE.md` | Bootstrapping a fresh dev machine |
| `docs/handoff/CURRENT.md` | **What is unfinished and why.** Read this before picking up work |
| `docs/design/` | Visual elevations, one standalone HTML each |

## Commands

Cargo and rustc are **not on PATH**. Dot-source the environment first, in every
new shell — neither the Bash nor the PowerShell tool inherits it:

```powershell
. .\scripts\Enter-DevEnvironment.ps1
```

The whole gate (fmt, clippy `-D warnings`, workspace tests, dependency policy,
frontend test/lint/typecheck) does its own env setup:

```powershell
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
```

**`speakeasy-granite` compiles, and the gate runs.** Both were open questions
until 2026-08-18: the crate had not been built since the fork, and the gate
itself threw on its own second step, so every "green" claim came from running
sub-commands by hand. `cargo build --release -p speakeasy-granite-worker` takes
about two minutes cold. Two prerequisites are easy to miss —
`git config --global core.longpaths true` (the llama.cpp fetch exceeds
`MAX_PATH` without it) and CMake plus libclang, which
`Enter-DevEnvironment.ps1` warns about rather than fails on.

**Run the gate, not `cargo test --workspace --lib`.** `--lib` builds no `--bin`
targets, so it silently skips the bootstrapper's binary tests — one of which had
been failing since the fork without anyone seeing it. That is the recorded
"a whole crate went red unnoticed" lesson one level down: a target filter rather
than a crate list.

Frontend-only, from `apps/desktop`: `npm run typecheck`, `npm run lint`,
`npm test`, `npm run build`. Rust: `cargo test -p <crate> --lib`,
`cargo clippy --all-targets`.

Re-pinning NVIDIA's CUDA redistributables in `models/trusted-manifest.json`,
which is the enforced requirement list rather than documentation:

```powershell
.\scripts\Get-CudaRuntime.ps1
```

Re-proving which provider an installed build runs on, without a reinstall:

```powershell
& "$env:LOCALAPPDATA\SpeakEasy Mini\speakeasy-bootstrapper.exe" --verify-provider
```

Building and proving the installer, which is a separate path the gate does not
touch:

```powershell
.\scripts\Build-LocalInstaller.ps1
.\scripts\Test-InstallerLifecycle.ps1 -ArtifactRoot 'target\local-development\<version>'
.\scripts\Test-SetupWizard.ps1 -ArtifactRoot 'target\local-development\<version>' -Uninstall
```

The build produces `SpeakEasyMiniSetup.exe` — one file, the payload appended to
the bootstrapper past the end of its PE image — plus the bare bootstrapper and
the `payload\` directory beside it, which is what the lifecycle test drives.
`Test-SetupWizard.ps1` drives the file a user actually downloads, through all
eight wizard pages to a launched app, and **asserts the page it is on before
every click**: a driver that presses Next eight times passes on a wizard stuck
on page one. It installs for real and leaves the app running unless you pass
`-Uninstall`.

Kill any `ai-speakeasy-mini` first. An aborted lifecycle run leaves the app it
launched for the running-app check alive, and the pre-flight guard then refuses
every retry — correctly, it will not terminate a process it does not own, but
the orphan is the script's own.

Most of the workspace is fast now. `speakeasy-worker` links no native
libraries at all and checks in seconds; only `speakeasy-granite` compiles C++.

## Running the app

`npm run tauri -- dev` from `apps/desktop` stages the native runtime first
(via `beforeDevCommand` → `scripts/Stage-DevRuntime.ps1`) and then launches.

Three things about that will waste your afternoon if you do not know them:

1. **Any already-running SpeakEasy Mini silently absorbs a dev launch.** The
   app uses `tauri_plugin_single_instance`, so the new process exits, Vite
   comes down with it, and you then test the *old* binary while everything
   looks like a clean start. Check `Get-Process SpeakEasy*` first. You have
   standing permission to stop and start SpeakEasy Mini's own processes for
   testing without asking. SpeakEasy and SpeakEasy Mini have different
   identifiers so they do *not* absorb each other — but that process filter
   matches both, which is the point of using it.
2. **Dev is not the installed build, and the difference has invalidated
   measurements twice.** A debug build's SHA-256 dominates any timing that
   verifies a model — the same rig reported a 17.5 s saving in debug and 2.36 s
   in release. Time things in release, on an installed build. The *resident*
   timing is the exception worth knowing: it is almost entirely the release
   worker's own inference, so it survives a debug harness where the cold number
   does not. Measured 2026-08-21 on an RTX 4070 Laptop GPU over a 6.42 s clip —
   2,928 ms on the processor against 361 ms on CUDA, and the transcript
   byte-identical on both.
3. **Trust the disk log over the UI and over proof scrapers.**
   `%APPDATA%\ai.speakeasy.mini\logs\speakeasy.log` carries a specific error
   code where the UI often shows generic text.

Driving the running app by selector, for measuring real layout:

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
.\scripts\Invoke-WebviewProbe.ps1 -Window settings -Expression '...'
```

`-Window` takes `settings`, `dock`, `log` or `notice`. A window declared
`visible: false` still runs its React tree, so all four answer without being
shown — which is how the notice window is measured without provoking a
two-minute dictation.

**A release frontend needs `--features custom-protocol`, and without it the
binary looks broken rather than misconfigured.** `cargo build --release -p
speakeasy-desktop` alone embeds `devUrl`, so every window loads
`http://localhost:1420`; with no Vite server running they all come up blank and
the probe reports each one absent, which reads exactly like the app failing to
start. Measured 2026-08-26: four CDP page targets, all at `localhost:1420`, no
`data-testid` anywhere. With the feature they load `tauri.localhost` and carry
the bundled `dist/`. This matters because **the startup race is only reproducible
on a release frontend** — Vite is slow enough that `setup` always wins under
`tauri dev`.

```powershell
cargo build --release -p speakeasy-desktop --features custom-protocol
```

## Failures are the product

There is one engine and nothing to fall back to, so "it didn't work" is a
first-class outcome rather than an edge case, and the app's honesty about it is
most of what it offers over a silent degradation.

- **`judge_granite_pass` decides**, in `speakeasy-worker`'s `verdict.rs`. It
  returns either a transcript to paste or a `FinalSourceReason` naming why not.
  Exactly one of the two, by construction.
- **`is_plausible` is the only guard left.** It catches Granite answering the
  prompt instead of transcribing — a real failure mode for an instruction model
  with an audio projector, and one that produces fluent, confident, entirely
  invented text. There used to be a second instrument comparing against the
  streaming transcript; there is no second transcript now, so what this misses
  reaches the user.
- **The gate is one-sided.** It bounds how much a transcript may say, never how
  little. A *truncated* transcript is perfectly plausible and nothing catches
  it — see the `max_new_tokens` trap below.
- **Every reason code needs a real instruction.** A code that falls through to
  `errorUnknown` fails the rule that errors report a specific, actionable
  condition in the words used for everything else. Failures surface in two
  places: the dock's action row, and a panel at the top of Settings →
  Transcription carrying the reason and the fix. A Windows toast was specified
  and rejected — the WinRT route needs an AUMID from an installed Start Menu
  shortcut and otherwise displays nothing while reporting success, which is the
  silent-success shape most of this file exists to avoid. `docs/UI-GUIDE.md`
  records the trade.
- **A CPU install running on CPU is normal; a GPU install that cannot load
  CUDA is an error.** Setup records which configuration it installed precisely
  so those two can be told apart — they were previously the same silent
  outcome. The record is **proof**, not a preference: see the trap below.

## Traps that fail silently

Every one of these produced a plausible, wrong result rather than an error.

- **A window created on demand from a `#[tauri::command]` deadlocks the whole
  app's IPC** — every window, every command, until the process is killed.
  Declare windows statically in `tauri.conf.json` and only ever call
  `get_webview_window`. All three windows (`main`, `hud-dock`, `log`) exist
  from launch; commands only show and hide them.
- **Those statically declared windows load before `setup` has managed the
  coordinators**, so any command a window fires on mount can arrive while its
  state is absent. `app.state::<T>()` **panics** there, inside a WebView
  callback that cannot unwind, and the process aborts — `0xc0000409`. Use
  `try_state` and return an error. This is structurally invisible to
  `tauri dev`: Vite is slow enough that `setup` wins the race, and a bundled
  release frontend is not. A `visible: false` window still runs its React tree
  and still polls.
- **Anything SpeakEasy Mini puts in the foreground becomes the delivery
  target.** `deliver_final_text` inspects the foreground window to decide where
  the transcript goes, so any window of its own that lands there silently
  hijacks a dictation — it does not error, it refuses with
  `target_inspect_refused` and falls back to the clipboard, which looks like a
  delivery bug in some other subsystem. Three separate causes have done this: a
  **hidden** window that declared `visible: false` but not `focus: false` held
  the foreground indefinitely; a release binary with no `windows_subsystem`
  attribute was a console app and its terminal window held it; and once that
  console was gone the console-subsystem workers each got their own window, so
  they need `CREATE_NO_WINDOW`. Every window here declares `focus: false` and is
  made non-focusable at startup, and a scaffold test asserts it. Any new window
  or spawned process is a candidate — check the foreground after launch, not
  just that the app looks right.

  **The mirror image of this has no error code at all**, and it is the common
  case on a CPU install. The foreground is read when the dictation *finishes*, so
  the exposure is however long inference takes: measured 2026-08-25, **4.2 s on
  the card and 44.5 s on the processor** for a full-length dictation. Any window
  the user moves to inside that gap receives their transcript, with
  `integrity=Equal result=committed` and nothing in the log to distinguish it
  from a correct delivery. It caught the session that was measuring it — the
  owner stopped speaking, switched windows to report back, and both processor
  transcripts landed there rather than in the prepared target. On the processor
  44 s is long enough that moving on is the *reasonable* thing to do, so treat a
  "delivery went to the wrong application" report as a timing question first.
- **A staged CUDA worker does not survive `npm run tauri -- dev`.**
  `beforeDevCommand` runs `Stage-DevRuntime.ps1`, which copies the CPU worker
  over `target/debug/proof/granite-worker.exe` — so a dev launch silently reverts
  a worker staged there and the app then reports `device=cpu`, which reads as a
  regression in whatever you were about to test. Measured 2026-08-21: 57,042,432
  bytes became 4,333,568. It also breaks the three `granite_engine` hardware
  tests, which read that same path, without failing them. Restore the worker and
  start `npm run dev` and `target/debug/ai-speakeasy-mini.exe` separately when
  the graphics-card path is what you are looking at. This is the same shape as
  `Enable-GraniteCuda.ps1` reverting on reinstall, one directory over — and it is
  the half that survived: setup re-stages the worker on every install now, so the
  installed build no longer reverts, but `Stage-DevRuntime.ps1` still overwrites
  the dev one.
- **The three CUDA libraries fail in three different ways, and one of them never
  loads at all.** Measured 2026-08-21 with the toolkit stripped from `PATH`,
  which is the only way to ask the question on a machine that can build the
  worker: deleting `cublas64_13.dll` stops the process before `main`, deleting
  `cublasLt64_13.dll` lets it start, load two gigabytes of weights and fail
  **~36 s later** at the first matmul with `AdapterFailed`, and deleting
  `cudart64_13.dll` does nothing at all — the worker transcribes and NVML
  confirms the context, because ggml links the CUDA runtime statically on
  Windows. So the catalog's requirement list is a deliberate superset, and the
  late failure is why the check is a *precondition* rather than something
  inferred from a worker that came up.
- **Never default llama.cpp threads to the logical processor count.** On a
  32-logical i9, 32 threads measured ~4x slower than 4. The default is
  `(available_parallelism / 2).clamp(1, 8)`, and 16 threads reproducibly
  changes Granite's greedy decode — every pinned transcript was recorded at 4
  and is byte-identical at 8.
- **A quality annotation that returns `Err` destroys the recording, and the
  buffer's byte limit bound *inside* the capture ceiling.** Two bugs that only
  showed up together, found 2026-08-25 from a user report of "long dictation
  errored, short one worked". The retained utterance costs **36 bytes per
  frame** — an `f32` plus a 32-byte `ProcessedSampleMetadata` — so a 64 MiB
  `max_buffered_bytes` was 116.5 s at 16 kHz against a **120 s** ceiling. Every
  maximum-length dictation therefore filled its buffer, rejected its last ~3.5 s,
  raised `BYTE_LIMIT`, and `capture` turned that into `Err` — which discarded
  the whole two minutes and logged `dictation_ceiling_stop result=no_audio`.
  Deterministic, not intermittent, which is why the symptom was so clean. Only
  `frames_buffered == 0` means there is nothing to transcribe; the other five
  `issue_code` conditions annotate audio that exists and are delivered with a
  warning now. The byte limit is 128 MiB.
- **`the_ceiling_stays_inside_the_pipeline_byte_limit` compared against its own
  copy of the constant.** It asserted the retained bytes were under
  `128 * 1_024 * 1_024` while `pipeline_config` was built with **64 MiB**, so it
  passed at 66.5 MiB with the real limit already exceeded — the exact
  relationship it exists to protect, unprotected. A test holding a hand-written
  copy of a value cannot see that value change; read it from the config the code
  actually builds. Rewritten and then made to fail by restoring 64 MiB.
- **Four of five capture reason codes had no catalog entry**, so a user who lost
  a two-minute dictation was shown `errorUnknown` — "The operation stopped
  safely". `every_capture_annotation_has_catalog_copy` now asserts every code
  `issue_code` can produce has copy, against `catalog.ts` source.
- **Malwarebytes quarantines Rust toolchain binaries as `Malware.AI.<number>`.**
  On 2026-08-25 it silently deleted `clippy-driver.exe` and `rustdoc.exe` from
  `1.97.1-x86_64-pc-windows-msvc` within ~16 s of each rustup extraction, three
  times in a row, while leaving the byte-identical copies in the `stable`
  toolchain alone. The gate then failed with `could not execute process
  clippy-driver.exe ... (never executed)`, which reads as a broken checkout
  rather than as an antivirus. Unsigned is normal for official Rust, so
  `Get-AuthenticodeSignature` proves nothing here. Check the bin directory for a
  missing `.exe` beside a surviving `.pdb` before believing anything else.
- **Granite's `max_new_tokens` is a silent ceiling — and it is currently
  unreachable, which is the trap.** The generation loop stops on reaching it with
  no error, no end-of-generation token, and nothing that distinguishes "the model
  finished" from "the model was cut off mid-clause". Nothing downstream catches a
  truncation, because it is *precise* — the plausibility gate only looks for
  transcripts that are too long.

  All of that is true and none of it can happen today. `max_new_tokens` is 2048;
  `MAX_CAPTURE_SECONDS` is **120**, which is ~310 words, which is **~400
  tokens**. A fifth of the budget. The longest utterance this product can make
  does not come close, confirmed 2026-08-25 by a 120.183 s dictation that
  transcribed complete with a six-word tripwire intact.

  Two things follow. **The hazard is latent, not absent**: raising the capture
  ceiling makes it reachable, and `capture_wizard.rs` already records wanting
  thirty minutes — which is ~4,600 tokens and would truncate silently at about
  nine minutes. **Raise `max_new_tokens` with any ceiling increase**, and since
  2026-08-26 the gate makes you: `the token budget covers the longest dictation
  the ceiling allows` compares the two as *source*, because they live in
  `speakeasy-desktop` and `speakeasy-granite` and the desktop crate deliberately
  does not depend on the one that compiles llama.cpp. It binds at ~410 s of
  ceiling, at deliberately pessimistic rates (200 wpm, 1.5 tokens per word
  against a measured 156 wpm and 1.29). And **do the division before
  inheriting a risk**: the reasoning that carried this entry cited "a 4-minute
  dictation would have lost roughly a third of itself" alongside the correct
  ~400-tokens-per-120 s figure, and a four-minute dictation cannot be recorded.
  Both numbers were right; nobody divided one by the other for months.
- **Assert whole transcripts for ASR, never a prefix or substring.** A
  `contains("ever tried")` assertion went green here on a transcript missing a
  third of the utterance. This is also why the installer's engine smoke test
  compares the whole string against a bundled fixture's ground truth: "it
  returned words" proves nothing about whether any audio was read at all, which
  is exactly how a detached audio projector presents.
- **A WASAPI output stream does not start when `play()` returns, and closing it
  early plays nothing at all.** Measured on this rig's default output (a
  SteelSeries Sonar virtual device, 8ch/96kHz): the callback took **zero**
  samples for the first 400 ms and the whole 245 ms cue by 653 ms. Sleeping for
  the sound's own duration and then dropping the stream — the obvious way to
  write it — destroyed the stream before one sample left, silently, every time.
  Nothing errors; the callback simply stops being called, which is
  indistinguishable from a cue that finished. **Wait for the device to take the
  last sample (`drained` in `cue.rs`), never for a clock**, then drain. Verify
  with `cargo run -p speakeasy-audio --example cue_diagnostics`.
- **A window is silently widened at creation, and `minWidth` does not stop it.**
  Windows clamps a new window to the default minimum tracking size while it is
  being created — `decorations: false` still leaves `WS_CAPTION` on — so a
  declared width below it is quietly ignored. Measured at 250% scale: 60 came
  back ~130, 96 came back ~130, and 96 with a matching `minWidth` came back
  ~130. Nothing errors, and the stylesheet then describes a window that does not
  exist. A `set_size` *after* creation is not subject to the clamp and holds
  (`enforce_declared_size` in `composition.rs`). Measure the running window; the
  declared size is a request.
- **A DPI-unaware process is shown virtualized coordinates, so a window that is
  exactly right and one that never scaled read the same.** `GetWindowRect` in a
  process that has not declared awareness returns physical pixels divided by the
  scale factor, silently. Measured 2026-08-15 on a 250% display, a window's
  client area came back as exactly its declared *logical* size under
  PerMonitorV2, under system-aware, and with no manifest at all — recorded as
  "three declarations, one number, so the declaration does nothing", and wrong.
  From a per-monitor-aware probe the same window is 1550x1150 physical, which is
  620x460 at 240 dpi: correct, and correct under all three. Measure a native
  window with `scripts/Measure-NativeWindow.ps1`, which declares awareness
  before it reads anything and refuses to report if that did not take. It is the
  WebView-free counterpart to `Invoke-WebviewProbe.ps1`, and `-Fit` additionally
  says whether a control's text fits its box — a separate question with a
  DPI-dependent answer, because a font cell is not linear in DPI.
- **Editing `tauri.conf.json` does not invalidate the cargo build.** A full
  `npm run tauri -- dev` restart after a config edit reported `Finished` in
  0.65s without recompiling and ran the *old* config — which looks exactly like
  a config change that had no effect. Touch `src-tauri/build.rs` to force the
  build script to re-run. (`tauri dev`'s file watcher did not pick up `.rs`
  edits reliably here either; restart rather than trust it.) The same shape
  bites a `cargo:rerun-if-changed` file restored from a copy: `Copy-Item`
  preserves the source's mtime, so cargo sees nothing newer, reports `Finished`
  in 0.4s and leaves the *previous* file embedded. Touch the file, then confirm
  what actually shipped by searching the built `.exe` for a *declaration* only
  one version contains — the linker strips manifest comments, so a comment is
  not a usable marker.
- **`GetForegroundWindow` and `WM_GETICON` both lie in ways that read as
  facts.** `WM_GETICON` with `SMTO_ABORTIFHUNG` returns 0 from a window that is
  merely still starting up, which is indistinguishable from "no icon"; and a
  window that cannot take the foreground looks identical to a caller that lacks
  foreground activation rights. Both cost a wrong conclusion here. **Run a
  control** — the same probe against a known-good window, or the same action
  with the app stopped — before believing either.
- **A `windows_subsystem = "windows"` binary cannot also be a usable CLI**, and
  it fails in the shape that looks like a working one. Measured 2026-08-15 on
  the bootstrapper: with the attribute set, `$out = & bootstrapper verify X`
  captured **nothing** and returned in milliseconds — PowerShell does not wait
  for a GUI-subsystem process — while the process itself was still alive behind
  a modal message box, because it correctly found no writable stdout and fell
  back to a dialog nobody was looking at. Two blocked windows were sitting on
  screen. Either half alone silently breaks `Test-InstallerLifecycle.ps1`, which
  parses `backup_manifest=` out of exactly that capture. A binary that must both
  draw a window and answer a script stays **console**-subsystem and re-launches
  itself with `DETACHED_PROCESS` for the window half (`relaunch_detached` in
  `apps/bootstrapper/src/main.rs`); the parent exits in ~37 ms, so the console
  flash is a flicker. Verify a `DETACHED_PROCESS` child is console-free by its
  lack of a child `conhost.exe`, not by looking at the screen — but **that test
  does not generalise to `CREATE_NO_WINDOW`**, which is what every worker gets
  (`process_worker.rs`). `CREATE_NO_WINDOW` still creates a console object and
  still gets a `conhost.exe`; it only declines to display one. Checked
  2026-08-18 after a conhost child of `granite-worker` was briefly mistaken for
  the delivery-target trap returning. The test that distinguishes them is
  whether the process owns a **visible top-level window**, which neither the
  worker nor its conhost does.
- **`Start-Process -ArgumentList` quotes nothing, and this repository's own path
  has a space in it.** The array is joined with spaces and handed over as one
  string, so `@('--install-root', 'C:\Coding Projects\...')` arrives at the
  child as **two** arguments — and no program can distinguish that from a caller
  who meant two. Measured 2026-08-15: `Test-InstallerLifecycle.ps1` drove the
  bootstrapper this way, the installer took `C:\Coding` as its destination,
  **created that directory, wrote 45 MB into it, reported success and exited
  0**, and the only symptom was the next assertion failing to find a file. Use
  the call operator (`& $exe --install-root $path`), which quotes correctly, and
  read `$LASTEXITCODE`. The receiving side is the other half: a program taking a
  path should refuse an argument list it cannot consume whole rather than use
  the first fragment — see `Mode::classify` in `apps/bootstrapper/src/main.rs`.
- **`cargo doc` does not check the doc links on private items, so a broken one
  passes.** Almost every doc comment in `apps/desktop/src-tauri` is on a private
  item, so `RUSTDOCFLAGS='-D rustdoc::broken_intra_doc_links' cargo doc` exits
  **0** over all of them — it never parses them at all. Add
  `--document-private-items` and the same command exits 101. Verified both ways
  on 2026-08-19 by restoring a known-broken link as a control: without the flag
  the check passed with the broken link in place, which is the shape of a
  verification that cannot fail. `HEAD` at `cf9c434` had three real ones
  (`StreamingPackAdapter`, `speakeasy_asr::FinalSourceReason`,
  `resident_retained_pass`), all pointing at things deleted by the fork, and the
  gate does not run this:

  ```powershell
  $env:RUSTDOCFLAGS = '-D rustdoc::broken_intra_doc_links'
  cargo doc --no-deps --document-private-items --workspace
  ```

  Two warnings are expected and pre-existing (`Self::INSPECT_DEADLINE`, and a
  `///`-indented CLI example in `apps/bootstrapper/src/main.rs` that rustdoc
  reads as Rust). A dead doc link is worth more than it looks: it is the only one
  of these citation classes a tool can find for you.
- **Invisible C1 control characters survive review and every test.** Eight
  U+009D bytes sat in comments across five files from the first commit, each one
  immediately after an em-dash from some encoding round-trip. They render as
  nothing, `git diff` shows nothing, and no check in the gate looks. They were
  found only because a scripted replacement refused to match a line that was
  identical on screen. **Scan C0 as well as C1** — the recorded detector
  looked only at `0x80..=0x9f`, so it could not see the other half of the same
  problem: a `U+0007` BEL had been sitting in `docs/handoff/CURRENT.md` since
  2026-08-18 (commit `281ce35`), where a Windows path had lost the letter after
  its separator and rendered as nothing. Widen the range to reject every C0
  control except tab, newline and carriage return, plus DEL and C1 — at zero
  across all 306 tracked files as of 2026-08-21.

  **The cause is a backslash escape interpreted one layer too early**, and it
  bites whoever is fixing it. Writing that very paragraph reintroduced two fresh
  BELs, from `\a` in a Windows path inside a shell heredoc: one level of escaping
  is consumed before the interpreter sees it, so a doubled backslash arrives
  single and `\a`, `\n`, `\t` become control characters. The same run also put a
  literal newline and tab inside a Rust string literal, and a ten-space gap into
  three catalog sentences where a line continuation should have been. **Write
  content containing backslashes with the file-editing tools rather than through
  a heredoc**, and re-scan after each such edit rather than once at the end.
  Related: the
  console here renders U+2014 as `?`, so **check codepoints numerically** rather
  than believing terminal output — a real em-dash and a corrupted one look the
  same in this shell.
- **A truncated download of the installer still runs.** The payload is appended
  past the end of `SpeakEasyMiniSetup.exe`'s PE image, and Windows' loader does
  not read that far — so a file that arrived 90% complete launches, draws the
  wizard, and would install whatever fragment of the archive parsed. Every entry
  carries a SHA-256 for exactly this, not for tampering: the whole executable is
  untrusted until someone runs it. `payload.rs`'s truncation test cuts from the
  *middle* and keeps the trailer, because a clean cut takes the magic with it and
  is the easy case.
- **`FindWindow($null, $title)` from PowerShell finds nothing, ever.** `$null`
  for a `string` parameter marshals as an empty string, so it searches for a
  window whose class name is `""`. It reported the setup wizard missing with the
  wizard on screen — a broken instrument reading exactly like the failure it was
  written to detect. Go through `Get-Process`'s `MainWindowTitle` instead.
- **`, @(...)` around a returned list defeats the next `Where-Object`.** The
  usual PowerShell guard against a one-element array unrolling hands the *whole*
  list downstream as a single object, and `$_.Property -eq 'x'` against an array
  filters rather than compares — so it comes back non-empty and truthy. A page
  heading came back as the entire window's text.
- **`Set-Location` does not move the process working directory.**
  `[IO.Path]::GetFullPath('target\...')` resolves against wherever PowerShell was
  started, not against the current location, so a relative path threw naming a
  directory nobody had typed. Resolve against `$repositoryRoot`.
- **`Start-Process notepad` does not open an empty document.** Windows 11
  Notepad restores its previous tabs, so it surfaces whatever was last open, and
  a proof that pastes into "a Notepad window" can write into someone's real
  unsaved note. It happened. A delivery target must be a file the script created
  and verified by name in the window title, never just a window owned by the
  right process.

  **And the pid you launch is not the pid that owns the window.** Notepad is a
  packaged app: `Start-Process notepad.exe -PassThru` returned pid 68176, which
  **exited immediately** after handing off to pid 44992, so
  `$np.MainWindowTitle` read empty and `$np.MainWindowHandle` read 0 — which is
  indistinguishable from a window that failed to open, and defeats the title
  check the rule above depends on. Enumerate `Get-Process notepad` and match on
  the title instead. Two related details from 2026-08-25: create the file with
  `[IO.File]::WriteAllText($p, '')` rather than `Set-Content -Value ''`, which
  writes a newline and leaves a leading CRLF that reads as the app prepending
  one to the transcript; and check the foreground **before** synthesising
  `Ctrl+S`, because `SetForegroundWindow` fails silently from a background
  process and the keystroke then lands in whatever the user is actually using.

- **A safety rule can outlive the danger it was written for, and then it only
  does harm.** `proof/` was emptied *selectively* — this installer's own files by
  name, everything else spared — on the recorded argument that an unrecognised
  file there was more likely 500 MB of fetched CUDA runtime than anything of ours,
  and that "leaving a file costs a few megabytes, deleting one costs a 2.97 GB
  download". Every word of that was true when written and none of it was true by
  2026-08-21: **this fork has no runtime download at all**. It left with the
  streaming engine. Nothing in the tree creates `.cuda-runtime-download` or
  `.cuda-runtime-stage`, the weights live under `%APPDATA%`, and the only thing
  the rule still spared was `Enable-GraniteCuda.ps1`'s 493 MB of staged libraries
  — through every uninstall, forever, on a machine the user believed was clean.
  The rule read as careful right up to the moment somebody checked whether its
  premise still existed. When a subsystem is deleted, grep the *safety rules* that
  mention it, not just the code that called it.
- **A pinned requirement nothing reads is not a fact, it is a plan — and it
  becomes a refusal the day something reads it.** The catalog pinned CUDA
  **12.9** (`cudart64_12.dll`, `cublas64_12.dll`, `cublasLt64_12.dll`) while
  `Enable-GraniteCuda.ps1` staged **13** and the only toolkit any machine here
  has is 13.3. Harmless for months, because no code compared the two. On
  2026-08-20 `speakeasy_models::required_cuda_runtime_files` began reading those
  `proof_files` as the enforced list, and a locally built CUDA worker was then
  refused as `RuntimeFilesMissing(cublas64_12.dll, cublasLt64_12.dll,
  cudart64_12.dll)` with three perfectly good CUDA 13 libraries sitting beside it
  under their real names. Re-pinned to 13.3.1 on 2026-08-21. Two things about
  doing that:
  - **CUDA 13 moved the libraries from `bin/` to `bin/x64/`.** A re-pin is not the
    old paths with a digit changed, and the first attempt refused for exactly
    that reason. `required_cuda_runtime_files` survived it only because it
    reduces `proof_files` to *base names* rather than stripping a known prefix.
  - **`scripts/Get-CudaRuntime.ps1` produces the entries.** Its predecessor was
    deleted with the streaming engine, which is how the 12.9 numbers came to sit
    in the catalog as constants nobody could re-derive. It cross-checks each
    archive against NVIDIA's own `redistrib_<version>.json`, takes the per-file
    digests from the verified archive (NVIDIA publishes none), and reports whether
    the installed toolkit's libraries are the *same bytes* — which is the only
    thing that makes `Enable-GraniteCuda.ps1` staging from the toolkit sound. All
    three matched on 2026-08-21; version strings agreeing would not have been
    that claim.
- **`--nocapture` delivered nothing from `speakeasy-desktop`'s test binary, and
  `--show-output` reported the test's stdout as empty.** Both hardware tests that
  exist partly to *measure* something were printing their numbers into a void,
  and passing, so nothing said the measurement was missing. Cause not found;
  the fix is that the resident-run timing is now written to
  `target/debug/granite-resident-timing.txt` as well as printed. **A measurement
  that only exists in captured stdout is not a measurement.** Also: an edit meant
  for one hardware test landed in a sibling with a similar name, and because the
  test filter selected the other one, everything passed while producing nothing —
  check *which* test ran, not just that one did.
- **A fixture under `.tools/` is a test with a deletion date.** The three
  granite_engine hardware tests read `.tools/fixtures/beckett.wav`, which is
  gitignored, existed only on the machine that made it, and **was gone** by
  2026-08-21 — so all three had been unrunnable for some unknown stretch while
  reading as merely `#[ignore]`d. This is the *second* time; the first is recorded
  in `.gitignore` beside the `!apps/bootstrapper/fixtures/smoke.wav` exception
  that was added for it. All three now use that committed clip, and its ground
  truth was **discovered by running the model**, not typed — the same rule that
  caught "Granit" and "dog. And Monday".
- **A claim assembled out of an intention is indistinguishable from a fact, and
  outlives every layer that would have checked it.** A support log read
  `engine=cpu_gpu_runtime_missing device=cpu installed=cuda`. Three correct
  fields; an impossible combination; nothing anywhere compared them. The install
  marker came from the wizard's provider radio button — which `UI-GUIDE.md` said
  to disable and nothing ever did — so a user on a CUDA-capable machine selected
  "Use the graphics card", setup installed the only configuration it carries, and
  wrote `cuda`. Fixed 2026-08-20, and the shape of the fix is the lesson:
  - **`speakeasy_models::granite_gpu` is the one place the question is answered**,
    and it separates *published* (a `native-runtime` artifact
    `granite-worker-cuda-windows-x64` in the trusted manifest), *present* (that
    worker plus every library its `proof_files` pin, beside it), and *operational*
    (NVML lists the worker's **own pid** as holding a compute context).
  - **The old check asked the manifest for a CUDA `final-asr` pack**, which
    answers none of the three. There is one GGUF and a CUDA worker offloads that
    same file, so a pack entry would be a duplicate of the CPU one.
  - **Compiled-in is not running-on.** `compiled_accelerators` at `Hello` says
    what the binary could do. A refusing driver, a claimed card or exhausted VRAM
    runs that same binary on the processor and llama.cpp notes it in *its own
    stderr*. `device=` reads `cuda` only where NVML confirmed it,
    `cuda_unverified` where it could not be asked, `cpu` otherwise.
  - **The marker is written after the engine check, from its verdict, and
    nowhere else.** A check that never ran writes nothing, which reads as
    `unrecorded` — guessing `cpu` would be a claim about a configuration nobody
    verified.
  - **Both sides are compared, once, by name.** `ProviderIntegrity` at warm:
    `ok`, `unrecorded`, `gpu_install_not_operational` (the actionable fault), or
    `running_beyond_record` (what `Enable-GraniteCuda.ps1` produces on purpose).
    It is in `granite_warm` as `provider=`.
- **An answer can reach disk and never reach the screen, and the screen is what
  the user judges.** Setup collects a vocabulary; it was in
  `personalization.json`, correct, three words — and Settings showed an empty
  dictionary list. The read is `personalization_status`, fired **once on mount
  with no rejection handler**. Every window's webview loads while `setup` is
  still managing coordinators, so it can be refused with "state not managed for
  field `state` on command …", and the page then shows an empty list for the life
  of the process. An empty list is not a blank page anyone reports: it says "you
  have no protected terms". `useProfile.ts` had carried a retry for that race
  since the day it was found and **nothing else did**. Status reads that can lose
  it now go through `readWithRetry` (2026-08-20). When an answer "did not
  arrive", check the disk *and* the window; they are two different failures with
  one symptom.

  **The sweep stopped at one file, and the next occurrence was worse.** Only
  `Transcription.tsx` was converted, so `readWithRetry.ts` had exactly one
  importer — and `General.tsx` read `hotkey_status` with a bare `invoke`, no
  rejection handler, rendering `hotkey?.registration ?? "pending"`. Found
  2026-08-25: Settings reported **"Shortcut not registered yet"** for the life of
  the process while `hotkey_status`, invoked directly against that same window,
  returned `registration: "registered"` and dictation worked twice. An empty list
  is a passive wrong answer; this one tells the user a working feature is broken,
  in the panel they opened *because* it seemed broken, and the remedy it implies
  — press "Save hotkey" to re-register — fixes a problem they do not have. When
  fixing a race in one reader, grep for every other reader of the same shape.

  **And that symptom had a second cause, which is the one that reproduced.** A
  refusal is not the only way to lose this race: a read can **succeed** and
  return a value that is only true for the first moment of the process.
  `HotkeyCoordinator` starts at `registration: "pending"` and
  `register_activation_hotkey` runs at the **end** of `setup`, after the tray is
  built — while all three eagerly mounted settings pages have already read. So
  the fix implied by the diagnosis above would have shipped without fixing the
  reported symptom. **Three causes, one appearance**: the backend is wrong, the
  read was refused, or the answer arrived before it was true. The rendered string
  separates none of them, and neither does the page's own state — `null` from a
  refusal and a transient value both render the fallback. Reload the window
  (`-Cdp 'Page.reload'`) and read the same page against the same backend: correct
  after a reload is a transient, and cannot be a refusal. `readWithRetry` takes a
  `settled` predicate for this, and an unsettled answer that survives all 20
  attempts is *returned* rather than thrown, because a startup value still there
  after five seconds has stopped being transient (2026-08-26).

  **Which pages can lose this race is decided by `SettingsApp.tsx`, and by
  accident.** General, Transcription and Advanced mount eagerly; Audio, Output &
  Privacy and the transcript log mount only while their tab is active, so their
  reads land long after `setup`. The three eager pages are exactly the three that
  had the defect — a correlation nobody chose, since Audio's conditional mount
  exists so a hidden page does not sample the microphone. Making one of the lazy
  pages eager brings the race back with it, which is why the guarantee lives in
  `no effect can read a race-prone command without retrying or polling` and not
  in the mounting.

  **The test written to prevent the recurrence is what let it happen.** It named
  `personalization_status` in `Transcription.tsx` and was green on the day the
  same defect was found in a second file with a second command: a record of where
  the bug had been seen, not a rule about where it can happen. The replacement
  derives the hazard from the Rust signatures — every `#[tauri::command]` taking
  a `tauri::State`, 41 of the 56 — and finds the readers by scanning every
  `useEffect` in `src/`, following one level of local function calls, which is
  what reaches `refreshCatalog`. **Every assertion in it is of the form "nothing
  was found", which is also what a broken scanner reports**, so it carries
  instrument self-checks and was proved able to fail by restoring the original
  defect in two files.
- **A merge keyed on positional ids fails closed on the whole batch.** Setup's
  words become dictionary entries named `installer-0`, `installer-1`, … *by
  position*, and an uninstall run with `--keep-user-data` keeps
  `personalization.json` — which is what both proof scripts pass, so this is
  still exactly the path they take. (A production uninstall removes it now; the
  default inverted on 2026-08-21.) So
  a second install merged a shorter list over the old ids, left one behind, and
  where the survivor held a word the new list also held the two were a
  `ConflictingRule` — which rejects **every entry in the merge**, not the
  duplicate. The user got none of their words and kept the previous install's.
  Two words differing only in case (`Ken, ken`) did it on a first install.
  Fixed three ways, all of them needed: the parse de-duplicates
  case-insensitively, `replace_user_entry_terms` replaces setup's entries rather
  than merging, and the outcome is logged (`installer_vocabulary count= result=`)
  instead of vanishing into a `let _ =`.
- **Waiting for a file to exist is not waiting for it to be written.**
  `Test-SetupWizard.ps1` waited for `personalization.json` to appear and then
  asserted its contents. An uninstall run with `--keep-user-data` keeps that
  file — and that is what the script passes — so on a reinstall
  the wait returned instantly and the assertion read the **previous** install's
  words — reporting the new ones lost against an app that had applied them
  correctly seconds later. Poll for the content you are asserting, with a
  deadline, so the instrument can still fail.
- **`WM_SETTEXT` does not raise `EN_CHANGE` on a multi-line edit**, so a driver
  that stuffs a box and reads back what the page says about it gets the answer
  from before the stuffing. Measured 2026-08-20: the vocabulary page reported "No
  words yet" with three words in the box. `Test-SetupWizard.ps1` presses Back and
  Next to force the recompute, which also proves Back does not lose what was
  typed.
- **The fork updated every path it executed and left every path it did not**,
  and those paths fail fast, so each one hides the next. Six were found this
  way on 2026-08-18 — the dev launcher, the quality gate, the dependency
  policy, the packager, the installer builder and the install proof — each
  still naming `speakeasy-inference-worker`, a sherpa runtime, or a script the
  fork deleted. All six had been dead since the fork and none of them was
  covered by a test, because they *are* the tests. **Before trusting anything a
  script asserts, confirm the script runs to the end.**
- **A check that asserts the presence of something deliberately deleted reports
  a bug in the thing it is checking.** `Test-LocalInstall.ps1` demanded a
  `nemotron-3.5-streaming-en-cpu` entry in the trusted manifest and threw
  against a manifest that was entirely correct. It resolves the pack by
  `install_eligible` now. When something is removed, grep the *proofs* for it,
  not just the code.
- **PowerShell turns a native command's redirected stderr into ErrorRecords,
  and under `$ErrorActionPreference = 'Stop'` those are terminating.** Every
  refusal `Test-InstallerLifecycle.ps1` exists to assert is written to stderr,
  so each one threw at the `2>&1` before its exit code could be read, and
  surfaced as a `NativeCommandError` quoting the refusal text — indistinguishable
  from the installer being broken rather than correctly refusing. `Assert-Refused`
  could never have passed. Leave the stream alone where the output is not needed
  (`Stage-DevRuntime.ps1`), or lower the preference for the duration of the call
  where it is (`Invoke-Installer`).
- **Anything the installer hardcodes is a chance to write into SpeakEasy's
  state, and none of it errors.** `%APPDATA%\<identifier>`, the ARP key, the
  version stamp under `Software\<name>`, the install root under
  `%LOCALAPPDATA%\<name>`, and the Start Menu folder were each inherited from
  the parent product. The install root was the worst: setup would have written
  over an existing SpeakEasy installation, and because uninstall removes the
  install directory whole — more completely since 2026-08-21, when the last
  spared thing in it stopped being spared — uninstalling this app would have
  deleted that one.
  Reachable only by building an installer and running it.
- **A window measured during startup is not the window.** `configure_hud` sets
  non-focusable, shows the dock and applies `enforce_declared_size` after the
  windows already exist, so a reading taken too early shows a hidden, focusable
  dock at the creation-time clamp width — identical to a real regression. Wait
  for `granite-worker` to appear, then measure.
- **`GetWindowRect` includes a resize border that `set_size` does not.** The
  pinned log declares 340x460 and its outer rect measures 882x1169 at 250%,
  which looks like `enforce_declared_size` failing. It is not: the log is
  `resizable: true` and keeps `WS_THICKFRAME`, while the dock is not and so
  measures exactly. `Measure-NativeWindow.ps1` reports the **client** rect —
  850x1150 physical, exactly 340x460 logical — and the webview independently
  agrees at 340x460 CSS px.

## Settled decisions — do not re-open without new evidence

- **Granite is the only engine, and its transcript is delivered
  unconditionally.** Punctuation and casing come from the same single pass, so
  a latency argument cannot be used to remove it — there is nothing left to
  fall back to.
- **No fallback, ever.** A pass that fails, returns nothing, or fails the
  plausibility gate ends the dictation with a named reason. Silently
  substituting a weaker result is the behaviour this fork exists to remove.
- **The dock never takes keyboard focus**, so it is deliberately not keyboard
  operable. Every action it offers must have a keyboard path elsewhere — the
  shortcut, or the settings window.
- **One dictation at a time, refused rather than queued.** Owner decision
  2026-08-26. A press between a recording ending and its transcript landing is
  the second press of a toggle for a dictation that is already over — commonly
  after a ceiling stop, where the recording ended without being asked to and the
  user was by definition still talking. It used to open a second dictation that
  queued behind the first and pasted up to a minute later, wherever the user had
  moved on to; nothing errored, because everything was working as built. The
  guard is in `start_dictation`, the single implementation behind both the
  shortcut and the dock's button, and `hud_session_with_delivery` is the one
  statement of when a dictation is over — the dock's `can_start` had refused this
  press all along while the shortcut accepted it, and the cause was a second copy
  of that rule. It **fails open**: a poisoned lock or an absent coordinator must
  never be able to suppress a dictation the user wanted.
- **Granite's GPU support is a build feature, not a downloadable pack**, so
  there is no provider-override setting: no setting can conjure a CUDA-capable
  worker binary. The installer fetches one when the user asks for it on hardware
  that warrants it; without it the app runs on CPU and says so. **The declaration
  lives in the trusted manifest**: the CUDA worker is the `native-runtime`
  artifact `granite-worker-cuda-windows-x64`, and its presence is what enables
  the wizard's option and permits an installation to record `cuda`.

  **Published 2026-08-26** to
  `orangeblue39/speakeasy-mini-runtime` on Hugging Face, pinned at an immutable
  commit, 42,162,465 bytes of zip around a 57,052,672-byte worker. Hugging Face
  carries the worker only; the three CUDA redistributables still come from
  NVIDIA's own CDN, which the manifest already pinned. Before that date the
  declaration was made *by absence* and seven tests asserted it — they inverted
  together, which is what said the change had landed everywhere it needed to.
  Two things that did not change and are the point: publishing is not
  installing (`inspect_gpu_payload` still answers `WorkerNotInstalled` on a
  machine without the files), and the recorded provider still comes from
  `smoke::verify_engine`'s verdict rather than from the option the user picked.
- **The whole workspace targets CUDA 13.x**, pinned at cudart 13.3.29 and
  libcublas 13.6.0.2 from `redistrib_13.3.1.json` — the versions whose bytes are
  byte-identical to this workspace's CUDA Toolkit, proved by digest. Accepting
  `cudart64_*.dll` by pattern was considered and rejected: presence would stop
  implying provenance, and every required file in this catalog is a file the
  catalog pins.
- **`install-provider.txt` has exactly one writer and two callers.** The writer is
  `seed::record_installed_provider`, fed from `smoke::verify_engine`'s verdict.
  The callers are the wizard's last page and the bootstrapper's
  `--verify-provider` verb. A third caller, `scripts/Enable-GraniteCuda.ps1`,
  invoked that verb after staging a CUDA worker by hand and after `-Revert`; it
  was **retired on 2026-08-26** when setup began fetching a published worker. It
  read no NVML, classified nothing and wrote no marker — a second implementation
  of the three-gate proof would have been a second source of truth for the same
  claim, which is the shape of the defect the 2026-08-20 work removed, and the
  reason retiring the script changed nothing about this rule. Anyone staging a
  worker by hand now inherits its obligation: **skipping the re-prove is not
  neutral in either direction.** Staging without re-proving reports
  `running_beyond_record`; putting the processor worker back without re-proving
  manufactures `gpu_install_not_operational`.
- **The NVML probe is a parameter threaded through `GraniteEnvironment`, not an
  environment variable.** A production switch whose only purpose is to make the
  app misreport its own provider is the same shape as the radio button that
  started all this. Threading it is what made `device=cuda_unverified` reachable:
  a working card always answers and always takes the worker, so "the driver would
  not answer" and "the driver said no" are precisely the two answers no real
  machine can be asked for.
- **The active provider is reported as the device, never as the pack.** The pack
  reason (`engine=`) and the device (`device=`) are different facts that disagree
  on any machine running a CUDA worker against the single CPU-named pack. Settings
  reads the device; the disclosure used to read the pack. Measured on an RTX 5090: Granite
  Q4 resident run 1,571.9 ms on CPU versus 156.4 ms on CUDA, RTF 0.158 versus
  0.0157, holding ~3.27 GiB of VRAM.
- **On real speech the card is 9.34x the processor, and inference is 98% of the
  latency.** The first non-harness numbers this product has, measured 2026-08-25
  on an installed release build, RTX 4070 Laptop: a 105.2 s dictation was
  **4,171 ms** of inference (RTF 0.0396) and **4,246 ms** press-to-paste, against
  **44,493 ms** (RTF 0.3702) for a 120.2 s dictation on the processor. The 6.42 s
  fixture predicted 8.1x, so it is a fair guide and errs optimistically about the
  processor. Press-to-paste is 54 ms of queueing, then inference, then 21 ms to
  inspect the foreground and paste — so **no latency argument that is not about
  inference is worth having**. Two caveats that keep this honest: the two runs are
  different recordings, so only RTF compares across them, and this says nothing
  about the byte-identical claim, which needs one WAV through both workers.
- **Q4_K_M is the shipped quantization**, chosen on measurement rather than by
  decision: ~21% faster than Q8_0 on a 120 s utterance with an identical
  transcript but for one punctuation choice. Q8_0 stays in the catalog as the
  recorded alternative, not as a second thing to keep working.
- **Two destructive cleanup rules never run.** `immediate_repetitions` and
  `self_corrections` are forced off in `apply_final_personalization` and have no
  settings toggles. `resolve_self_correction` discards everything before
  `" I mean "`, which is live data loss, and it fires more often on Granite's
  fluent output than on a transducer's. A test pins this precisely because the
  rules are now unreachable from the UI.
- **The setup wizard sets its own type, and it is the only crate in the
  workspace allowed `unsafe`.** Owner decision 2026-08-26, inverting "colour and
  no bold". `winsafe` builds one `HFONT` per process from `lfMenuFont` and offers
  nothing that changes a control's font afterwards — that is `WM_SETFONT` through
  `SendMessage`, which it marks `unsafe`. So both wizard windows drew everything
  at Segoe UI 9pt, the size Windows uses for *menu bars*, on a ~105-character
  measure with the heading, step counter, key line and body all identical.
  Correctly scaled and too small to read, which is not a bug a measurement finds.

  `apps/bootstrapper/Cargo.toml` declares `[lints.rust] unsafe_code = "deny"`
  instead of inheriting; `src/typeface.rs` holds the only two `#[allow]`s under
  it. **The root stays `forbid`** — it is load-bearing beyond style, because
  under edition 2024 it is what makes `std::env::set_var` unreachable from a
  test. Two rules for anyone touching this: sizes are a **ratio** of
  `SPI_GETNONCLIENTMETRICS` (four thirds for body, five thirds semibold for the
  heading), never absolute points, because a reader who raised Windows' own text
  size is the person the change is for and "12pt" would shrink the wizard for
  them; and **every control that carries words gets one of the two**, buttons and
  check-box labels included — `apply_typeface` lists them by hand rather than
  enumerating children, so a control added later is a control someone must
  remember. Colour is unchanged and still never the only signal.
- **Every wizard page is a question, one key line, and at most two short
  sentences.** Rewritten 2026-08-20 from four-sentence paragraphs that were
  correct and unread. Nobody reads an installer, so an honesty obligation
  discharged in the third paragraph is discharged on paper only. `catalog::Step`
  holds the three parts separately so a page cannot drift back into prose.
- **The vocabulary box takes a comma-separated list**, and reports back how many
  words it read and which. Newlines still separate — the old one-per-line form
  means the same thing and losing a word to punctuation would be indefensible —
  and the count comes from the same parse that writes the seed, so it cannot
  describe a list the file disagrees with.
- **The uninstall page is the confirmation, and its Remove button is focused.**
  Owner decision 2026-08-21, and it inverts the earlier reading. One page, a
  check box per `Removable`, every box checked, no dialog behind it — a second
  prompt re-asking what the page just asked teaches people to press Enter twice.
  `BS::DEFPUSHBUTTON` makes a button the *default* and does not focus it:
  measured here, removing the explicit `SetFocus` puts the focus on the heading
  static. Only the models entry names a size, and `uninstall::measure` walks the
  same path table the deletion does, so the figure cannot describe one set of
  files while another is removed.
- **An uninstall leaves nothing, and keeping things is a testing flag.** Owner
  decision 2026-08-21, inverting the inherited `/SD IDYES` default. `--uninstall`
  removes the program directory whole *and* the profile — settings, transcript
  history, the 2.14 GB of weights, recovery backups, logs — and removes the
  directories themselves, not just their contents. `--keep-user-data` is the
  opt-out, and exists so an install/uninstall cycle does not re-download the
  weights; both proof scripts pass it. The interactive path **asks first**, on the
  page described above, with any unrecognised files in `proof/` listed.
  `--remove-all` is gone and is deliberately
  not accepted as an alias: it named the thorough behaviour, that behaviour is now
  the default, and a flag meaning "do what you were going to do anyway" lets a
  caller keep believing it is choosing. `Removals::default()` still selects
  nothing, because a *caller* that forgets to ask must delete nothing; the
  inversion is at the command line, where somebody has actually been asked.

  **`--keep-user-data` seeds the page's check boxes, and until 2026-08-26 it did
  not.** `remove()` computed the `Removals` and then discarded it on the
  interactive path, because `uninstall_page::ask` took no argument and hardcoded
  every box checked — so `--uninstall --keep-user-data` *without* `--silent` drew
  a page primed to delete the profile, and one did go: 4.28 GB of weights, the
  settings and the vocabulary. The flag worked only alongside `/S`, which is the
  one combination both proof scripts pass, so nothing had ever run the other. A
  flag that states an intention has to reach the control that acts on it.
- **Local-only.** No GitHub Actions, no Dependabot, no hosted runners.
  `scripts/Test-LocalOnlyPolicy.ps1` fails if `.github` config reappears. A
  GitHub *Release* is not automation and is how the installer is published; the
  build, the proofs and the upload are all run by hand from this machine.
- **Setup asks eight questions and every answer reaches the app**, through
  one-shot seed files under `%APPDATA%\ai.speakeasy.mini\config\` that the app
  reads and deletes on first launch. The deletion is the contract: a seed is a
  starting value, never a policy, so a setting the user changes afterwards must
  never revert. `install-provider.txt` is the one exception and persists,
  because it records what was *installed* rather than what to start with.
- **Setup launches the app, and says so if it could not.** Ending by closing its
  own window leaves someone who watched every step succeed looking at an empty
  desktop.
- **The dictation floor is Granite's floor** (8 GiB), raised from 4 GiB on
  2026-08-18. The two were split so a machine that could not host Granite still
  dictated through the streaming path; with one engine that only let someone
  speak into a guaranteed `GraniteUnavailable`. Refusing at `begin`, before a
  sample is captured, is the same answer at the only useful moment.
- **The app installs beside SpeakEasy, never into it.** Identifier
  `ai.speakeasy.mini`, binary `ai-speakeasy-mini.exe`, install root
  `%LOCALAPPDATA%\SpeakEasy Mini`, version stamp under
  `Software\SpeakEasy Mini`, its own ARP key and its own Start Menu folder.
  Every one of those was inherited from the parent until 2026-08-18. The Rust
  **crate** names stay `speakeasy-*` deliberately, and the IPC schema `$id`s
  keep the old string, because neither is user-visible or a filesystem path.

## Conventions

- **`docs/UI-GUIDE.md` is a living spec**, not a record. A UI change that does
  not amend it is incomplete. Superseded briefs get a `> **Superseded.**` note
  rather than a rewrite.
- **Design drawings go in `docs/design/`** as single standalone HTML files —
  inlined CSS, `data:` URIs, nothing fetched — plus a row in that directory's
  contents table. Not chat artifacts. Read geometry from the real stylesheet,
  and say what a drawing is and when it was true.
- **Measure the running window, not the stylesheet.**
  `getBoundingClientRect()` accounts for the cascade you did not know was
  there; several layout bugs here were invisible in the CSS and obvious in the
  window. **`height: 100vh` with `justify-content: space-between` is the worst
  case**, because it describes a box that always looks correctly filled no matter
  how much content is in it — the `notice` window read fine in CSS while needing
  188 px in a 172 px window, and its only control sat 16 px below the fold from
  the day it shipped. `scrollHeight - clientHeight` is the reading; anything
  above zero is a clip. Do it for any window that is `resizable: false`, which is
  all of them but the log.

  **Do it after the fix as well, because an estimate of the requirement is still
  an estimate.** The notice was raised to 192 on the recorded finding that its
  content needed 188 and that 192 would leave 4 px spare. Measured on the running
  window: overflow 0, and the button **2 px** clear, not 4 — the real box needs
  190. Both readings were taken the same way, the earlier one was taken with the
  scrollbar suppressed, and it was optimistic by exactly that difference. A
  measurement of the *problem* does not carry over to the *fix*.
- **UI copy is honest about what happened.** Delivery is never claimed unless
  insertion succeeded; colour is never the only signal. New error codes get
  their own catalog entry with a real instruction.
- **Comments explain why, not what**, and record the failure that motivated
  the code. The existing comments are dense and load-bearing; match them.
- **Assert invariants against source.** The scaffold suite reads config and
  source files to pin things review would otherwise have to catch — the window
  allowlist, the IPC schema, and the non-focusable rule among them.
- **Every citation in a comment must resolve, and must not be a number.** Cite a
  doc by heading — `UI-GUIDE "Information architecture"` — never by section
  number. The fork inherited ~96 `§N` references pointing at a deleted doc's
  numbering plus 35 naming deleted files, and all of them read as authoritative
  while pointing at nothing; they were cleared on 2026-08-19. A heading survives
  a renumber, and it can be checked. Three citation classes exist and a sweep
  that misses one looks complete: the **filename** (`granite-final-pass.md`),
  the **bare number** (`§9.4`, `Phase 6`), and the **prose** ("the handoff",
  "the brief", "the GPU migration handoff, item 14") — that last one matches no
  grep for a path or a `§`.
- **Prefer naming the fact over citing where it was recorded.** Most of those
  citations were carrying a fact perfectly well stated inline: `Phase 9` meant
  `2026-08-04`, `Known risk #12` meant "the stale-clock deadline bug". Absorb it
  and the comment stops depending on a document surviving.
- **A comment about something deleted is only allowed to be history**, and has
  to read as history. `speakeasy-asr`, `streaming_engine.rs` and
  `inference-worker.exe` are still named in 12 places on purpose — "It was
  `speakeasy-asr`, and it did link one", "were listed here until the fork
  removed the engine". Those are load-bearing and rewriting them would make them
  false. A *present-tense* claim about the same thing is a bug: that is the other
  ~39, and one of them had the crate doc of `speakeasy-granite` asserting that
  the delivered transcript came from the streaming model.

## Layout

```
apps/desktop/src            React frontend (hud/ is the dock + pinned log)
apps/desktop/src-tauri/src  Tauri commands, coordinators, composition root
apps/bootstrapper           The one-exe installer, and the backup/restore tool
crates/speakeasy-*          Domain, audio, worker boundary, delivery, storage, models
workers/granite-worker      The supervised llama.cpp inference child process
scripts/                    Every build, proof and packaging step (PowerShell)
```

`apps/desktop/src-tauri/src/lib.rs` `include!`s its sibling modules, so those
files share one namespace and need no imports between them. One consequence
worth knowing before you write a header: an inner doc comment (`//!`) at the top
of an included file is a compile error, because it would be documenting
`lib.rs`.
