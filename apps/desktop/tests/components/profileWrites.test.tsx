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

const automaticPaste = () => screen.getByRole("checkbox", { name: messages.autoPaste });

/**
 * Turning automatic paste off must reach the backend and be adopted from its
 * answer.
 *
 * The backend has always branched on `delivery.auto_paste`, but nothing in
 * these settings could reach it: a user who wanted to read an uncertain
 * transcript before it went into another application had no way to ask for
 * that. This is the control, and it writes the preference only -- the box
 * moving is the backend's answer, never an assumption.
 */
test("turning automatic paste off is written and adopted from the backend answer", async () => {
  const double = install(invokeDouble({ profile_status: profileStatus() }));
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

/** And a refused one is said, with the box left where the backend has it. */
test("a refused automatic-paste change is reported and the box does not move", async () => {
  const double = install(invokeDouble({ profile_status: profileStatus() }));
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

/**
 * A failed Retry is reported even when the status re-read after it fails too.
 *
 * The re-read retries for seconds before it gives up, and the failure message
 * used to be set only after it returned -- so when both failed, the handler
 * rejected and the message was never shown.
 */
test("a failed retry is reported when the status re-read also fails", async () => {
  const double = install(
    invokeDouble({
      profile_status: profileStatus(),
      result_status: {
        state: "failed",
        text: null,
        provenance: null,
        input_samples: null,
        final_segments: null,
        draft_revisions: null,
        error_code: null,
        retry_available: true,
      },
    }),
  );
  render(<Page />);

  const retry = await screen.findByRole("button", { name: messages.retryTranscription });
  await waitFor(() => {
    expect((retry as HTMLButtonElement).disabled).toBe(false);
  });
  double.reject("dictation_retry", "retry_unavailable");
  double.reject("result_status", "result_state_unavailable");
  fireEvent.click(retry);

  expect(await screen.findByText(messages.retryFailed)).toBeDefined();
});
