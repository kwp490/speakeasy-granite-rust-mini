# Prompt for the next session — run the hardware proofs, then decide about pushing

Copy everything below the line into a new Claude Code session in
`C:\Coding Projects\speakeasy-granite-rust-mini`.

> **Superseded 2026-08-30.** The previous version staged the clean-clone build
> failure. That is still open and is now the *second* job here rather than the
> first, because four unpushed commits changed the engine's warm path and the
> only tests that exercise a real worker have not run against them. The version
> before that staged Phase 3.5 of the phased-fixes brief, which is finished.

---

You are picking up work on SpeakEasy Mini, a local-only Windows dictation app
(Rust + Tauri 2 + React, all inference on-device) at
`C:\Coding Projects\speakeasy-granite-rust-mini`, branch `main`.

## Where the tree is

Commits are on `main` that **have not been pushed**. `origin/main` was at
`a34adc3` when this was written. Ask git rather than trusting the list below,
which cannot include the commit that updates it:

```powershell
git log --oneline origin/main..HEAD
```

They were held for review on purpose. The four that change code:

| commit | what it is |
| --- | --- |
| `effd467` | Four defects from `a34adc3`: the warm verdict was read from shared state; `AlreadyLoaded` certified the requested pack while returning the resident one; the dock and the shortcut disagreed about refusing a dictation; two tests reported success they had not earned |
| `1f1dd38` | Ten more settings actions can report a refusal; a vitest/jsdom component harness; per-file coverage floors |
| `ae79567` | A mismatched resident pack refuses instead of running; the `HKCU` proof leaves the default gate; the transcript list stops calling itself session-only; the comment convention changed |
| `0adbd2f` | The transcript list stops polling (event-driven); Audio makes one call where it made two; the install poll is self-scheduling |

The full gate passes: **444 Rust tests, 0 failed, 7 ignored**;
frontend **72 unit + 22 component, 0 failed**; coverage floors pass.

## Read first, in this order

1. `CLAUDE.md` — orientation. "Traps that fail silently" and "Settled decisions"
   are not optional. Four that bite *this* work specifically:
   - dot-source `scripts\Enter-DevEnvironment.ps1` in **every** new shell;
   - a **stale** staged worker fails as `StaleEvent`, which names the protocol
     rather than the binary — re-stage before believing a hardware-test failure;
   - `npm run tauri -- dev` overwrites `target\debug\proof\granite-worker.exe`
     with the CPU worker, so it silently reverts a staged CUDA one;
   - a control that undoes itself with `git checkout` reverts to **HEAD** and
     destroys uncommitted work. Copy files aside instead.
2. `docs/handoff/CURRENT.md` — "What is open" is the table to act on. Items 19–28
   are current state; items 0–18 are history.
3. `CLAUDE.md` "Conventions" — the comment rule changed on 2026-08-30. State the
   current invariant concisely; no dated defect narratives or session diaries in
   production source or in `CURRENT.md`.

## Job 1: run the hardware proofs against the new warm path

**This is the reason the commits are unpushed.**
`GraniteEngineCoordinator::ensure_ready` changed signature twice — it gained a
`verifier` parameter and now returns `EnsureReadyOutcome` instead of a `Result` —
and every test that drives a real worker process was edited without being run.
Five of the seven ignored tests are those proofs. A signature change that
compiles is not a signature change that works.

Everything they need is already on this machine:

- `target\debug\proof\granite-worker.exe` — **4,333,056 bytes, dated 2026-08-28.
  That is the CPU worker and it is two days old.** Re-stage it before running
  anything (`scripts\Stage-DevRuntime.ps1`), or a `StaleEvent` will read as a
  code fault.
- `target\debug\model-lifecycle\models\granite-speech-4.1-2b-q4_k_m-cpu\q4_k_m-2026-05-11\`
  — both GGUFs, present.
- `apps\bootstrapper\fixtures\smoke.wav` — committed.

```powershell
. .\scripts\Enter-DevEnvironment.ps1
.\scripts\Stage-DevRuntime.ps1
cargo test -p speakeasy-desktop granite -- --ignored --nocapture
cargo test -p speakeasy-bootstrapper smoke -- --ignored --nocapture
```

`Stage-DevRuntime.ps1` builds the workers `--release` first, so budget about two
minutes for the llama.cpp compile if it is not cached.

`run_granite_final_pass_survives_an_idle_gap_before_a_second_dictation` sleeps
for 300 s by default — run it deliberately or not at all, and say which.

The other two ignored tests are runnable here as well, so the whole set of seven
can be cleared in one sitting:

```powershell
cargo test -p speakeasy-bootstrapper registry_hive -- --ignored --nocapture
cargo test -p speakeasy-models the_real_nvidia -- --ignored --nocapture
```

`registry_hive` writes to the real `HKCU` under a pid-suffixed key of its own and
fails rather than skipping if the hive is unwritable. The NVIDIA one needs
`.tools\cuda-runtime\download`, which is populated on this machine — both
archives are there.

**Assert whole transcripts, never a prefix.** A `contains("ever tried")`
assertion once went green on a transcript missing a third of the utterance.

If a proof fails, the first question is whether the worker was re-staged, not
whether the code is wrong.

## Job 2: the two open review questions

Both are judgement calls made in `ae79567` that a reviewer may want reversed.

1. **`history_delete_all` now clears the in-memory transcript list.** The list is
   seeded at launch from the optional on-disk history, so before this a
   delete-all emptied the database and left those entries readable *and copyable*
   in the window the user deleted them from. Clearing them is one of the two
   options — the other was removing the seeding entirely, which would make the
   list genuinely session-only and lose the retention feature's read side.
2. **The comment convention trims rather than sweeps.** `ae79567` condensed only
   what the previous two commits added. Roughly forty long-form narratives
   elsewhere in the tree are untouched. If they should go, that is its own
   commit and its own review.

## Job 3: a clean clone still does not build

Open since 2026-08-28 and **not retried since**. `git clone` of `main` into an
empty directory: the frontend half of the gate passes completely; the Rust half
fails at `speakeasy-granite` on `llama-cpp-sys-2`'s CMake configure with
`No CMAKE_C_COMPILER could be found`, then the same for CXX. It reproduces with
the sandbox off and inside an `Enter-VsDevShell` Developer shell where `cl.exe`
is on `PATH`.

The leading explanation is one untested command away. `Enter-DevEnvironment.ps1`
prefers `.tools\cmake-4.4.0-windows-x86_64\bin\cmake.exe` and falls back to
whatever is on `PATH` **with a warning rather than a failure**. That directory
does not exist on this machine, so every build here has used the system CMake
**4.4.2**. Stage 4.4.0 and re-run.

If 4.4.0 fixes it, the warning is the real defect: a build dependency that
silently degrades to a different version is the same shape as
`Stage-DevRuntime.ps1` reverting a staged CUDA worker — correct-looking output,
wrong binary, an error naming something else. Decide whether the fallback should
refuse.

If it does not fix it, do not guess. Read the CMake error log and amend
`docs/NEW-MACHINE.md`, which currently describes prerequisites this failure
appears to contradict.

## How to work

- **Run the whole gate**, not `cargo test --workspace --lib`:
  `.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall`. It must exit 0.
- **Report counts including ignored**, and name which ignored tests did not run.
  "0 failed" is not a pass condition; a suite that got smaller reads the same.
- **Prove every new test can fail** by restoring the *real* defect. A control
  that does not go red has verified nothing — one in `0adbd2f` passed on the
  first attempt because a `.catch` above the change already swallowed the
  rejection.
- **Do not push** until the review questions above are answered.
