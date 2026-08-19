# Handoff — SpeakEasy Mini, as of 2026-08-19

The state of the fork, what is finished, what is not, and the things that will
cost you an afternoon if you rediscover them yourself.

Read `CLAUDE.md` first. This file assumes it.

## Start here

**Read in this order:** `CLAUDE.md`, then this file, then `docs/ARCHITECTURE.md`
if you need the shape of the system.

**Prove the tree is where this file says it is**, before believing anything
below. Two commands, about six minutes cold:

```powershell
. .\scripts\Enter-DevEnvironment.ps1
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
```

That should end with `no leaks found` and exit 0. If it does not, something has
changed since 2026-08-18 and the rest of this file is describing a tree you do
not have.

**What is true as of the last commit**, all of it measured rather than
predicted:

| | |
| --- | --- |
| Full gate | passes end to end |
| A real dictation | delivered, `hotkey_delivery result=committed` (2026-08-18) |
| Installer lifecycle | `Test-InstallerLifecycle.ps1` passes, against a built installer |
| Setup's engine check | transcribes the bundled clip through the real worker in ~5 s |
| `speakeasy-granite` | compiles, ~2 min cold |
| Broken doc links | none, `--document-private-items` and denied, workspace-wide |
| Branch | `main`, pushed to `kwp490/speakeasy-granite-rust-mini` |

**One thing is left of the four this file used to list.** The other three landed
on 2026-08-19; see "What happened on 2026-08-19" below for what each turned out
to involve, which was in every case more than the entry described.

1. **Finish setup's seed channel** — the whole of it, decided 2026-08-19, not
   just the retention question the old entry named. `apps/bootstrapper` writes
   **no** config seed at all today, while `apps/desktop` already reads two:
   `consume_installer_logging_seed` and its hotkey sibling in
   `commands/dictation.rs` look for `config/install-logging.txt` and
   `config/install-hotkey.txt`. Nothing writes either, so the shortcut a user
   picks in setup and the logging choice they make are **collected and silently
   discarded**. That is the same read-side-complete, no-writer shape that left
   `smoke.rs` unbuilt behind a comment promising it existed.

   So the work is one writer in the bootstrapper, used three times:

   - **the shortcut**, which setup already collects and throws away;
   - **the logging choice**, likewise;
   - **the retention question**, which is new. Setup asks whether to keep
     transcripts between sessions. **Default off, framed as privacy** (owner
     decision, 2026-08-19): "Transcripts are kept only while the app is open",
     preselected. That matches what the app already does — retention clears on
     close by never writing rather than deleting on exit, so a crash cannot leak
     them — and a privacy-preserving default needs no justification to the user.
     It needs a `consume_installer_retention_seed` on the desktop side; follow
     `consume_installer_logging_seed` exactly rather than inventing a channel.

   Seeds are one-shot: the file carries `0` or `1` and is deleted after reading,
   so a later change by the user always wins. A new wizard control is needed for
   the retention question — the wizard has `gui::Label`, `gui::Button` and
   `gui::ProgressBar` today and no checkbox, and **every control must be created
   in `Wizard::new`**, because `winsafe` panics if one is created after its
   parent window.

   **Recording which configuration was installed** belongs in the same pass and
   is the smaller half: setup does not record CPU-versus-GPU anywhere, and the
   app needs it to tell "a CPU install running on CPU", which is normal, from "a
   GPU install that cannot load CUDA", which is an error.

**Two things need you rather than an agent**: publishing the CUDA worker
(item 3, needs the CUDA Toolkit and Hugging Face credentials), and any decision
about what setup *says*, since its copy is reviewable by rule.

**Before running the installer lifecycle test**, kill any `ai-speakeasy-mini`.
An aborted run leaves the app it launched for the running-app check alive, and
the pre-flight guard then refuses every retry.

## What happened on 2026-08-19

Six commits, `e03eb78`..`da612fa`. Three of the four "most worth doing next"
entries above are gone; each was larger than its entry said, and the overrun was
the same shape every time — **the entry described the symptom someone had
noticed, not the condition underneath it.** Budget for that on the fourth.

### The citation sweep (`e03eb78`) — 3× the recorded size

Recorded as "25 comments across 21 files cite deleted handoff documents". It was
**35 citations across 27 files naming six deleted docs**, because the original
count searched three of the six names. None of the six was ever in this
repository's history, so most citations had nothing to be rewritten *against*.

Three citation classes exist, and a sweep that finds one looks finished:

- the **filename** (`granite-final-pass.md`) — greppable;
- the **bare number** (`§9.4`, `Phase 6`) — 96 and 22 of them, and the larger
  half of the debt. `docs/UI-GUIDE.md` does not number its headings, so nothing
  could be carried across;
- the **prose** ("the handoff", "the brief", "the GPU migration handoff, item
  14") — matches no grep for a path or a `§`.

A fourth trap: the first sweep filtered `*.ts`/`*.tsx` and the scaffold suite is
`.mjs`, which hid 21 more. **Enumerate extensions before believing a zero.**

Three defects fell out that no citation sweep was looking for:

- `speakeasy-granite`'s crate doc claimed the delivered transcript came from the
  *streaming* model run twice. It has not since the fork.
- **Eight invisible U+009D control characters** in comments across five files,
  present since the first commit, each following an em-dash. Found only because
  a scripted replacement refused to match a line identical on screen.
- **Three broken rustdoc links**, which `cargo doc` had never reported —
  see `CLAUDE.md`'s entry on `--document-private-items`.

### `install_root` (`21f2884`) — the value was already right

The recorded defect was the leaf, and the leaf had already been fixed. What
nothing had noticed is that the **fallback** returned `C:\` when `LOCALAPPDATA`
was unset: setup would have unpacked into the drive root, registered it as the
install location, and uninstall would then have walked `C:\`. Worse than the
recorded bug, and reachable from the same function.

It returns `Option<PathBuf>` now and the three writing callers refuse. Empty
counts as absent too — `PathBuf::from("").join(PRODUCT)` is a bare relative path.

**A test cannot set `LOCALAPPDATA`.** Edition 2024 under `unsafe_code =
"forbid"` makes `std::env::set_var` unsafe, so the decision lives in
`install_root_under`, which takes the environment as an argument. Any future
env-dependent decision worth pinning needs the same split.

### The two dead subsystems (`fa49173`)

Onboarding was recorded as "10 references"; it was 16 across 7 files **plus a
persisted settings field, an IPC view field, a registered Tauri command, and a
validation gate that could reject a whole settings file**. `setup_requirement`
was listed with them and is *live* — it is on the 10 Hz HUD poll.

Removing a persisted field is safe here because `Settings` has a
`#[serde(default, flatten)]` catch-all, so an older profile's `onboarding`
object lands in `extensions` and is written back. **That is pinned, not
assumed**: delete the catch-all and the migration test fails.

`proof-mode` went entirely. Its own `main.rs` comment already said no script
built it, and `run_phase2_installed_smoke` resolved a `StreamingAsr` pack that
cannot exist. Removing it also took a `not(feature = "proof-mode")` arm off the
release `windows_subsystem` attribute — an opt-out nobody could select, on the
one attribute deciding whether the shipped binary allocates a console and steals
the foreground from delivery.

### Setup's engine check (`f5e951a`, `0606275`, `da612fa`)

The spawn went to **`speakeasy-windows`**, not `speakeasy-worker`. Both are
reachable from both crates, but the spawn's hard parts — job-object ownership
and `CREATE_NO_WINDOW` — are Windows concerns already living there, and
`speakeasy-worker` depends on `speakeasy-domain` alone and checks in seconds.
`append_diagnostics_line` moved with it: it is the single redacting boundary for
diagnostics and `worker_process` writes through it, so leaving it behind would
have split that boundary across two crates.

The scaffold assertion pinning `CREATE_NO_WINDOW` followed and got stronger — it
now asserts the flag appears in exactly one place workspace-wide. Writing that
check immediately found a second `creation_flags` call, the bootstrapper's
`relaunch_detached`, which sets `DETACHED_PROCESS` for a different job. **The
rule names the constant, not the method**, so it does not sweep that up.

**The check compares words, and that is measured rather than cautious.** The
clip says

> The quick brown fox jumps over the lazy dog, and Monday begins at dawn.

and Granite `Q4_K_M` returned, 2026-08-19,

> The quick brown fox jumps over the lazy dog. And Monday begins at dawn.

A period for a comma, a capital for a lowercase, every word right. **An
exact-transcript comparison would have refused a working install.** The verbatim
pin stays in `granite_worker_smoke.rs`, where a change is a developer's finding
rather than a blocked user.

A failed check does not block the install (owner decision): Retry, with Continue
and Cancel both live, and copy that says what continuing costs. The clip is
`include_bytes!`'d rather than staged — the bootstrapper is one executable, and
that removes the "clip missing" verdict entirely.

`smoke.rs` also settled an open promise: `New-SmokeFixture.ps1` had claimed
since it was written that its sentence was "kept here and in `smoke.rs` — and
checked against each other by the bootstrapper's own test". Neither existed.

### How the engine check is proven, and how to re-run it

Not by its unit tests. `the_real_engine_transcribes_the_bundled_clip` spawns the
real worker, loads the real model and transcribes the committed clip in ~5 s. It
is `#[ignore]`d for hardware. Two env overrides let it run against an existing
install rather than a second copy of ~2 GB:

```powershell
$env:SPEAKEASY_GRANITE_WORKER = (Resolve-Path 'target\release\speakeasy-granite-worker.exe').Path
$env:SPEAKEASY_GRANITE_MODEL_ROOT = '<a directory holding both GGUFs>'
cargo test -p speakeasy-bootstrapper --offline the_real_engine -- --ignored
```

Both controls were run and both fail as they should: a nonexistent model root
gives `model_did_not_load`, and expecting a sentence the clip does not say gives
`Mismatch` carrying the real transcript. **Re-run the controls if you change the
comparison** — a smoke test that cannot fail is the exact thing this step exists
to prevent elsewhere.

On this machine the GGUFs are under the **parent** product's data directory
(`ai.speakeasy.desktop`), left by an earlier install. `.tools/` is empty.

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

## The installer has now been built and run

`Test-InstallerLifecycle.ps1` passes end to end, which is the first
behavioural evidence about setup on this fork. Getting there took thirteen
fixes, and the shape of them is the lesson: **the fork updated every path it
executed and left every path it did not.** The dev launcher, the quality gate,
the dependency policy, the packager, the installer builder and the install
proof were all in the second category, and each one hid the next because they
fail fast.

Four were real product defects rather than harness debris, and all four were
this product writing into the parent's state:

- **`probe::install_root()` defaulted to `%LOCALAPPDATA%\SpeakEasy`.** Setup
  would have written this app's executables over an existing SpeakEasy
  installation, and `uninstall` removes the install directory whole -- so
  uninstalling Mini would have deleted SpeakEasy. `shortcut::start_menu_folder`
  had the same collision.
- **Add/Remove Programs showed `DisplayName: SpeakEasy`**, so the two products
  were indistinguishable in the list a user goes to in order to remove one.
- **The wizard called itself SpeakEasy throughout** -- 22 of 23 user-facing
  strings in `catalog.rs`, plus the window title and the repair message box.

What the passing run proves, precisely: fresh install places both executables;
refuse-while-running refuses and leaves the installed binary's SHA-256
unchanged; refuse-same-version and refuse-downgrade both fire with their own
messages; repair backup and verify round-trip; and a silent uninstall leaves
the install root and both registry keys clean, with `HKCU:\Software\SpeakEasy`
untouched throughout.

### What that proof does not cover

- **`install_root()`'s default is never exercised.** The test passes
  `--install-root` explicitly, so the worst defect above lived in a code path
  the proof does not touch. Nothing pins that leaf to the product identity, so
  it can regress in silence. A test asserting it is the cheapest insurance
  available and does not exist.
- **No model download and no transcription.** The engine smoke test still has
  no runner; the clip and its verified ground truth are committed and unused.
- **Nothing asserts the ARP strings.** The proof checks the key is created and
  removed, not what is in it. That is exactly how `DisplayName: SpeakEasy`
  survived.
- **Uninstall leaves an empty `HKCU:\Software\SpeakEasy Mini`.** It removes the
  `LocalDevelopment` subkey the test asserts on, not the now-empty parent.
  Cosmetic, no data, deliberately left alone: deleting a parent key should be a
  decision rather than a tidy-up.

### A rough edge in the harness

An aborted lifecycle run leaves the app it launched for the running-app check
alive. The pre-flight guard then refuses every retry -- correctly, it will not
terminate a process it does not own -- but the orphan is the script's own, and
it cost three runs to notice. Kill `ai-speakeasy-mini` before re-running.

## What is outstanding

Ordered by what unblocks the most.

### 1. Run the app end to end — done 2026-08-18
See above. It found two blocking defects; both are fixed and the first real
dictation delivered. What remains from this item is the pinned log window, a
long dictation, and an installed release build.

### 2. Finish the installer (`apps/bootstrapper`)
The bootstrapper is further along than it looks: the hardware probe, the
resumable digest-verified download, the native wizard, Start Menu shortcuts,
WebView2 provisioning and the uninstaller all exist and work. The engine smoke
test joined them on 2026-08-19; the seed channel is what is left.

**The engine smoke test — done 2026-08-19.** Built, wired into the last step,
and proven against the real worker and model. See "What happened on 2026-08-19"
above for how it is re-run and why it compares words rather than the transcript
verbatim. The original entry follows, kept because its reasoning is why the
step exists at all: a speech model whose audio projector failed to attach does
not error, it writes fluent text from the instruction alone, so "it returned a
transcript" proves nothing and only content does.

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

Since verified: an installer was built and `Test-InstallerLifecycle.ps1` was
run against it. It found three more collisions this section had not — the
default install root, the Start Menu folder and the Add/Remove Programs
`DisplayName` — all recorded above under "The installer has now been built and
run". The lifecycle test now passes.

### 5. Dead onboarding plumbing — done 2026-08-19
Larger than the 10 references recorded here: 16 across 7 files, plus the
persisted `OnboardingProgress`, its profile IPC field, the `onboarding_advance`
command, and a `current_step > 7` validation that could reject a settings file.
All gone. `setup_requirement` was listed with them and **is live** — it is on
the 10 Hz HUD poll and answers whether this profile can dictate right now.

The persisted field was safe to drop because `Settings` has a
`#[serde(default, flatten)]` catch-all; the migration test pins that rather
than assuming it.

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

### 7. Documentation debt the fork left behind
Found while bringing the markdown up to date on 2026-08-18. None of it breaks a
build; all of it misleads a reader.

- **Named citations of deleted docs: done on 2026-08-19.** 35 of them across 27
  files, not the 25 across 21 recorded here — the original count searched three
  deleted docs and there were **six**: `granite-final-pass.md`,
  `setup-wizard-redesign.md`, `hud-side-dock.md`, `migrate-to-nvidia-gpu.md`,
  `transcribe-cpp-benchmark.md` and `docs/archive/UI-REDESIGN.md`. **None of the
  six was ever in this repository's history** (`git log --all` finds no commit
  touching any of them), so there was nothing to rewrite most citations
  *against*: each one either dropped, or absorbed into the sentence the fact it
  had been carrying. Four were rewritten against something that still exists —
  three proof scripts now cite `docs/UI-GUIDE.md`, and `speakeasy-worker`'s
  `BatchFinalPass` now points at `admissible_delivered_transcript` for the
  `NoSpeechDetected` split. Bulk `sed` would have mangled these: the citation is
  fused into the sentence in most of them.
- **Nemotron licence material: deleted on 2026-08-19.**
  `models/huggingface/nemotron-3.5-streaming-en-cuda/` (README, NOTICE, licence,
  checksums) and `packaging/licenses/OpenMDW-1.1.txt`. Nothing bundled or copied
  either — no packaging script touches `packaging/licenses/` or
  `models/huggingface/` — and the fork distributes no NVIDIA model bytes, so no
  licence obligation attached to keeping the text. Both are recoverable from git
  if that judgement is ever revisited. Two further Nemotron residuals went with
  them: `crates/speakeasy-models/examples/phase2_current_host.rs`, which
  resolved the removed `nemotron-3.5-streaming-en-cpu` pack id and so could
  never have succeeded since the fork, and the dead `nemotron_3_5_streaming`
  row in `catalog.ts`'s display-name table.
- **The rest of it went on 2026-08-19 too.** All four remaining categories, and
  three defects found while doing it. `grep` for any of these now returns zero:
  - **96 bare `§N` references across 30 files**, which pointed at
    `UI-REDESIGN.md`'s section numbering. `docs/UI-GUIDE.md` does not number its
    headings, so a number could not be carried across; each was mapped to a
    named heading (`UI-GUIDE "Information architecture"`, matching the one
    pre-existing citation of that form in `styles.css`) or dropped where it was
    pointing at implementation rather than spec. **Every heading cited this way
    is checked to exist** — replacing a dead reference with a new dead reference
    is the failure mode to avoid here.
  - **22 bare `Phase N` references across 6 files**, pointing at
    `granite-final-pass.md`'s phases. Dropped, or replaced with the fact the
    phase number was standing in for (`Phase 9` → `2026-08-04`, `Known risk #12`
    → "the stale-clock deadline bug").
  - **6 prose references** — "the handoff", "the brief", "the GPU migration
    handoff, item 14" — which no grep for a filename or a `§` would have found.
    Worth knowing that this class exists before believing a citation sweep is
    complete.
  - **51 references to deleted crates and files**, of which **12 remain
    deliberately.** Every survivor is *history* that reads as history:
    "It was `speakeasy-asr`, and it did link one", "the crate was renamed when
    the streaming engine left", "were listed here until the fork removed the
    engine". Those are correct and load-bearing — rewriting them would make them
    wrong. What was fixed is the ~39 that made **present-tense claims** about
    things that no longer exist.
- **Three defects found while doing that pass**, none of which a citation sweep
  was looking for:
  - **`speakeasy-granite`'s crate doc claimed the wrong engine shipped.** Its
    "What it is for" section said the delivered transcript came from the
    *streaming* model run a second time over the retained audio. Corrected to
    what the fork actually does.
  - **8 invisible U+009D control characters** in comments across 5 files, each
    an em-dash followed by a stray byte from some encoding round-trip, all
    present since the first commit. They render as nothing and survive review.
    Found only because a scripted replacement refused to match a line that
    looked identical on screen — the failure was the instrument working.
  - **A doc comment attached to the wrong item.** In
    `workers/granite-worker/tests/granite_worker_smoke.rs`, the 11-line
    description of the residency proof sat on the `run_dictation` *helper*
    below it, so the helper's own three-line description was fused onto the end
    of it and the test itself had none. Reattached.
- **The shipped notices were describing a different product.**
  `THIRD-PARTY-NOTICES.txt` declared sherpa-onnx, ONNX Runtime and the CUDA
  redistributables as bundled, and `MODEL-NOTICES.md` described two Nemotron
  packs as the models in use. Both are corrected — a legal notice claiming to
  distribute something it does not is worse than one that is merely stale — but
  they are worth re-reading whenever the payload changes, because nothing checks
  them against `Build-LocalInstaller.ps1`'s actual output.

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

- **Three checks reported clean because they were broken, not because
  anything passed.** A PowerShell parse check passed `[ref]$errs` for an
  undeclared variable, so it printed "parses clean" for any input, including
  a file it had never read. A wait loop grepped for `error` and matched
  `thiserror`, reporting a build finished while it was still compiling. A
  `Win32_Process` filter used URL-encoded quotes, matched nothing, and was
  read as "no instance running" -- a `Get-Process` a second later found one.
  Each was caught only by looking at something else afterwards. **An
  instrument that fails silently is indistinguishable from one that passes**,
  which is the failure this codebase's comments are mostly about, so run the
  control *first* rather than after being surprised.

- **A regression was reported that was not one.** Two window measurements
  taken during a slower cold start showed the dock hidden, focusable and the
  wrong width; both were mid-startup, before `configure_hud` had run. The
  settled state was correct. Measure after the thing has settled, and say
  "not yet settled" rather than "regressed" when it has not.

- **A failure was diagnosed twice from a line number.** The installer
  lifecycle error pointed inside a helper called three times, and it was read
  as the first call. The first call had succeeded; the failure was two calls
  later, under different conditions. A line number inside a shared helper does
  not say which invocation.

### 2026-08-19

- **A bulk edit cut the wrong lines, twice, and only a dry run caught it.**
  A script deleting whole Rust items ended each cut at "the first line equal to
  `}`", which for any item with a nested block is an *inner* brace: it removed
  the first third of two functions and left the remainder dangling. The second
  attempt matched multi-line byte strings and silently found nothing, because
  the files are CRLF and the patterns were LF. Both were caught by printing the
  intended cut before writing, not by reading the result afterwards. **A
  structural edit needs a dry run and brace counting**; line-shape heuristics
  are guesses about syntax.

- **A reflow pass churned comments nobody had edited.** Rewrapping every ragged
  paragraph in a touched *file* rewrote text the change never went near, which
  buries the real diff. Scope a formatting sweep to paragraphs containing a line
  the diff actually added. Related: a short comment line is only a defect when
  it is **not** the paragraph's last line — a one-line doc comment is supposed
  to be short, and a detector that misses that flags dozens of false positives.

- **`repr()` is not a way to count backslashes.** Repairing a mangled JS regex,
  a Python `repr` showing `/\\/g` was read as two backslashes when it is one, so
  the "fix" replaced the broken form with itself and reported success. The file
  still had a syntax error. Build such strings from `chr(92)` and assert on the
  count, or check with something that is not itself escaping the output.

- **A heredoc is the wrong tool for a file full of quotes.** Two attempts to
  write `smoke.rs` through a shell heredoc died on its own quoting before a byte
  reached disk. The dedicated file-writing tool took it unchanged. Reach for the
  shell for edits, not for authoring source that is dense in `"` and `\`.

- **New assertions found real things immediately, which is the argument for
  writing them.** "The flag appears in exactly one place" found a second
  `creation_flags` call in its first run. "The generator's sentence matches the
  constant" existed only as a promise in a comment. Neither was hypothetical.

- **Three scoping questions were answered by looking rather than guessing**, and
  every one changed the work: `install_root`'s recorded defect was already
  fixed and a worse one sat beside it; onboarding reached persisted settings and
  an IPC contract; the retention question turned out to be one third of a seed
  channel with no writer. **Cost a few minutes of grep each.** The pattern in
  this file is that an outstanding entry describes the symptom someone noticed,
  not the condition under it.

## Repository facts worth knowing

- 21 commits on `main`, private, `kwp490/speakeasy-granite-rust-mini`.
  Six of them are 2026-08-19's, `e03eb78`..`da612fa`.
- The tree is ~300 files, down from 2,611 — `vendor/transcribe.cpp` alone was
  2,265 of them.
- `speakeasy-worker` (was `speakeasy-asr`) links **no native libraries** and
  checks in seconds. Only `speakeasy-granite` compiles C++.
- `.cargo/config.toml` is gone. It existed only to point `SHERPA_ONNX_LIB_DIR`
  at a pre-fetched archive. **`.cargo/audit.toml` is back**, though: it was
  collateral damage of deleting that directory wholesale, and the gate needs it.
- `docs/handoff/` and `docs/archive/` were deleted with the streaming engine.
  This file is the new one.
