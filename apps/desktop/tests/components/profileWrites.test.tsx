import { expect, test, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { messages } from "../../src/catalog";
import { Advanced } from "../../src/settings/Advanced";
import { General } from "../../src/settings/General";
import { useProfile, type ProfileController } from "../../src/settings/useProfile";
import { diagnosticsStatus, invokeDouble, profileStatus, type InvokeDouble } from "./fixtures";

const backend = vi.hoisted(() => ({
  invoke: (_command: string, _args?: Record<string, unknown>): Promise<unknown> =>
    Promise.resolve(undefined),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: Record<string, unknown>) => backend.invoke(command, args),
}));

function install(double: InvokeDouble) {
  backend.invoke = double.invoke;
  return double;
}

const hotkey = {
  binding: "Ctrl+Alt+P",
  mode: "toggle",
  enabled: true,
  registration: "registered",
};

/** Every read General and Advanced fire on mount, answered. */
function reads(overrides: Record<string, unknown> = {}) {
  return invokeDouble({
    profile_status: profileStatus(),
    hotkey_status: hotkey,
    diagnostics_status: diagnosticsStatus(),
    ...overrides,
  });
}

/**
 * General and Advanced with the **real** `useProfile` behind them, plus the
 * banner `SettingsApp` renders for a failed write.
 *
 * The controller is what is under test, not a stand-in for it: the defect these
 * tests exist for was in its mutators, each of which was
 * `setProfile(await invoke(...))` with no rejection handler. A harness that
 * substituted a fake controller would be asserting its own stub.
 */
function Page() {
  const profile: ProfileController = useProfile();
  return (
    <>
      {profile.write.error !== null && <p role="alert">{profile.write.error}</p>}
      <General profile={profile} />
      <Advanced profile={profile} />
    </>
  );
}

const checked = (element: HTMLElement) => (element as HTMLInputElement).checked;
const automaticPaste = () => screen.getByRole("switch", { name: messages.autoPaste });
const diagnosticLogging = () => screen.getByRole("switch", { name: messages.diagnosticLogging });

/**
 * Turning automatic paste off must reach the backend and be adopted from its
 * answer. The switch moving is the backend's answer, never an assumption.
 */
test("turning automatic paste off is written and adopted from the backend answer", async () => {
  const double = install(reads());
  double.answer("auto_paste_configure", profileStatus({ auto_paste_enabled: false }));
  render(<Page />);

  await waitFor(() => {
    expect(checked(automaticPaste())).toBe(true);
  });
  fireEvent.click(automaticPaste());

  await waitFor(() => {
    expect(checked(automaticPaste())).toBe(false);
  });
  expect(double.count("auto_paste_configure")).toBe(1);
  expect(screen.queryByRole("alert")).toBeNull();
});

/**
 * A refused write is *said*, and the switch keeps the stored value.
 *
 * Honest about the state and silent about the event is the half of the
 * truthful-disclosure rule that is easy to miss: the user sees a control that
 * will not move.
 */
test("a refused automatic-paste change is reported and the switch does not move", async () => {
  const double = install(reads());
  double.reject("auto_paste_configure", "profile_state_unavailable");
  render(<Page />);

  await waitFor(() => {
    expect(checked(automaticPaste())).toBe(true);
  });
  fireEvent.click(automaticPaste());

  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toBe(messages.errors.profile_state_unavailable);
  expect(checked(automaticPaste())).toBe(true);
});

/**
 * The disk-logging switch is the second privacy write, and it fails the same
 * way through the same mutation -- which is the point of there being one.
 */
test("a refused disk-logging change is reported and the switch does not move", async () => {
  const double = install(reads());
  double.reject("disk_logging_configure", "profile_state_unavailable");
  render(<Page />);

  await waitFor(() => {
    expect(checked(diagnosticLogging())).toBe(false);
  });
  fireEvent.click(diagnosticLogging());

  await screen.findByRole("alert");
  expect(checked(diagnosticLogging())).toBe(false);
});

/**
 * One write at a time.
 *
 * `useMutation` refuses a second submission while one is in flight, and the
 * profile writers share one instance for exactly this reason: they write one
 * `ProfileView`, so two of them racing means the later answer overwrites the
 * earlier one and one of the user's two clicks is silently lost.
 */
test("a second profile write is refused while the first is still in flight", async () => {
  const double = install(reads());
  let release: (value: unknown) => void = () => {};
  const pending = new Promise((resolve) => {
    release = resolve;
  });
  backend.invoke = (command, args) => {
    if (command === "auto_paste_configure" || command === "disk_logging_configure") {
      double.calls.push({ command, args });
      return pending.then(() => profileStatus({ auto_paste_enabled: false }));
    }
    return double.invoke(command, args);
  };
  render(<Page />);

  await waitFor(() => {
    expect(checked(automaticPaste())).toBe(true);
  });
  fireEvent.click(automaticPaste());
  fireEvent.click(diagnosticLogging());
  expect(double.count("auto_paste_configure") + double.count("disk_logging_configure")).toBe(1);

  release(undefined);
  await waitFor(() => {
    expect(checked(automaticPaste())).toBe(false);
  });
});

/**
 * Change records the next key press and writes it, with the stored mode and
 * enabled state rather than defaults.
 */
test("a recorded shortcut is written with the stored mode", async () => {
  const double = install(reads({ hotkey_status: { ...hotkey, mode: "push_to_talk" } }));
  render(<Page />);

  const change = await screen.findByRole("button", { name: messages.changeShortcut });
  await waitFor(() => {
    expect((change as HTMLButtonElement).disabled).toBe(false);
  });
  fireEvent.click(change);
  const recorder = await screen.findByRole("button", { name: messages.shortcutRecording });
  fireEvent.keyDown(recorder, { key: "k", code: "KeyK", ctrlKey: true, altKey: true });

  await waitFor(() => {
    expect(double.count("hotkey_configure")).toBe(1);
  });
  const call = double.calls.find((entry) => entry.command === "hotkey_configure");
  expect(call?.args).toEqual({ binding: "Ctrl+Alt+K", mode: "push_to_talk", enabled: true });
});

/** A key with no Ctrl, Alt or Windows is not a shortcut, and nothing is written. */
test("a key press without a modifier is refused and nothing is written", async () => {
  const double = install(reads());
  render(<Page />);

  const change = await screen.findByRole("button", { name: messages.changeShortcut });
  await waitFor(() => {
    expect((change as HTMLButtonElement).disabled).toBe(false);
  });
  fireEvent.click(change);
  const recorder = await screen.findByRole("button", { name: messages.shortcutRecording });
  fireEvent.keyDown(recorder, { key: "k", code: "KeyK" });

  expect(await screen.findByText(messages.shortcutNeedsModifier)).toBeDefined();
  expect(double.count("hotkey_configure")).toBe(0);
});
