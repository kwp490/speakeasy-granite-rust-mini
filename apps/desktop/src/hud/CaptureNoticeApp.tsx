import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";

import { messages } from "../catalog";

/**
 * The window that tells the user the safety ceiling ended their recording.
 *
 * # Why this exists as a window
 *
 * The dock is 62 px wide and holds a glyph and a hover tooltip. Neither reaches
 * somebody who has just stopped speaking and is looking at the application they
 * dictated into — which is everybody, because that is where the transcript
 * lands. A Windows toast would be the conventional answer and was specified and
 * rejected: the WinRT route needs an AUMID from an installed Start Menu
 * shortcut and otherwise displays nothing while reporting success.
 *
 * # What it must never do
 *
 * **Never take focus.** It is shown while `deliver_final_text` is deciding
 * where the transcript goes by inspecting the foreground window, so a window of
 * the app's own that activates here hijacks the dictation it is reporting on.
 * The window declares `focus: false` and `configure_hud` calls
 * `set_focusable(false)`; nothing in here may call `setFocus`, and the dismiss
 * control is deliberately a click target rather than something that wants the
 * keyboard. A non-focusable window still receives mouse input, which is all
 * this needs.
 */
export function CaptureNoticeApp() {
  const [shownAt, setShownAt] = useState(() => Date.now());

  const dismiss = useCallback(() => {
    // Through a command rather than the window API, matching how the pinned log
    // closes itself: hidden, never closed. A closed window cannot be shown
    // again without building one, and building a window from a command handler
    // deadlocks this app's entire IPC.
    void invoke("capture_notice_dismiss");
  }, []);

  useEffect(() => {
    // Re-arms the timer when the ceiling fires again while the notice is still
    // up. Without this a second long dictation would inherit the remains of the
    // first one's countdown and could vanish almost immediately.
    const pending = listen("capture-limit-reached", () => {
      setShownAt(Date.now());
    });
    return () => {
      void pending.then((unlisten) => {
        unlisten();
      });
    };
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(dismiss, VISIBLE_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [dismiss, shownAt]);

  return (
    <main className="capture-notice" data-testid="capture-notice">
      {/* `alert` rather than `status`: the recording ended without the user
          asking, which is the interruption case the assertive role is for. The
          window is not focusable, so a screen reader user reaches it through
          this announcement rather than by tabbing to it. */}
      <div aria-live="assertive" role="alert">
        <h1 className="capture-notice-title">{messages.captureLimitNoticeTitle}</h1>
        <p className="capture-notice-body">{messages.captureLimitNoticeBody}</p>
      </div>
      <button className="capture-notice-dismiss" onClick={dismiss} type="button">
        {messages.captureLimitNoticeDismiss}
      </button>
    </main>
  );
}

/**
 * How long the notice stays up on its own.
 *
 * Long enough to read the body twice at an unhurried pace, because the reader
 * is not looking at this window when it appears — they are looking at whatever
 * they dictated into. It dismisses itself rather than waiting to be closed: a
 * notice that persists becomes furniture, and this one sits on top of every
 * other window.
 */
const VISIBLE_MS = 15_000;
