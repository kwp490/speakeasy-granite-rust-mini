import { useEffect, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Makes a no-activate window's chrome drag the window, and reports the
 * settled position to `persist` once the drag ends.
 *
 * Shared by the default HUD and the side dock, which are the same
 * no-activate window family (§ side-dock mode) and move the same way — only
 * where each persists its landing position differs, which is why this takes
 * `persist` as a parameter rather than a hard-coded command name.
 *
 * Verified against a running no-activate, undecorated window before any of
 * this was built (§17 step 3): the cursor delta and the window delta match
 * exactly, and the foreground window does not change. `startDragging` needs
 * `core:window:allow-start-dragging`, which `core:default` does *not*
 * include — without it the call is refused silently and the window simply
 * never moves.
 */
export function useDragToMove(persist: (x: number, y: number) => void) {
  const dragging = useRef(false);
  // Read by the settle handler only, so passing a fresh inline closure on
  // every render does not tear down and rebuild the listeners below.
  const persistRef = useRef(persist);
  persistRef.current = persist;

  useEffect(() => {
    const window_ = getCurrentWindow();

    const onMouseDown = (event: MouseEvent) => {
      if (event.button !== 0) return;
      const target = event.target as HTMLElement | null;
      // The controls sit inside the drag region; without this their clicks get
      // swallowed by the OS move loop (§6.1).
      if (target?.closest("button, select, input, a") !== null) return;
      if (target?.closest("[data-tauri-drag-region]") === null) return;
      dragging.current = true;
      void window_.startDragging();
    };

    const persistPosition = () => {
      if (!dragging.current) return;
      dragging.current = false;
      void window_
        .outerPosition()
        .then((position) => persistRef.current(position.x, position.y))
        .catch(() => {
          // Placement is a convenience. Failing to store it must not surface as
          // an error over a dictation.
        });
    };

    document.addEventListener("mousedown", onMouseDown);
    // The OS move loop swallows mouseup, so the drag is settled on the next
    // interaction or when the pointer comes back over the window.
    document.addEventListener("mouseup", persistPosition);
    window.addEventListener("blur", persistPosition);
    window.addEventListener("mouseover", persistPosition);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("mouseup", persistPosition);
      window.removeEventListener("blur", persistPosition);
      window.removeEventListener("mouseover", persistPosition);
    };
  }, []);
}
