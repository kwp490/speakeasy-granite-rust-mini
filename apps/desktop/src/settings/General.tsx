import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import { formatShortcutState } from "./format";
import type { HotkeyStatus } from "./types";
import type { ProfileController } from "./useProfile";

/**
 * General (§9.1): the shortcut, the transcriber window, Windows startup, and the
 * keyboard paths that compensate for the transcriber never taking focus.
 *
 * Registration state is reported in plain language — "Shortcut active", not
 * "HOTKEY REGISTRATION / Registered" (§12). The contract vocabulary for it lives
 * on the Advanced page.
 */
export function General({
  profile,
}: {
  profile: ProfileController;
}) {
  const [hotkey, setHotkey] = useState<HotkeyStatus | null>(null);
  const [binding, setBinding] = useState("Ctrl+Alt+L");
  const [mode, setMode] = useState<HotkeyStatus["mode"]>("toggle");
  const [enabled, setEnabled] = useState(true);
  const [hotkeyAction, setHotkeyAction] = useState("");
  const bindingField = useRef<HTMLInputElement>(null);

  useEffect(() => {
    void invoke<HotkeyStatus>("hotkey_status").then((status) => {
      setHotkey(status);
      setBinding(status.binding);
      setMode(status.mode);
      setEnabled(status.enabled);
    });
  }, []);

  async function saveHotkey() {
    try {
      await invoke("hotkey_configure", { binding, mode, enabled });
      setHotkey(await invoke<HotkeyStatus>("hotkey_status"));
      setHotkeyAction(messages.hotkeySaved);
    } catch {
      setHotkeyAction(messages.hotkeySaveFailed);
    }
  }

  return (
    <>
      <section aria-labelledby="general-shortcut">
        <h3 id="general-shortcut">{messages.shortcutSection}</h3>
        <p className="setting-status" data-testid="shortcut-state">
          {formatShortcutState(hotkey?.registration ?? "pending")}
        </p>
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
          <button onClick={() => void saveHotkey()} type="button">
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
        §13: the transcriber is not keyboard operable by design, so every action it
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
