import { useCallback, useRef, useState } from "react";

import { messages } from "../catalog";
import { formatError } from "./format";

/**
 * One user-initiated settings action, with a visible pending and error state.
 *
 * Every action on these pages used to be `void invoke(...).then(setSomething)`
 * with **no rejection handler**. Three things followed from that, and all three
 * are the same defect the rest of this app spends its comments avoiding: an
 * unhandled promise rejection, a control that reports success by not erroring,
 * and — on the destructive ones — optimistic state that stayed changed after
 * the backend refused. A user who pressed "Delete persisted history" and got a
 * `history_delete_failed` saw the confirmation check box clear and the word
 * "Deleted" appear.
 *
 * The rule this enforces is the UI-GUIDE one: **nothing is claimed unless it
 * happened.** `run` resolves to the backend's value or `null`; the caller
 * updates its own state only on the former. `status` is what the button
 * renders, and it is exhaustive rather than a boolean, because "not pending"
 * and "succeeded" are different things to say.
 *
 * Duplicate submissions are refused while one is in flight. That is not a
 * nicety on this page: `history_delete_all` and `reset_commit` are destructive
 * and `diagnostics_export` writes a file per press.
 */
export type MutationStatus = "idle" | "pending" | "succeeded" | "failed";

export type Mutation<T> = {
  /** What the control should render. */
  status: MutationStatus;
  /** Catalog prose for the failure, or `null` while it has not failed. */
  error: string | null;
  /** True while a request is outstanding, for `disabled`. */
  pending: boolean;
  /** The success message the caller set, or `null`. */
  message: string | null;
  /**
   * Runs `action`, resolving to its value on success and `null` on refusal.
   *
   * A second call while one is in flight resolves to `null` without running
   * anything, so a double click cannot export twice or delete twice.
   */
  run: (action: () => Promise<T>, describe?: (value: T) => string) => Promise<T | null>;
  /** Returns the control to rest, for a cancel or a dismissed panel. */
  reset: () => void;
};

export function useMutation<T>(): Mutation<T> {
  const [status, setStatus] = useState<MutationStatus>("idle");
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  // A ref rather than `status`, because two clicks in one frame both read the
  // same stale `status` and both would pass.
  const inFlight = useRef(false);

  const run = useCallback(
    async (action: () => Promise<T>, describe?: (value: T) => string): Promise<T | null> => {
      if (inFlight.current) return null;
      inFlight.current = true;
      setStatus("pending");
      setError(null);
      setMessage(null);
      try {
        const value = await action();
        setStatus("succeeded");
        setMessage(describe ? describe(value) : messages.done);
        return value;
      } catch (rejection: unknown) {
        // Through the catalog, so a backend code becomes a sentence with an
        // instruction in it. An unmapped code lands on `errorUnknown` rather
        // than being rendered raw — Advanced's "Show raw values" is the one
        // place an identifier may appear.
        setStatus("failed");
        setError(formatError(String(rejection)));
        return null;
      } finally {
        inFlight.current = false;
      }
    },
    [],
  );

  const reset = useCallback(() => {
    setStatus("idle");
    setError(null);
    setMessage(null);
  }, []);

  return { status, error, pending: status === "pending", message, run, reset };
}
