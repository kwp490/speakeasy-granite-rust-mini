import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import { formatShortcutState } from "./format";
import { readWithRetry } from "./readWithRetry";
import type { HotkeyStatus } from "./types";
import type { ProfileController } from "./useProfile";

/**
 * General: the shortcut, the side dock, recording feedback, Windows startup,
 * and the keyboard paths that compensate for the dock never taking focus.
 *
 * Registration state is reported in plain language — "Shortcut active", not
 * "HOTKEY REGISTRATION / Registered" (UI-GUIDE "Two vocabulary registers").
 * The contract vocabulary for it lives
 * on the Advanced page.
 */
export function General({
  profile,
}: {
  profile: ProfileController;
}) {
  const [hotkey, setHotkey] = useState<HotkeyStatus | null>(null);
  const [hotkeyUnavailable, setHotkeyUnavailable] = useState(false);
  // Empty until the read answers, and never a literal shortcut. This used to be
  // `Ctrl+Alt+L` -- SpeakEasy's binding, inherited by the fork and never
  // rebranded -- which made the lost read below actively destructive rather than
  // merely wrong: the field showed a shortcut this app does not use, and the
  // remedy the panel implied ("Save hotkey") would have rebound the working
  // `Ctrl+Alt+P` to the *other* product's shortcut, on a machine where both are
  // installed side by side and would then conflict.
  const [binding, setBinding] = useState("");
  const [mode, setMode] = useState<HotkeyStatus["mode"]>("toggle");
  const [enabled, setEnabled] = useState(true);
  const [hotkeyAction, setHotkeyAction] = useState("");
  const bindingField = useRef<HTMLInputElement>(null);

  // Retried until registration is no longer `pending`, which is two fixes for
  // one symptom -- and the second is the one that reproduces on every launch.
  //
  // This read was fired once with no rejection handler, so it could lose the race
  // against `setup` managing `HotkeyCoordinator` and stay `null` forever. But it
  // can also *succeed* and answer `pending`: the coordinator starts there and
  // `register_activation_hotkey` runs at the **end** of `setup`, after the tray
  // is built, while every window's React tree has already mounted and read. The
  // page then held a value that was true for one moment of the process,
  // rendering "Shortcut not registered yet" with the shortcut registered and
  // working.
  //
  // The two are indistinguishable from the screen -- same string, and the panel
  // cannot say which happened. They were separated by reloading this window and
  // watching the same page report "Shortcut active" from the same backend
  // (2026-08-26, installed release frontend). Whichever it was, this is worse
  // than the empty dictionary list the first occurrence produced: it names a
  // working feature as broken, in the one panel someone opens *because* their
  // shortcut seems not to work.
  useEffect(() => {
    void readWithRetry<HotkeyStatus>(
      "hotkey_status",
      (status) => status.registration !== "pending",
    ).then(
      (status) => {
        setHotkey(status);
        setHotkeyUnavailable(false);
        setBinding(status.binding);
        setMode(status.mode);
        setEnabled(status.enabled);
      },
      () => {
        setHotkeyUnavailable(true);
      },
    );
  }, []);

  async function saveHotkey() {
    try {
      await invoke("hotkey_configure", { binding, mode, enabled });
      setHotkey(await readWithRetry<HotkeyStatus>("hotkey_status"));
      setHotkeyUnavailable(false);
      setHotkeyAction(messages.hotkeySaved);
    } catch {
      setHotkeyAction(messages.hotkeySaveFailed);
    }
  }

  return (
    <>
      <section aria-labelledby="general-shortcut">
        <h3 id="general-shortcut">{messages.shortcutSection}</h3>
        {/*
          `unknown`, never `pending`. Both are real backend values and they mean
          different things: `pending` is "registration has not been attempted
          yet", which is a claim about the app, and its copy reads "Shortcut not
          registered yet". Defaulting to it reported an unanswered *read* as an
          unregistered *shortcut*. `undefined` means the page does not know, and
          "Shortcut state unknown" is what that is.
        */}
        <p className="setting-status" data-testid="shortcut-state">
          {formatShortcutState(hotkey?.registration ?? "unknown")}
        </p>
        {hotkeyUnavailable && <p className="warning">{messages.shortcutStateUnavailable}</p>}
        <p className="setting-detail">{messages.shortcutDetail}</p>
        {hotkey?.registration === "conflict" && (
          <button
            className="secondary"
            onClick={() => bindingField.current?.focus()}
            type="button"
          >
            {messages.changeShortcut}
          </button>
        )}
        <div className="setting-fields">
          <label>
            <span>{messages.hotkeyBinding}</span>
            <input
              onChange={(event) => setBinding(event.target.value)}
              ref={bindingField}
              type="text"
              value={binding}
            />
          </label>
          <label>
            <span>{messages.hotkeyMode}</span>
            <select
              onChange={(event) => setMode(event.target.value as HotkeyStatus["mode"])}
              value={mode}
            >
              <option value="toggle">{messages.hotkeyModeToggle}</option>
              <option value="push_to_talk">{messages.hotkeyModePushToTalk}</option>
              <option value="hands_free">{messages.hotkeyModeHandsFree}</option>
            </select>
          </label>
        </div>
        <label className="confirmation">
          <input
            checked={enabled}
            onChange={(event) => setEnabled(event.target.checked)}
            type="checkbox"
          />
          {messages.hotkeyEnabledLabel}
        </label>
        <div className="actions">
          {/*
            Disabled until the status is known. Saving from an unanswered read
            would write the empty binding and the default mode over settings this
            page has never read -- a Save that silently changes what it claims to
            be preserving.
          */}
          <button disabled={hotkey === null} onClick={() => void saveHotkey()} type="button">
            {messages.saveHotkey}
          </button>
          <output aria-live="polite">{hotkeyAction}</output>
        </div>
      </section>

      <section aria-labelledby="general-dock">
        <h3 id="general-dock">{messages.dockSection}</h3>
        <p className="setting-detail">{messages.dockAlwaysOnTop}</p>
      </section>

      <section aria-labelledby="general-feedback">
        <h3 id="general-feedback">{messages.recordingFeedbackSection}</h3>
        <label className="confirmation">
          <input
            checked={profile.profile?.recording_feedback_enabled ?? true}
            onChange={(event) => void profile.setRecordingFeedback(event.target.checked)}
            type="checkbox"
          />
          {messages.recordingFeedback}
        </label>
        <p className="setting-detail">{messages.recordingFeedbackDetail}</p>
      </section>

      <section aria-labelledby="general-startup">
        <h3 id="general-startup">{messages.startupSection}</h3>
        <label className="confirmation">
          <input
            checked={profile.profile?.startup_with_windows ?? false}
            onChange={(event) => void profile.setStartup(event.target.checked)}
            type="checkbox"
          />
          {messages.startupWithWindows}
        </label>
      </section>

      {/*
        UI-GUIDE "Accessibility and input": the dock is not keyboard operable by
        design, so every action it
        offers needs a path that is. The shortcut covers start and stop, the Audio
        page covers the microphone, and these two cover the rest.
      */}
      <section aria-labelledby="general-keyboard">
        <h3 id="general-keyboard">{messages.keyboardPathsSection}</h3>
        <p className="setting-detail">{messages.keyboardPathsDetail}</p>
        <div className="actions">
          <button
            className="destructive"
            onClick={() => {
              void invoke("app_quit");
            }}
            type="button"
          >
            {messages.quitApp}
          </button>
        </div>
        <p className="setting-detail">{messages.quitAppDetail}</p>
      </section>
    </>
  );
}
