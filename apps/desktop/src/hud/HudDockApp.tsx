import type { MouseEvent } from "react";
import { useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { messages } from "../catalog";
import type { TranscriberModel } from "../state/transcriberState";
import { useHudStatus } from "../state/useHudStatus";
import { DockLevelMeter } from "./DockLevelMeter";
import { formatElapsed } from "./format";
import { useDragToMove } from "./useDragToMove";

/**
 * The side dock: a narrow strip that clings to a screen edge, showing the
 * engine indicator, the level meter, the elapsed clock, and one button to end
 * the dictation.
 *
 * Six rows in a fixed order, and none of them is conditional — the same rule
 * `.capture-hud` follows, for the same reason. Only what sits *in* the last two
 * changes with state:
 *
 *     20px  chrome    the close button
 *    104px  wordmark  vertical, and this undecorated window's whole titlebar
 *     14px  engine    which device Granite runs on, and whether it is up
 *      1fr  meter     the waveform — 112px, after the engine row took 22
 *     16px  clock     the elapsed time, while a dictation is running
 *     28px  action    Stop, the working indicator, or how it ended
 *
 * The action row is the dock's whole account of what happened after the user
 * let go of the key. It used to hold Stop and nothing else, which meant that
 * from the moment a dictation ended until the text appeared — Granite's pass
 * plus finalization, comfortably over a second — the dock was indistinguishable
 * from idle, and a failure was indistinguishable from idle forever.
 *
 * Stop is here despite dictation being hotkey-driven. The dock's whole promise
 * is staying reachable while the user works in another window, and the hotkey
 * has three activation modes (Settings › General) — in hands-free mode there is
 * no key that ends a recording at all, so a dock with no Stop button leaves the
 * user's only way out on a window they moved away from on purpose.
 *
 * Drag persists the landing edge and y through `hud_dock_placement_configure`
 * rather than a raw position — see that command for why. Right-click opens the
 * native menu (`hud_dock_context_menu`) for Settings and Close; there is
 * deliberately no left-click equivalent, so a drag in progress can never be
 * mistaken for one of those two.
 */
export function HudDockApp() {
  const { model, stop } = useHudStatus();

  useDragToMove((x, y) => {
    void invoke("hud_dock_placement_configure", { x, y });
  });

  const onContextMenu = useCallback((event: MouseEvent) => {
    event.preventDefault();
    void invoke("hud_dock_context_menu");
  }, []);

  const elapsedMs = model.state.kind === "listening" ? model.state.elapsedMs : null;
  const listening = elapsedMs !== null;

  return (
    <main
      aria-label={messages.transcriber}
      className="hud-dock"
      data-session={model.state.kind}
      data-tauri-drag-region
      data-testid="capture-hud-dock"
      onContextMenu={onContextMenu}
      // The dock has no room to print the shortcut, and the window it would
      // have printed it on is the one the user docked to get out of the way.
      // A hover tooltip is the only surface left. It states the binding and
      // nothing about how to hold it: which of press-to-toggle, push-to-talk
      // and hands-free is configured is not in the status payload, so a
      // "hold to talk" here would be wrong in two of the three modes.
      title={shortcutTooltip(model)}
    >
      <header className="hud-dock-chrome">
        <button
          aria-label={messages.closeTranscriber}
          className="hud-icon hud-close"
          onClick={() => {
            void getCurrentWindow().close();
          }}
          title={messages.closeTranscriber}
          type="button"
        >
          <CloseGlyph />
        </button>
      </header>
      <div className="hud-dock-wordmark">{messages.productName}</div>
      <EngineChip engine={model.engine} device={model.engineDevice} />
      <div className="hud-dock-level-wrap">
        <DockLevelMeter active={listening} level={model.level} />
      </div>
      {/*
        Both slots keep their height in every state. They are empty most of the
        time, which costs 44px the dock does not otherwise need — and buys that
        the waveform's box is the same box before, during and after a dictation
        rather than jumping 44px the moment one starts.
      */}
      <div className="hud-dock-clock">
        {elapsedMs !== null && (
          <output aria-live="polite" data-testid="hud-dock-timer">
            {formatElapsed(elapsedMs)}
          </output>
        )}
      </div>
      <div className="hud-dock-action">
        {listening && (
          <button
            aria-label={messages.stopDictationName}
            className="hud-dock-stop"
            data-testid="hud-dock-stop"
            onClick={stop}
            title={messages.stopDictationName}
            type="button"
          >
            {messages.stopDictation}
          </button>
        )}
        <DockOutcome model={model} />
      </div>
    </main>
  );
}

/**
 * Warm states that are still on their way to being loaded. Kept in step with
 * `ENGINE_LOADING` in `transcriberState.ts`, which is the same question asked
 * for a different purpose — one decides the session state, this decides a
 * colour.
 */
const ENGINE_PENDING: ReadonlySet<string> = new Set(["cold", "warming"]);

/**
 * Warm states that are not loading and are not a fault: the engine simply has
 * nothing to load yet.
 *
 * Only `not_configured`, and the distinction is the point. A machine with no
 * model pack has not finished setting up and the dock already says so in
 * words; painting that red would make a fresh install's first impression a
 * fault chip. A *missing worker binary* is `granite_worker_missing` and is not
 * in here, because that is a broken installation rather than a to-do.
 */
const ENGINE_UNCONFIGURED: ReadonlySet<string> = new Set(["not_configured"]);

/** Backend device codes that are not devices, and must never be shown as one. */
const NOT_A_DEVICE: ReadonlySet<string> = new Set([
  "unknown",
  "not_configured",
  "granite_state_unavailable",
]);

/**
 * The engine indicator: which device Granite runs on, and whether it is up.
 *
 * Readable without hovering, which is why it costs a row rather than living in
 * the tooltip — the dock exists to be glanceable while the user works in
 * another window, and a fact you have to hover for is a fact you do not have.
 *
 * **It never claims a device the worker has not reported.** `device()` answers
 * `not_configured` before any warm and `unknown` for a pre-v2 worker, and
 * during `warming` the worker has usually not answered `Hello` yet. The honest
 * chip there is amber with an em dash, not amber with a guess — inferring
 * `GPU` from a CUDA-capable *binary* is exactly the overreach that once put
 * `device=cuda` in a support log for a worker running on the processor.
 *
 * State is carried by the pip's shape as well as its hue, because colour is
 * never the only signal (UI-GUIDE "Contrast, themes, and motion") and because
 * under `forced-colors` every fill flattens to the same system colour.
 */
function EngineChip({ engine, device }: { engine: string; device: string }) {
  const health = ENGINE_PENDING.has(engine)
    ? "warming"
    : ENGINE_UNCONFIGURED.has(engine)
      ? "unconfigured"
      : engine === "ready"
        ? "ready"
        : "failed";
  const label = NOT_A_DEVICE.has(device) ? messages.engineDeviceUnknown : deviceLabel(device);
  const description = engineChipDescription(health, engine, label);
  return (
    <div className="hud-dock-engine">
      <span
        aria-label={description}
        className="hud-dock-engine-chip"
        data-health={health}
        data-testid="hud-dock-engine"
        role="img"
        title={description}
      >
        {health === "failed" ? <AlertPip /> : <span className="hud-dock-engine-pip" />}
        {label}
      </span>
    </div>
  );
}

/** `cpu` and `cuda` are wire codes; `CPU` and `GPU` are what a person reads. */
function deviceLabel(device: string): string {
  return messages.engineDevices[device as keyof typeof messages.engineDevices] ?? device;
}

/**
 * The chip's whole sentence, for the accessible name and the tooltip alike.
 *
 * Both facts in words, because the dock never takes keyboard focus and this
 * name is the entirety of what a screen reader gets — "GPU ready" would be
 * neither of them. The failed state names its code, because the dock's tooltip
 * is the only surface on this window that can say *which* failure.
 */
function engineChipDescription(health: string, engine: string, label: string): string {
  if (health === "ready") return messages.engineChipReady(label);
  if (health === "warming") return messages.engineChipWarming;
  if (health === "unconfigured") return messages.engineChipUnconfigured;
  return messages.engineChipFailed(formatError(engine));
}

/**
 * A triangle rather than a circle, so the failed state differs in shape and not
 * only in hue. Inline SVG to match the close, clipboard and alert glyphs this
 * file already draws.
 */
function AlertPip() {
  return (
    <svg aria-hidden="true" className="hud-dock-engine-pip" focusable="false" viewBox="0 0 10 10">
      <path d="M5 1 9.5 9 0.5 9Z" fill="currentColor" />
    </svg>
  );
}

/**
 * What the action row shows when it is not showing Stop.
 *
 * Three states earn a mark and the rest deliberately do not:
 *
 * - **working** — `stopping` and `transcribing`. Three dots that fade in
 *   sequence, in the same `--hud-busy` amber the deleted HUD's record button
 *   used for its own processing tone — one colour for "the app is thinking",
 *   kept so the vocabulary survived the window. This is the state the dock had
 *   no answer for.
 * - **refused** — delivered, but the target app would not take the text, so it
 *   is on the clipboard. A clipboard mark rather than a warning colour, because
 *   *what to do next* is the message and it is a different action from a
 *   failure (UI-GUIDE "Contrast, themes, and motion": never colour alone).
 * - **failed** — a warning triangle, and the specific error on hover.
 *
 * A successful insertion shows nothing. The text arriving in the app the user
 * was typing into is the confirmation, and a dock that also announced it would
 * be claiming credit for something already visible — while costing a mark that
 * has to clear itself, or linger and mean nothing.
 */
function DockOutcome({ model }: { model: TranscriberModel }) {
  const state = model.state;
  if (state.kind === "stopping" || state.kind === "transcribing") {
    return (
      <span
        aria-label={messages.transcriberStates.transcribing}
        className="hud-dock-working"
        data-testid="hud-dock-working"
        role="img"
        title={messages.transcriberStates.transcribing}
      >
        {/* Three elements rather than one animated glyph: the animation is a
            delay per dot, so it degrades to three visible dots under
            `prefers-reduced-motion` instead of degrading to nothing. */}
        <span className="hud-dock-working-dot" />
        <span className="hud-dock-working-dot" />
        <span className="hud-dock-working-dot" />
      </span>
    );
  }
  if (state.kind === "delivered" && state.outcome === "refused") {
    return (
      <span
        aria-label={messages.deliveredRefusedStatus}
        className="hud-dock-outcome"
        data-testid="hud-dock-outcome"
        data-outcome="refused"
        role="img"
        title={messages.deliveredRefusedStatus}
      >
        <ClipboardGlyph />
      </span>
    );
  }
  if (state.kind === "failed") {
    return (
      <span
        aria-label={messages.transcriberStates.failed}
        className="hud-dock-outcome"
        data-testid="hud-dock-outcome"
        data-outcome="failed"
        role="img"
        // The code, not the generic line: this is the only place the dock can
        // say *which* failure, and it has one tooltip to do it in.
        title={formatError(state.code)}
      >
        <AlertGlyph />
      </span>
    );
  }
  return null;
}

function formatError(code: string): string {
  return messages.errors[code as keyof typeof messages.errors] ?? messages.errorUnknown;
}

/**
 * What the window says about itself on hover: the shortcut when one is
 * actually registered, and what the window is when none is.
 *
 * There was a `shortcutUnavailable` string for the no-shortcut case and this
 * deliberately did not reuse it: it ended by pointing at "Start recording", a
 * control the deleted HUD carried and the dock does not. Saying so here was the
 * only thing referencing it, which kept it in `catalog.ts` reading as live copy
 * until the unreachable-entry sweep on 2026-08-28 removed it.
 */
function shortcutTooltip(model: TranscriberModel): string {
  if (model.hotkeyRegistration !== "registered" || model.hotkeyBinding === "") {
    return messages.transcriber;
  }
  return messages.shortcutHint(model.hotkeyBinding);
}

function CloseGlyph() {
  return (
    <svg aria-hidden="true" focusable="false" height="13" viewBox="0 0 16 16" width="13">
      <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeLinecap="round" strokeWidth="1.6" />
    </svg>
  );
}

/**
 * Two sheets, for "the text is on the clipboard". Deliberately the same drawing
 * the deleted default HUD's Copy button used — it reports the same fact, and
 * two glyphs for one meaning is how a user learns that they are different
 * meanings.
 */
function ClipboardGlyph() {
  return (
    <svg aria-hidden="true" focusable="false" height="15" viewBox="0 0 16 16" width="15">
      <rect fill="none" height="9" rx="1.6" stroke="currentColor" strokeWidth="1.5" width="8" x="6" y="5" />
      <path
        d="M4 11H3.4A1.4 1.4 0 0 1 2 9.6V3.4A1.4 1.4 0 0 1 3.4 2h6.2A1.4 1.4 0 0 1 11 3.4V4"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
      />
    </svg>
  );
}

/**
 * A warning triangle, for a dictation that ended without a transcript.
 *
 * A triangle rather than a circle so it is distinguishable from the clipboard
 * mark by outline alone, which is what has to carry it under `forced-colors:
 * active` where both flatten to the same system colour.
 */
function AlertGlyph() {
  return (
    <svg aria-hidden="true" focusable="false" height="15" viewBox="0 0 16 16" width="15">
      <path
        d="M8 2.4 14.4 13.4H1.6z"
        fill="none"
        stroke="currentColor"
        strokeLinejoin="round"
        strokeWidth="1.5"
      />
      <path d="M8 6.4v3.1" stroke="currentColor" strokeLinecap="round" strokeWidth="1.6" />
      <circle cx="8" cy="11.5" fill="currentColor" r="0.9" />
    </svg>
  );
}
