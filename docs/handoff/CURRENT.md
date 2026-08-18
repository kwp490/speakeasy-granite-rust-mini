# Handoff — SpeakEasy Mini, as of 2026-08-18

The state of the fork, what is finished, what is not, and the things that will
cost you an afternoon if you rediscover them yourself.

Read `CLAUDE.md` first. This file assumes it.

## Where the project is

SpeakEasy Mini was forked from SpeakEasy in one session and reduced to two
changes that cascade through everything: **Granite is the only engine**, and
**the dock is the only HUD**. Four commits, on a fresh history, pushed to a
private repository.

Verified state, and the exact commands that produce it:

```powershell
. .\scripts\Enter-DevEnvironment.ps1
cargo test --workspace --exclude speakeasy-granite --exclude speakeasy-granite-worker --lib
cargo clippy --workspace --exclude speakeasy-granite --exclude speakeasy-granite-worker --all-targets
```

293 Rust tests pass, clippy is silent. From `apps/desktop`, `npm test`,
`npm run lint` and `npm run typecheck` are clean — 55 frontend tests.

**The exclusions are load-bearing and are also a gap.** `speakeasy-granite`
compiles llama.cpp from source, so the full gate is slow; it has been excluded
from every check this session, which means *it has not been compiled since the
fork*. It should build — nothing in it changed — but that is a prediction, not
a measurement. Run `.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall` once
and find out before trusting anything downstream of it.

## The largest gap: nothing has been run

Every claim above is static verification. **The app has never been launched in
this project.** No dictation has been performed, the dock has never been on
screen, and the pinned log window has never been opened.

That is the highest-value next step and it should come before the installer,
because it is the cheapest way to find out whether the amputation broke
something no test covers. Specifically worth watching for:

- The dock appearing at all, and at the right size — `configure_hud` now shows
  it unconditionally where the old code only showed it in dock mode.
- Whether `capture_hud_status` survives the startup race. It polls at 10 Hz
  from a window that loads before `setup` finishes, and the coordinator set it
  reads changed this session.
- A real dictation end to end, and what the dock's action row says when it
  finishes.
- The pinned log opening, staying on top, and **not stealing the foreground** —
  if the next dictation after pinning lands on the clipboard instead of in your
  document, that window took focus and the whole delivery-target trap has
  reappeared.

## What is outstanding

Ordered by what unblocks the most.

### 1. Run the app end to end
Above. Nothing depends on it, and everything is more trustworthy after it.

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

> **The trap here is packaging, not inference.** `*.wav` is in `.gitignore`, and
> the existing fixture (`beckett.wav`, ground truth "Ever tried. Ever failed. No
> matter. Try again. Fail again. Fail better.") lives in `.tools/fixtures/`,
> which is not committed. So there is currently **no clip the installer could
> embed**. Either add a `.gitignore` exception for one committed fixture and
> `include_bytes!` it, or fetch it with the weights and verify it by digest like
> everything else. Decide this before writing the step.
>
> `crates/speakeasy-granite/src/granite_smoke.rs` is the model to copy — it
> already asserts whole transcripts against that exact fixture.

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

### 4. The rebrand tail
15 files under `scripts/` still say "SpeakEasy", and the desktop binary is still
`ai-speakeasy-desktop`. The decision was to rename everything the user or the
filesystem sees — scripts, binary name, installer strings, install root, log
paths — and to leave the Rust **crate** names alone, because renaming
`speakeasy-desktop` and friends churns every manifest and path dependency for no
observable difference.

### 5. Dead onboarding plumbing
`OnboardingProgress` and `setup_requirement` still compile with nothing driving
them (10 references under `apps/desktop/src-tauri/src`). The in-app setup wizard
is gone and setup is the installer's job, so these should be removed rather than
left as a flag nothing sets.

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

## Repository facts worth knowing

- Four commits on `main`, private, `kwp490/speakeasy-granite-rust-mini`.
- The tree is ~300 files, down from 2,611 — `vendor/transcribe.cpp` alone was
  2,265 of them.
- `speakeasy-worker` (was `speakeasy-asr`) links **no native libraries** and
  checks in seconds. Only `speakeasy-granite` compiles C++.
- `.cargo/config.toml` is gone. It existed only to point `SHERPA_ONNX_LIB_DIR`
  at a pre-fetched archive.
- `docs/handoff/` and `docs/archive/` were deleted with the streaming engine.
  This file is the new one.
