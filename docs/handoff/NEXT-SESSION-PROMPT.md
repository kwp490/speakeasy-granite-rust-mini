# Prompt for the next session — the first real dictation

Copy everything below the line into a new Claude Code session in
`C:\Coding Projects\speakeasy-granite-rust-mini`.

---

You are picking up work on SpeakEasy Mini, a local-only Windows dictation app
(Rust + Tauri 2 + React, all inference on-device) at
`C:\Coding Projects\speakeasy-granite-rust-mini`, branch `main`, HEAD `a7e82e6`,
released version **1.5.1**.

**This session is one task: get a real human dictation through an installed
release build, on the graphics card, and write down what actually happens.**
It needs me at a microphone, so you cannot do it alone — you are running the
instruments and I am the voice.

## Read first, in this order

1. `CLAUDE.md` — orientation. "Traps that fail silently" and "Settled decisions"
   are not optional; four traps were added on 2026-08-25.
2. `docs/handoff/CURRENT.md` — start at the banner, then **item 1b**, which is
   this task and already carries the procedure, the watch-list and the warnings.
   Read "What happened on 2026-08-25 (fifth session)" too: the capture path
   changed underneath this task and you need to know how.
3. `docs/UI-GUIDE.md` § "The two-minute ceiling ends the recording out loud" and
   § "Imperfect audio is delivered with a warning, never discarded".

## Why this matters, and what is genuinely unknown

Every latency figure in this repository comes from a test harness. A person has
never spoken into an installed release build. Three things are unverified and
only a real dictation can settle them:

- **Truncation.** Granite's `max_new_tokens` is 2048 and the generation loop
  stops on reaching it with no error, no end-of-generation token, and nothing
  that distinguishes "the model finished" from "the model was cut off
  mid-clause". A 120 s clip needs roughly 400 tokens for 312 words, so a
  full-length dictation is the first thing anyone will have run that gets within
  sight of the ceiling. **Nothing downstream catches a truncation** — the
  plausibility gate only rejects transcripts that are too *long*. I am the only
  instrument, which is why I have to be reading along.
- **Real end-to-end latency on the card.** Press-to-paste, on real speech, in
  release. The only comparable numbers are a 6.42 s fixture (2,928 ms processor
  vs 361 ms CUDA) and an RTX 5090 figure from a different machine.
- **Whether the transcript is any good.** Punctuation and casing come from the
  same single pass. Nobody has judged them on natural speech.

## What changed on 2026-08-25 that you must not assume away

A user hit the two-minute ceiling and the recording was destroyed. Fixed in
1.5.1, and the fix is what makes a long dictation worth attempting now:

- The buffer's byte limit used to bind at 116.5 s inside a 120 s ceiling, so
  **every** maximum-length recording was discarded. `max_buffered_bytes` is
  128 MiB now and a full-length capture reports `quality=none`.
- Five of the six capture outcomes are annotations, not failures; only
  `frames_buffered == 0` discards.
- Reaching the ceiling now sounds the stop cue and shows a `notice` window.
  **The audible cue has never been confirmed by ear** — that is a thing for me
  to listen for, and worth capturing in this session.

## State of the machine

- Installed at `%LOCALAPPDATA%\SpeakEasy Mini`, **1.5.1**, with the **CUDA worker
  staged**. Resting state is
  `granite_warm result=ok engine=cpu_gpu_pack_not_installed device=cuda
  installed=cpu provider=running_beyond_record` — `running_beyond_record` is
  disclosed as *not a fault* and is correct here.
- Toolchain **Rust 1.98.0** (pinned). Cargo/rustc are **not on PATH** — dot-source
  `. .\scripts\Enter-DevEnvironment.ps1` in every new shell.
- Malwarebytes quarantined the old 1.97.1 toolchain's `clippy-driver.exe` and
  `rustdoc.exe` as `Malware.AI.3172041259` (a false positive; the owner excluded
  that exact versioned directory). If a build suddenly fails with
  `could not execute process ... (never executed)`, look for a missing `.exe`
  beside a surviving `.pdb` before believing anything else.
- Hardware: RTX 4070 Laptop GPU, compute 8.9.

## How to run it

The full procedure is item 1b in `docs/handoff/CURRENT.md`. The short form:

1. `Get-Process SpeakEasy*` — nothing running, or the single-instance lock
   absorbs the launch and you test the old binary.
2. Confirm the disclosure in Settings → Transcription reads
   `Dictation runs on: Graphics card (GPU)` **before** I speak.
3. Set up a delivery target **you created and verified by name in the window
   title**. Never `Start-Process notepad` bare — Windows 11 Notepad restores its
   previous tabs and a proof has already written into somebody's real unsaved
   note.
4. Tail `%APPDATA%\ai.speakeasy.mini\logs\speakeasy.log`.
5. I speak. Ask me to read something I can compare against afterwards, and tell
   me roughly how long — **two to three minutes**, because length is the
   interesting variable. Check the *end* of the transcript, not the beginning.
6. Repeat on the processor via `Enable-GraniteCuda.ps1 -Revert` if I have the
   patience, from the same script, for the first real processor-versus-card
   comparison on speech.

Standing permission: you may stop and start SpeakEasy Mini's own processes for
testing without asking.

## What to capture

`hotkey_delivery result=`, the `granite_warm` four-tuple, wall-clock from second
press to paste, the whole transcript, and my judgement of it. A measurement that
exists only in captured stdout is not a measurement — write the numbers down.

## What would be a finding

A transcript ending mid-clause (the truncation nobody has produced). A fluent,
confident transcript unrelated to what I said (Granite answering the prompt
instead of transcribing — `is_plausible` is the only guard left). Delivery
landing on the clipboard instead of the target. The ceiling cue not sounding.

## When we are done

Update item 1b in `docs/handoff/CURRENT.md` with what was measured, replace the
harness figures in `docs/ARCHITECTURE.md` if real speech contradicts them, and
say plainly what is still unverified. Run the gate before committing:
`.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall`.

**Ask me before starting** whether I want the card, the processor, or both, and
how long I am willing to talk for.
