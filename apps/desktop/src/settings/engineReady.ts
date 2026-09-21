import { invoke } from "@tauri-apps/api/core";

import { ENGINE_LOADING, type HudStatus } from "../state/transcriberState";

/**
 * How long to wait for a warming engine before giving up on it.
 *
 * A cold Granite warm hashes the pack and loads roughly 2 GB, which is tens of
 * seconds on a processor install. The wait is generous because the alternative
 * is telling a user their restart or switch failed while it is still working.
 */
export const ENGINE_READY_TIMEOUT_MS = 180_000;

/** Gap between polls. One request at a time; the next is scheduled after it settles. */
export const ENGINE_POLL_GAP_MS = 500;

/**
 * Waits for a warming engine to settle, and answers what it settled on.
 *
 * Shared by `runtime_recover` (Advanced → Maintenance) and
 * `runtime_switch_engine_provider` (Transcription): both commands only *start*
 * a warm -- neither may hold an IPC call for a 2 GB load -- so a caller cannot
 * claim success from either command returning. This polls the same status the
 * dock reads and returns `ready`, a named engine failure code, or
 * `engine_restart_timed_out`.
 *
 * Self-scheduling rather than an interval, so a slow read cannot queue
 * overlapping calls. A refused read is not a failed restart and is retried:
 * only the clock ends the wait.
 */
export async function awaitEngineReady(): Promise<string> {
  const deadline = Date.now() + ENGINE_READY_TIMEOUT_MS;
  for (;;) {
    try {
      const status = await invoke<HudStatus>("capture_hud_status");
      if (!ENGINE_LOADING.has(status.engine)) return status.engine;
    } catch {
      // A read that lost a race says nothing about the engine. Keep waiting.
    }
    if (Date.now() >= deadline) return "engine_restart_timed_out";
    await new Promise((resolve) => {
      window.setTimeout(resolve, ENGINE_POLL_GAP_MS);
    });
  }
}
