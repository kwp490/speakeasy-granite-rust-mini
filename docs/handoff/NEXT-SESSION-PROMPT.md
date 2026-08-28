# Prompt for the next session — Phase 3.5

Copy everything below the line into a new Claude Code session in
`C:\Coding Projects\speakeasy-granite-rust-mini`.

> **Superseded 2026-08-28.** The previous version staged item 3, publishing the
> CUDA worker. That landed on 2026-08-26 (`e0278cc`). Everything the phased-fixes
> brief carries is now done except **Phase 3.5**, which is what this prompt is
> for.

---

You are picking up work on SpeakEasy Mini, a local-only Windows dictation app
(Rust + Tauri 2 + React, all inference on-device) at
`C:\Coding Projects\speakeasy-granite-rust-mini`, branch `main`.

**This session is Phase 3.5 of `C:\Users\User\OneDrive\PHASED-FIXES.md`:
`engine=` reads as a fault in the log people actually open.** It is the only
phase in that file still open, and it is confirmed open rather than assumed —
`cpu_gpu_runtime_missing` is a live reason code. The phase describes itself as
ten minutes, one file, no code; treat that as the author's estimate rather than
a promise, because the last three phases were each larger than their brief said.

## Read first, in this order

1. `CLAUDE.md` — orientation. "Traps that fail silently" and "Settled decisions"
   are not optional. Three that bite this work specifically: dot-source
   `scripts\Enter-DevEnvironment.ps1` in every new shell; run the whole gate
   rather than `cargo test --workspace --lib`; and a control that undoes itself
   with `git checkout` deletes uncommitted work in that file.
2. `docs/handoff/CURRENT.md` — the 2026-08-28 sections, which record the citation
   sweeps, the seven revived scaffold tests and the `#[ignore]` audit. Phase 3.5
   touches `docs/RUNBOOK.md`, and the same "does this citation resolve" discipline
   applies.
3. `C:\Users\User\OneDrive\PHASED-FIXES.md` — the status banner at the top, then
   Phase 3.5 in full. Its own verification block is the authority on when it is
   finished, but **re-derive rather than trusting its tables**: Phase 3's grep
   missed twelve citations and Phase 5's count was 61 rather than 59, both
   because a table written earlier had gone stale.

## What is already true

- The gate is green and runs clean: `.\scripts\Invoke-ScaffoldChecks.ps1
  -SkipNpmInstall`, exit 0, **68 frontend tests, 0 skipped**, and no `test.skip`
  or stubbed input anywhere in the scaffold suite.
- All seven `#[ignore]`d hardware tests were run on 2026-08-28 and pass, except
  the CUDA one, which correctly refuses because no CUDA worker is staged. See
  `CURRENT.md` for what that run found.
- `cargo doc --no-deps --document-private-items --workspace` under
  `RUSTDOCFLAGS='-D rustdoc::broken_intra_doc_links'` exits 0 with the two known
  pre-existing warnings and no third.

## One thing that is open and is not a phase

**A clean clone does not build the Rust workspace on this machine.**
`llama-cpp-sys-2`'s CMake configure fails with `No CMAKE_C_COMPILER could be
found`, in a plain shell and in a Developer shell, with the VS Build Tools and
Windows SDK that `NEW-MACHINE.md` requires both installed. The leading
explanation is that `Enter-DevEnvironment.ps1` prefers a CMake staged under
gitignored `.tools\cmake-4.4.0-windows-x86_64\` which is not there any more, so it
falls back with a warning to the system CMake 4.4.2. `CURRENT.md` has the full
finding.

It does not block Phase 3.5, which is documentation-only — but it means **"the
gate is green" is currently a statement about an incrementally built tree**, and
anyone setting up a second machine hits it first. Staging CMake 4.4.0 under
`.tools/` and re-running is the first thing to try.

## What to be careful about

Phase 3.5 is documentation-only by its own checklist — "no code, no strings, no
reason codes changed". If a fix appears to need a code change, stop: that is a
different phase, and the last session found three cases where a comment claimed a
guarantee no test made. Naming the gap is the correct outcome; inventing the
guarantee is not.
