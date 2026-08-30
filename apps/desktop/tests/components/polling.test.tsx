import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";

import { Audio } from "../../src/settings/Audio";
import { TranscriptLog } from "../../src/settings/TranscriptLog";
import { deferred, invokeDouble, type InvokeDouble } from "./fixtures";

const backend = vi.hoisted(() => ({
  invoke: (_command: string, _args?: Record<string, unknown>): Promise<unknown> =>
    Promise.resolve(undefined),
}));

/**
 * The event bus, recorded rather than stubbed away.
 *
 * `listen` resolves to an unlisten function, and whether that function is
 * actually called on unmount is half of what these tests are about — a listener
 * left attached to an unmounted tree is the leak an interval could not have.
 */
const events = vi.hoisted(() => ({
  handlers: new Map<string, Set<() => void>>(),
  unlistened: 0,
  emit(name: string) {
    for (const handler of events.handlers.get(name) ?? []) handler();
  },
  count(name: string) {
    return events.handlers.get(name)?.size ?? 0;
  },
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: Record<string, unknown>) => backend.invoke(command, args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: () => void) => {
    const set = events.handlers.get(name) ?? new Set();
    set.add(handler);
    events.handlers.set(name, set);
    return Promise.resolve(() => {
      set.delete(handler);
      events.unlistened += 1;
    });
  },
}));

function install(double: InvokeDouble) {
  backend.invoke = double.invoke;
  return double;
}

beforeEach(() => {
  vi.useFakeTimers();
  events.handlers.clear();
  events.unlistened = 0;
});

afterEach(() => {
  // Unmount before restoring real timers, so a component that schedules on
  // cleanup does it under the clock this test controls.
  cleanup();
  vi.useRealTimers();
});

/**
 * Lets every queued microtask and timer callback run, without advancing time.
 *
 * Inside `act`, because the state these polls set comes from a promise callback
 * rather than an event handler: without it the call counts move and the rendered
 * tree does not, so an assertion about the DOM reads the mount render.
 */
async function settle() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
}

const TRANSCRIPT_LOG_CHANGED = "transcript-log-changed";

/**
 * An idle transcript list makes no calls after the first.
 *
 * It polled `session_transcript_log` every 1.5 s for the life of the process —
 * 40 IPC calls a minute with nothing happening, and from the `log` window as
 * well as the settings page, because a `visible: false` window still runs its
 * React tree. Dictation is bursty and rare, so almost every one of those returned
 * the list it had just returned.
 */
test("an idle transcript list reads once and then not at all", async () => {
  const double = install(invokeDouble({ session_transcript_log: [] }));
  render(<TranscriptLog />);
  await settle();
  expect(double.count("session_transcript_log")).toBe(1);

  // A minute of doing nothing. The old poll would have made 40 calls here.
  await vi.advanceTimersByTimeAsync(60_000);
  expect(double.count("session_transcript_log")).toBe(1);
});

/** One event, one refresh. */
test("one transcript event causes exactly one refresh", async () => {
  const double = install(invokeDouble({ session_transcript_log: [] }));
  render(<TranscriptLog />);
  await settle();
  expect(events.count(TRANSCRIPT_LOG_CHANGED)).toBe(1);

  events.emit(TRANSCRIPT_LOG_CHANGED);
  await settle();
  expect(double.count("session_transcript_log")).toBe(2);

  await vi.advanceTimersByTimeAsync(60_000);
  expect(double.count("session_transcript_log")).toBe(2);
});

/** Unmounting detaches the listener rather than leaving it on a dead tree. */
test("unmounting the transcript list removes its listener", async () => {
  install(invokeDouble({ session_transcript_log: [] }));
  const view = render(<TranscriptLog />);
  await settle();
  expect(events.count(TRANSCRIPT_LOG_CHANGED)).toBe(1);

  view.unmount();
  await settle();
  expect(events.count(TRANSCRIPT_LOG_CHANGED)).toBe(0);
  expect(events.unlistened).toBe(1);
});

const audioSnapshot = () => ({
  level: 0.4,
  active: true,
  device_diagnostic: "opened",
  state: "capturing",
  device_name: "Logitech BRIO",
  error_code: null,
});

/** The mount reads Audio fires once, so a test can subtract them. */
function audioDouble() {
  return install(
    invokeDouble({
      capture_devices: [],
      capture_audio_snapshot: audioSnapshot(),
    }),
  );
}

/**
 * Audio samples at most once at a time, and never overlaps.
 *
 * It was two `invoke`s on a 100 ms `setInterval` — twenty round trips a second,
 * and the interval kept firing while they were outstanding, so a slow answer
 * accumulated concurrent IPC and the two halves could describe different moments.
 * One command, self-scheduled from `.finally`, is at most ten.
 */
test("Audio has at most one sample outstanding and does not overlap", async () => {
  const double = audioDouble();
  const first = deferred<unknown>();
  backend.invoke = (command, args) => {
    if (command === "capture_audio_snapshot") {
      double.calls.push({ command, args });
      return double.count("capture_audio_snapshot") === 1
        ? first.promise
        : Promise.resolve(audioSnapshot());
    }
    return double.invoke(command, args);
  };
  render(<Audio preferredId="" />);
  await settle();
  expect(double.count("capture_audio_snapshot")).toBe(1);

  // A full second of a hung round trip. An interval would have fired ten times.
  await vi.advanceTimersByTimeAsync(1_000);
  expect(double.count("capture_audio_snapshot")).toBe(1);

  // Once it settles, sampling resumes on the gap rather than immediately.
  first.resolve(audioSnapshot());
  await settle();
  expect(double.count("capture_audio_snapshot")).toBe(1);
  await vi.advanceTimersByTimeAsync(100);
  expect(double.count("capture_audio_snapshot")).toBe(2);
});

/**
 * A rejected sample recovers, and does not double up on the way.
 *
 * The rescheduling is in `.finally`, so a refusal schedules exactly as a success
 * does. A page that stopped sampling after one transient refusal would sit on a
 * frozen meter with no way back.
 */
test("a rejected Audio sample recovers without overlapping the next one", async () => {
  const double = audioDouble();
  double.reject("capture_audio_snapshot", "capture_status_unavailable");
  render(<Audio preferredId="" />);
  await settle();
  expect(double.count("capture_audio_snapshot")).toBe(1);

  await vi.advanceTimersByTimeAsync(100);
  await settle();
  expect(double.count("capture_audio_snapshot")).toBe(2);

  // Ten gaps, ten samples: one per gap even while every one of them fails.
  double.answer("capture_audio_snapshot", audioSnapshot());
  await vi.advanceTimersByTimeAsync(1_000);
  expect(double.count("capture_audio_snapshot")).toBe(12);
});

/** And unmounting stops the timer rather than leaving it running. */
test("unmounting Audio stops its sampling", async () => {
  const double = audioDouble();
  const view = render(<Audio preferredId="" />);
  await settle();
  await vi.advanceTimersByTimeAsync(300);
  const sampled = double.count("capture_audio_snapshot");
  expect(sampled).toBeGreaterThan(1);

  view.unmount();
  await vi.advanceTimersByTimeAsync(5_000);
  expect(double.count("capture_audio_snapshot")).toBe(sampled);
});

/** The rendered meter comes from the snapshot, so one call feeds both panels. */
test("one snapshot feeds the meter and the device-health panel", async () => {
  audioDouble();
  render(<Audio preferredId="" />);
  await settle();

  // The attribute rather than `HTMLMeterElement.value`, which jsdom does not
  // reflect from the attribute and reports as 0 whatever is rendered.
  const meter = screen.getByTestId("settings-input-level");
  expect(meter.getAttribute("value")).toBe("0.4");
  expect(screen.getByText("Logitech BRIO")).toBeDefined();
});
