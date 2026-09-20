import { expect, test, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { messages } from "../../src/catalog";
import { Advanced } from "../../src/settings/Advanced";
import { useProfile } from "../../src/settings/useProfile";
import {
  credentialStatus,
  diagnosticsStatus,
  invokeDouble,
  profileStatus,
  type InvokeDouble,
} from "./fixtures";

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

function AdvancedPage() {
  return <Advanced profile={useProfile()} />;
}

/**
 * The reads Advanced fires on mount, plus a settled engine for the restart to
 * observe. `engine` is what `capture_hud_status` reports and is the only field
 * the restart waits on.
 */
function ready(engine: string) {
  return install(
    invokeDouble({
      profile_status: profileStatus(),
      diagnostics_status: diagnosticsStatus(),
      credential_status: credentialStatus(),
      runtime_recover: undefined,
      capture_hud_status: { engine },
    }),
  );
}

/**
 * The restart button, by either of its names.
 *
 * Its label becomes "Restarting..." while a restart is in flight, so a lookup
 * pinned to the idle name stops finding it at exactly the moment these tests
 * care about.
 */
const restart = () =>
  screen.getByRole("button", {
    name: (name: string) => name === messages.restartEngine || name === messages.engineRestarting,
  }) as HTMLButtonElement;

/** The `output` beside it, found through the button rather than by its text. */
const output = () => restart().closest(".actions")?.querySelector("output");

/**
 * Success is the engine reporting `ready`, never the command returning.
 *
 * `runtime_recover` starts a warm that hashes the pack and loads roughly 2 GB;
 * it cannot hold an IPC call for that, so it returns immediately. The button
 * used to announce a restart from that return alone — while the resident worker
 * and its quarantine were untouched, so every later dictation still failed.
 */
test("a restart claims success only after the engine reports ready", async () => {
  const double = ready("ready");
  render(<AdvancedPage />);

  fireEvent.click(restart());

  await waitFor(() => {
    expect(output()?.textContent).toBe(messages.engineRestarted);
  });
  expect(double.count("runtime_recover")).toBe(1);
  // The readiness was observed, not assumed.
  expect(double.count("capture_hud_status")).toBeGreaterThanOrEqual(1);
});

/** An engine that settles on a failure code is reported as that failure. */
test("an engine that comes back quarantined is reported, not called restarted", async () => {
  ready("granite_quarantined");
  render(<AdvancedPage />);

  fireEvent.click(restart());

  await waitFor(() => {
    expect(output()?.textContent).toBe(messages.errors.granite_quarantined);
  });
});

/** A refused command never reaches the wait, and says why. */
test("a refused restart reports the backend code", async () => {
  const double = ready("ready");
  double.reject("runtime_recover", "dictation_active_operation_deferred");
  render(<AdvancedPage />);

  fireEvent.click(restart());

  await waitFor(() => {
    expect(output()?.textContent).toBe(
      messages.errors.dictation_active_operation_deferred,
    );
  });
  // It refused before waiting on anything.
  expect(double.count("capture_hud_status")).toBe(0);
});

/** One restart at a time: `useMutation` refuses the second click. */
test("the restart button is disabled while a restart is in flight", async () => {
  const double = ready("ready");
  const base = double.invoke;
  let release: (value: unknown) => void = () => {};
  const pending = new Promise((resolve) => {
    release = resolve;
  });
  // Counted here rather than through the double, because this test replaces
  // `invoke` to hold `runtime_recover` open and the double never sees it.
  let recoveries = 0;
  backend.invoke = (command, args) => {
    if (command !== "runtime_recover") return base(command, args);
    recoveries += 1;
    return pending.then(() => undefined);
  };

  render(<AdvancedPage />);
  fireEvent.click(restart());

  await waitFor(() => {
    expect(restart().disabled).toBe(true);
  });
  // The second press lands while the first is still outstanding.
  fireEvent.click(restart());
  expect(recoveries).toBe(1);

  release(undefined);
  await waitFor(() => {
    expect(output()?.textContent).toBe(messages.engineRestarted);
  });
  expect(recoveries).toBe(1);
});
