# Prompt for the next session — the copy that reads as a contradiction

Copy everything below the line into a new Claude Code session in
`C:\Coding Projects\speakeasy-granite-rust-mini`.

> **Superseded.** The previous version staged item 14, the status reads that lose
> the startup race. That closed on 2026-08-26 — along with item 17 and the latent
> half of item 11 — and it was **two defects on two pages with one symptom**, not
> the one it was written for. Its findings are in `docs/handoff/CURRENT.md`,
> item 14, and the shape of what went wrong is worth reading before picking
> anything up: the original diagnosis was structurally sound, never proved, and
> would have shipped without fixing the reported symptom.

---

You are picking up work on SpeakEasy Mini, a local-only Windows dictation app
(Rust + Tauri 2 + React, all inference on-device) at
`C:\Coding Projects\speakeasy-granite-rust-mini`, branch `main`, released
version **1.5.1** with unreleased fixes on `main`.

**There is no defect left in this file that an agent can close alone.** What is
left is one copy task, one owner task, and a release. Take them in that order.

## Read first, in this order

1. `CLAUDE.md` — orientation. "Traps that fail silently" and "Settled decisions"
   are not optional.
2. `docs/handoff/CURRENT.md` — the banner, then **item 16**, which is the task
   below. Read **item 14** too, not for what to do but for how it went wrong:
   a symptom with two causes, one of them never proved, and a test that was green
   on the day the bug it existed for recurred.
3. `docs/UI-GUIDE.md` — it is a living spec. A copy change that does not amend it
   is incomplete.

## The task: item 16

Four true sentences in `Settings → Transcription` that read as a contradiction
when stacked. **Read item 16 before changing anything** — the last sentence is
deliberate, and the item says why. This is copy, not logic: no behaviour changes,
and `catalog.ts` plus `UI-GUIDE.md` are the whole surface.

It is worth doing carefully rather than quickly. Every sentence involved is
individually accurate and was written to discharge an honesty obligation; the
failure is that four of them together tell a user something none of them says.

## Then, if you have the owner

- **Item 3** — publishing the CUDA Granite worker. Needs the owner, not an agent.
  It closes items 12 and 16 on its own, so if this is happening, do it before
  rewriting the copy in item 16 rather than after.
- **Item 2b** — cutting the next release. `main` carries unreleased fixes now:
  the status-read sweep (item 14), the notice window height (item 17) and the
  token-budget check (item 11). Nothing in them is user-visible except the three
  panels that stopped lying, which is worth a release note that says so plainly.

## What is finished, so you do not re-open it

- **Item 14** — all five settings surfaces read through `readWithRetry`, which now
  also takes a `settled` predicate for an answer that arrives before it is true.
  The enforcement is `no effect can read a race-prone command without retrying or
  polling`: the hazard is derived from the Rust signatures, the readers are found
  by scanning every `useEffect`, and it carries instrument self-checks because
  every assertion in it is of the form "nothing was found".
- **Item 17** — the notice window is 192 and was measured at zero overflow.
  `Invoke-WebviewProbe.ps1` addresses it now (`-Window notice`), which it could
  not before.
- **Item 11's latent half** — raising `MAX_CAPTURE_SECONDS` past ~410 s fails the
  gate until `max_new_tokens` moves with it.

## State of the machine

- Installed at `%LOCALAPPDATA%\SpeakEasy Mini`, **1.5.1**, with the **CUDA worker
  staged** (54.4 MB, all three CUDA 13 libraries beside it). That build still has
  the item-14 defects; a release build of `main` sits in `target/release` and does
  not. Resting state is `granite_warm result=ok engine=cpu_gpu_pack_not_installed
  device=cuda installed=cpu provider=running_beyond_record`. That last field is
  correct and **cannot be cleared on any machine** — see item 12.
- Toolchain **Rust 1.98.0** (pinned). Cargo/rustc are **not on PATH** — dot-source
  `. .\scripts\Enter-DevEnvironment.ps1` in every new shell.
- `Get-Process SpeakEasy*` before launching anything: the single-instance lock
  silently absorbs a second launch and you then test the old binary. You have
  standing permission to stop and start SpeakEasy Mini's own processes.
- Hardware: RTX 4070 Laptop GPU, compute 8.9.

## Two things item 14 learned about verifying a UI claim

Worth carrying into anything that touches a settings surface.

**A release frontend is not optional, and `cargo build --release` alone does not
give you one.** Without `--features custom-protocol` the binary loads
`http://localhost:1420` — the dev URL — so with no Vite server running all four
windows come up blank and the probe reports every window as absent, which reads
exactly like the app failing to start. The full command is:

```powershell
cargo build --release -p speakeasy-desktop --features custom-protocol
```

**Ask the backend, and then ask it a second way.** The rendered string cannot
tell you whether the backend is wrong, the read was refused, or the answer
arrived early and never changed — three causes, one appearance. Start the app with
the port open and invoke through the webview, which needs no focus and cannot
hijack a dictation:

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
```

Then a two-step probe, because the expression is evaluated synchronously:

```powershell
.\scripts\Invoke-WebviewProbe.ps1 -Window settings -Expression 'window.__p=null; window.__TAURI_INTERNALS__.invoke("hotkey_status").then(r=>window.__p={ok:r}).catch(e=>window.__p={err:String(e)}); "fired"'
.\scripts\Invoke-WebviewProbe.ps1 -Window settings -Expression 'JSON.stringify(window.__p)'
```

`window.__TAURI__` is undefined; `window.__TAURI_INTERNALS__` is the bridge. The
second way that separated the two causes was `-Cdp 'Page.reload'`: the same page,
the same backend, reading correctly after a reload, which is a transient value
and cannot be a refusal.

## When you are done

Run the gate before committing:

```powershell
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
```

Amend `docs/UI-GUIDE.md` in the same change — it is a living spec, not a record —
and update the item you closed in `docs/handoff/CURRENT.md` with what you found,
including what you checked and cleared.
