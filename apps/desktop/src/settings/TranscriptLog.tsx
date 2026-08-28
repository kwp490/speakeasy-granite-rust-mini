import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import { formatError, formatTimeOfDay } from "./format";
import type { SessionTranscriptEntry } from "./types";

/** How often the log is refreshed while this page is open. */
const LOG_INTERVAL_MS = 1_500;

/**
 * The session transcript log.
 *
 * This is where the recoverable result went when it left the transcriber. Every
 * finished transcript from this run of the app, newest first, each with Copy.
 *
 * Three things about it are load-bearing rather than incidental:
 *
 * 1. **Nothing is written to disk.** The list lives in the app's memory and dies
 *    with the process, which is why it needs no plaintext-at-rest disclosure, no
 *    retention setting and no delete action — there is nothing stored to disclose,
 *    retain or delete. It sits next to the on-disk history feature and is labelled
 *    so the difference is unmistakable.
 * 2. **Copy is backend-owned.** The window sends an id, never text; Rust looks up
 *    the entry and writes it. Clipboard authority stays out of the transcriber
 *    entirely.
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
