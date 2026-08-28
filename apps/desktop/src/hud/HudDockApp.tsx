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
 * The side dock: a narrow strip that clings to a screen edge, showing the level
 * meter, the engine indicator, the elapsed clock, and one button that starts and
 * ends the dictation.
 *
 * Six rows in a fixed order, and none of them is conditional — the same rule
 * `.capture-hud` follows, for the same reason. Only what sits *in* the last two
 * changes with state:
 *
 *     20px  chrome    settings and close
 *    104px  wordmark  vertical, and this undecorated window's whole titlebar
 *      1fr  meter     the waveform — 152px in a 400px window
 *     14px  engine    which device Granite runs on, and whether it is up
 *     16px  status    the elapsed time during a run, how it ended after one
 *     28px  action    the one button, present in every state
 *
 * **The engine row sits below the meter, not above it** (owner, 2026-08-28).
 * Between the wordmark and the meter it was a filled horizontal pill cutting
 * across a 52px-wide vertical column — it severed the mark from the waveform,
 * the dotted meter read as hanging off it, and the card's brightest element sat
 * at the visual centre while the bottom third was empty. Below the meter the
 * three state rows cluster at the bottom and the top is pure identity. Measured
 * on the running window before and after: the reorder costs the waveform
 * nothing, because the meter is the only `1fr` either way.
 *
 * **The action row's button is present in every state** (owner, 2026-08-28), and
 * that is why the window grew from 360 to 400. It used to appear only while
 * listening, which meant the dock offered no way to *begin* a dictation at all —
 * the one surface whose whole promise is staying reachable while the user works
 * elsewhere had a control that only existed once they had already started. The
 * label is the state: `Ready`, `Stop`, or the working dots.
 *
 * `Transcribing` is not among those labels because it does not fit, which was
 * measured rather than assumed: the label has 36px between the row's inset and
 * the button's own padding, and the word needs 60.2px at the button's 0.68rem
 * and still 47.8px at 0.54rem — below the smallest type anywhere in the app. The
 * working dots carry that state instead, they are already this window's idiom
 * for "thinking", and the full `Transcribing…` is in the accessible name and the
 * tooltip where there is room for it.
 *
 * The status row above it carries the elapsed time during a run and the outcome
 * glyph after one. Those used to live in two rows, and the outcome was in the
 * action row — which a permanently present button leaves no room for at 52px.
 * Both are facts about one dictation and neither is ever needed at the same
 * moment as the other, so they share.
 *
 * A dock button is not redundant with the hotkey. The hotkey has three
 * activation modes (Settings › General) and in hands-free mode there is no key
 * that ends a recording at all, so a dock with no Stop leaves the user's only
 * way out on a window they moved away from on purpose.
 *
 * Drag persists the landing edge and y through `hud_dock_placement_configure`
 * rather than a raw position — see that command for why. Right-click opens the
 * native menu (`hud_dock_context_menu`) for Settings and Close; there is
 * deliberately no left-click equivalent, so a drag in progress can never be
 * mistaken for one of those two.
 */
export function HudDockApp() {
  const { model, start, stop } = useHudStatus();

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
        {/* Settings is on the right-click menu too, and that is exactly why this
            is here: a native popup is discoverable only by someone who already
            guessed to try it, on a window that never takes keyboard focus. */}
        <button
          aria-label={messages.settings}
          className="hud-icon hud-dock-gear"
          data-testid="hud-dock-settings"
          onClick={() => {
            void invoke("open_settings_window");
          }}
          title={messages.settings}
          type="button"
        >
          <GearGlyph />
        </button>
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
      <div className="hud-dock-level-wrap">
        <DockLevelMeter active={listening} level={model.level} />
      </div>
      <EngineChip engine={model.engine} device={model.engineDevice} />
      {/*
        Keeps its height in every state, and is empty between dictations. That
        costs 16px the dock does not otherwise need, and buys the waveform's box
        being the same box before, during and after a dictation rather than
        moving the moment one starts.
      */}
      <div className="hud-dock-status">
        {elapsedMs !== null ? (
          <output aria-live="polite" data-testid="hud-dock-timer">
            {formatElapsed(elapsedMs)}
          </output>
        ) : (
          <DockOutcome model={model} />
        )}
      </div>
      <div className="hud-dock-action">
        <DockActionButton model={model} onStart={start} onStop={stop} />
      </div>
    </main>
  );
}

/**
 * The dock's one button, in every state the session can be in.
 *
 * Three presentations and exactly one of them is a lie waiting to happen, which
 * is why the resting label is keyed off `idle` alone rather off "not listening":
 * saying `Ready` while Granite loads two gigabytes is the defect 1.7.0 fixed in
 * the engine chip, and a button is a louder place to repeat it. Every state that
 * is not idle and not listening shows the working dots and is disabled.
 *
 * `canStart` and `canStop` gate the presses rather than the presentation. The
 * backend owns whether a dictation may begin — one at a time, refused rather
 * than queued — and a button that looks pressable and is refused is better than
 * one that vanishes, because the refusal is the honest answer to a press the
 * user meant.
 */
function DockActionButton({
  model,
  onStart,
  onStop,
}: {
  model: TranscriberModel;
  onStart: () => void;
  onStop: () => void;
}) {
  const kind = model.state.kind;
  if (kind === "listening" || kind === "starting") {
    return (
      <button
        aria-label={messages.stopDictationName}
        className="hud-dock-stop"
        data-testid="hud-dock-stop"
        disabled={!model.canStop}
        onClick={onStop}
        title={messages.stopDictationName}
        type="button"
      >
        {messages.stopDictation}
      </button>
    );
  }
  if (kind === "idle" || kind === "delivered" || kind === "failed") {
    return (
      <button
        aria-label={messages.startDictationName}
        className="hud-dock-stop"
        data-testid="hud-dock-start"
        disabled={!model.canStart}
        onClick={onStart}
        title={messages.startDictationName}
        type="button"
      >
        {messages.transcriberStates.idle}
      </button>
    );
  }
  // Loading, setting up, stopping, transcribing: busy, and not pressable. The
  // name says which, because the dots cannot.
  const busy =
    kind === "setup_required"
      ? messages.transcriberStates.setupRequired
      : kind === "loading_model"
        ? messages.transcriberStates.loadingModel
        : messages.transcriberStates.transcribing;
  return (
    <span
      aria-label={busy}
      className="hud-dock-working"
      data-testid="hud-dock-working"
      role="img"
      title={busy}
    >
      {/* Three elements rather than one animated glyph: the animation is a delay
          per dot, so it degrades to three visible dots under
          `prefers-reduced-motion` instead of degrading to nothing. */}
      <span className="hud-dock-working-dot" />
      <span className="hud-dock-working-dot" />
      <span className="hud-dock-working-dot" />
    </span>
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
 * How the last dictation ended, in the status row.
 *
 * Two states earn a mark and the rest deliberately do not:
 *
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
 *
 * **`stopping` and `transcribing` used to be a third case here**, and are not
 * any more: the action row's button carries them now, because it is present in
 * every state and a busy mark in two rows at once is one row of it lying about
 * being a second fact. This component is only reached when there is no elapsed
 * time to show, so it can never race the clock for the row either.
 */
function DockOutcome({ model }: { model: TranscriberModel }) {
  const state = model.state;
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
 * The settings gear, drawn as a ring and six teeth rather than a filled cog.
 *
 * Stroked at the same 1.6 weight as the × beside it, because the two sit 20px
 * apart on the thinnest strip in the app and a filled glyph next to a stroked
 * one reads as the heavier of the two being the primary action. It is not: the
 * one that closes the window is.
 */
function GearGlyph() {
  return (
    <svg aria-hidden="true" focusable="false" height="13" viewBox="0 0 16 16" width="13">
      <circle cx="8" cy="8" fill="none" r="2.5" stroke="currentColor" strokeWidth="1.5" />
      <path
        d="M8 1.4v2.1M8 12.5v2.1M2.34 4.7l1.82 1.05M11.84 10.25l1.82 1.05M2.34 11.3l1.82-1.05M11.84 5.75l1.82-1.05"
        stroke="currentColor"
        strokeLinecap="round"
        strokeWidth="1.5"
      />
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
