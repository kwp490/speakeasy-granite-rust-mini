import { useEffect, useState } from "react";
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
 *    spans earlier runs. `sessionLogDetail` states that, and states what deleting
 *    the saved transcripts does to this list — the entries restored from disk go,
 *    the ones this run produced stay, which `clear_seeded_history` enforces
 *    inside `history_delete_all`.
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
   * One read once subscribed, then one read per change, never two at once.
   *
   * This polled every 1.5 s for the life of the process -- about 40 IPC calls a
   * minute with nothing happening, from the `log` window as well as this page,
   * because a `visible: false` window still runs its React tree. Dictation is
   * bursty and rare: almost every one of those calls returned the list it had
   * just returned.
   *
   * Three invariants hold the event-driven read together, and none of them was
   * needed while a next tick was always coming.
   *
   * 1. **Subscribe, then snapshot.** The first read is issued from `listen`'s
   *    resolution, not before it. `listen` attaches the handler asynchronously,
   *    so a read issued first leaves a window in which the list can change with
   *    nobody subscribed: the answer already in flight predates the change and
   *    the event reaches no handler, so the window renders a stale list for the
   *    life of the process. Subscribing first makes that window carry no reads.
   * 2. **One read in flight.** A second read issued while one is outstanding can
   *    answer out of order, and the loser overwrites the newer list. An event
   *    arriving during a read sets `stale` instead, and the read that settles
   *    issues exactly one follow-up however many events it coalesced.
   * 3. **An invalidated answer is discarded.** A read that was outstanding when
   *    an event arrived describes a list that has already changed, so its answer
   *    is dropped rather than rendered on the way to the follow-up.
   *
   * The read goes through `readWithRetry` because `session_transcript_log` takes
   * a `tauri::State`, and both windows that render this load before `setup` has
   * managed the coordinator -- the pinned `log` window runs its React tree
   * whether or not it is shown. The poll healed a lost race on its next tick; a
   * single event-driven read has no next tick. On the event path it costs
   * nothing, since `readWithRetry` returns on the first success.
   */
  useEffect(() => {
    let cancelled = false;
    let reading = false;
    let stale = false;

    const read = () => {
      if (cancelled) return;
      if (reading) {
        stale = true;
        return;
      }
      reading = true;
      stale = false;
      void readWithRetry<SessionTranscriptEntry[]>("session_transcript_log")
        .then((next) => {
          if (!cancelled && !stale) setEntries(next);
        })
        .catch(() => {
          // The list is a convenience. A failed read leaves the last known one
          // in place rather than blanking it, which would look like data loss.
        })
        .finally(() => {
          reading = false;
          if (cancelled || !stale) return;
          stale = false;
          read();
        });
    };

    // `listen` resolves to the unlisten function asynchronously, so a component
    // unmounted before it resolves would otherwise leave a listener attached to
    // a dead tree. `cancelled` covers that window and the returned cleanup
    // covers the rest.
    const pending = listen(TRANSCRIPT_LOG_CHANGED, read);
    void pending.then(read);

    return () => {
      cancelled = true;
      void pending.then((unlisten) => {
        unlisten();
      });
    };
  }, []);

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
