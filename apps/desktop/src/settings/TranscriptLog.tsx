import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { messages } from "../catalog";
import { formatError, formatTimeOfDay } from "./format";
import { readWithRetry } from "./readWithRetry";
import type { SessionTranscriptEntry } from "./types";

/**
 * The backend's signal that the list changed. Content-free: it says *that*, and
 * the text is fetched through the window-guarded `session_transcript_log`.
 *
 * Emitted by `notify_transcript_log_changed` when a transcript is published and
 * when the saved history is deleted -- the only two things that move the list
 * while a window is open.
 */
const TRANSCRIPT_LOG_CHANGED = "transcript-log-changed";

/**
 * The recent-transcripts list, newest first, each with Copy.
 *
 * Rendered in two places: the settings Log page and the pinned always-on-top
 * window. One component deliberately, so two lists of the same transcripts
 * cannot disagree about what was said.
 *
 * Three invariants:
 *
 * 1. **This is not "this session only".** The backing list is in memory but is
 *    seeded at launch from the optional on-disk history, so with retention on it
 *    spans earlier runs. `sessionLogDetail` states that, and states that deleting
 *    the saved transcripts empties this list too — which the backend enforces by
 *    clearing the list inside `history_delete_all`.
 * 2. **Copy is backend-owned.** The window sends an id, never text; Rust looks up
 *    the entry and writes the clipboard.
 * 3. **Transcript text is untrusted inert content.** `<pre>` with a `<bdi>`, no
 *    HTML interpretation, no linkification, no normalization.
 */
export function TranscriptLog() {
  const [entries, setEntries] = useState<SessionTranscriptEntry[]>([]);
  const [copied, setCopied] = useState("");
  const [copyError, setCopyError] = useState("");

  /**
   * Through the retry, because the mount read can lose the startup race.
   *
   * `session_transcript_log` takes a `tauri::State`, and both windows that
   * render this load before `setup` has managed the coordinator -- the pinned
   * `log` window runs its React tree whether or not it is shown. The poll this
   * replaced healed that on its next tick; a single event-driven read has no
   * next tick, so the retry is what takes its place.
   *
   * On the event path it costs nothing: `readWithRetry` returns on the first
   * success.
   */
  const reload = useCallback(() => {
    void readWithRetry<SessionTranscriptEntry[]>("session_transcript_log")
      .then(setEntries)
      .catch(() => {
        // The list is a convenience. A failed read leaves the last known one in
        // place rather than blanking it, which would look like data loss.
      });
  }, []);

  /**
   * One read on mount, then one read per change.
   *
   * This polled every 1.5 s for the life of the process -- about 40 IPC calls a
   * minute with nothing happening, from the `log` window as well as this page,
   * because a `visible: false` window still runs its React tree. Dictation is
   * bursty and rare: almost every one of those calls returned the list it had
   * just returned.
   *
   * `listen` resolves to the unlisten function asynchronously, so a component
   * unmounted before it resolves would otherwise leave a listener attached to a
   * dead tree. `cancelled` covers that window and the returned cleanup covers
   * the rest.
   */
  useEffect(() => {
    let cancelled = false;
    reload();
    const pending = listen(TRANSCRIPT_LOG_CHANGED, () => {
      if (cancelled) return;
      reload();
    });
    return () => {
      cancelled = true;
      void pending.then((unlisten) => {
        unlisten();
      });
    };
  }, [reload]);

  async function copyEntry(id: string) {
    setCopyError("");
    try {
      await invoke<number>("session_transcript_copy", { id });
      setCopied(id);
    } catch (error: unknown) {
      setCopied("");
      setCopyError(formatError(String(error)));
    }
  }

  return (
    <section aria-labelledby="session-log-title" className="session-log">
      <div className="section-heading">
        <h3 id="session-log-title">{messages.sessionLog}</h3>
        <output aria-live="polite">{messages.sessionLogCount(entries.length)}</output>
      </div>
      <p className="setting-detail">{messages.sessionLogDetail}</p>
      {copyError !== "" && <p role="alert">{copyError}</p>}
      {entries.length === 0 ? (
        <p className="setting-detail">{messages.sessionLogEmpty}</p>
      ) : (
        <ol className="plain-list" data-testid="session-transcript-log">
          {entries.map((entry) => (
            <li className="session-log-entry" key={entry.id}>
              <div className="session-log-meta">
                <span>{formatTimeOfDay(entry.recorded_unix_ms)}</span>
                <button onClick={() => void copyEntry(entry.id)} type="button">
                  {messages.copyEntry}
                </button>
                <output aria-live="polite">{copied === entry.id ? messages.copied : ""}</output>
              </div>
              <pre className="result-text">
                <bdi>{entry.text}</bdi>
              </pre>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
