import assert from "node:assert/strict";
import test from "node:test";

import {
  AGE_TAPER,
  HISTORY,
  MAX_BAR_PERCENT,
  TONE_HIGH,
  TONE_MID,
  barTone,
  barWidth,
  clampLevel,
  shapeLevel,
} from "../src/hud/levelShaping.ts";

/** The width the dock's waveform draws for a raw level, at the centre row. */
function centreWidth(level) {
  return barWidth(shapeLevel(level), 0);
}

/** The oldest row's width for a raw level — the outer tip of the spindle. */
function oldestWidth(level) {
  return barWidth(shapeLevel(level), HISTORY - 1);
}

test("silence draws nothing, and the resting mark is left to the stylesheet", () => {
  // Not a floor in here. `MIN_BAR` used to be added to every width, which meant
  // this module had to know how many pixels wide the card was; the resting dot
  // is `min-width: 3px` on `.hud-dock-level-bar` now, so silence and "no
  // capture running" draw the same mark without that.
  assert.equal(centreWidth(0), 0);
  assert.equal(oldestWidth(0), 0);
});

test("ordinary speech fills the card rather than drawing a stub", () => {
  // The defect this shaping replaced. `level` is a 100 ms peak of samples
  // normalised to ±1.0 with no gain anywhere behind it, so speech at a
  // comfortable distance peaks around 0.1-0.3 — and the old curve was a bare
  // `Math.sqrt` scaled into 48px, which put a 0.15 peak at sqrt(0.15) * 48 =
  // 17% of a 120px card. Reported, accurately, as "just little spikes".
  const quietSpeech = centreWidth(0.1);
  const normalSpeech = centreWidth(0.15);
  const strongSpeech = centreWidth(0.3);

  assert.ok(normalSpeech > 45, `normal speech drew ${normalSpeech}% of the meter`);
  assert.ok(strongSpeech > 75, `strong speech drew ${strongSpeech}% of the meter`);
  // Still monotonic, and still leaving somewhere to go: a meter that pegs on
  // ordinary speech reports one bit rather than a level.
  assert.ok(quietSpeech < normalSpeech);
  assert.ok(normalSpeech < strongSpeech);
  assert.ok(quietSpeech > 30, `quiet speech drew ${quietSpeech}% of the meter`);
});

test("room tone stays near the rail instead of reading as speech", () => {
  // The other side of the gain: a meter amplified until a quiet room lights it
  // up says "the microphone is hearing you" when nothing is being said.
  assert.ok(centreWidth(0.01) < 12, `room tone drew ${centreWidth(0.01)}%`);
  assert.equal(barTone(shapeLevel(0.01)), "low");
});

test("a genuinely loud passage pegs, and never overruns the card", () => {
  assert.equal(centreWidth(0.45), MAX_BAR_PERCENT);
  assert.equal(centreWidth(1), MAX_BAR_PERCENT);
  // Short of 100 so the widest bar reads as a bar inside the card rather than
  // as a rule touching both walls.
  assert.ok(MAX_BAR_PERCENT < 100);
});

test("the age taper is what keeps a sustained loud passage moving", () => {
  // Without it every row saturates and the meter becomes a solid block that no
  // longer reports anything. Held at the pegged level, where the failure shows.
  const centre = centreWidth(1);
  const oldest = oldestWidth(1);
  assert.ok(oldest < centre);
  assert.equal(oldest, Math.round(MAX_BAR_PERCENT * (1 - AGE_TAPER) * 10) / 10);

  // Monotonic outward, so the silhouette is a spindle rather than a shape with
  // a waist in it.
  let previous = Infinity;
  for (let age = 0; age < HISTORY; age += 1) {
    const width = barWidth(shapeLevel(1), age);
    assert.ok(width < previous, `age ${age} was not narrower than age ${age - 1}`);
    previous = width;
  }
});

test("loud is purple, middling is blue, quiet is green", () => {
  assert.equal(barTone(shapeLevel(0.02)), "low");
  assert.equal(barTone(shapeLevel(0.12)), "mid");
  assert.equal(barTone(shapeLevel(0.35)), "high");

  // The bands themselves, at their own boundaries — inclusive at the bottom of
  // each, so a value exactly on a threshold takes the louder band.
  assert.equal(barTone(TONE_HIGH), "high");
  assert.equal(barTone(TONE_HIGH - 0.001), "mid");
  assert.equal(barTone(TONE_MID), "mid");
  assert.equal(barTone(TONE_MID - 0.001), "low");
});

test("a bar's colour is its sample's loudness, not its drawn width", () => {
  // The bug this forecloses: the drawn width also carries the age taper, so
  // colouring by width would repaint a bar as it aged outward — quietly
  // restating how loud that moment had been. A loud sample stays purple all the
  // way to the tip even though it is drawn at a quarter of the width there.
  const loud = shapeLevel(0.5);
  assert.equal(barTone(loud), "high");
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
  assert.equal(centreWidth(Number.NaN), 0);
  assert.equal(barTone(shapeLevel(Number.NaN)), "low");
});
