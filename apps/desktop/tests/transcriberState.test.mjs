import assert from "node:assert/strict";
import test from "node:test";

import {
  HUD_SCHEMA_VERSION,
  PENDING_TIMEOUT_MS,
  REQUEST_DEBOUNCE_MS,
  ceilingWarningActive,
  initialTranscriberModel,
  remainingBeforeCeiling,
  transcriberReducer,
} from "../src/state/transcriberState.ts";

/** A ready, configured, idle backend snapshot. Override one field per test. */
function status(overrides = {}) {
  return {
    schema_version: HUD_SCHEMA_VERSION,
    sequence: 1,
    session_id: "session-a",
    session: "idle",
    vad: "manual_stop_only",
    level: 0,
    device_diagnostic: "opened",
    streaming_mode: "final_only",
    mutable_text: "",
    stable_display_text: "",
    final_text: "",
    device_name: "Logitech BRIO",
    hotkey_binding: "Ctrl+Alt+L",
    hotkey_registration: "registered",
    can_start: true,
    can_stop: false,
    setup_complete: true,
    setup_reason: null,
    elapsed_ms: 0,
    ceiling_ms: 120_000,
    preferred_device_id: "device-brio",
    delivery_outcome: "held",
    engine: "ready",
    error_code: null,
    final_source_reason: null,
    ...overrides,
  };
}

function apply(model, ...actions) {
  return actions.reduce((current, action) => transcriberReducer(current, action), model);
}

const ready = transcriberReducer(initialTranscriberModel, {
  type: "status",
  status: status(),
  now: 0,
});

test("the reducer walks idle -> starting -> listening -> stopping -> transcribing -> delivered", () => {
  assert.deepEqual(ready.state, { kind: "idle" });

  const starting = transcriberReducer(ready, { type: "start_requested", now: 1_000 });
  assert.equal(starting.state.kind, "starting");

  const listening = transcriberReducer(starting, {
    type: "status",
    status: status({ sequence: 2, session: "streaming", can_start: false, can_stop: true, elapsed_ms: 7_000, level: 0.4 }),
    now: 1_200,
  });
  assert.deepEqual(listening.state, {
    kind: "listening",
    elapsedMs: 7_000,
    level: 0.4,
    device: "Logitech BRIO",
  });

  const stopping = transcriberReducer(listening, { type: "stop_requested", now: 8_000 });
  assert.equal(stopping.state.kind, "stopping");

  const transcribing = transcriberReducer(stopping, {
    type: "status",
    status: status({ sequence: 3, session: "finalizing", can_start: false, can_stop: false, elapsed_ms: 8_400 }),
    now: 8_200,
  });
  assert.deepEqual(transcribing.state, { kind: "transcribing", capturedSeconds: 8.4 });

  const delivered = transcriberReducer(transcribing, {
    type: "status",
    status: status({
      sequence: 4,
      session: "complete",
      final_text: "Ever tried? Ever failed.",
      delivery_outcome: "inserted",
    }),
    now: 9_000,
  });
  assert.deepEqual(delivered.state, {
    kind: "delivered",
    outcome: "inserted",
    text: "Ever tried? Ever failed.",
    sourceReasonCode: null,
  });
});

test("a Granite disclosure code is carried into the delivered state verbatim", () => {
  const delivered = transcriberReducer(ready, {
    type: "status",
    status: status({
      sequence: 2,
      session: "complete",
      final_text: "kept",
      delivery_outcome: "inserted",
      final_source_reason: "granite_failed",
    }),
    now: 0,
  });
  assert.deepEqual(delivered.state, {
    kind: "delivered",
    outcome: "inserted",
    text: "kept",
    sourceReasonCode: "granite_failed",
  });
});

test("a stale sequence and an unexpected schema version are both dropped", () => {
  const advanced = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 9, session: "streaming", can_stop: true, elapsed_ms: 3_000 }),
    now: 0,
  });
  assert.equal(advanced.sequence, 9);

  const late = transcriberReducer(advanced, {
    type: "status",
    status: status({ sequence: 8, session: "idle" }),
    now: 10,
  });
  assert.equal(late, advanced, "a response older than the current sequence is ignored entirely");

  const wrongSchema = transcriberReducer(advanced, {
    type: "status",
    status: status({ sequence: 10, schema_version: HUD_SCHEMA_VERSION + 1, session: "idle" }),
    now: 20,
  });
  assert.equal(wrongSchema, advanced, "an unexpected schema_version is rejected, not coerced");
});

test("the §8.3 fields alone still advance the sequence and reach the model", () => {
  // Only the new fields change: same session, same text, higher sequence.
  const updated = transcriberReducer(ready, {
    type: "status",
    status: status({
      sequence: 2,
      device_name: "Yeti Nano",
      hotkey_registration: "conflict",
      preferred_device_id: "device-yeti",
    }),
    now: 100,
  });
  assert.equal(updated.sequence, 2);
  assert.equal(updated.deviceName, "Yeti Nano");
  assert.equal(updated.hotkeyRegistration, "conflict");
  // The picker reads this rather than holding its own selection, so a preference
  // changed anywhere — including from settings — reaches the transcriber.
  assert.equal(updated.preferredDeviceId, "device-yeti");
});

test("rapid repeated start events open exactly one session", () => {
  const first = transcriberReducer(ready, { type: "start_requested", now: 1_000 });
  const second = transcriberReducer(first, { type: "start_requested", now: 1_010 });
  const third = transcriberReducer(second, { type: "start_requested", now: 1_020 });
  assert.equal(second, first, "a second press while a start is in flight is ignored");
  assert.equal(third, first, "and so is a third");

  // Even once the backend confirms, a repeat inside the debounce window is dropped.
  const listening = transcriberReducer(first, {
    type: "status",
    status: status({ sequence: 2, session: "streaming", can_start: false, can_stop: true }),
    now: 1_050,
  });
  assert.equal(listening.pending, null);
  const repeat = transcriberReducer(listening, {
    type: "start_requested",
    now: 1_050 + REQUEST_DEBOUNCE_MS - 1,
  });
  assert.equal(repeat, listening, "a repeat inside the debounce window cannot start a second session");
});

test("a second stop does not queue a second transcription", () => {
  const listening = apply(
    ready,
    { type: "status", status: status({ sequence: 2, session: "streaming", can_start: false, can_stop: true }), now: 0 },
  );
  const stopping = transcriberReducer(listening, { type: "stop_requested", now: 5_000 });
  const again = transcriberReducer(stopping, { type: "stop_requested", now: 5_400 });
  assert.equal(again, stopping, "stop is idempotent while the first stop is in flight");
});

test("an optimistic start survives polls that have not caught up, but not forever", () => {
  const starting = transcriberReducer(ready, { type: "start_requested", now: 1_000 });

  // The backend has not observed the start yet; the HUD must not flap back to idle.
  const notYet = transcriberReducer(starting, {
    type: "status",
    status: status({ sequence: 2, session: "idle" }),
    now: 1_100,
  });
  assert.equal(notYet.state.kind, "starting");
  assert.equal(notYet.pending, "start");

  // A start that never lands must not strand the HUD in `starting`.
  const timedOut = transcriberReducer(starting, {
    type: "status",
    status: status({ sequence: 3, session: "idle" }),
    now: 1_000 + PENDING_TIMEOUT_MS + 1,
  });
  assert.deepEqual(timedOut.state, { kind: "idle" });
  assert.equal(timedOut.pending, null);
});

test("a model still loading is reported as loading, not as ready to record", () => {
  // The regression this exists for: a verified-on-disk model reports
  // setup_complete, can_start and session: "idle" the instant the app launches,
  // while the launch warm is still loading it. Ready and loading were literally
  // the same payload, so the button claimed the app could record when a press
  // would have blocked on the load's mutex.
  assert.deepEqual(initialTranscriberModel.state, { kind: "loading_model" });

  for (const engine of ["cold", "warming"]) {
    const loading = transcriberReducer(initialTranscriberModel, {
      type: "status",
      status: status({ engine }),
      now: 0,
    });
    assert.deepEqual(loading.state, { kind: "loading_model" }, `engine: ${engine}`);
  }

  // A warm that failed costs live streaming text, not the ability to dictate, so
  // it must not park the button on a load that has already given up.
  for (const engine of ["ready", "streaming_model_load_failed", "streaming_worker_unavailable"]) {
    const usable = transcriberReducer(initialTranscriberModel, {
      type: "status",
      status: status({ engine }),
      now: 0,
    });
    assert.deepEqual(usable.state, { kind: "idle" }, `engine: ${engine}`);
  }
});

test("loading outranks nothing that is already running, and setup outranks it", () => {
  // Capture runs whether or not the engine warmed — only live text depends on it —
  // so a load finishing mid-dictation is not the user's problem to look at.
  const listening = transcriberReducer(initialTranscriberModel, {
    type: "status",
    status: status({ session: "streaming", engine: "warming", can_stop: true }),
    now: 0,
  });
  assert.equal(listening.state.kind, "listening");

  // A missing model is something to go and fix, which outranks a wait.
  const missing = transcriberReducer(initialTranscriberModel, {
    type: "status",
    status: status({ engine: "cold", setup_complete: false, setup_reason: "model_missing" }),
    now: 0,
  });
  assert.deepEqual(missing.state, { kind: "setup_required", reason: "model_missing" });
});

test("a start cannot be issued while the model is loading", () => {
  const loading = transcriberReducer(initialTranscriberModel, {
    type: "status",
    // `can_start` is deliberately true, exactly as the backend reports it during
    // the warm: nothing is missing, so the gate here is the load and not setup.
    status: status({ engine: "warming", can_start: true }),
    now: 0,
  });
  assert.deepEqual(loading.state, { kind: "loading_model" });

  const pressed = transcriberReducer(loading, { type: "start_requested", now: 1_000 });
  assert.equal(pressed, loading, "dictation_start would have blocked on the load's own mutex");
  assert.equal(pressed.pending, null);

  // And the moment the load lands, the same press works.
  const warmed = transcriberReducer(loading, { type: "status", status: status({ sequence: 2 }), now: 1_100 });
  assert.deepEqual(warmed.state, { kind: "idle" });
  assert.equal(transcriberReducer(warmed, { type: "start_requested", now: 2_000 }).state.kind, "starting");
});

test("delivery outcome is never upgraded to inserted", () => {
  const refused = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 2, session: "complete", final_text: "kept", delivery_outcome: "refused" }),
    now: 0,
  });
  assert.deepEqual(refused.state, {
    kind: "delivered",
    outcome: "refused",
    text: "kept",
    sourceReasonCode: null,
  });

  const unknown = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 3, session: "complete", final_text: "kept", delivery_outcome: "who_knows" }),
    now: 0,
  });
  assert.equal(unknown.state.outcome, "held", "an unrecognised outcome is never reported as inserted");
});

test("a delivery failure keeps the text and a transcription failure stays retryable", () => {
  const refused = transcriberReducer(ready, {
    type: "status",
    status: status({
      sequence: 2,
      session: "complete",
      final_text: "the text the target refused",
      delivery_outcome: "refused",
    }),
    now: 0,
  });
  assert.equal(refused.finalText, "the text the target refused");
  assert.equal(refused.state.text, "the text the target refused");

  const failed = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 3, session: "failed", error_code: "runtime_adapter_failed" }),
    now: 0,
  });
  assert.deepEqual(failed.state, {
    kind: "failed",
    code: "runtime_adapter_failed",
    recoverable: true,
  });
});

test("incomplete setup outranks every session state and names the requirement", () => {
  const blocked = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 2, setup_complete: false, setup_reason: "model_missing" }),
    now: 0,
  });
  assert.deepEqual(blocked.state, { kind: "setup_required", reason: "model_missing" });

  const unknownReason = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 3, setup_complete: false, setup_reason: "not_a_reason" }),
    now: 0,
  });
  assert.equal(unknownReason.state.reason, "onboarding_incomplete");
});

test("Done clears a finished outcome and never touches an active session", () => {
  const delivered = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 2, session: "complete", final_text: "done", delivery_outcome: "inserted" }),
    now: 0,
  });
  assert.deepEqual(transcriberReducer(delivered, { type: "dismissed", now: 1 }).state, { kind: "idle" });

  const listening = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 3, session: "streaming", can_stop: true }),
    now: 0,
  });
  assert.equal(
    transcriberReducer(listening, { type: "dismissed", now: 1 }),
    listening,
    "Done cannot discard an active recording",
  );
});

test("Done survives the next poll, and a new dictation is unaffected", () => {
  // Regression. Done used to set the state to idle and then lose to the poll
  // 100 ms later, which still read `session: "complete"` from the backend and put
  // the delivered state straight back. Found on the installed build, where three
  // clicks on the primary button all reported its label as "Done" and no dictation
  // ever started, because Start was never actually on screen to be pressed.
  const delivered = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 2, session: "complete", final_text: "kept", delivery_outcome: "inserted" }),
    now: 0,
  });
  assert.equal(delivered.state.kind, "delivered");

  const dismissed = transcriberReducer(delivered, { type: "dismissed", now: 100 });
  assert.deepEqual(dismissed.state, { kind: "idle" });

  // The backend keeps reporting the same finished session. That must stay idle.
  const nextPoll = transcriberReducer(dismissed, {
    type: "status",
    status: status({ sequence: 3, session: "complete", final_text: "kept", delivery_outcome: "inserted" }),
    now: 200,
  });
  assert.deepEqual(nextPoll.state, { kind: "idle" }, "Done must not be undone by the next poll");

  // The text has to go with it. This half of Done was broken for as long as it
  // existed and no test caught it, because these assertions only ever looked at
  // `state` — and no component called `dismiss`, so nobody saw it either. Clicking
  // Done in the running window left the transcript on screen under an idle
  // transcriber: the reducer cleared `finalText` and the poll 100 ms later wrote it
  // straight back from `status.final_text`.
  assert.equal(dismissed.finalText, "", "Done must clear the final it was dismissing");
  assert.equal(
    nextPoll.finalText,
    "",
    "a dismissed session's transcript must not return on the next poll",
  );
  assert.equal(nextPoll.stableDisplayText, "");
  assert.equal(nextPoll.mutableText, "");

  // …and the next dictation's text must still arrive normally.
  assert.equal(
    transcriberReducer(nextPoll, {
      type: "status",
      status: status({ sequence: 4, session_id: "session-b", session: "streaming", stable_display_text: "live" }),
      now: 250,
    }).stableDisplayText,
    "live",
    "only the dismissed session is suppressed, not every session after it",
  );

  // A *different* session's outcome is a new thing to report, not a dismissed one.
  const nextDictation = transcriberReducer(nextPoll, {
    type: "status",
    status: status({
      sequence: 4,
      session_id: "session-b",
      session: "complete",
      final_text: "second",
      delivery_outcome: "inserted",
    }),
    now: 300,
  });
  assert.deepEqual(nextDictation.state, {
    kind: "delivered",
    outcome: "inserted",
    text: "second",
    sourceReasonCode: null,
  });

  // A dismissed failure stays dismissed too.
  const failed = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 5, session: "failed", error_code: "capture_empty" }),
    now: 400,
  });
  assert.equal(failed.state.kind, "failed");
  const clearedFailure = transcriberReducer(failed, { type: "dismissed", now: 500 });
  const afterFailure = transcriberReducer(clearedFailure, {
    type: "status",
    status: status({ sequence: 6, session: "failed", error_code: "capture_empty" }),
    now: 600,
  });
  assert.deepEqual(afterFailure.state, { kind: "idle" });
});

test("the three streaming tiers stay separate fields and are never collapsed", () => {
  const streaming = transcriberReducer(ready, {
    type: "status",
    status: status({
      sequence: 2,
      session: "streaming",
      can_stop: true,
      streaming_mode: "live_qualified",
      stable_display_text: "Ever tried? Ever failed. No matter. Try again.",
      mutable_text: "Fail again. Fail bett",
      final_text: "",
    }),
    now: 0,
  });
  assert.equal(streaming.streamingMode, "live_qualified");
  assert.equal(streaming.stableDisplayText, "Ever tried? Ever failed. No matter. Try again.");
  assert.equal(streaming.mutableText, "Fail again. Fail bett");
  assert.equal(streaming.finalText, "");
});

test("the ceiling warning arms near the ceiling of an active recording", () => {
  const early = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 2, session: "streaming", can_stop: true, elapsed_ms: 30_000 }),
    now: 0,
  });
  assert.equal(remainingBeforeCeiling(early), 90_000);
  assert.equal(ceilingWarningActive(early), false);

  const late = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 3, session: "streaming", can_stop: true, elapsed_ms: 110_000 }),
    now: 0,
  });
  assert.equal(ceilingWarningActive(late), true);

  const idleNearCeiling = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 4, session: "idle", elapsed_ms: 110_000 }),
    now: 0,
  });
  assert.equal(ceilingWarningActive(idleNearCeiling), false);
});

test("a short ceiling does not leave the warning permanently lit", () => {
  // A warning that is on for the whole recording carries no information. With
  // a two-minute ceiling the fixed two-minute band would never turn off, so
  // the band is capped at a quarter of the ceiling.
  const ceiling = 120_000;
  const justStarted = transcriberReducer(ready, {
    type: "status",
    status: status({
      sequence: 2,
      session: "streaming",
      can_stop: true,
      ceiling_ms: ceiling,
      elapsed_ms: 1_000,
    }),
    now: 0,
  });
  assert.equal(ceilingWarningActive(justStarted), false);

  const nearlyThere = transcriberReducer(ready, {
    type: "status",
    status: status({
      sequence: 3,
      session: "streaming",
      can_stop: true,
      ceiling_ms: ceiling,
      elapsed_ms: 105_000,
    }),
    now: 0,
  });
  assert.equal(ceilingWarningActive(nearlyThere), true);
});

test("start and stop are refused when the backend says they are not available", () => {
  const notReady = transcriberReducer(ready, {
    type: "status",
    status: status({ sequence: 2, can_start: false, can_stop: false }),
    now: 0,
  });
  assert.equal(transcriberReducer(notReady, { type: "start_requested", now: 9_000 }), notReady);
  assert.equal(transcriberReducer(notReady, { type: "stop_requested", now: 9_000 }), notReady);
});
