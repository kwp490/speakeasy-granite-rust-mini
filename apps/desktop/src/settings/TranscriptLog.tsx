import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import { formatError, formatTimeOfDay } from "./format";
import type { SessionTranscriptEntry } from "./types";

/** How often the log is refreshed while this page is open. */
const LOG_INTERVAL_MS = 1_500;

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

  const reload = useCallback(() => {
    void invoke<SessionTranscriptEntry[]>("session_transcript_log")
      .then(setEntries)
      .catch(() => {
        // The log is a convenience. A failed read leaves the last known list in
        // place rather than blanking it, which would look like data loss.
      });
  }, []);

  useEffect(() => {
    reload();
    const timer = window.setInterval(reload, LOG_INTERVAL_MS);
    return () => {
      window.clearInterval(timer);
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
