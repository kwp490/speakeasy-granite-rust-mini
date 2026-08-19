/**
 * How a raw input level becomes a bar in the side dock's waveform.
 *
 * Split out of `DockLevelMeter.tsx` rather than left inline because this is the
 * part with numbers in it, and the test runner strips types from `.ts` but
 * cannot compile the `.tsx` the component lives in. Inline, the only thing a
 * test could do with the shaping was match its source text — which would have
 * passed just as happily on the curve that drew 3px stubs. See
 * `levelShaping.test.mjs`, which asserts the responses instead.
 *
 * `level` arriving here is a 100 ms *peak* of samples normalised to ±1.0, with
 * no gain stage anywhere behind it (`block_peak` in `capture_wizard.rs`).
 */

/** Rows drawn. Odd, so one of them is the centre. */
export const ROWS = 21;

/** Samples retained: one per distance from the centre row. */
export const HISTORY = (ROWS + 1) / 2;

/**
 * The loudest a row gets, as a percentage of the meter's width.
 *
 * A percentage rather than the pixel count this was, because the dock's width
 * is a measured Windows floor rather than a design choice (UI-GUIDE
 * "Responsive, high-DPI, and multi-monitor behavior") and
 * has now moved once. A `px` maximum tuned against one width silently stops
 * filling the card at the next one; a percentage cannot.
 *
 * Short of 100 so the widest bar still reads as a bar inside the card rather
 * than as a rule touching both walls.
 */
export const MAX_BAR_PERCENT = 92;

/**
 * Gain applied before the curve.
 *
 * Speech at a comfortable distance from a typical microphone peaks around
 * 0.1–0.3 of the range described above. Unshaped, and then tapered by age on
 * top, that drew the 3–5px stubs the meter was reported as: technically correct
 * and useless as a signal that the microphone is hearing anything.
 *
 * Deliberately enough gain to peg on a genuinely loud passage. Headroom above a
 * shout is headroom nothing ever uses, and it is paid for by every ordinary
 * sentence rendering in the bottom third of the card.
 */
export const LEVEL_GAIN = 2.6;

/**
 * The response curve applied after `LEVEL_GAIN`.
 *
 * Below 1, so the curve is concave and quiet input still moves the bar — the
 * same job `Math.sqrt` (0.5) did here before. Shallower than a square root
 * because the gain above already does most of the lifting, and stacking the two
 * at full strength put ordinary room tone a third of the way up the card.
 */
export const LEVEL_CURVE = 0.65;

/**
 * How much of its width a row loses per step away from the centre.
 *
 * An envelope on the drawing, not on the data: without it a sustained loud
 * passage fills all `ROWS` rows to `MAX_BAR_PERCENT` and the meter reads as a
 * solid block that no longer moves. The `<meter>` in the component reports the
 * level unshaped.
 */
export const AGE_TAPER = 0.72;

/**
 * Where the colour bands fall on the shaped 0–1 value.
 *
 * Loud is purple, middling is blue, quiet is green (owner decision 2026-08-12).
 * The bands are read off the sample's own loudness and never off the drawn
 * width: the width also carries the age taper, so colouring by it would repaint
 * a bar as it aged and quietly restate how loud that moment had been.
 */
export const TONE_MID = 0.4;
export const TONE_HIGH = 0.72;

export type BarTone = "low" | "mid" | "high";

export function clampLevel(level: number): number {
  // `Math.min` first so a NaN — which every comparison rejects — lands on 0
  // rather than propagating into a `width` the browser ignores.
  return Math.min(1, Math.max(0, level)) || 0;
}

/** A raw 0–1 sample as the 0–1 the drawing works in: gain, then the curve. */
export function shapeLevel(sample: number): number {
  return clampLevel(clampLevel(sample) * LEVEL_GAIN) ** LEVEL_CURVE;
}

/**
 * A shaped sample's row width, as a percentage of the meter.
 *
 * Zero rather than a floor: the resting dot is a `min-width` in the stylesheet,
 * so silence and "no capture running" draw the same 3px mark without this
 * having to know the card's width in pixels.
 */
export function barWidth(shaped: number, age: number): number {
  const taper = 1 - (AGE_TAPER * age) / Math.max(1, HISTORY - 1);
  return Math.round(MAX_BAR_PERCENT * shaped * taper * 10) / 10;
}

export function barTone(shaped: number): BarTone {
  if (shaped >= TONE_HIGH) return "high";
  if (shaped >= TONE_MID) return "mid";
  return "low";
}
