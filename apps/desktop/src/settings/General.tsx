import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import { bindingFromKey, formatShortcutState } from "./format";
import { readWithRetry } from "./readWithRetry";
import { SettingGroup, SettingRow, StatusText, Switch } from "./Rows";
import type { HotkeyStatus } from "./types";
import type { ProfileController } from "./useProfile";

/**
 * General: the shortcut, automatic paste, recording sounds and Windows startup.
 *
 * Registration state is reported in plain language — "Shortcut active", not
 * "HOTKEY REGISTRATION / Registered" (UI-GUIDE "Two vocabulary registers").
 *
 * Every control applies when it changes. The shortcut's three fields are one
 * transactional `hotkey_configure`: a refusal means the previous binding is
 * registered and live again, so the page reads that back rather than keeping
 * the refused value on screen.
 */
export function General({ profile }: { profile: ProfileController }) {
  const [hotkey, setHotkey] = useState<HotkeyStatus | null>(null);
  const [hotkeyUnavailable, setHotkeyUnavailable] = useState(false);
  const [hotkeyAction, setHotkeyAction] = useState("");
  const [recording, setRecording] = useState(false);
  const [saving, setSaving] = useState(false);
  const recorder = useRef<HTMLButtonElement>(null);

  // Retried until registration is no longer `pending`, which is two fixes for
  // one symptom. The read can lose the race against `setup` managing
  // `HotkeyCoordinator`, and it can also *succeed* and answer `pending`: the
  // coordinator starts there and `register_activation_hotkey` runs at the end of
  // `setup`, after every window's React tree has already mounted and read. The
  // page would then render "Shortcut not registered yet" for the life of the
  // window with the shortcut registered and working.
  useEffect(() => {
    void readWithRetry<HotkeyStatus>(
      "hotkey_status",
      (status) => status.registration !== "pending",
    ).then(
      (status) => {
        setHotkey(status);
        setHotkeyUnavailable(false);
      },
      () => {
        setHotkeyUnavailable(true);
      },
    );
  }, []);

  useEffect(() => {
    if (recording) recorder.current?.focus();
  }, [recording]);

  /**
   * Writes the whole shortcut and adopts what the backend then reports.
   *
   * Only ever called with fields read from `hotkey`, which is non-null here: a
   * write from an unanswered read would put an empty binding and a default mode
   * over settings this page has never seen.
   */
  async function applyHotkey(next: Pick<HotkeyStatus, "binding" | "mode" | "enabled">) {
    if (saving) return;
    setSaving(true);
    setHotkeyAction("");
    try {
      await invoke("hotkey_configure", next);
      setHotkeyAction(messages.hotkeySaved);
    } catch {
      setHotkeyAction(messages.hotkeySaveFailed);
    }
    await readWithRetry<HotkeyStatus>("hotkey_status").then(
      (status) => {
        setHotkey(status);
        setHotkeyUnavailable(false);
      },
      () => {
        setHotkeyUnavailable(true);
      },
    );
    setSaving(false);
  }

  function onRecordKey(event: KeyboardEvent<HTMLButtonElement>) {
    if (hotkey === null) return;
    event.preventDefault();
    if (event.key === "Escape") {
      setRecording(false);
      setHotkeyAction("");
      return;
    }
    if (event.key === "Tab") {
      setRecording(false);
      return;
    }
    const binding = bindingFromKey(event);
    if (binding === null) {
      if (!["Control", "Alt", "Shift", "Meta", "OS", "AltGraph"].includes(event.key)) {
        setHotkeyAction(messages.shortcutNeedsModifier);
      }
      return;
    }
    setRecording(false);
    void applyHotkey({ binding, mode: hotkey.mode, enabled: hotkey.enabled });
  }

  const registration = hotkey?.registration ?? "unknown";
  // `unknown`, never `pending`, when the read has not answered. Both are real
  // backend values: `pending` is "registration has not been attempted yet", a
  // claim about the app whose copy reads "Shortcut not registered yet".
  // `undefined` means the page does not know, and "Shortcut state unknown" is
  // what that is.
  const shortcutState = formatShortcutState(hotkey?.registration ?? "unknown");
  const tone =
    registration === "registered" ? "ok" : registration === "conflict" ? "bad" : "neutral";

  return (
    <>
      <SettingGroup label={messages.dictationGroup}>
        <SettingRow
          control={
            recording ? (
              <>
                <button
                  className="shortcut-recorder"
                  onBlur={() => setRecording(false)}
                  onKeyDown={onRecordKey}
                  ref={recorder}
                  type="button"
                >
                  {messages.shortcutRecording}
                </button>
                <button onClick={() => setRecording(false)} type="button">
                  {messages.cancel}
                </button>
              </>
            ) : (
              <>
                {hotkey !== null && <kbd className="keycap">{hotkey.binding}</kbd>}
                <button
                  disabled={hotkey === null}
                  onClick={() => {
                    setHotkeyAction("");
                    setRecording(true);
                  }}
                  type="button"
                >
                  {saving ? messages.working : messages.changeShortcut}
                </button>
              </>
            )
          }
          detail={
            recording ? (
              messages.shortcutRecordingDetail
            ) : (
              <StatusText testId="shortcut-state" tone={tone}>
                {shortcutState}
              </StatusText>
            )
          }
          title={messages.shortcutSection}
        >
          {hotkeyUnavailable && <p className="row-note warning">{messages.shortcutStateUnavailable}</p>}
          {hotkeyAction !== "" && (
            <output aria-live="polite" className="row-note">
              {hotkeyAction}
            </output>
          )}
        </SettingRow>
        <SettingRow
          control={
            <select
              disabled={hotkey === null || saving}
              id="general-hotkey-mode"
              onChange={(event) => {
                if (hotkey === null) return;
                void applyHotkey({
                  binding: hotkey.binding,
                  mode: event.target.value as HotkeyStatus["mode"],
                  enabled: hotkey.enabled,
                });
              }}
              value={hotkey?.mode ?? "toggle"}
            >
              <option value="toggle">{messages.hotkeyModeToggle}</option>
              <option value="push_to_talk">{messages.hotkeyModePushToTalk}</option>
              <option value="hands_free">{messages.hotkeyModeHandsFree}</option>
            </select>
          }
          detail={messages.hotkeyModeDetail}
          labelFor="general-hotkey-mode"
          title={messages.hotkeyMode}
        />
        <SettingRow
          control={
            <Switch
              checked={profile.profile?.auto_paste_enabled ?? true}
              describedBy="general-auto-paste-detail"
              label={messages.autoPaste}
              onChange={(next) => void profile.setAutoPaste(next)}
            />
          }
          detail={messages.protectedTargetsDetail}
          detailId="general-auto-paste-detail"
          title={messages.autoPaste}
        />
        <SettingRow
          control={
            <Switch
              checked={profile.profile?.recording_feedback_enabled ?? true}
              describedBy="general-feedback-detail"
              label={messages.recordingFeedback}
              onChange={(next) => void profile.setRecordingFeedback(next)}
            />
          }
          detail={messages.recordingFeedbackDetail}
          detailId="general-feedback-detail"
          title={messages.recordingFeedback}
        />
      </SettingGroup>

      <SettingGroup label={messages.startupGroup}>
        <SettingRow
          control={
            <Switch
              checked={profile.profile?.startup_with_windows ?? false}
              label={messages.startupWithWindows}
              onChange={(next) => void profile.setStartup(next)}
            />
          }
          title={messages.startupWithWindows}
        />
        <SettingRow
          control={
            <Switch
              checked={hotkey?.enabled ?? true}
              describedBy="general-hotkey-enabled-detail"
              disabled={hotkey === null || saving}
              label={messages.hotkeyEnabledLabel}
              onChange={(next) => {
                if (hotkey === null) return;
                void applyHotkey({ binding: hotkey.binding, mode: hotkey.mode, enabled: next });
              }}
            />
          }
          detail={messages.hotkeyEnabledDetail}
          detailId="general-hotkey-enabled-detail"
          title={messages.hotkeyEnabledLabel}
        />
      </SettingGroup>
    </>
  );
}
