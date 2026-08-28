import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import {
  type HudStatus,
  type TranscriberModel,
  initialTranscriberModel,
  transcriberReducer,
} from "./transcriberState";

/**
 * The compact transcriber's single poll.
 *
 * Exactly one command at exactly 10 Hz. The device name, shortcut binding and
 * gating flags ride along in the same response, so adding them to the UI cost
 * no extra IPC. Do not raise this frequency without measuring: the level meter
 * redraws from it, and 10 Hz is already inside a 20-30 Hz redraw budget.
 */
const POLL_INTERVAL_MS = 100;

/** How long the Copy button confirms for before returning to its resting label. */
const COPY_CONFIRM_MS = 2_000;

export type TranscriberController = {
  model: TranscriberModel;
  start: () => void;
  stop: () => void;
  cancel: () => void;
  dismiss: () => void;
  copy: () => void;
  /** True for a few seconds after a copy succeeded, so the button can confirm it. */
  copied: boolean;
};

export function useHudStatus(): TranscriberController {
  const [model, dispatch] = useReducer(transcriberReducer, initialTranscriberModel);
  // Read by the poll callback only, so a stale closure cannot resurrect a
  // request that has already been answered.
  const inFlight = useRef(false);
  const [copied, setCopied] = useState(false);

  // The confirmation is about one transcript, so it expires rather than
  // persisting: a "Copied" that outlives the copy is a claim about the clipboard
  // that this window cannot actually still vouch for.
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => {
      setCopied(false);
    }, COPY_CONFIRM_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [copied]);

  // A new dictation clears it immediately, without waiting for the timer.
  useEffect(() => {
    setCopied(false);
  }, [model.sessionId]);

  useEffect(() => {
    let stopped = false;
    const refresh = () => {
      invoke<HudStatus>("capture_hud_status")
        .then((status) => {
          if (stopped) return;
          dispatch({ type: "status", status, now: Date.now() });
        })
        .catch(() => {
          // A failed poll is not a failed dictation. The capture tap is
          // display-only and infallible by contract, so the transcriber going
          // quiet must never cost the user their recording — keep the last
          // known state and try again on the next tick.
        });
    };
    refresh();
    const timer = window.setInterval(refresh, POLL_INTERVAL_MS);
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, []);

  // Every command name is a literal at its call site rather than a parameter.
  // The allowlist test scans for `invoke("…")`, and routing these through a
  // variable would let a command slip past it unnoticed.
  const request = useCallback(
    (action: "start_requested" | "stop_requested", run: () => Promise<unknown>) => {
      if (inFlight.current) return;
      inFlight.current = true;
      dispatch({ type: action, now: Date.now() });
      run()
        .catch((error: unknown) => {
          dispatch({ type: "request_failed", code: String(error), now: Date.now() });
        })
        .finally(() => {
          inFlight.current = false;
        });
    },
    [],
  );

  const start = useCallback(() => {
    request("start_requested", () => invoke("dictation_start"));
  }, [request]);

  const stop = useCallback(() => {
    request("stop_requested", () => invoke("dictation_stop"));
  }, [request]);

  const cancel = useCallback(() => {
    void invoke("capture_transcribe_cancel").catch(() => {
      // Cancel is best-effort: the backend refuses when nothing is cancellable,
      // and the next poll reports whatever actually happened.
    });
  }, []);

  const dismiss = useCallback(() => {
    dispatch({ type: "dismissed", now: Date.now() });
  }, []);

  const copy = useCallback(() => {
    // No argument: `hud_transcript_copy` resolves the newest final in Rust, so
    // this window names no transcript and hands back no text. That is what keeps
    // the clipboard amendment narrow.
    void invoke("hud_transcript_copy")
      .then(() => {
        setCopied(true);
      })
      .catch(() => {
        // Refused because there is nothing recorded yet, or the clipboard was
        // locked by another process. Either way the transcript is still on
        // screen, so this must not escalate into the failed state — the button
        // simply does not confirm.
        setCopied(false);
      });
  }, []);

  return { model, start, stop, cancel, dismiss, copy, copied };
}
