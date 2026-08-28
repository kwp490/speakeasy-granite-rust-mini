# Prompt for the next session — a clean clone does not build

Copy everything below the line into a new Claude Code session in
`C:\Coding Projects\speakeasy-granite-rust-mini`.

> **Superseded 2026-08-28 (second rewrite).** The previous version staged
> **Phase 3.5** of the phased-fixes brief. That landed the same day, and with it
> the brief is empty: all six phases are done. The version before that staged
> publishing the CUDA worker, which landed on 2026-08-26. Both are recorded in
> `docs/handoff/CURRENT.md`.

---

You are picking up work on SpeakEasy Mini, a local-only Windows dictation app
(Rust + Tauri 2 + React, all inference on-device) at
`C:\Coding Projects\speakeasy-granite-rust-mini`, branch `main`.

**`C:\Users\User\OneDrive\PHASED-FIXES.md` is finished. Do not start there.**
All six phases are done, and the last of them closed on 2026-08-28. Read it only
for what it found.

**This session's job is the one thing that is open and was never a phase: a
clean clone of `main` does not build the Rust workspace on this machine.**
`llama-cpp-sys-2`'s CMake configure fails with `No CMAKE_C_COMPILER could be
found`, then the same for CXX — in a plain shell and in a
`Enter-VsDevShell` Developer shell where `cl.exe` is on `PATH`, with the Build
Tools and Windows SDK that `NEW-MACHINE.md` requires both installed. The
frontend half of the gate passes completely on the same clone.

## Read first, in this order

1. `CLAUDE.md` — orientation. "Traps that fail silently" and "Settled decisions"
   are not optional. Four that bite this work specifically: dot-source
   `scripts\Enter-DevEnvironment.ps1` in every new shell; run the whole gate
   rather than `cargo test --workspace --lib`; a control that undoes itself with
   `git checkout` deletes uncommitted work in that file; and a fixture or tool
   under gitignored `.tools\` is a dependency with a deletion date, which is the
   shape this failure has.
2. `docs/handoff/CURRENT.md` — "A clean clone does not build on this machine —
   open, found 2026-08-28" carries the full finding, including what was ruled
   out. Everything else dated 2026-08-28 is history.
3. `docs/NEW-MACHINE.md` — the prerequisite list this failure appears to
   contradict. If the fix turns out to be a missing prerequisite, that document
   is wrong and amending it is part of the work.

## What to try, in order

1. **Stage CMake 4.4.0 under `.tools\` and re-run.**
   `Enter-DevEnvironment.ps1` prefers
   `.tools\cmake-4.4.0-windows-x86_64\bin\cmake.exe` and falls back to whatever
   is on `PATH` with a **warning rather than a failure**. That directory does not
   exist here, so the fallback is the system CMake **4.4.2**. This is the leading
   explanation and it is untested — establishing it means staging 4.4.0 and
   re-running, which nobody has done.
2. **If 4.4.0 fixes it, the warning is the real defect.** A build dependency that
   silently degrades to a different version is the same shape as
   `Stage-DevRuntime.ps1` reverting a staged CUDA worker: correct-looking output,
   wrong binary, an error that names something else. Decide whether the fallback
   should refuse, and whether `.tools\` should be fetched rather than assumed.
3. **If 4.4.0 does not fix it, do not guess.** Read the CMake error log
   `llama-cpp-sys-2` leaves in `target\`, which says which compiler probe failed
   and why, before forming a second theory.

**Reproduce it before believing this prompt.** `target\` on this machine carries
a successful llama.cpp build from 2026-08-18 and 2026-08-25, so an incremental
build skips configure entirely and looks fine. Clone into an empty directory, or
`cargo clean -p llama-cpp-sys-2`, and do not conclude anything from the working
tree.

## What is already true

- The gate is green: `.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall`,
  exit 0, **71 frontend tests, 0 skipped**. Read that as a statement about an
  **incrementally built tree** until this item is closed.
- **1.7.1 is released** (2026-08-28) and installed on this machine, running on
  the graphics card. All three proofs ran against the published artifact and the
  published download was re-hashed against its own `SHA256SUMS`. See item 2b in
  `CURRENT.md` for the digest and for why v1.7.0 was never tagged.
- All seven `#[ignore]`d hardware tests were run on 2026-08-28 and pass, except
  the CUDA one, which correctly refuses because no CUDA worker is staged.
  **Re-stage before believing a hardware-test failure** — a two-day-old staged
  worker reports `StaleEvent`, which names the protocol rather than the binary.
- `cargo doc --no-deps --document-private-items --workspace` under
  `RUSTDOCFLAGS='-D rustdoc::broken_intra_doc_links'` exits 0 with the two known
  pre-existing warnings and no third.

## Two coverage gaps that are stated as gaps, if this finishes early

Both were found on 2026-08-28 by the backticked-identifier sweep, and both are
comments that cited a test which does not exist. They are now written as gaps
rather than as guarantees, which is the honest state but not the finished one:

- Nothing pins the bootstrapper's download policy to the app's; they are
  identical by inspection only.
- Nothing pins `main::remove`'s inverted default — that an uninstall removes the
  profile unless `--keep-user-data` says otherwise.

Writing either test is real work with a real invariant behind it. **Prove it can
fail**, and per `CLAUDE.md` commit first and control against the commit — a
control that reverts with `git checkout` deletes the test it was checking, and a
test that is absent cannot fail.
