# Feature brief — dock right-click: reload the model, switch CPU/GPU

Not started. Owner-requested 2026-09-20. This is a brief, not a design.

## What the owner asked for

Extend the dock's right-click menu with two items:

1. **Reload the model**, for when the engine has stopped working.
2. **Switch CPU to GPU**, when the PC supports it, and **GPU back to CPU**.

## Reload is nearly free

`runtime_recover` already does it: takes an exclusive operation, clears
`GraniteEngineCoordinator`'s crash quarantine, discards the resident worker,
starts a fresh warm, and reports success only once the engine reports `ready`.

- `runtime_recover` and `restart_granite_engine` in
  `apps/desktop/src-tauri/src/commands/dictation.rs`
- Already surfaced as **Restart transcription engine** in
  `apps/desktop/src/settings/Advanced.tsx`, with the `awaitEngineReady` poll
- Contract written up in `docs/UI-GUIDE.md`, "Restart transcription engine
  claims nothing until the engine is ready"

Reuse it. Do not write a second recovery path.

## Switching providers is currently forbidden, and the reason is physical

`CLAUDE.md` settles it twice: **"No provider-override control. Granite's GPU
support is a build feature; no setting can conjure a worker binary"**, and **"No
setting can turn a CPU worker into a CUDA worker."**

The owner has explicitly reopened this. It still cannot be designed past these
three facts:

1. **The CUDA worker is a different binary** — `granite-worker.exe` built with
   `--features cuda`, plus three NVIDIA redistributable DLLs, fetched by the
   installer. It is not built on the user's machine.
2. **There is one worker slot and the two builds overwrite each other.**
   `RuntimeWizardCoordinator::paths().granite_worker` resolves exactly
   `proof/granite-worker.exe`
   (`apps/desktop/src-tauri/src/runtime_wizard.rs`), and setup's GPU staging
   copies the CUDA worker over that same path
   (`place_beside_the_worker` in `apps/bootstrapper/src/download.rs`). Both
   cannot be present today.
3. **A GPU cannot be promised in advance.** `GpuQualification::Qualified` needs
   an `ExecutionEvidence` with an `inference_sample_count`, and nothing at warm
   time has that number — see "GPU qualification cannot be proved" in
   `docs/handoff/CURRENT.md`. `engine=` says why a pack was selected; `device=`
   says where the worker actually ran; the UI reports the device, never the pack.

## Decide between these before writing code

- **A. Offer only what is installed.** On a CPU-only install, "Switch to GPU" is
  absent or disabled with a reason. No download, no staging change. Smallest
  honest version.
- **B. Keep both workers on disk.** Makes a switch real. Changes how setup
  stages workers and how the desktop resolves the worker path.
- **C. Fetch the CUDA worker on demand.** Also reopens "No in-app setup wizard",
  and puts a multi-hundred-megabyte download behind a context-menu click.

Recommend one, with costs, before starting.

## Where the menu lives

- `onContextMenu` in `apps/desktop/src/hud/HudDockApp.tsx` invokes the command
- `hud_dock_context_menu` in `apps/desktop/src-tauri/src/commands/capture.rs`
  builds a native Tauri `Menu` with "Settings" and "Close SpeakEasy"
- Ids are dispatched by `dispatch_menu_action`; the tray menu in
  `composition.rs` shares them
- **Native menu strings live in
  `apps/desktop/src-tauri/src/native_catalog.rs`**, not `catalog.ts`

## Rules this will run into

- The dock never takes focus and is not a second keyboard-access surface, so
  anything in its menu needs an equivalent control in Settings. Two surfaces,
  not one.
- A control may not claim something it did not do. A switch that only takes
  effect at the next warm must say so.
- Every failure code needs actionable catalog copy; `errorUnknown` is not
  acceptable.
- One dictation at a time. Reloading or switching mid-dictation is refused, not
  queued — `ExclusiveOperation::EngineRestart` and `OperationCoordinator` exist
  for this.
- A CPU installation running on CPU is normal. A recorded CUDA installation that
  cannot load CUDA is a specific failure, not a silent fallback.
- Update `docs/UI-GUIDE.md` in the same change as the visible behaviour.
