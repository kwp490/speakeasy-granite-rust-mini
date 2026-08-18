import type { MouseEvent } from "react";
import { useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import { TranscriptLog } from "../settings/TranscriptLog";
import { useDragToMove } from "./useDragToMove";

/**
 * The transcript log, detached into its own always-on-top window.
 *
 * The same `TranscriptLog` the settings page renders, in a frame that can sit
 * over another application. Deliberately the same component and not a second
 * implementation: two lists of the same transcripts that could disagree about
 * what was said is exactly the kind of divergence this project has been bitten
 * by before, and the copy path is backend-owned either way — the window sends
 * an id and Rust writes the clipboard, so nothing here has clipboard authority
 * to duplicate.
 *
 * Undecorated, so this header row is the whole titlebar: it is what
 * `useDragToMove` reads to move the window, and it carries the only close
 * control. Right-click closes it too, mirroring the dock, so a user who drags
 * it somewhere awkward is never stuck.
 *
 * **Non-focusable**, set from Rust at startup rather than declared here.
 * `deliver_final_text` inspects the foreground window to decide where a
 * transcript goes, so a log window that could take the foreground would quietly
 * become the paste target for the next dictation — which does not error, it
 * refuses with `target_inspect_refused` and falls back to the clipboard, and
 * reads as a delivery bug in some other subsystem entirely.
 */
export function PinnedLogApp() {
  useDragToMove(() => {
    // Position is not persisted. The dock's placement is, because it snaps to a
    // screen edge and is the app's permanent furniture; this window is opened
    // for a task and closed again, and restoring it to wherever it was days ago
    // is not obviously what anyone wants.
  });

  const close = useCallback(() => {
    void invoke("transcript_log_unpin");
  }, []);

  const onContextMenu = useCallback(
    (event: MouseEvent) => {
      event.preventDefault();
      close();
    },
    [close],
  );

  return (
    <main className="pinned-log" data-testid="pinned-log" onContextMenu={onContextMenu}>
      <div className="pinned-log-chrome" data-drag-region>
        <span className="pinned-log-title">{messages.settingsGroups.log}</span>
        <button
          aria-label={messages.transcriptLogUnpin}
          className="pinned-log-close"
          onClick={close}
          title={messages.transcriptLogUnpin}
          type="button"
        >
          ×
        </button>
      </div>
      <div className="pinned-log-body">
        <TranscriptLog />
      </div>
    </main>
  );
}
