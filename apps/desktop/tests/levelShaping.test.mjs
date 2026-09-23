import assert from "node:assert/strict";
import test from "node:test";

import {
  AGE_TAPER,
  FLOOR_SHARE,
  HISTORY,
  MAX_BAR_PERCENT,
  MIN_SPAN,
  NORMALISE_WINDOW,
  TONE_HIGH,
  TONE_MID,
  barTone,
  barWidth,
  clampLevel,
  normaliser,
  shapeLevel,
} from "../src/hud/levelShaping.ts";

/**
 * A deterministic stand-in for the capture tap: `NORMALISE_WINDOW` samples of
 * speech whose 100 ms peaks sweep between `low` and `high`, newest first.
 */
function speech(low, high) {
  return Array.from({ length: NORMALISE_WINDOW }, (_unused, index) => {
    const phase = Math.abs(Math.sin(index * 1.3));
    return low + (high - low) * phase;
  });
}

/** How the meter would shape `sample` arriving after `window`. */
function shapedAfter(window, sample) {
  const recent = [sample, ...window.slice(0, NORMALISE_WINDOW - 1)];
  return shapeLevel(sample, normaliser(recent));
}

test("silence draws nothing, and the resting mark is left to the stylesheet", () => {
  // Not a floor in here: the resting dash is `min-width: 3px` on
  // `.hud-dock-level-bar`, so silence and "no capture running" draw the same
  // mark without this module knowing how many pixels wide the card is.
  const silent = new Array(NORMALISE_WINDOW).fill(0);
  assert.equal(barWidth(shapedAfter(silent, 0), 0), 0);
  assert.equal(barWidth(shapedAfter([], 0), 0), 0);
});

test("a quiet microphone's ordinary speech fills the card", () => {
  // The defect this replaced. A fixed gain tuned for speech peaking at 0.1-0.3
  // drew a device that peaks at 0.05 as thin spikes along the rail. The range is
  // measured from the recent samples now, so the loudest recent syllable reaches
  // most of the card on either device.
  const quiet = speech(0.004, 0.05);
  const loudest = barWidth(shapedAfter(quiet, 0.05), 0);
  assert.ok(loudest > 70, `a quiet device's loudest syllable drew ${loudest}% of the meter`);

  const middling = barWidth(shapedAfter(quiet, 0.03), 0);
  assert.ok(middling > 35, `a quiet device's middling syllable drew ${middling}%`);
  assert.ok(middling < loudest, "a softer syllable must still draw narrower than a louder one");
});

test("a loud microphone still shows the shape of speech instead of pegging", () => {
  // The other half of the same problem: a fixed gain high enough for the quiet
  // device pegs every syllable on a loud one, and a meter pinned at full width
  // reports one bit rather than a level.
  const loud = speech(0.02, 0.5);
  const peak = shapedAfter(loud, 0.5);
  const syllable = shapedAfter(loud, 0.25);
  assert.ok(peak > 0.95, `the loudest syllable shaped to ${peak}`);
  assert.ok(syllable < 0.5, `a middling syllable on a loud device shaped to ${syllable}`);
  assert.equal(barTone(syllable), "mid");
});

test("room tone stays near the rail instead of reading as speech", () => {
  // The limit on the gain. A window holding only a quiet room has a span of a
  // few thousandths, and stretching that across the card would say "the
  // microphone is hearing you" when nothing is being said.
  const room = speech(0.004, 0.012);
  const width = barWidth(shapedAfter(room, 0.012), 0);
  assert.ok(width < 15, `room tone drew ${width}% of the meter`);
  assert.equal(barTone(shapedAfter(room, 0.012)), "low");
  assert.equal(normaliser(room).span, MIN_SPAN);
});

test("unbroken speech does not lose its softer syllables to the floor", () => {
  // Three seconds with no pause has no room tone in it, and a floor at the
  // quietest syllable would draw that syllable as silence.
  const unbroken = speech(0.15, 0.3);
  const range = normaliser(unbroken);
  assert.ok(range.floor <= Math.max(...unbroken) * FLOOR_SHARE + 1e-9);
  assert.ok(shapeLevel(0.15, range) > 0.2, "the softest syllable must stay visible");
});

test("no width overruns the card, whatever arrives", () => {
  for (const sample of [0, 0.05, 0.3, 1, 4]) {
    const width = barWidth(shapedAfter(speech(0, 0.01), sample), 0);
    assert.ok(width <= MAX_BAR_PERCENT, `${sample} drew ${width}%`);
  }
  // Short of 100 so the widest bar reads as a bar inside the card rather than
  // as a rule touching both walls.
  assert.ok(MAX_BAR_PERCENT < 100);
});

test("the age taper keeps a loud passage moving without thinning it to tips", () => {
  const centre = barWidth(1, 0);
  const oldest = barWidth(1, HISTORY - 1);
  assert.equal(oldest, Math.round(MAX_BAR_PERCENT * (1 - AGE_TAPER) * 10) / 10);
  // Without a taper a sustained loud passage is a solid block; with too much of
  // one the outer rows are the thin tips the dock was reported for.
  assert.ok(oldest < centre);
  assert.ok(oldest >= centre / 2, `the oldest row is ${oldest}% against ${centre}% at the centre`);

  let previous = Infinity;
  for (let age = 0; age < HISTORY; age += 1) {
    const width = barWidth(1, age);
    assert.ok(width < previous, `age ${age} was not narrower than age ${age - 1}`);
    previous = width;
  }
});

test("loud is purple, middling is blue, quiet is green", () => {
  assert.equal(barTone(0.1), "low");
  assert.equal(barTone(0.5), "mid");
  assert.equal(barTone(0.9), "high");

  // Inclusive at the bottom of each band, so a value exactly on a threshold
  // takes the louder band.
  assert.equal(barTone(TONE_HIGH), "high");
  assert.equal(barTone(TONE_HIGH - 0.001), "mid");
  assert.equal(barTone(TONE_MID), "mid");
  assert.equal(barTone(TONE_MID - 0.001), "low");
});

test("a bar's colour is its sample's loudness, not its drawn width", () => {
  // The drawn width also carries the age taper, so colouring by width would
  // repaint a bar as it aged outward. The meter stores the shaped value once and
  // reads both width and tone from it.
  const loud = 0.9;
  assert.ok(barWidth(loud, HISTORY - 1) < barWidth(loud, 0));
  for (let age = 0; age < HISTORY; age += 1) {
    assert.equal(barTone(loud), "high", `the sample changed band at age ${age}`);
  }
});

test("levels outside the meter's range, and non-numbers, cannot escape it", () => {
  assert.equal(clampLevel(-1), 0);
  assert.equal(clampLevel(4), 1);
  assert.equal(clampLevel(Number.NaN), 0);
  // A NaN reaching `width` would be dropped by the browser and the bar would
  // silently keep its previous width — a frozen meter that still looks live.
  const range = normaliser([Number.NaN, 0.2, Number.NaN]);
  assert.ok(Number.isFinite(range.floor) && Number.isFinite(range.span));
  assert.equal(barWidth(shapeLevel(Number.NaN, range), 0), 0);
  assert.equal(barWidth(Number.NaN, 0), 0);
  assert.equal(barTone(shapeLevel(Number.NaN, range)), "low");
});
