# Prompt for the next session — the status reads that lose their race

Copy everything below the line into a new Claude Code session in
`C:\Coding Projects\speakeasy-granite-rust-mini`.

> **Superseded.** The previous version of this file staged the first real human
> dictation (item 1b). That was done on 2026-08-25 on both providers and is
> closed; its measurements and its six findings are in
> `docs/handoff/CURRENT.md`. The task below is the one defect it turned up.

---

You are picking up work on SpeakEasy Mini, a local-only Windows dictation app
(Rust + Tauri 2 + React, all inference on-device) at
`C:\Coding Projects\speakeasy-granite-rust-mini`, branch `main`, released
version **1.5.1**.

**This session is one task: fix the status reads that lose the startup race, and
find every one of them.** It needs no microphone and no owner — unlike the last
three sessions, you can finish this alone.

## Read first, in this order

1. `CLAUDE.md` — orientation. "Traps that fail silently" and "Settled decisions"
   are not optional.
2. `docs/handoff/CURRENT.md` — the banner, then **item 14**, which is this task.
   Read **item 11** too: it is the clearest example in the file of a wrong belief
   surviving for months because nobody did the arithmetic, and this task has the
   same shape.
3. The trap in `CLAUDE.md` beginning "**An answer can reach disk and never reach
   the screen**" — that is the first occurrence of this bug, from 2026-08-20.

## The defect

`Settings -> General` reports **"Shortcut not registered yet"** while the
shortcut is registered and working. Confirmed both ways on 2026-08-25: the panel
said it for the life of the process, and `hotkey_status` invoked directly against
that same window returned `binding: "Ctrl+Alt+P", registration: "registered",
enabled: true`. Two dictations then ran fine.

`apps/desktop/src/settings/General.tsx` reads it with a bare
`invoke<HotkeyStatus>("hotkey_status")` — no rejection handler, no retry — and
renders `formatShortcutState(hotkey?.registration ?? "pending")`. Every window's
webview loads while `setup` is still managing coordinators, so that read can be
refused with "state not managed for field `state` on command …", and `hotkey`
then stays `null` forever.

This is the 2026-08-20 `personalization_status` defect in a second location. The
fix there was `readWithRetry`, and **`readWithRetry.ts` still has exactly one
importer** — `Transcription.tsx`, for two of its reads. The sweep stopped there.

## Why this one matters more than the first

An empty dictionary list is a passive wrong answer: it says "you have no
protected terms". This says a **working feature is broken**, in the one panel a
user opens *because* their shortcut appears not to work — and the remedy it
implies, pressing "Save hotkey" to re-register, fixes a problem they do not have.

## What the task actually is

The one-line fix is not the task. **Finding the others is.**

1. Convert `General.tsx`'s read, matching how `Transcription.tsx` does it.
2. **Enumerate every mount-time `invoke` in `apps/desktop/src` that reads state
   from a coordinator**, and decide for each whether it can lose the race. The
   lesson of the first occurrence was recorded as "one reader had carried a retry
   since the day it was found and nothing else did" — and then the fix for the
   second occurrence repeated it. Do not fix two and leave four.
3. Consider whether a test can assert this shape rather than leaving it to
   review. The scaffold suite already asserts invariants against source (the
   window allowlist, the IPC schema, the non-focusable rule), and
   `every_capture_annotation_has_catalog_copy` reads `catalog.ts` as text — so a
   check that no settings page calls a status command without a rejection path is
   in keeping with what is already there. **This is the part that stops a third
   occurrence**, and it is worth more than the fix.
4. A `?? "pending"` fallback that renders as a *claim about the system* is the
   deeper problem. "Shortcut not registered yet" is a statement of fact;
   `undefined` means "not known yet". Consider whether the unresolved state
   should say so instead, here and anywhere else the same pattern renders a
   default as an assertion.

## How to verify it, and the trap in verifying it

**The rendered string cannot tell you whether the backend is wrong or the read
was refused** — that is what cost time on 2026-08-25. Ask the backend directly.
Start the installed app with the debugging port open and invoke the command
through the webview, which needs no focus and cannot hijack a dictation:

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
```

Then, with the app running, a two-step probe (the expression is evaluated
synchronously, so stash the promise result and read it back):

```powershell
.\scripts\Invoke-WebviewProbe.ps1 -Window settings -Expression 'window.__p=null; window.__TAURI_INTERNALS__.invoke("hotkey_status").then(r=>window.__p={ok:r}).catch(e=>window.__p={err:String(e)}); "fired"'
.\scripts\Invoke-WebviewProbe.ps1 -Window settings -Expression 'JSON.stringify(window.__p)'
```

`window.__TAURI__` is undefined; `window.__TAURI_INTERNALS__` is the bridge.

**Reproducing it at all needs a release frontend.** The race is structurally
invisible under `npm run tauri -- dev` — Vite is slow enough that `setup` wins —
which is the same reason the first occurrence was found only on an installed
build. Do not conclude it is fixed because dev looks right.

## State of the machine

- Installed at `%LOCALAPPDATA%\SpeakEasy Mini`, **1.5.1**, with the **CUDA worker
  staged** (54.4 MB, all three CUDA 13 libraries beside it). Resting state is
  `granite_warm result=ok engine=cpu_gpu_pack_not_installed device=cuda
  installed=cpu provider=running_beyond_record`. That last field is correct and
  **cannot be cleared on any machine** — see item 12 before treating it as a bug.
- Toolchain **Rust 1.98.0** (pinned). Cargo/rustc are **not on PATH** — dot-source
  `. .\scripts\Enter-DevEnvironment.ps1` in every new shell.
- `Get-Process SpeakEasy*` before launching anything: the single-instance lock
  silently absorbs a second launch and you then test the old binary. You have
  standing permission to stop and start SpeakEasy Mini's own processes.
- Hardware: RTX 4070 Laptop GPU, compute 8.9.

## Also open, if this finishes early

- **Item 17, and take this one first — it is the smallest real fix in the file.**
  The `notice` window declares 360x172 and its content needs 188 CSS px, so the
  dismiss button is 16 px below the fold behind a scrollbar. Raise `height` and
  `minHeight` to 192 in `tauri.conf.json` and **measure the running window
  afterwards**, because a stylesheet reading `height: 100vh` cannot show you
  this. Editing `tauri.conf.json` does not invalidate the cargo build — touch
  `src-tauri/build.rs` or you will test the old config and see no change. Item 17
  records the measurement and the two plausible-sounding causes it disproved.
- **Item 11's latent half.** `max_new_tokens` (2048) and `MAX_CAPTURE_SECONDS`
  (120) are unconnected, and the truncation hazard becomes real the moment the
  ceiling is raised. A test asserting the token budget covers the ceiling's worth
  of speech would close it permanently and is a few lines.
- **Item 16.** Four true sentences in Settings -> Transcription that read as a
  contradiction when stacked. Copy, not logic — read the item before changing
  anything, because the last sentence is deliberate.
- **Item 3** (publishing the CUDA worker) still needs the owner, and closes items
  12 and 16 on its own.

## When you are done

Run the gate before committing:

```powershell
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
```

Amend `docs/UI-GUIDE.md` in the same change if any copy moves — it is a living
spec, not a record. Update item 14 in `docs/handoff/CURRENT.md` with what the
sweep found, including the reads you checked and cleared, because the next
occurrence will be in whichever file nobody listed.
