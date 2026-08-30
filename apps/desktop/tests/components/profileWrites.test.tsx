import { expect, test, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { messages } from "../../src/catalog";
import { OutputPrivacy } from "../../src/settings/OutputPrivacy";
import { useProfile, type ProfileController } from "../../src/settings/useProfile";
import { invokeDouble, profileStatus, type InvokeDouble } from "./fixtures";

// The one seam every component test needs. `vi.hoisted` runs before the
// `vi.mock` factory, which itself runs before the imports above, so the holder
// exists by the time any module captures `invoke` -- and each test installs its
// own double into it without re-importing anything.
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

/**
 * Output & Privacy with the **real** `useProfile` behind it, plus the banner
 * `SettingsApp` renders for a failed write.
 *
 * The controller is what is under test, not a stand-in for it: the defect these
 * tests exist for was in its five mutators, each of which was
 * `setProfile(await invoke(...))` with no rejection handler. A harness that
 * substituted a fake controller would be asserting its own stub.
 */
function Page() {
  const profile: ProfileController = useProfile();
  return (
    <>
      {profile.write.error !== null && <p role="alert">{profile.write.error}</p>}
      <OutputPrivacy profile={profile} />
    </>
  );
}

/** `toBeChecked` lives in `@testing-library/jest-dom`, which this harness
 * deliberately does not carry: one matcher is not worth a dependency, and
 * reading `.checked` says what is being asserted without a second vocabulary. */
const checked = (element: HTMLElement) => (element as HTMLInputElement).checked;

const explicitCopy = () => screen.getByLabelText(messages.explicitCopy);
const diagnosticLogging = () =>
  screen.getByRole("checkbox", { name: messages.diagnosticLogging });

/**
 * A refused write is *said*, and the control keeps the stored value.
 *
 * Before this, `setDelivery` was `setProfile(await invoke(...))`: the rejection
 * was unhandled and nothing rendered, so the radio snapped back to the stored
 * preference with no explanation. That is honest about the state and silent
 * about the event, which is the half of the truthful-disclosure rule that is
 * easy to miss -- the user sees a control that will not move.
 *
 * `delivery_configure` is the write worth pinning first: it decides whether a
 * transcript is pasted into the focused window or held for an explicit copy,
 * which is a privacy choice rather than a convenience.
 */
test("a refused delivery preference is reported and the stored choice stands", async () => {
  const double = install(invokeDouble({ profile_status: profileStatus() }));
  double.reject("delivery_configure", "profile_state_unavailable");
  render(<Page />);

  await waitFor(() => {
    expect(checked(explicitCopy())).toBe(false);
  });
  fireEvent.click(explicitCopy());

  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toBe(messages.errors.profile_state_unavailable);
  expect(checked(explicitCopy())).toBe(false);
  expect(double.count("delivery_configure")).toBe(1);
});

/** And an accepted one is adopted from the backend's answer, not assumed. */
test("an accepted delivery preference is adopted from the value the backend returned", async () => {
  const double = install(invokeDouble({ profile_status: profileStatus() }));
  double.answer("delivery_configure", profileStatus({ delivery_preference: "explicit_copy" }));
  render(<Page />);

  await waitFor(() => {
    expect(checked(explicitCopy())).toBe(false);
  });
  fireEvent.click(explicitCopy());

  await waitFor(() => {
    expect(checked(explicitCopy())).toBe(true);
  });
  expect(screen.queryByRole("alert")).toBeNull();
});

/**
 * The disk-logging toggle is the second privacy write, and it fails the same
 * way through the same mutation -- which is the point of there being one.
 */
test("a refused disk-logging change is reported and the box does not move", async () => {
  const double = install(invokeDouble({ profile_status: profileStatus() }));
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
 * five profile writers share one instance for exactly this reason: they write
 * one `ProfileView`, so two of them racing means the later answer overwrites
 * the earlier one and one of the user's two clicks is silently lost.
 */
test("a second profile write is refused while the first is still in flight", async () => {
  const double = install(invokeDouble({ profile_status: profileStatus() }));
  let release: (value: unknown) => void = () => {};
  const pending = new Promise((resolve) => {
    release = resolve;
  });
  const answers = profileStatus({ delivery_preference: "explicit_copy" });
  backend.invoke = (command, args) => {
    if (command === "delivery_configure") {
      double.calls.push({ command, args });
      return pending.then(() => answers);
    }
    return double.invoke(command, args);
  };
  render(<Page />);

  await waitFor(() => {
    expect(checked(explicitCopy())).toBe(false);
  });
  fireEvent.click(explicitCopy());
  fireEvent.click(explicitCopy());
  expect(double.count("delivery_configure")).toBe(1);

  release(undefined);
  await waitFor(() => {
    expect(checked(explicitCopy())).toBe(true);
  });
});
