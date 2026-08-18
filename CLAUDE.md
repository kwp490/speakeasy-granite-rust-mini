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

Building and proving the installer, which is a separate path the gate does not
touch:

```powershell
.\scripts\Build-LocalInstaller.ps1
.\scripts\Test-InstallerLifecycle.ps1 -ArtifactRoot 'target\local-development\<version>'
```

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
   in release. Time things in release, on an installed build.
3. **Trust the disk log over the UI and over proof scrapers.**
   `%APPDATA%\ai.speakeasy.mini\logs\speakeasy.log` carries a specific error
   code where the UI often shows generic text.

Driving the running app by selector, for measuring real layout:

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
.\scripts\Invoke-WebviewProbe.ps1 -Window settings -Expression '...'
```

`-Window` takes `settings`, `dock` or `log`.

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
  outcome.

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
- **Never default llama.cpp threads to the logical processor count.** On a
  32-logical i9, 32 threads measured ~4x slower than 4. The default is
  `(available_parallelism / 2).clamp(1, 8)`, and 16 threads reproducibly
  changes Granite's greedy decode — every pinned transcript was recorded at 4
  and is byte-identical at 8.
- **Granite's `max_new_tokens` is a silent ceiling.** The generation loop stops
  on reaching it with no error, no end-of-generation token, and nothing that
  distinguishes "the model finished" from "the model was cut off mid-clause".
  The old default of 512 was closer to biting than anyone had checked: a 120 s
  clip needs ~400 tokens for its 312 words, and a 4-minute dictation would have
  lost roughly a third of itself. The shipped value is 2048. Nothing downstream
  catches a truncation, because it is *precise* — the plausibility gate only
  looks for transcripts that are too long.
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
- **`Start-Process notepad` does not open an empty document.** Windows 11
  Notepad restores its previous tabs, so it surfaces whatever was last open, and
  a proof that pastes into "a Notepad window" can write into someone's real
  unsaved note. It happened. A delivery target must be a file the script created
  and verified by name in the window title, never just a window owned by the
  right process.

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
  install directory whole, uninstalling this app would have deleted that one.
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
- **Granite's GPU support is a build feature, not a downloadable pack**, so
  there is no provider-override setting: no setting can conjure a CUDA-capable
  worker binary. The installer fetches one when the hardware warrants it;
  without it the app runs on CPU and says so. Measured on an RTX 5090: Granite
  Q4 resident run 1,571.9 ms on CPU versus 156.4 ms on CUDA, RTF 0.158 versus
  0.0157, holding ~3.27 GiB of VRAM.
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
- **Local-only.** No GitHub Actions, no Dependabot, no hosted runners.
  `scripts/Test-LocalOnlyPolicy.ps1` fails if `.github` config reappears.
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
  window.
- **UI copy is honest about what happened.** Delivery is never claimed unless
  insertion succeeded; colour is never the only signal. New error codes get
  their own catalog entry with a real instruction.
- **Comments explain why, not what**, and record the failure that motivated
  the code. The existing comments are dense and load-bearing; match them.
- **Assert invariants against source.** The scaffold suite reads config and
  source files to pin things review would otherwise have to catch — the window
  allowlist, the IPC schema, and the non-focusable rule among them.

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
