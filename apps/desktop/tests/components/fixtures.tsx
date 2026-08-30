import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

import type {
  CredentialStatus,
  DiagnosticsStatus,
  ProfileStatus,
} from "../../src/settings/types";

/**
 * Shared setup for every component test.
 *
 * `cleanup` unmounts between tests. Without it React Testing Library leaves the
 * previous test's tree in the document and `getByRole` finds two of everything
 * -- which fails as "found multiple elements", a message that reads like a
 * duplicated control in the component rather than like leaked state.
 */
afterEach(cleanup);

/** A saved profile with everything off, which is the shipped default. */
export function profileStatus(overrides: Partial<ProfileStatus> = {}): ProfileStatus {
  return {
    schema_version: 1,
    startup_with_windows: false,
    history_enabled: false,
    history_retention_days: 30,
    history_plaintext_disclosure_accepted: false,
    delivery_preference: "result_view_only",
    recording_feedback_enabled: false,
    disk_logging_enabled: false,
    preferred_capture_device_id: null,
    ...overrides,
  };
}

/**
 * A processor install with nothing measured, which is what every page reads on
 * mount and no test here asserts on.
 *
 * Present because an *unanswered* read resolves to `undefined`, the component
 * stores it, and the next render dereferences it -- so a missing stub surfaces
 * as a `TypeError` inside a render, which reads like a component defect rather
 * than like an incomplete test.
 */
export function diagnosticsStatus(
  overrides: Partial<DiagnosticsStatus> = {},
): DiagnosticsStatus {
  return {
    schema_version: 1,
    engine: "granite",
    worker: "granite-worker",
    runtime: "llama.cpp",
    provider: "cpu",
    rtf_median: null,
    rtf_p95: null,
    latency_p50_ms: null,
    latency_p95_ms: null,
    audio_overflow_count: 0,
    device: "cpu",
    vad: "manual_stop_only",
    delivery_capability: "auto_paste",
    delivery_reason: "ready",
    model_id: "granite-speech-4.1-2b-q4_k_m-cpu",
    model_revision: "1",
    model_source: "https://example.test/granite@" + "a".repeat(40),
    final_source_reason: null,
    recent_reason_codes: [],
    logs_sanitized: true,
    ...overrides,
  };
}

export function credentialStatus(): CredentialStatus {
  return { openai_legacy: "absent", remote_legacy: "absent", values_exposed: false };
}

/**
 * A recording `invoke` double: what was called, with what, and what it answers.
 *
 * Answers are keyed by command name so a test can let the reads on mount
 * succeed and make exactly the one write it is about reject. A command with no
 * entry resolves to `undefined` rather than throwing, because a component's
 * mount fires several reads that are not what the test is asserting and making
 * each of them a required stub would turn every test into a list of unrelated
 * setup.
 */
export type InvokeDouble = {
  invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  /** Every call, in order. */
  calls: { command: string; args?: Record<string, unknown> }[];
  /** How many times `command` was called. */
  count: (command: string) => number;
  answer: (command: string, value: unknown) => void;
  reject: (command: string, code: string) => void;
};

export function invokeDouble(initial: Record<string, unknown> = {}): InvokeDouble {
  const answers = new Map<string, { value: unknown } | { rejection: string }>();
  for (const [command, value] of Object.entries(initial)) answers.set(command, { value });
  const calls: { command: string; args?: Record<string, unknown> }[] = [];

  return {
    calls,
    count: (command) => calls.filter((call) => call.command === command).length,
    answer: (command, value) => answers.set(command, { value }),
    reject: (command, code) => answers.set(command, { rejection: code }),
    invoke: (command, args) => {
      calls.push({ command, args });
      const staged = answers.get(command);
      if (staged !== undefined && "rejection" in staged) {
        // A bare string, not an `Error`. Tauri rejects an `invoke` with
        // whatever the command's `Err` serialized to, and every command in this
        // app returns `Result<_, &'static str>` -- so the value the frontend
        // sees is the code itself. `useMutation` puts it through
        // `formatError(String(rejection))`, and `String(new Error("x"))` is
        // "Error: x", which maps to no catalog entry and would have every one
        // of these tests asserting `errorUnknown` regardless of the code.
        return Promise.reject(staged.rejection);
      }
      return Promise.resolve(staged?.value);
    },
  };
}
