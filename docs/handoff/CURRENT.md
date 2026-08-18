# Handoff — SpeakEasy Mini, as of 2026-08-18

The state of the fork, what is finished, what is not, and the things that will
cost you an afternoon if you rediscover them yourself.

Read `CLAUDE.md` first. This file assumes it.

## Where the project is

SpeakEasy Mini was forked from SpeakEasy in one session and reduced to two
changes that cascade through everything: **Granite is the only engine**, and
**the dock is the only HUD**. Five commits, on a fresh history, pushed to a
private repository.

Verified state, and the exact commands that produce it:

```powershell
. .\scripts\Enter-DevEnvironment.ps1
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
```

That is the whole gate — fmt, clippy `-D warnings`, workspace tests, coverage,
the four frontend steps, dependency policy, `cargo deny`, both audits and
gitleaks. It had not been runnable at all since the fork; see below.

**As of 2026-08-18 it runs to the end and passes** — the first time it has, on
this fork. `cargo audit` finds 0 vulnerabilities and 13 informational warnings
(18 less the 5 the reviewed allowlist covers, which is also the check that
`.cargo/audit.toml` is being read at all); gitleaks scans 5 commits and finds
nothing. Getting there took six separate fixes to the gate and the things it
checks, none of which were reachable before it could start; they are recorded
below.

**Do not substitute `cargo test --workspace --lib` for it.** That was the
command this file used to quote, and `--lib` builds no `--bin` targets, which is
exactly how a bootstrapper test stayed red since the fork without anyone seeing
it. 294 Rust `--lib` tests and 36 bootstrapper binary tests pass; from
`apps/desktop`, `npm test`, `npm run lint` and `npm run typecheck` are clean —
55 frontend tests.

**There are no longer any crate exclusions, and that is the news.**
`speakeasy-granite` had not been compiled since the fork, and every command in
this file used to exclude it. It builds:
`cargo build --release -p speakeasy-granite-worker` finished in **1 m 50 s** on
2026-08-18 and produced a 4.3 MB `granite-worker.exe`. The prediction was right,
but it was a prediction until that ran.

Two prerequisites had to be satisfied first, and neither was on this machine:

- `git config --global core.longpaths true`, which `docs/ARCHITECTURE.md`
  already named as required. It was unset.
- `.tools/` did not exist at all — no staged CMake, no staged Node. The system
  CMake on `PATH` and the libclang `Resolve-Libclang.ps1` found were enough, so
  `Enter-DevEnvironment.ps1` warned about neither.

## The app has now been run, and it was broken in two places

The previous version of this file said the app had never been launched and that
this was the largest gap. It was, and launching it found what it was supposed to
find: **two leftovers from the amputation, neither covered by any test, that
between them made the app unlaunchable and dictation impossible.** Both are
fixed.

**1. Every dev launch threw before Vite started.**
`apps/desktop/src-tauri/tauri.proof.conf.json` still listed the removed
streaming engine — `inference-worker.exe`, both ONNX Runtime DLLs, both sherpa
APIs and `cargs.dll`. `Stage-DevRuntime.ps1` reads that file to learn the
installed layout, so it ran `cargo build -p speakeasy-inference-worker` against
a crate the fork deleted, failed, and took `beforeDevCommand` down with it. The
payload manifest and the uninstaller's `INSTALLED_PROOF_FILES` are pinned to
each other by `scaffold.test.mjs`, so all three moved together.

**2. `paths()` failed on every call, so every dictation would have failed.**
`RuntimeWizardCoordinator::paths()` resolved the streaming worker,
`onnxruntime.dll` and `sherpa-onnx-c-api.dll` as **required**, and did it
*before* `granite_worker` — which was `Option` and, in its own words, "never in
`required`". Those three can never exist again, so `paths()` returned
`runtime_resources_unavailable` unconditionally, `granite_worker_exe` was always
`None`, and `judge_granite_pass` would have ended every dictation with
`GraniteUnavailable`. The requirement is now inverted: the Granite worker is the
required path and the other three are gone.

That second one is worth dwelling on, because it shows how the gap hid itself.
Four tests covered `paths()`. Every one of them constructed the coordinator with
a root that does not exist, so every one asserted the *error* and not one
asserted success — a suite that was fully green about a function that could
never succeed. `a_staged_root_resolves_and_the_granite_worker_is_what_makes_it_one`
now asserts the positive case, and it was confirmed by reintroducing the bug and
watching it fail while the other four stayed green.

### What the first real dictation recorded

Measured 2026-08-18 from `%APPDATA%i.speakeasy.mini\logs\speakeasy.log`,
which is the record to trust:

```text
event=granite_warm result=ok engine=cpu_gpu_runtime_missing device=cpu
event=dictation_start result=ok
event=dictation_stop result=ok
event=dictation_finalize result=started engine=llama.cpp
event=hotkey_delivery_target integrity=Equal executable=<redacted> process_id=…
event=hotkey_delivery result=committed
event=dictation_finalize result=finished
```

- 9.64 s of speech; **4.20 s** from finalize to delivery; 4.85 s total after
  stop. Delivery `committed` — a real paste into the focused window, not the
  clipboard fallback.
- `engine=cpu_gpu_runtime_missing device=cpu` is the correct code for this
  machine: an NVIDIA card with no published CUDA worker to load. This is the
  case `docs/ARCHITECTURE.md` says must be distinguishable from a chosen CPU
  install, and item 2 below is what makes it so.
- **That 4.20 s is not a measurement of anything.** It is a dev run — a debug
  desktop host around a release worker — and `CLAUDE.md` is explicit that dev
  timings have invalidated conclusions here twice. It says the path works. It
  does not say what it costs. The apparent RTF of 0.436 should not be compared
  against the recorded 0.158 until it is re-taken on an installed release build.

Also verified in that session, each against the running window rather than the
declaration:

- All three windows exist. The dock is **visible at exactly its declared size** —
  62×360 logical, 155×900 physical at this display's 250% scale — so
  `enforce_declared_size` defeats the creation-time width clamp in practice, not
  just in principle.
- The foreground window was never one of ours, before or after the worker
  spawned, and neither `granite-worker` nor its `conhost` has a visible window.
  The delivery-target trap has not reappeared.
- `granite-worker` goes resident at ~2.47 GiB with the weights loaded, and the
  desktop process falls back to ~65 MB after the warm.

One correction to `CLAUDE.md`'s guidance, found while checking this: a
`CREATE_NO_WINDOW` child **does** have a child `conhost.exe`. That flag creates
a console object and declines to display it. The "verify by the lack of a child
`conhost.exe`" rule is sound for the bootstrapper's `DETACHED_PROCESS` relaunch
and gives a false positive here; the test that actually distinguishes them is
whether the process owns a visible top-level window.

### Still not exercised

- The pinned log window has been created but never **shown**. Opening it is
  still the moment to re-check the foreground, because that is the window most
  likely to take it.
- Long dictations. The one recorded above was 9.6 s, so the ~30 s post-recording
  wait a two-minute dictation implies has not been felt by anyone yet, and the
  question of whether the dock's working indicator is enough is still open.
- Anything at all on an installed release build.

## The gate had not been runnable either

`Invoke-ScaffoldChecks.ps1` is this project's definition of green, and
`docs/handoff/CURRENT.md` told the next session to run it once and find out
whether `speakeasy-granite` still built. Running it found something else first:
**the gate threw on its own second step, and had done so since the fork.**

It asserted a version-stamped sherpa-onnx runtime under
`.tools/sherpa-onnx/current` before letting Cargo start — reasonable when
`sherpa-onnx-sys` linked against it through `SHERPA_ONNX_LIB_DIR`. The fork
removed the engine, `.cargo/config.toml`, and both scripts that block named
(`Get-NativeRuntimeVersion.ps1` and `Get-GpuRuntime.ps1`), but not the block.
So the gate died calling a script that no longer exists, with a
`CommandNotFoundException` that reads like a broken machine rather than a broken
gate — and every "the gate is green" claim in the previous handoff actually came
from running its sub-commands by hand. The block is gone; `speakeasy-granite`
compiles llama.cpp itself and there is nothing left to stage.

Two things surfaced the moment it could run to the end, and neither was
reachable by the commands that had been standing in for it:

- **`cargo fmt --check` fails are invisible to `cargo test` and `cargo clippy`.**
  The gate caught formatting in this session's own new test.
- **`cargo test --workspace --lib` builds no `--bin` targets**, so it never ran
  the bootstrapper's binary tests. One of them,
  `the_plan_names_both_engines_and_totals_their_transfer_sizes`, had been
  **failing since the fork** — it demanded the download plan name two packs,
  "one streaming pack and one Granite pack", and there is one engine now. It is
  rewritten as `the_plan_names_one_engine_and_totals_its_transfer_size`, and it
  now also pins that a GPU machine plans the same list, so the CUDA worker
  landing is what makes the count 2.

This is the previous session's "a whole crate went red unnoticed" one level
down: not a crate list this time, but a **target filter**. `--lib` is not the
workspace. Run the gate.

### The dependency policy was stale in four places

Reaching `Test-DependencyPolicy.ps1` for the first time since the fork found it
describing a workspace that no longer exists. Each of these threw in turn:

- **`.cargo/audit.toml` was missing.** The fork deleted the whole `.cargo/`
  directory to be rid of `config.toml`, whose only job was pointing
  `SHERPA_ONNX_LIB_DIR` at a pre-fetched archive — and took the advisory
  configuration with it. Regenerated, not invented: it is a projection of
  `dependency-policy/advisory-allowlist.json`, which survived intact with all
  five advisories, each carrying a package, a reason and an expiry of
  2026-10-19. The regenerated file is advisory-for-advisory identical to the
  parent repository's, which is the confirmation that the derivation is the
  right one.
- **The dependency table still called the worker crate `speakeasy-asr`**, so
  `speakeasy-desktop`'s real dependency on `speakeasy-worker` was "forbidden".
  Reconciled against `cargo metadata` rather than by hand. The entry is now one
  line — `speakeasy-worker = @('speakeasy-domain')` — where it used to carry
  `sherpa-onnx`, `transcribe-cpp`, `nvml-wrapper` and `unicode-segmentation`.
  That single line is the machine-checked evidence for this file's claim that
  the crate links nothing native.
- **Three entries named crates that no longer exist** — `speakeasy-remote`,
  `speakeasy-bench` and `speakeasy-inference-worker`. Removed, with a note
  saying they are absent deliberately, because a crate with no entry throws on
  sight and a quiet resurrection is exactly what this table exists to prevent.
- **`$allowedPatchedSources` still listed both `transcribe-cpp` path patches.**
  `[patch.crates-io]` has one entry now, the `llama-cpp-sys-2` fork.

Worth recording because it was the question underneath all of this: `cargo audit`
reports **zero vulnerabilities**. There are 18 informational warnings — 16
unmaintained, 2 unsound — of which the reviewed allowlist covers five. The
remaining thirteen are not suppressed anywhere and were not suppressed before
the fork either.

## What is outstanding

Ordered by what unblocks the most.

### 1. Run the app end to end — done 2026-08-18
See above. It found two blocking defects; both are fixed and the first real
dictation delivered. What remains from this item is the pinned log window, a
long dictation, and an installed release build.

### 2. Finish the installer (`apps/bootstrapper`)
The bootstrapper is further along than it looks: the hardware probe, the
resumable digest-verified download, the native wizard, Start Menu shortcuts,
WebView2 provisioning and the uninstaller all exist and work. Three things are
missing.

**The engine smoke test.** Setup must transcribe a short bundled clip and
compare the result against known ground truth, word for word — the decision is
recorded in `docs/ARCHITECTURE.md` under Setup. A speech model whose audio
projector failed to attach does not error; it writes fluent text from the
instruction alone, so "it returned a transcript" proves nothing and only content
does.

> **The clip exists now; the step that runs it does not.** `beckett.wav` was
> gone — not in this repository, not in the parent, not in either git history
> (it was gitignored, so it never entered one), and nowhere else on this
> machine. So was `Obama.wav`. The synthetic clips in `speakeasy-ai-granite`
> are not a substitute: their own generator says they are frequency sweeps
> plus noise, deliberately not speech, carrying no reference transcript.
>
> `scripts/New-SmokeFixture.ps1` generates the replacement with Windows' own
> synthesiser at 16 kHz mono PCM16, and `apps/bootstrapper/fixtures/smoke.wav`
> is committed behind a `.gitignore` exception — 200 KB, so that the clip
> travels with the code that asserts on it rather than being fetched by the
> one step whose whole job is to be trustworthy.
>
> Its ground truth is **verified, not typed**:
>
> ```text
> The quick brown fox jumps over the lazy dog. And Monday begins at dawn.
> ```
>
> Both guesses at it were wrong, which is the argument for
> `cargo run --release -p speakeasy-granite --example transcribe_file`
> existing at all. The first sentence ended "and Granite writes it down" and
> the model returned "Granit". The second was spoken with a comma — "dog, and
> Monday" — and Granite chose "dog. And Monday", a punctuation decision nobody
> would have written from memory. Swept across every thread count
> `recommended_thread_count` can return (1 through 8) and byte-identical at all
> of them, which is what makes a whole-transcript comparison safe on a machine
> whose core count nobody chose.
>
> **What remains is the runner.** Setup still has to load the model, transcribe
> those bytes and compare. The bootstrapper has no worker-protocol client —
> `speakeasy-worker` exposes the adapter but not the process spawning, which
> lives in the desktop crate's `process_worker.rs` — so this needs either a
> small framed-JSON client in `apps/bootstrapper` (and a dependency-policy
> entry for it) or that spawning lifted somewhere both can reach.
> `crates/speakeasy-granite/src/granite_smoke.rs` remains the model for the
> assertion itself: whole transcripts, never a prefix.

**The retention question.** Setup asks whether to keep transcripts between
sessions, default no, and seeds the answer into the profile. The read side is
already built (`SessionTranscriptCoordinator::seed_from_history`), and
`consume_installer_logging_seed` in `composition.rs` is the existing pattern for
handing a setup answer to the app — follow it rather than inventing a channel.

**Recording the installed configuration.** Setup writes down whether it
installed the CPU or the GPU configuration, so the app can tell "running on CPU
because you chose CPU" from "running on CPU because the GPU worker will not
load". Today those are the same silent outcome. This is `docs/ARCHITECTURE.md`'s
"Which provider runs, and how you find out", and it is the reason
`granite_selection` takes `cuda_worker_available` rather than probing for it.

### 3. Publish the CUDA Granite worker — needs the owner
Blocked on two things nobody else can supply: the CUDA Toolkit to build it
(`scripts\Enable-GraniteCuda.ps1` builds one locally) and Hugging Face
credentials to publish it. Target repo `orangeblue39/speakeasy-mini-runtime`,
carrying `granite-worker.exe`, `cudart64_12.dll` and `cublas64_12.dll` as **one
artifact** — a CUDA worker without cudart beside it fails to launch outright,
so they are physically one unit.

Once it exists: add it to `models/trusted-manifest.json` pinned by SHA-256, and
give `download::plan` its second item. That function already takes `provider`
and deliberately ignores it, with a comment saying why, for exactly this.

Until then a GPU machine gets the CPU worker and the app says so honestly.

### 4. The rebrand tail — done 2026-08-18, and it was not cosmetic
The remaining "SpeakEasy" strings were filed as naming. Three of them were
correctness bugs, each one this product writing into the *parent* product's
state, and all three were invisible because the installer had not been run
since the fork.

- **`uninstall::data_root()` returned `%APPDATA%\ai.speakeasy.desktop`.** Setup
  puts the model weights under that root, so a fresh install would have
  downloaded ~2.3 GB into SpeakEasy's data directory and the app — which reads
  `ai.speakeasy.mini`, verified, because that is where a working dictation
  found them — would then have reported Granite as not installed. In the other
  direction it is worse: uninstalling Mini removes this tree, which was
  SpeakEasy's.
- **The ARP key was `…\Uninstall\ai.speakeasy.desktop`**, so setup registered
  Mini over SpeakEasy's Add/Remove Programs entry — its `DisplayName`, version
  and uninstall command — and Mini's uninstaller then deleted it, leaving the
  parent installed and unlisted.
- **`VERSION_KEY` was `Software\SpeakEasy\LocalDevelopment`.** Found while
  fixing the other two, and the same shape one level down: installing Mini
  overwrote SpeakEasy's version stamp, and Mini's downgrade refusal compared
  against whatever SpeakEasy had installed, so the two products could refuse
  each other's upgrades. Its inherited justification — that the key is shared
  with the NSIS hooks so an upgrade finds its predecessor's stamp — cannot
  apply to a product that has never shipped and has no predecessor.

Also renamed: the desktop binary is `ai-speakeasy-mini.exe`, in the cargo
manifest, `install.rs`'s `APP_EXE` and `RUNNING_NAMES`,
`speakeasy-windows::startup`, and the seven scripts that name it. The
refuse-while-running check lost `speakeasy-v2-preview.exe`, which belonged to
the parent's legacy preview and shared its install directory, never Mini's;
keeping it only risked refusing a Mini install because something unrelated was
running.

Two things deliberately left alone. The Rust **crate** names still say
`speakeasy-*`, per the original decision, because renaming them churns every
manifest and path dependency for no observable difference. And the IPC schema
`$id`s still read `ai.speakeasy.desktop/ipc/...`; a schema identifier is
neither user-visible nor a filesystem path, which is the line that decision
drew.

Not verified: no installer has been built or run. The strings are consistent
and the workspace is green, which is not the same as `Test-InstallerLifecycle.ps1`
having passed. That script is the next thing to run against this.

### 5. Dead onboarding plumbing
`OnboardingProgress` and `setup_requirement` still compile with nothing driving
them (10 references under `apps/desktop/src-tauri/src`). The in-app setup wizard
is gone and setup is the installer's job, so these should be removed rather than
left as a flag nothing sets.

### 6. Two comments that outlived their reasoning — done 2026-08-18
Both were found while fixing the launch. One turned out to be a live defect
rather than a stale comment.

**The dictation floor was 4 GiB and Granite's is 8 GiB**, and the split was
deliberate: `runtime_wizard`'s gate is asked before the engine is chosen, so
holding it below Granite's meant a mid-range machine still dictated through the
streaming path and merely declined the second pass. With one engine that split
only bought the user a delay. A machine with, say, 6 GiB passed the gate,
recorded, waited out the whole post-recording pass, and got `GraniteUnavailable`
— after they had already spoken, with nothing to fall back to. The floor is now
Granite's floor. Refusing at `begin`, before a sample is captured, says the same
thing at the only moment it is useful.

The test that pinned the old ordering asserted the dictation floor stayed
*strictly below* Granite's, "or the split is meaningless". It now asserts the
opposite bound — the floor must never sit below Granite's, or a dictation is
admitted that cannot possibly finish — and keeps its original body, which is
still worth having: too little memory must read as "Granite is not part of this
install", never as a fault worth a quarantine strike.

**`run_granite_final_pass`'s doc** said `Ok(None)` let "the ordinary
single-engine fallback" run. There is no fallback. The behaviour was already
right — `judge_granite_pass` maps it to `FinalSourceReason::GraniteUnavailable`,
checked rather than assumed — so this was a comment fix.

## Decisions already made — do not re-open without new evidence

Every one of these was an explicit owner decision this session.

- **One engine, one pass, no fallback.** Granite produces the transcript and its
  punctuation together. A pass that fails, returns nothing, or fails the
  plausibility gate **ends the dictation with a named reason**. Substituting a
  weaker result is the behaviour this fork exists to remove.
- **Failures must be actionable.** Two surfaces: the dock's action row, and a
  panel at the top of Settings → Transcription. A Windows toast was specified,
  then refused — the WinRT route needs an AppUserModelID from an installed Start
  Menu shortcut and otherwise displays nothing *while reporting success*.
- **No provider-override control.** Granite's GPU support is a build feature; no
  setting can conjure a worker binary.
- **The dictation floor is Granite's floor** (8 GiB), raised from 4 GiB on
  2026-08-18. The two were split so a machine that could not host Granite still
  dictated through the streaming path; with one engine that split only let
  someone speak into a guaranteed `GraniteUnavailable`. Refusing before capture
  is the same answer at the only useful moment.
- **No in-app setup wizard.** The installer is the only setup path.
- **`immediate_repetitions` and `self_corrections` never run**, and have no
  toggles.
- **Q4_K_M ships; Q8_0 is catalogued and not install-eligible.** That asymmetry
  is deliberate and is now pinned by a test.
- **The transcript log is pinnable**, into its own non-focusable always-on-top
  window, and retention defaults to clearing on close — implemented by never
  writing rather than deleting on exit, because a delete-on-exit is a promise a
  crash breaks.
- **Distribution is two paths**: a one-exe installer from GitHub Releases, and
  handing the repo to an LLM so a cautious user can watch every step. Both are
  in `README.md` and both must keep working.

## Mistakes made this session, so they are not repeated

- **A whole crate went red unnoticed.** The manifest trim broke
  `speakeasy-models`, and "tests pass" was reported after running three crates
  by name. Run the workspace, not a list.
- **A code was renamed without following it.** Silence changed from
  `runtime_no_speech_detected` to `no_speech`, and three match arms still tested
  the old spelling — so saying nothing took the failure path and earned a
  quarantine strike. `is_no_speech` and a test cover it now. When a stable code
  changes, grep every literal.
- **A replacement test asserted the opposite of the decision.** Rewriting the
  pack test, the first version demanded both Granite packs be installable. Q8_0
  is deliberately not. A test written from the code's shape rather than the
  decision's intent will happily pin the bug.
- **Two edits were lost to a script that raised before writing.** Several
  multi-edit Python passes assert on each replacement and write at the end; when
  a later assertion failed, the earlier successful edits were discarded silently
  and the reported "ok" lines were lies. Write incrementally or verify after.

  **This recurred on 2026-08-18**, in a two-edit pass over
  `Stage-DevRuntime.ps1`, having read the warning above earlier the same
  session. The second assertion failed on a backslash the heredoc had mangled,
  and the first — already applied in memory — went with it. The rule is not
  "be careful"; it is *write after each replacement*, because the failure is
  silent and the transcript still reads as progress.

- **A syntax check reported success without running.** A PowerShell parse check
  passed `[ref]$errs` for an undeclared `$errs`; the statement failed, `$errs`
  stayed null, and the `else` branch printed "parses clean" — for any input,
  including a file that had never been read. The re-run declared the variable
  *and* fed a deliberately broken script through the same check to confirm it
  reported 2 errors. A verification that cannot fail is not a verification, and
  this one had the exact silent-success shape the rest of this project is
  written to avoid.

## Repository facts worth knowing

- Five commits on `main`, private, `kwp490/speakeasy-granite-rust-mini`.
- The tree is ~300 files, down from 2,611 — `vendor/transcribe.cpp` alone was
  2,265 of them.
- `speakeasy-worker` (was `speakeasy-asr`) links **no native libraries** and
  checks in seconds. Only `speakeasy-granite` compiles C++.
- `.cargo/config.toml` is gone. It existed only to point `SHERPA_ONNX_LIB_DIR`
  at a pre-fetched archive. **`.cargo/audit.toml` is back**, though: it was
  collateral damage of deleting that directory wholesale, and the gate needs it.
- `docs/handoff/` and `docs/archive/` were deleted with the streaming engine.
  This file is the new one.
