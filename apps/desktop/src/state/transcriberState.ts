/**
 * The one state model for dictation (docs/archive/UI-REDESIGN.md §7).
 *
 * Transcription state used to be spread across `CaptureWizardStatus.state` plus
 * three booleans, `CaptureHudView.session`, `RecoverableResult.state` and 30+
 * React hooks. It is consolidated here, derived from the backend views rather
 * than re-implemented: the Rust coordinators stay authoritative.
 *
 * Components read this union. They never infer state from a label, a CSS class
 * or a scattered boolean.
 */

/** Schema version this reducer understands. Mismatches are rejected, not coerced. */
export const HUD_SCHEMA_VERSION = 1;

/**
 * How long a locally-optimistic start/stop is trusted before the poll is
 * believed again. `HotkeyCoordinator::DEBOUNCE` is 150 ms; a request that never
 * lands must not strand the HUD in `starting` forever, so this is the outer
 * bound on the optimistic overlay rather than the debounce itself.
 */
export const PENDING_TIMEOUT_MS = 4_000;

/** Minimum gap between two accepted start (or stop) requests. Mirrors `HotkeyCoordinator::DEBOUNCE`. */
export const REQUEST_DEBOUNCE_MS = 150;

export type SetupReason =
  | "onboarding_incomplete"
  | "model_missing"
  | "microphone_missing"
  | "shortcut_unavailable";

/**
 * What actually happened to the authoritative final.
 *
 * §6.3 forbids showing "Text inserted" unless `CommitWriter::write_focused`
 * returned `Ok`, so a refusal has to be representable. The brief's union lists
 * `inserted | copied`; `held` and `refused` are added because the two states
 * they describe are reachable today and silently mislabelling either of them
 * as `inserted` is exactly the lie §6.3 prohibits.
 */
export type DeliveryOutcome = "inserted" | "copied" | "held" | "refused";

export type TranscriberState =
  | { kind: "setup_required"; reason: SetupReason }
  /**
   * Installed and verified, but the resident engine has not finished loading
   * the model into the worker.
   *
   * Distinct from `setup_required` because there is nothing for the user to do
   * except wait, and distinct from `idle` because starting here blocks inside
   * `dictation_start` on the same mutex the load holds. Only reachable when
   * setup is otherwise complete and no dictation is running.
   */
  | { kind: "loading_model" }
  | { kind: "idle" }
  | { kind: "starting" }
  | { kind: "listening"; elapsedMs: number; level: number; device: string }
  | { kind: "stopping" }
  | { kind: "transcribing"; capturedSeconds: number }
  | {
      kind: "delivered";
      outcome: DeliveryOutcome;
      text: string;
      /**
       * A `speakeasy_asr::FinalSourceReason` code (e.g. `granite_failed`)
       * disclosing why Granite did not deliver. `null` whenever Granite
       * delivered or was never configured for this dictation.
       */
      sourceReasonCode: string | null;
    }
  | { kind: "failed"; code: string; recoverable: boolean };

/**
 * The extended `CaptureHudView` (§8.3). One 100 ms poll carries everything the
 * HUD needs, so the device name, shortcut and gating flags cost no extra IPC.
 */
export type HudStatus = {
  schema_version: number;
  sequence: number;
  session_id: string;
  session: string;
  vad: string;
  level: number;
  device_diagnostic: string;
  streaming_mode: string;
  mutable_text: string;
  stable_display_text: string;
  final_text: string;
  device_name: string;
  hotkey_binding: string;
  hotkey_registration: string;
  can_start: boolean;
  can_stop: boolean;
  setup_complete: boolean;
  setup_reason: string | null;
  elapsed_ms: number;
  ceiling_ms: number;
  preferred_device_id: string;
  delivery_outcome: string;
  /** `cold`, `warming`, `ready`, or an error code. See `StreamingEngineCoordinator::status`. */
  engine: string;
  error_code: string | null;
  final_source_reason: string | null;
};

/**
 * Everything a HUD component may read. `state` is the discriminated union; the
 * text tiers stay separate fields because §6.2 forbids collapsing them into one
 * string, and keeping them out of the union means a state transition can never
 * drop live text on the floor.
 */
export type TranscriberModel = {
  schemaVersion: number;
  sequence: number;
  state: TranscriberState;
  streamingMode: string;
  stableDisplayText: string;
  mutableText: string;
  finalText: string;
  deviceName: string;
  deviceDiagnostic: string;
  hotkeyBinding: string;
  hotkeyRegistration: string;
  vad: string;
  level: number;
  elapsedMs: number;
  ceilingMs: number;
  canStart: boolean;
  canStop: boolean;
  /** The resident engine's warm state, verbatim from the backend. */
  engine: string;
  /**
   * The stored microphone preference, or `""` when none is saved. The picker
   * completes the same fallback `hotkey_capture_device` does, so it shows the
   * device the next dictation will really use rather than claiming none is set.
   */
  preferredDeviceId: string;
  /** The dictation the backend is currently reporting on. */
  sessionId: string;
  /**
   * The session whose finished outcome the user has dismissed with Done.
   *
   * Without this, Done did nothing that lasted: it set the state to idle and the
   * next poll — 100 ms later — read `session: "complete"` from the backend and put
   * the delivered state straight back. The button looked broken because it was.
   * Dismissal is per session, so a new dictation is unaffected.
   */
  dismissedSessionId: string | null;
  /** A locally-optimistic request awaiting confirmation from the next poll. */
  pending: "start" | "stop" | null;
  pendingSince: number;
  /** Monotonic clock of the last accepted request, for debouncing. */
  lastRequestAt: number;
};

export type TranscriberAction =
  | { type: "status"; status: HudStatus; now: number }
  | { type: "start_requested"; now: number }
  | { type: "stop_requested"; now: number }
  | { type: "request_failed"; code: string; now: number }
  | { type: "dismissed"; now: number };

export const initialTranscriberModel: TranscriberModel = {
  schemaVersion: HUD_SCHEMA_VERSION,
  sequence: 0,
  // Before the first poll answers, the truthful claim is that the model is not
  // known to be ready — not that it is. The first response, 0 ms later, replaces
  // this with whatever the backend actually reports.
  state: { kind: "loading_model" },
  streamingMode: "final_only",
  stableDisplayText: "",
  mutableText: "",
  finalText: "",
  deviceName: "",
  deviceDiagnostic: "not_opened",
  hotkeyBinding: "",
  hotkeyRegistration: "pending",
  vad: "manual_stop_only",
  level: 0,
  elapsedMs: 0,
  ceilingMs: 0,
  canStart: false,
  canStop: false,
  // Assumed still loading until a poll says otherwise, so the first frame after
  // launch cannot flash a green Start Recording over a model that is not there.
  engine: "cold",
  preferredDeviceId: "",
  sessionId: "",
  dismissedSessionId: null,
  pending: null,
  pendingSince: 0,
  lastRequestAt: Number.NEGATIVE_INFINITY,
};

const SETUP_REASONS: ReadonlySet<string> = new Set<SetupReason>([
  "onboarding_incomplete",
  "model_missing",
  "microphone_missing",
  "shortcut_unavailable",
]);

const DELIVERY_OUTCOMES: ReadonlySet<string> = new Set<DeliveryOutcome>([
  "inserted",
  "copied",
  "held",
  "refused",
]);

/** Failures the user can retry from without leaving the HUD. */
const RECOVERABLE_CODES: ReadonlySet<string> = new Set([
  "runtime_adapter_failed",
  "runtime_deadline_exceeded",
  "runtime_worker_out_of_memory",
  "capture_empty",
  "capture_queue_overflow",
  "capture_device_unavailable",
  "capture_start_failed",
]);

function setupReasonOf(raw: string | null): SetupReason {
  return raw !== null && SETUP_REASONS.has(raw) ? (raw as SetupReason) : "onboarding_incomplete";
}

function deliveryOutcomeOf(raw: string): DeliveryOutcome {
  // Anything unrecognised is reported as `held`, never as `inserted`: an
  // unknown delivery result is not evidence that the text reached another app.
  return DELIVERY_OUTCOMES.has(raw) ? (raw as DeliveryOutcome) : "held";
}

/**
 * Engine warm states that mean "not ready, but on its way".
 *
 * `cold` is included because it covers the gap between the window appearing and
 * the launch warm thread reaching `ensure_ready`. Anything else — `ready` or an
 * error code — reads as ready: a failed warm costs live streaming text, not the
 * ability to dictate, so it must not park the button on a load that has already
 * given up.
 */
const ENGINE_LOADING: ReadonlySet<string> = new Set(["cold", "warming"]);

/** Derives the union from one backend snapshot, ignoring any optimistic overlay. */
function stateFromStatus(status: HudStatus, dismissedSessionId: string | null): TranscriberState {
  if (!status.setup_complete) {
    return { kind: "setup_required", reason: setupReasonOf(status.setup_reason) };
  }
  // A finished outcome the user has already dismissed reads as idle. The backend
  // keeps reporting `complete` until the next dictation begins, which is correct —
  // it is still the last thing that happened — but the user has said they are done
  // reading it.
  if (
    dismissedSessionId !== null &&
    status.session_id === dismissedSessionId &&
    (status.session === "complete" || status.session === "failed")
  ) {
    return idleOrLoading(status);
  }
  switch (status.session) {
    case "starting":
      return { kind: "starting" };
    case "streaming":
      return {
        kind: "listening",
        elapsedMs: status.elapsed_ms,
        level: status.level,
        device: status.device_name,
      };
    case "stopping":
      return { kind: "stopping" };
    case "finalizing":
      return { kind: "transcribing", capturedSeconds: status.elapsed_ms / 1_000 };
    case "complete":
      return {
        kind: "delivered",
        outcome: deliveryOutcomeOf(status.delivery_outcome),
        text: status.final_text,
        sourceReasonCode: status.final_source_reason,
      };
    case "failed": {
      const code = status.error_code ?? "runtime_adapter_failed";
      return { kind: "failed", code, recoverable: RECOVERABLE_CODES.has(code) };
    }
    default:
      return idleOrLoading(status);
  }
}

/**
 * Resting state: waiting on the model load, or genuinely ready.
 *
 * Only reached when no dictation is running. A load that is still finishing
 * during a dictation is not the user's problem — capture runs either way, and
 * the only casualty is live text — so an active session always outranks it.
 */
function idleOrLoading(status: HudStatus): TranscriberState {
  return ENGINE_LOADING.has(status.engine) ? { kind: "loading_model" } : { kind: "idle" };
}

/** True once the backend has caught up with (or overtaken) an optimistic request. */
function pendingSettled(pending: "start" | "stop", status: HudStatus): boolean {
  const active = status.session === "streaming" || status.session === "starting";
  return pending === "start" ? active : !active;
}

export function transcriberReducer(
  current: TranscriberModel,
  action: TranscriberAction,
): TranscriberModel {
  switch (action.type) {
    case "status": {
      const { status } = action;
      // Both guards carried over from phase1Reducer.ts, applied to every field
      // the extended view carries — including the §8.3 additions, which bump
      // `sequence` on their own publishes.
      if (status.schema_version !== HUD_SCHEMA_VERSION) return current;
      if (status.sequence < current.sequence) return current;

      const stale =
        current.pending !== null &&
        !pendingSettled(current.pending, status) &&
        action.now - current.pendingSince < PENDING_TIMEOUT_MS;

      const optimistic: TranscriberState =
        current.pending === "start" ? { kind: "starting" } : { kind: "stopping" };

      /**
       * Whether this snapshot belongs to a session the user has dismissed.
       *
       * `stateFromStatus` already consults the dismissal, so Done put the *state*
       * back to idle and it stayed there. The text tiers are separate fields by
       * §6.2, and they were not consulted at all — so the poll 100 ms after Done
       * wrote `final_text` straight back and the transcript the outcome referred
       * to stayed on screen underneath an idle transcriber. Dismissing has to
       * clear both or it clears neither in any way the user can see.
       *
       * Found by clicking Done in the running window, not by reading this file:
       * the reducer's own tests asserted `state` and never looked at the text.
       */
      const dismissed =
        current.dismissedSessionId !== null && status.session_id === current.dismissedSessionId;

      return {
        ...current,
        schemaVersion: status.schema_version,
        sequence: status.sequence,
        state: stale ? optimistic : stateFromStatus(status, current.dismissedSessionId),
        streamingMode: status.streaming_mode,
        stableDisplayText: dismissed ? "" : status.stable_display_text,
        mutableText: dismissed ? "" : status.mutable_text,
        finalText: dismissed ? "" : status.final_text,
        deviceName: status.device_name,
        deviceDiagnostic: status.device_diagnostic,
        hotkeyBinding: status.hotkey_binding,
        hotkeyRegistration: status.hotkey_registration,
        vad: status.vad,
        level: status.level,
        elapsedMs: status.elapsed_ms,
        ceilingMs: status.ceiling_ms,
        canStart: status.can_start,
        canStop: status.can_stop,
        preferredDeviceId: status.preferred_device_id,
        sessionId: status.session_id,
        pending: stale ? current.pending : null,
        pendingSince: stale ? current.pendingSince : 0,
      };
    }

    case "start_requested": {
      // Idempotent and debounced: a second press inside the debounce window, or
      // while a request is already in flight, cannot open a second session.
      if (current.pending !== null) return current;
      if (action.now - current.lastRequestAt < REQUEST_DEBOUNCE_MS) return current;
      if (!current.canStart) return current;
      // `can_start` is true throughout the model load, because nothing is
      // missing — but `dictation_start` would block on the load's mutex for up
      // to a minute with the window frozen. The button is disabled here too;
      // this guard covers the click that lands before the render does.
      if (current.state.kind === "loading_model") return current;
      return {
        ...current,
        state: { kind: "starting" },
        pending: "start",
        pendingSince: action.now,
        lastRequestAt: action.now,
        finalText: "",
      };
    }

    case "stop_requested": {
      if (current.pending !== null) return current;
      if (action.now - current.lastRequestAt < REQUEST_DEBOUNCE_MS) return current;
      if (!current.canStop) return current;
      return {
        ...current,
        state: { kind: "stopping" },
        pending: "stop",
        pendingSince: action.now,
        lastRequestAt: action.now,
      };
    }

    case "request_failed":
      return {
        ...current,
        state: {
          kind: "failed",
          code: action.code,
          recoverable: RECOVERABLE_CODES.has(action.code),
        },
        pending: null,
        pendingSince: 0,
      };

    case "dismissed":
      // Only clears a finished outcome. An active session is never dismissible,
      // so Done can never silently discard speech.
      if (current.state.kind !== "delivered" && current.state.kind !== "failed") return current;
      return {
        ...current,
        state: { kind: "idle" },
        finalText: "",
        // Recorded so the next poll does not undo this.
        dismissedSessionId: current.sessionId,
      };

    default:
      return current;
  }
}

/** Remaining time before the safety ceiling, or `null` when no ceiling applies. */
export function remainingBeforeCeiling(model: TranscriberModel): number | null {
  if (model.ceilingMs <= 0) return null;
  return Math.max(0, model.ceilingMs - model.elapsedMs);
}

/**
 * Whether the ceiling warning band is active.
 *
 * The shipped ceiling is two minutes, so the warning band is the final 30
 * seconds. It is capped at a quarter of the ceiling so it stays a warning
 * rather than becoming permanently lit whenever a shorter ceiling is used — a
 * notice that is always on tells the user nothing.
 */
export function ceilingWarningActive(model: TranscriberModel): boolean {
  if (model.state.kind !== "listening") return false;
  const remaining = remainingBeforeCeiling(model);
  if (remaining === null) return false;
  return remaining <= Math.min(120_000, model.ceilingMs / 4);
}
