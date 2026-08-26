import { invoke } from "@tauri-apps/api/core";

/**
 * Attempts before giving up, and the gap between them.
 *
 * 20 x 250 ms covers a slow cold start, matching `useProfile.ts` — the two
 * numbers describe the same race and drifting apart would mean one page
 * recovering from a startup that the other reported as broken.
 */
const ATTEMPTS = 20;
const DELAY_MS = 250;

/**
 * Reads a status command, retrying a rejection that lost the startup race.
 *
 * Every window's webview loads while Tauri's `setup` is still running, so a read
 * fired on mount can arrive before the coordinator it needs is managed and be
 * refused outright — "state not managed for field `state` on command …". The
 * window does not have to be visible for this to happen: `main` and `hud-dock`
 * are declared statically and both run their React tree from launch.
 *
 * `useProfile.ts` has carried a retry for exactly this since the race was first
 * observed, and its comment names the error string. Nothing else did, and the
 * one that mattered most was `personalization_status`: the Transcription page
 * read it once, with no `catch`, so a lost race left the dictionary list **empty
 * for the life of the process**. That is not a blank page someone reports — it
 * is a page that looks finished and says the user has no protected terms, which
 * is how setup's vocabulary appeared to have been discarded when it had in fact
 * been written to disk correctly (measured 2026-08-20, three words on disk and
 * an empty list on screen).
 *
 * Rejects with the last error if every attempt fails, so a caller still has to
 * decide what to show. Retrying forever would trade a wrong answer for a
 * permanent spinner.
 *
 * # `settled`, and why a refusal is not the only way to lose this race
 *
 * A read can also **succeed** and return a value that is only true for the first
 * moment of the process. `hotkey_status` is the case that proved it:
 * `HotkeyCoordinator` starts at `registration: "pending"` and
 * `register_activation_hotkey` runs at the *end* of `setup`, after the tray is
 * built — while every window's React tree has already mounted and read. So the
 * page held `pending` for the life of the process, rendered as "Shortcut not
 * registered yet", with the shortcut registered and working.
 *
 * That symptom is **indistinguishable** from a refused read: the rendered string
 * is the same, and the panel's own state cannot say which happened. This is the
 * documented trap one level deeper than it was written — the rendered string
 * cannot tell you whether the backend is wrong, the read was refused, or the
 * answer arrived early and never changed. It was separated by reloading the
 * window and watching the same page report "Shortcut active" from the same
 * backend (2026-08-26, installed release frontend).
 *
 * So a caller may declare which answers are final. An unsettled one is retried
 * like a refusal, and if every attempt is unsettled the **last value is
 * returned** rather than thrown: a startup value that is still there after five
 * seconds has stopped being transient and is the truth. Only a refusal has
 * nothing to report.
 */
export async function readWithRetry<T>(
  command: string,
  settled?: (value: T) => boolean,
): Promise<T> {
  let lastError: unknown;
  let lastValue: T | undefined;
  let read = false;
  for (let attempt = 0; attempt < ATTEMPTS; attempt += 1) {
    try {
      const value = await invoke<T>(command);
      if (settled === undefined || settled(value)) return value;
      lastValue = value;
      read = true;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => {
      window.setTimeout(resolve, DELAY_MS);
    });
  }
  if (read) return lastValue as T;
  throw lastError;
}
