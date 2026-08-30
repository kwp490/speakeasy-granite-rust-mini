import { expect, test, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { messages } from "../../src/catalog";
import { Transcription } from "../../src/settings/Transcription";
import type {
  GpuStatus,
  ModelCatalogItem,
  ModelInstallStatus,
  PersonalizationStatus,
} from "../../src/settings/types";
import { diagnosticsStatus, invokeDouble, type InvokeDouble } from "./fixtures";

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

function catalogRow(): ModelCatalogItem {
  return {
    id: "granite-speech-4.1-2b-q4_k_m-cpu",
    revision: "1",
    display_name: "Granite Speech",
    archive_bytes: 0,
    installed_bytes: 2_298_601_952,
    confirmation_required: false,
    source_repository: "https://example.test/granite",
    source_revision: "a".repeat(40),
    license_name: "Apache 2.0",
    license_spdx: "Apache-2.0",
    license_url: "https://example.test/license",
    runtime: "granite",
    provider: "cpu",
    capabilities: [],
    hardware_evidence: "",
    downloadable: true,
    installed: false,
  };
}

function personalization(): PersonalizationStatus {
  return {
    schema_version: 1,
    transform_pipeline_version: 1,
    locale_status: "qualified",
    hotword_path: "",
    contacts_import_enabled: false,
    dictionary: [],
    snippets: [],
  };
}

function gpuStatus(): GpuStatus {
  return {
    status: "admissible",
    qualified: false,
    admissible: false,
    adapter_name: null,
    compute_capability: null,
    total_vram_bytes: null,
    free_vram_bytes: null,
    driver_version: null,
    minimum_compute_capability: "7.0",
    active_provider: "cpu",
    engine_reason: "probe_preferred",
    active_device: "cpu",
    provider_integrity: "ok",
    provider_fault: false,
  };
}

function page(status: ModelInstallStatus) {
  return install(
    invokeDouble({
      model_catalog: [catalogRow()],
      model_install_status: status,
      // Every read this page fires on mount is answered, including the ones no
      // test here asserts on. An unanswered read resolves to `undefined`, which
      // the component stores and then dereferences -- so a missing stub surfaces
      // as a `TypeError` deep in a render, reading like a component defect.
      gpu_status: gpuStatus(),
      model_hardware: {
        operating_system: "Windows",
        operating_system_build: null,
        logical_processors: 8,
        total_memory_bytes: 34_359_738_368,
      },
      personalization_status: personalization(),
      diagnostics_status: diagnosticsStatus(),
    }),
  );
}

/**
 * A refused cancel is reported, rather than looking exactly like a cancel.
 *
 * `onClick={() => void invoke("model_install_cancel")}` -- no rejection handler
 * of any kind. An `install_not_active` refusal was an unhandled promise
 * rejection and the button appeared to have worked, over a 2.30 GB download the
 * user was trying to stop.
 */
test("a refused install cancel says so", async () => {
  const double = page({ state: "downloading", error: null, bytes_downloaded: 1, bytes_total: 2 });
  double.reject("model_install_cancel", "install_not_active");
  render(<Transcription />);

  const cancel = await screen.findByRole("button", { name: messages.cancel });
  fireEvent.click(cancel);

  await waitFor(() => {
    expect(screen.getByText(messages.errors.install_not_active)).toBeDefined();
  });
  // And it does not become a claim about the *install*, which is a different
  // fact and has its own line.
  expect(screen.queryByText(new RegExp(messages.installationFailed, "u"))).toBeNull();
});

/**
 * A poll that stops answering says the progress is stale, and does not condemn
 * the pack.
 *
 * The 750 ms poll had no rejection handler: the refusal was unhandled and the
 * page went on rendering the last progress it received, so an install whose
 * status became unreadable sat on "Downloading" with disabled buttons for the
 * life of the window. Setting `state: "failed"` instead would be the other
 * error -- a poll that could not be read is not evidence about the bytes on
 * disk, and condemning them on it is the manufactured claim this repository
 * keeps finding.
 */
test("an unreadable install poll reports stale progress, not a failed install", async () => {
  vi.useFakeTimers();
  try {
    const double = page({ state: "downloading", error: null, bytes_downloaded: 1, bytes_total: 2 });
    render(<Transcription />);
    // Let the mount reads settle before the poll is allowed to reject, so the
    // page is genuinely mid-download rather than never started.
    await vi.advanceTimersByTimeAsync(0);
    double.reject("model_install_status", "model_state_unavailable");
    await vi.advanceTimersByTimeAsync(800);

    expect(screen.getByText(messages.modelStatusPollUnavailable)).toBeDefined();
    expect(screen.queryByText(new RegExp(messages.installationFailed, "u"))).toBeNull();
  } finally {
    vi.useRealTimers();
  }
});

/**
 * "Deleted" is printed only when something was.
 *
 * `resetPersonalization` had no rejection handler and set the message
 * unconditionally, so a refused reset cleared the import preview and announced
 * a deletion that had not happened -- the same defect as the history delete,
 * one fieldset above it on the same page.
 */
test("a refused personalization reset does not claim a deletion", async () => {
  const double = page({ state: "verified_on_disk", error: null });
  double.reject("personalization_reset", "personalization_reset_failed");
  render(<Transcription />);

  const reset = await screen.findByRole("button", { name: messages.resetPersonalization });
  fireEvent.click(reset);

  await waitFor(() => {
    expect(screen.getByText(messages.errors.personalization_reset_failed)).toBeDefined();
  });
  expect(screen.queryByText(messages.deleted)).toBeNull();
});

/** And a refused export reports its reason where the file name would have gone. */
test("a refused personalization export reports the reason", async () => {
  const double = page({ state: "verified_on_disk", error: null });
  double.reject("personalization_export", "personalization_export_failed");
  render(<Transcription />);

  const exportButton = await screen.findByRole("button", {
    name: messages.exportPersonalization,
  });
  fireEvent.click(exportButton);

  await waitFor(() => {
    expect(screen.getByText(messages.errors.personalization_export_failed)).toBeDefined();
  });
});

/** An export that succeeded shows the file it wrote, which is the whole answer. */
test("a completed personalization export names the file", async () => {
  const double = page({ state: "verified_on_disk", error: null });
  double.answer("personalization_export", "personalization-2026-08-30.json");
  render(<Transcription />);

  fireEvent.click(
    await screen.findByRole("button", { name: messages.exportPersonalization }),
  );

  await waitFor(() => {
    expect(screen.getByText("personalization-2026-08-30.json")).toBeDefined();
  });
});
