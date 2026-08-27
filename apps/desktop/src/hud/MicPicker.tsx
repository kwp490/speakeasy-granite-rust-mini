import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";

export type CaptureDevice = {
  id: string;
  name: string;
  is_default: boolean;
  supported: boolean;
};

/**
 * Resolves the microphone a dictation would actually record from.
 *
 * Mirrors `hotkey_capture_device` exactly: the stored preference when it is still
 * present and supported, otherwise the OS default, otherwise the first supported
 * device. Doing it here rather than in the 10 Hz poll is deliberate — resolving
 * it in Rust means enumerating capture devices, and this component already holds
 * a device list on a far slower timer.
 */
function resolveDevice(devices: ReadonlyArray<CaptureDevice>, preferredId: string): string {
  if (devices.some((device) => device.id === preferredId && device.supported)) return preferredId;
  return (
    devices.find((device) => device.is_default && device.supported)?.id ??
    devices.find((device) => device.supported)?.id ??
    ""
  );
}

/**
 * Microphone selection (UI-GUIDE "Information architecture", the Audio group).
 *
 * Selecting writes `preferred_capture_device_id`, which is the same setting the
 * shortcut path reads — so choosing here changes what a shortcut-driven
 * dictation records from, with no second preference to keep in sync.
 *
 * What it *shows* is the resolved device, not just an explicit selection. A
 * fresh profile has no preference stored but still records from a real
 * microphone, and a picker reading "Select a microphone" while a microphone is
 * in fact selected states something untrue about the next dictation.
 *
 * Long device names truncate in CSS rather than widening the window. That rule
 * was written for the large 420x280 transcriber, which the fork deleted; no
 * window mounts this component today, and the side dock at 96x360 has even less
 * room to grow, so the rule holds for wherever it lands next.
 */
export function MicPicker({
  preferredId,
  disabled,
  onSelect,
}: {
  preferredId: string;
  disabled: boolean;
  onSelect: (deviceId: string) => void;
}) {
  const [devices, setDevices] = useState<CaptureDevice[]>([]);

  useEffect(() => {
    let stopped = false;
    const load = () => {
      invoke<CaptureDevice[]>("capture_devices")
        .then((found) => {
          if (!stopped) setDevices(found);
        })
        .catch(() => {
          // Enumeration can fail transiently while Windows settles a device
          // change. The setup line reports a missing microphone; the picker
          // just stays with what it last knew.
        });
    };
    load();
    // Devices change rarely, so this is deliberately far slower than the status
    // poll rather than riding along with it.
    const timer = window.setInterval(load, 5_000);
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, []);

  const shownId = resolveDevice(devices, preferredId);

  return (
    <select
      aria-label={messages.chooseMicrophone}
      className="hud-mic"
      data-testid="hud-mic-picker"
      disabled={disabled}
      onChange={(event) => {
        onSelect(event.target.value);
      }}
      title={devices.find((device) => device.id === shownId)?.name ?? messages.chooseMicrophone}
      value={shownId}
    >
      {/* Only offered while nothing resolves — with no microphone at all there is
          genuinely nothing selected, and the setup line says so. */}
      {shownId === "" && <option value="">{messages.selectMicrophone}</option>}
      {devices.map((device) => (
        <option disabled={!device.supported} key={device.id} value={device.id}>
          {device.name}
          {device.is_default ? messages.defaultDeviceSuffix : ""}
        </option>
      ))}
    </select>
  );
}
