import { expect, test, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { messages } from "../../src/catalog";
import { Advanced } from "../../src/settings/Advanced";
import { TranscriptLogPage } from "../../src/settings/TranscriptLogPage";
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

/** The transcript log page over a profile that is already keeping history. */
function Page() {
  return <TranscriptLogPage profile={useProfile()} />;
}

/** Advanced over the real controller, for the reset pair below. */
function AdvancedPage() {
  return <Advanced profile={useProfile()} />;
}

function retaining() {
  return install(
    invokeDouble({
      profile_status: profileStatus({
        history_enabled: true,
        history_plaintext_disclosure_accepted: true,
      }),
      session_transcript_log: [],
    }),
  );
}

const confirmDelete = () => screen.getByLabelText(messages.confirmDeleteHistory) as HTMLInputElement;
const deleteButton = () => screen.getByRole("button", { name: messages.deleteHistory });

/**
 * A refused deletion leaves the confirmation ticked and says what happened.
 *
 * This is the exact defect that motivated `useMutation`: the check box used to
 * clear itself and the word "Deleted" appeared whether or not the database had
 * been touched, so a `history_delete_failed` was indistinguishable from a
 * completed deletion -- over the one control in the product that destroys the
 * user's transcripts. Nothing could observe that before this harness existed:
 * both halves of it are what a *rendered* control does after a rejection.
 */
test("a refused history deletion keeps its confirmation and names the failure", async () => {
  const double = retaining();
  double.reject("history_delete_all", "history_delete_failed");
  render(<Page />);

  const box = await waitFor(confirmDelete);
  fireEvent.click(box);
  expect(box.checked).toBe(true);
  fireEvent.click(deleteButton());

  await waitFor(() => {
    expect(screen.getByText(messages.errors.history_delete_failed)).toBeDefined();
  });
  expect(confirmDelete().checked).toBe(true);
  expect(screen.queryByText(messages.deleted)).toBeNull();
});

/** And a deletion that happened clears the confirmation and says so. */
test("a completed history deletion clears its confirmation", async () => {
  const double = retaining();
  double.answer("history_delete_all", undefined);
  render(<Page />);

  const box = await waitFor(confirmDelete);
  fireEvent.click(box);
  fireEvent.click(deleteButton());

  await waitFor(() => {
    expect(screen.getByText(messages.deleted)).toBeDefined();
  });
  expect(confirmDelete().checked).toBe(false);
});

/**
 * A second press while the first deletion is in flight deletes nothing.
 *
 * Two mechanisms hold this and the test does not care which one fires: the
 * button is `disabled` while pending, and `useMutation` refuses a second `run`
 * behind a ref -- a ref rather than the rendered status, because two presses in
 * one frame both read the same stale state and both would pass it. Asserted on
 * the destructive command deliberately: an export running twice writes a file
 * twice, and a delete running twice is only harmless by luck.
 *
 * The button is captured before the first press. Its label changes to "Working"
 * while pending, so looking it up again by name finds nothing -- which fails as
 * "unable to find a button", reading like a missing control rather than like a
 * control that is doing its job.
 */
test("a second press while a deletion is in flight deletes nothing", async () => {
  const double = retaining();
  let release: () => void = () => {};
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  backend.invoke = (command, args) => {
    if (command === "history_delete_all") {
      double.calls.push({ command, args });
      return pending;
    }
    return double.invoke(command, args);
  };
  render(<Page />);

  fireEvent.click(await waitFor(confirmDelete));
  const button = deleteButton();
  fireEvent.click(button);
  fireEvent.click(button);
  expect(double.count("history_delete_all")).toBe(1);

  release();
  await waitFor(() => {
    expect(confirmDelete().checked).toBe(false);
  });
});

/**
 * A refused export names the failure rather than leaving the last message up.
 *
 * The success message is the file path the backend returned, so "nothing new
 * appeared" is indistinguishable from "the previous export is still on screen"
 * -- which is why the assertion is that the *error* is rendered rather than
 * that the path is not.
 */
test("a refused history export reports the reason", async () => {
  const double = retaining();
  double.reject("history_export", "history_export_failed");
  render(<Page />);

  const button = await waitFor(() =>
    screen.getByRole("button", { name: messages.exportHistory }),
  );
  fireEvent.click(button);

  await waitFor(() => {
    expect(screen.getByText(messages.errors.history_export_failed)).toBeDefined();
  });
  expect(double.count("history_export")).toBe(1);
});

/**
 * A refused reset leaves the warning panel open and says why.
 *
 * `reset_commit` is the most destructive command in the product -- settings,
 * transcript history, personalization and logs, in one press -- and it was
 * `profile.replace(await invoke(...))` with no rejection handler on either
 * half. A `reset_remove_failed` was an unhandled promise rejection, the panel
 * stayed open with nothing written in it, and the user was looking at a
 * confirm button that appeared to do nothing. The preview beside it had had a
 * mutation since the day the other four settings actions got one; the
 * destructive half did not.
 */
test("a refused reset keeps its confirmation panel and names the failure", async () => {
  const double = install(
    invokeDouble({
      profile_status: profileStatus(),
      reset_preview: {
        nonce: "n-1",
        categories: ["settings"],
        excludes_v1: false,
        excludes_custom_models: true,
        excludes_credentials: true,
      },
      diagnostics_status: diagnosticsStatus(),
      credential_status: credentialStatus(),
    }),
  );
  double.reject("reset_commit", "reset_remove_failed");
  render(<AdvancedPage />);

  fireEvent.click(await screen.findByRole("button", { name: messages.previewReset }));
  const confirm = await screen.findByRole("button", { name: messages.resetNow });
  fireEvent.click(confirm);

  await waitFor(() => {
    expect(screen.getByText(messages.errors.reset_remove_failed)).toBeDefined();
  });
  // Still open. Closing it on a refusal would be the deletion defect again: the
  // confirmation clears itself and the failure looks like a completed reset.
  expect(screen.getByRole("button", { name: messages.resetNow })).toBeDefined();
  expect(double.count("reset_commit")).toBe(1);
});

/** And a reset that happened closes the panel and adopts the fresh profile. */
test("a completed reset closes its confirmation panel", async () => {
  const double = install(
    invokeDouble({
      profile_status: profileStatus({ startup_with_windows: true }),
      reset_preview: {
        nonce: "n-1",
        categories: ["settings"],
        excludes_v1: false,
        excludes_custom_models: true,
        excludes_credentials: true,
      },
      diagnostics_status: diagnosticsStatus(),
      credential_status: credentialStatus(),
      reset_commit: profileStatus(),
    }),
  );
  render(<AdvancedPage />);

  fireEvent.click(await screen.findByRole("button", { name: messages.previewReset }));
  fireEvent.click(await screen.findByRole("button", { name: messages.resetNow }));

  await waitFor(() => {
    expect(screen.queryByRole("button", { name: messages.resetNow })).toBeNull();
  });
  expect(double.count("reset_commit")).toBe(1);
});
