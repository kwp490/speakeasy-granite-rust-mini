# Design drawings

Visual proposals for the app's surfaces: mockups, elevations, before/after
comparisons, state galleries.

Each drawing is a **single standalone HTML file**. Open it in a browser — no
server, no build step, and nothing loaded over the network, so it keeps working
offline and inside a clone with no toolchain installed. Inline the CSS and embed
any asset as a `data:` URI rather than adding files beside it.

## What belongs here

Anything whose job is to *show* rather than to *specify*. Written specs stay in
[`../UI-GUIDE.md`](../UI-GUIDE.md); a drawing that argues for a change lives
here and links back to the spec section it would amend.

## Two rules that keep these honest

1. **Read geometry and palette from the real stylesheet, not from memory.**
   Every dimension in a drawing should be traceable to
   `apps/desktop/src/styles.css`. A mockup with invented spacing looks
   authoritative and is not.
2. **Say what a drawing is.** A proposal is not a live view of the app. When the
   stylesheet moves, these files are stale until someone updates them — note the
   surface, the date, and the branch in the drawing itself so a reader can tell.

## Contents

| File | Surface | What it argues |
| --- | --- | --- |
| [side-dock-elevation.html](side-dock-elevation.html) | Side dock (130×360) | The dock shipped as a 240 px opaque slab with an upside-down wordmark and nothing on it outside a dictation. Before/at-rest/recording elevations at 1:1, the row arithmetic, the placement diagram for the 24 px work-area inset, a change ledger, and the two defects only the running window showed. **Built** — geometry and placement measured against the live app. **The geometry it draws is superseded**: it depicts a 130-wide card with five rows (20 / 86 / 152 / 16 / 28), and the dock has since narrowed to 62 and gained a sixth row for the engine indicator (20 / 104 / 14 / 112 / 16 / 28, 2026-08-27). The argument it makes still holds; the numbers on it were true when drawn and are not now. |

## Verifying a drawing against the real window

`scripts/Invoke-WebviewProbe.ps1` evaluates JavaScript inside the running app,
which is how the dock's row heights were checked rather than asserted. Start the app with the DevTools port open first:

```powershell
. .\scripts\Enter-DevEnvironment.ps1
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
npm run tauri -- dev            # from apps/desktop
```

Then measure — `getBoundingClientRect()` on each row beats reading the
stylesheet, because it accounts for the cascade you did not know was there:

```powershell
.\scripts\Invoke-WebviewProbe.ps1 -Window dock `
  -Expression 'Math.round(document.querySelector(".hud-transcript").getBoundingClientRect().height)'
```

Note that the app uses `tauri_plugin_single_instance`. An already-running build —
including an installed one under `%LOCALAPPDATA%\SpeakEasy Mini` — will absorb the
launch and leave you measuring the *old* frontend while everything looks fine.
Stop it first.

That launch also stages the native runtime, via `beforeDevCommand` →
[`../../scripts/Stage-DevRuntime.ps1`](../../scripts/Stage-DevRuntime.ps1), so
the first run after a clean checkout spends a minute building the two worker
binaries before Vite starts. Without it a dev build has no `proof/` directory
and every dictation fails at finalization — which measuring a *layout* does not
care about, but driving a real dictation very much does. Set
`SPEAKEASY_SKIP_DEV_STAGING=1` to skip it for pure layout work.
