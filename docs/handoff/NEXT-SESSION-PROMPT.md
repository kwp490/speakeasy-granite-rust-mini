# Prompt for the next session

Copy everything below the line into a new Claude Code session in
`C:\Coding Projects\speakeasy-granite-rust-mini`.

Keep this file to the work that is actually next. It is a pointer into
`docs/handoff/CURRENT.md`, not a second copy of it.

---

You are picking up work on SpeakEasy Mini, a local-only Windows dictation app
(Rust + Tauri 2 + React, all inference on-device) at
`C:\Coding Projects\speakeasy-granite-rust-mini`, branch `main`.

Establish where the branch stands before starting rather than assuming it — the
push state changes with every commit and cannot be written down here:

```powershell
git status -sb
git log --oneline origin/main..HEAD
```

`git status -sb` reports both the working tree and the ahead/behind count in one
line. Anything listed by the second command is unpushed.

## Read first, in this order

1. **`CLAUDE.md`** — orientation. "Traps that fail silently" and "Settled
   decisions" are not optional. Four that bite most work here:
   - dot-source `scripts\Enter-DevEnvironment.ps1` in **every** new shell;
   - a **stale** staged worker fails as `StaleEvent`, which names the protocol
     rather than the binary — re-stage before believing a hardware-test failure;
   - `npm run tauri -- dev` overwrites `target\debug\proof\granite-worker.exe`
     with the CPU worker, silently reverting a staged CUDA one;
   - a control that undoes itself with `git checkout` reverts to **HEAD** and
     destroys uncommitted work. Copy the file aside instead.
2. **`docs/handoff/CURRENT.md`** — "What is open" is the table to act on, and
   "Before the next release" is what a release needs that the tree does not.

## Job 1: run the hardware proofs against `HEAD`

Seven tests are `#[ignore]`d and five of them drive a real worker process. They
last ran on 2026-08-28. `GraniteEngineCoordinator::ensure_ready` changed
signature twice since, and the resident pack began refusing a mismatch. A
signature change that compiles is not a signature change that works.

**Re-stage first.** `target\debug\proof\granite-worker.exe` goes stale between
sessions, and a stale worker fails as `StaleEvent`, which reads as a protocol bug
rather than an old binary.

Six of the seven do not require a CUDA worker: four drive the processor worker
(the three desktop passes and the bootstrapper's engine smoke) and two drive no
worker at all (`registry_hive` writes to the real `HKCU`, `the_real_nvidia`
extracts a pinned NVIDIA archive).

```powershell
. .\scripts\Enter-DevEnvironment.ps1
.\scripts\Stage-DevRuntime.ps1
cargo test -p speakeasy-desktop --lib granite_final_pass -- --ignored --nocapture
cargo test -p speakeasy-bootstrapper smoke -- --ignored --nocapture
cargo test -p speakeasy-bootstrapper registry_hive -- --ignored --nocapture
cargo test -p speakeasy-models the_real_nvidia -- --ignored --nocapture
```

**Use `granite_final_pass`, not `granite`.** The broader filter also selects
`a_cuda_worker_reports_the_device_its_context_probe_can_prove`, which *asserts*
rather than skips when the staged worker is not a CUDA build — so it fails
immediately after `Stage-DevRuntime.ps1` stages the processor one, and that
failure reads like a broken engine rather than the wrong binary.

**The seventh is unrun and needs a CUDA worker staged by hand.**
`Enable-GraniteCuda.ps1` did this and was retired, so there is no script. The
three CUDA libraries are already in `target\debug\proof\`; replace only the
worker, and do it **after** `Stage-DevRuntime.ps1`, which overwrites that path:

```powershell
cargo build --release -p speakeasy-granite-worker --features cuda
Copy-Item target\release\speakeasy-granite-worker.exe `
  target\debug\proof\granite-worker.exe -Force
cargo test -p speakeasy-desktop --lib a_cuda_worker_reports -- --ignored --nocapture
```

`Stage-DevRuntime.ps1` builds the workers `--release` first, so budget about two
minutes for the llama.cpp compile if it is not cached.
`run_granite_final_pass_survives_an_idle_gap_before_a_second_dictation` sleeps
300 s by default — run it deliberately or not at all, and say which. Say which
worker was staged for anything you report.

If a proof fails, the first question is whether the worker was re-staged, not
whether the code is wrong.

## Job 2: a clean clone still does not build

Open and not retried. The frontend half of the gate passes in a fresh clone; the
Rust half fails at `speakeasy-granite` on `llama-cpp-sys-2`'s CMake configure with
`No CMAKE_C_COMPILER could be found`.

The leading explanation is one untested command away: `Enter-DevEnvironment.ps1`
prefers `.tools\cmake-4.4.0-windows-x86_64\bin\cmake.exe` and falls back to `PATH`
**with a warning rather than a failure**. That directory does not exist here, so
every build on this machine has used the system CMake, 4.4.2. Stage 4.4.0 and
re-run. `CURRENT.md` has what to do with either answer.

## Then: pick from "What is open"

Everything else worth doing is in that table, with what each one needs. Two of
them — the model-integrity threat model and what a non-delivering path should
inspect — are decisions rather than code, and are yours to make rather than an
agent's.

## How to work

- **Run the whole gate**, not `cargo test --workspace --lib`:
  `.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall`. It must exit 0.
- **Report counts including ignored**, and name which ignored tests did not run.
  "0 failed" is not a pass condition; a suite that got smaller reads the same.
- **Prove every new test can fail** by restoring the *real* defect. A control that
  does not go red has verified nothing.
- **Do not start a release** without reading "Before the next release" in
  `CURRENT.md`: the version is already tagged, so a build from `main` as it stands
  cannot install over the published one.
