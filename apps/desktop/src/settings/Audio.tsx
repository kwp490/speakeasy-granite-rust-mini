import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import { formatError, formatState } from "./format";
import type { CaptureAudioSnapshot, CaptureDevice } from "./types";

/**
 * The gap between the end of one sample and the start of the next. 10 Hz,
 * matching the transcriber's poll.
 *
 * A gap rather than a period, because the poll is self-scheduling: an interval
 * keeps firing while a request is outstanding, so a round trip slower than the
 * period accumulates concurrent IPC and the answers can arrive out of order.
 */
const SAMPLE_GAP_MS = 100;

/**
 * Audio: device selection, input level, microphone status, refresh.
 *
 * There are **no capture controls** here. Settings never starts, stops or
 * cancels a dictation — that is the transcriber's job and the shortcut's, and
 * a second start path is what the single-controller rule exists to prevent.
 *
 * The level meter is honest about its own limits: `level` is written by the
 * capture loop, so it moves only while a dictation is running. Opening a
 * microphone here to animate a bar would be capture by another name.
 */
export function Audio({ preferredId }: { preferredId: string }) {
  const [devices, setDevices] = useState<CaptureDevice[]>([]);
  const [selected, setSelected] = useState("");
  const [action, setAction] = useState("");
  /**
   * The meter and the device-health facts, from one command.
   *
   * They were two `invoke`s on the same 10 Hz timer -- twenty round trips a
   * second on a page built to be watched while nothing happens -- and two calls
   * can straddle a state change, so the meter could describe a dictation the
   * health panel had not seen start. `null` until the first answer arrives; the
   * panel renders "starting" rather than inventing a state.
   */
  const [audio, setAudio] = useState<CaptureAudioSnapshot | null>(null);

  const [enumeration, setEnumeration] = useState<"pending" | "ready" | "unavailable">("pending");

  const loadDevices = useCallback(() => {
    void invoke<CaptureDevice[]>("capture_devices")
      .then((found) => {
        setDevices(found);
        setEnumeration("ready");
        setSelected((current) => {
          if (found.some((device) => device.id === current && device.supported)) return current;
          // Resolve exactly as `hotkey_capture_device` does: the stored
          // preference first, then the OS default. Showing the OS default while a
          // different microphone is stored names a device the next dictation will
          // not use — which is what this page was doing.
          if (found.some((device) => device.id === preferredId && device.supported)) {
            return preferredId;
          }
          return (
            found.find((device) => device.is_default && device.supported)?.id ??
            found.find((device) => device.supported)?.id ??
            ""
          );
        });
      })
      .catch(() => {
        setEnumeration("unavailable");
        setAction(formatError("capture_device_enumeration_failed"));
      });
  }, [preferredId]);

  // Also re-runs when the stored preference arrives from the profile, which is
  // after the first render.
  useEffect(loadDevices, [loadDevices]);

  /**
   * Retries enumeration while it is unavailable.
   *
   * Carried over from the previous settings build, where it earned its keep:
   * Windows can refuse to list capture devices for a second or two after a
   * device change or at cold start, and without this the page settles on "no
   * microphone" and stays there until the user thinks to press Refresh. It gives
   * up after 20 attempts rather than polling a genuinely absent device set
   * forever.
   */
  useEffect(() => {
    if (enumeration !== "unavailable") return;
    let attempts = 0;
    const timer = window.setInterval(() => {
      attempts += 1;
      if (attempts >= 20) {
        window.clearInterval(timer);
        return;
      }
      loadDevices();
    }, 500);
    return () => {
      window.clearInterval(timer);
    };
  }, [enumeration, loadDevices]);

  /**
   * One request at a time, and the next is scheduled only once the previous one
   * has settled.
   *
   * A rejected sample schedules the next one exactly as a successful one does:
   * the meter is display-only, and a page that stopped sampling after a single
   * transient refusal would sit on a frozen bar with no way back.
   */
  useEffect(() => {
    let stopped = false;
    let timer = 0;
    const schedule = () => {
      if (stopped) return;
      timer = window.setTimeout(sample, SAMPLE_GAP_MS);
    };
    const sample = () => {
      invoke<CaptureAudioSnapshot>("capture_audio_snapshot")
        .then((snapshot) => {
          if (stopped) return;
          setAudio(snapshot);
        })
        .catch(() => {
          // A missed sample is a missed sample. The last one stands.
        })
        .finally(schedule);
    };
    sample();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, []);

  async function chooseDevice(deviceId: string) {
    setSelected(deviceId);
    setAction("");
    if (deviceId === "") return;
    try {
      await invoke("capture_device_configure", { deviceId });
      setAction(messages.deviceSaved);
    } catch {
      setAction(messages.deviceSaveFailed);
      loadDevices();
    }
  }

  return (
    <>
      <section aria-labelledby="audio-device">
        <h3 id="audio-device">{messages.audioDeviceSection}</h3>
        <p className="setting-detail">{messages.audioDeviceDetail}</p>
        {devices.length === 0 ? (
          <p className="warning" role="alert">
            {messages.noDevices}
          </p>
        ) : (
          <label className="setting-field">
            <span>{messages.microphone}</span>
            <select
              onChange={(event) => void chooseDevice(event.target.value)}
              value={selected}
            >
              <option value="">{messages.selectMicrophone}</option>
              {devices.map((device) => (
                <option disabled={!device.supported} key={device.id} value={device.id}>
                  {device.name}
                  {device.is_default ? messages.defaultDeviceSuffix : ""}
                  {device.supported ? "" : messages.unsupportedDeviceSuffix}
                </option>
              ))}
            </select>
          </label>
        )}
        <div className="actions">
          <button onClick={loadDevices} type="button">
            {messages.refreshDevices}
          </button>
          <output aria-live="polite">{action}</output>
        </div>
      </section>

      <section aria-labelledby="audio-recording-behavior">
        <h3 id="audio-recording-behavior">{messages.recordingBehaviorSection}</h3>
        <p className="setting-detail">{messages.recordingBehaviorDetail}</p>
      </section>

      <section aria-labelledby="audio-level">
        <h3 id="audio-level">{messages.inputLevelSection}</h3>
        <label className="setting-field">
          <span>{messages.inputLevel}</span>
          <meter
            className="input-level"
            data-testid="settings-input-level"
            high={0.85}
            low={0.05}
            max={1}
            optimum={0.6}
            value={audio?.level ?? 0}
          />
        </label>
        {audio?.active !== true && (
          <p className="setting-detail">{messages.inputLevelWhileDictating}</p>
        )}
      </section>

      <section aria-labelledby="audio-health">
        <h3 id="audio-health">{messages.deviceHealthSection}</h3>
        <dl className="fact-grid">
          <div>
            <dt>{messages.captureStateLabel}</dt>
            <dd>{formatState(audio?.state ?? "starting")}</dd>
          </div>
          <div>
            <dt>{messages.deviceStatus}</dt>
            <dd>
              <bdi>{audio?.device_name ?? messages.unknown}</bdi>
            </dd>
          </div>
        </dl>
        {audio?.error_code != null && (
          <p role="alert">
            {messages.captureFailed} {formatError(audio.error_code)}
          </p>
        )}
      </section>
    </>
  );
}
