import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";

import { messages } from "../../src/catalog";
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
  /** Held open by a test that needs `listen` to resolve late; see the mock. */
  attach: null as { promise: Promise<void>; resolve: (value: void) => void } | null,
  /** Makes `listen` reject, which the component has to survive. */
  refuse: false,
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

/**
 * Registration is asynchronous, and the handler is attached when it completes —
 * not when `listen` is called.
 *
 * Both halves match Tauri and both are load-bearing. The gap between calling
 * `listen` and being subscribed is where an event has nobody to reach, so a mock
 * that registers synchronously cannot express a lost one. `events.attach` holds
 * that gap open for a test that needs to fire an event inside it; left null it
 * closes on the next microtask.
 */
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: () => void) =>
    (events.attach?.promise ?? Promise.resolve()).then(() => {
      if (events.refuse) throw "event_subscription_refused";
      const set = events.handlers.get(name) ?? new Set();
      set.add(handler);
      events.handlers.set(name, set);
      return () => {
        set.delete(handler);
        events.unlistened += 1;
      };
    }),
}));

function install(double: InvokeDouble) {
  backend.invoke = double.invoke;
  return double;
}

beforeEach(() => {
  vi.useFakeTimers();
  events.handlers.clear();
  events.unlistened = 0;
  events.attach = null;
  events.refuse = false;
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

/** One listed transcript, as `session_transcript_log` answers it. */
function transcript(id: string, text: string) {
  return { id, text, provenance: "finalized_stream", recorded_unix_ms: 1_700_000_000_000 };
}

/**
 * Holds `session_transcript_log` open, one call at a time, so a test decides
 * when each read stops being outstanding.
 *
 * `answer` settles the nth read, counting from zero in call order. A read beyond
 * the staged ones resolves to the empty list rather than hanging, so a read the
 * test did not plan for cannot be mistaken for one it is holding.
 */
function stagedReads(count: number) {
  const double = install(invokeDouble({ session_transcript_log: [] }));
  const staged = Array.from({ length: count }, () => deferred<unknown>());
  backend.invoke = (command, args) => {
    if (command !== "session_transcript_log") return double.invoke(command, args);
    double.calls.push({ command, args });
    return staged[double.count(command) - 1]?.promise ?? Promise.resolve([]);
  };
  const answer = (nth: number, entries: ReturnType<typeof transcript>[]) => {
    const read = staged[nth];
    if (read === undefined) throw new Error(`read ${nth} was not staged`);
    read.resolve(entries);
  };
  return { double, answer };
}

/**
 * A change between mounting and being subscribed reaches the window.
 *
 * The read was issued before `listen` resolved, so the answer already in flight
 * predated the change and the event that announced it reached no handler. With
 * nothing else scheduled — the poll that used to heal this on its next tick is
 * gone — the window rendered a stale list for the life of the process.
 *
 * Subscribing first makes the unsubscribed gap carry no reads, so there is no
 * answer for an event to invalidate.
 */
test("an update between mount and subscription is not lost", async () => {
  const attach = deferred<void>();
  events.attach = attach;
  const double = install(invokeDouble({ session_transcript_log: [] }));
  render(<TranscriptLog />);
  await settle();
  expect(double.count("session_transcript_log")).toBe(0);

  // A transcript lands while nobody is subscribed. The event reaches no handler,
  // which is exactly the wakeup that used to be lost.
  double.answer("session_transcript_log", [transcript("t-1", "spoken while subscribing")]);
  events.emit(TRANSCRIPT_LOG_CHANGED);

  attach.resolve();
  await settle();
  expect(double.count("session_transcript_log")).toBe(1);
  expect(screen.getByText("spoken while subscribing")).toBeDefined();
});

/**
 * Events arriving during a read coalesce into exactly one follow-up, and the
 * window ends on the fresh list.
 *
 * A read per event would let two answers race and the loser overwrite the
 * winner; ignoring them would leave the outstanding read's already-stale answer
 * standing.
 */
test("an event during an outstanding read produces one fresh follow-up", async () => {
  const { double, answer } = stagedReads(2);
  render(<TranscriptLog />);
  await settle();
  expect(double.count("session_transcript_log")).toBe(1);

  // Three events against one outstanding read.
  for (let i = 0; i < 3; i += 1) events.emit(TRANSCRIPT_LOG_CHANGED);
  await settle();
  expect(double.count("session_transcript_log")).toBe(1);

  answer(0, [transcript("t-1", "the answer that was already stale")]);
  await settle();
  expect(double.count("session_transcript_log")).toBe(2);

  answer(1, [transcript("t-2", "the list as it now stands")]);
  await settle();
  expect(screen.getByText("the list as it now stands")).toBeDefined();
  expect(screen.queryByText("the answer that was already stale")).toBeNull();

  // And it stops there: three events, one follow-up, no tail of reads.
  await vi.advanceTimersByTimeAsync(60_000);
  expect(double.count("session_transcript_log")).toBe(2);
});

/**
 * An answer invalidated while outstanding never reaches the screen.
 *
 * It describes the list as it was before the event that superseded it, so
 * rendering it on the way to the follow-up shows the user a list that is already
 * known to be wrong — and if the follow-up is slow, shows it for as long as the
 * follow-up takes.
 */
test("an older response cannot overwrite a newer list", async () => {
  const { double, answer } = stagedReads(3);
  render(<TranscriptLog />);
  await settle();

  answer(0, [transcript("t-1", "the list as it stands")]);
  await settle();
  expect(screen.getByText("the list as it stands")).toBeDefined();

  // A second read, invalidated by a further event while it is outstanding.
  events.emit(TRANSCRIPT_LOG_CHANGED);
  await settle();
  expect(double.count("session_transcript_log")).toBe(2);
  events.emit(TRANSCRIPT_LOG_CHANGED);
  await settle();

  answer(1, [transcript("t-0", "an answer overtaken in flight")]);
  await settle();
  expect(screen.queryByText("an answer overtaken in flight")).toBeNull();
  expect(screen.getByText("the list as it stands")).toBeDefined();

  answer(2, [transcript("t-2", "the list after both changes")]);
  await settle();
  expect(screen.getByText("the list after both changes")).toBeDefined();
});

/**
 * Unmounting mid-read detaches the listener and lands nothing on the dead tree.
 *
 * The read outstanding at unmount still settles, and its `.finally` is what
 * issues a follow-up — so a cleanup that only unsubscribed would leave a
 * coalesced event re-reading, and setting state, after the component was gone.
 */
test("unmounting during a read removes the listener and updates nothing", async () => {
  const { double, answer } = stagedReads(2);
  const errors = vi.spyOn(console, "error").mockImplementation(() => {});
  const view = render(<TranscriptLog />);
  await settle();
  expect(double.count("session_transcript_log")).toBe(1);
  events.emit(TRANSCRIPT_LOG_CHANGED);

  view.unmount();
  await settle();
  expect(events.count(TRANSCRIPT_LOG_CHANGED)).toBe(0);
  expect(events.unlistened).toBe(1);

  answer(0, [transcript("t-1", "answered after the window closed")]);
  await settle();
  await vi.advanceTimersByTimeAsync(60_000);
  // No follow-up read, nothing rendered, and no React complaint about a state
  // update on an unmounted tree.
  expect(double.count("session_transcript_log")).toBe(1);
  expect(screen.queryByText("answered after the window closed")).toBeNull();
  expect(errors).not.toHaveBeenCalled();
  errors.mockRestore();
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

/**
 * A refused subscription still shows the list, and says it will not update.
 *
 * `listen` can reject. Left to the promise that was an unhandled rejection and
 * -- because the snapshot is issued from the subscription's resolution -- no
 * read at all, so the window sat on an empty list for the life of the process
 * with nothing on screen to say why. Both halves are asserted: the one
 * authorized snapshot happens, and the page stops claiming to be current.
 */
test("a refused subscription still snapshots once and says it is not live", async () => {
  events.refuse = true;
  const double = install(
    invokeDouble({
      session_transcript_log: [transcript("t-1", "recorded before the refusal")],
    }),
  );
  const errors = vi.spyOn(console, "error").mockImplementation(() => {});
  render(<TranscriptLog />);
  await settle();

  expect(double.count("session_transcript_log")).toBe(1);
  expect(screen.getByText("recorded before the refusal")).toBeDefined();
  expect(screen.getByTestId("session-log-not-live").textContent).toBe(
    messages.sessionLogNotLive,
  );
  expect(events.count(TRANSCRIPT_LOG_CHANGED)).toBe(0);

  // And it stays at one read: no listener means no follow-up, and the notice is
  // what carries that rather than a silent poll reappearing.
  await vi.advanceTimersByTimeAsync(60_000);
  expect(double.count("session_transcript_log")).toBe(1);
  expect(errors).not.toHaveBeenCalled();
  errors.mockRestore();
});

/** A successful subscription says nothing, because there is nothing to say. */
test("a live transcript list carries no unavailable notice", async () => {
  install(invokeDouble({ session_transcript_log: [] }));
  render(<TranscriptLog />);
  await settle();

  expect(screen.queryByTestId("session-log-not-live")).toBeNull();
});
