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
 *
 * **The scale is relative to the last few seconds, not fixed.** A fixed gain
 * has to be tuned for one microphone. Tuned for speech peaking at 0.1–0.3, a
 * device that peaks at 0.05 drew thin spikes, and one at 0.5 pegged on every
 * syllable.
 * `normaliser` measures the recent quietest and loudest samples and maps that
 * span onto the card, so ordinary speech on any device fills it. `MIN_SPAN`
 * is what stops that from amplifying a silent room into a waveform.
 */

/** Rows drawn. Odd, so one of them is the centre. */
export const ROWS = 15;

/** Samples drawn: one per distance from the centre row. */
export const HISTORY = (ROWS + 1) / 2;

/**
 * Samples the normaliser looks back over — 3 s at the meter's 10 Hz.
 *
 * Long enough to span the pause between two sentences, so the floor it finds
 * is room tone rather than the quietest syllable; short enough that a cough
 * stops shrinking everything after it within a breath or two.
 */
export const NORMALISE_WINDOW = 30;

/**
 * The narrowest loudness span the normaliser will stretch across the card, in
 * raw level units. Its inverse is the most gain the meter ever applies (about
 * 17x), which is what lets a microphone peaking at 0.05 still fill the card.
 *
 * The lower bound that keeps a quiet room quiet. Without it a window holding
 * only room tone has a span of a few thousandths, and the normaliser would
 * blow those fluctuations up to full width — a meter that says "the
 * microphone is hearing you" when nothing is being said.
 */
export const MIN_SPAN = 0.06;

/**
 * The floor never rises above this share of the window's peak.
 *
 * The floor is the quietest recent sample, which is room tone when the window
 * holds a pause. Three seconds of unbroken speech has no pause in it, and a
 * floor at the quietest syllable would drop the softer half of every word to
 * nothing.
 */
export const FLOOR_SHARE = 0.25;

/**
 * The loudest a row gets, as a percentage of the meter's width.
 *
 * A percentage rather than a pixel count, because the dock's width is a
 * measured Windows floor rather than a design choice (UI-GUIDE "Responsive,
 * high-DPI, and multi-monitor behavior") and has moved before. A `px` maximum
 * tuned against one width silently stops filling the card at the next one.
 *
 * Short of 100 so the widest bar still reads as a bar inside the card rather
 * than as a rule touching both walls.
 */
export const MAX_BAR_PERCENT = 96;

/**
 * How much of its width the oldest row loses relative to the centre.
 *
 * An envelope on the drawing, not on the data: without it a sustained loud
 * passage fills every row to `MAX_BAR_PERCENT` and the meter reads as a solid
 * block that no longer moves. Kept under half so the outer rows still read as
 * bars rather than as the thin tips the dock was reported for.
 */
export const AGE_TAPER = 0.45;

/**
 * Where the colour bands fall on the shaped 0–1 value.
 *
 * Loud is purple, middling is blue, quiet is green (owner decision 2026-08-12).
 * The band is fixed when the sample is shaped and never read off the drawn
 * width: the width also carries the age taper, so colouring by it would repaint
 * a bar as it aged and quietly restate how loud that moment had been.
 */
export const TONE_MID = 0.4;
export const TONE_HIGH = 0.75;

export type BarTone = "low" | "mid" | "high";

/** The loudness range the card currently represents, in raw level units. */
export type Normaliser = { floor: number; span: number };

export function clampLevel(level: number): number {
  // `Math.min` first so a NaN — which every comparison rejects — lands on 0
  // rather than propagating into a `width` the browser ignores.
  return Math.min(1, Math.max(0, level)) || 0;
}

/**
 * The span the next sample is drawn against, from the recent raw samples
 * (newest first, including the sample about to be drawn).
 */
export function normaliser(recent: readonly number[]): Normaliser {
  const samples = recent.map(clampLevel);
  const peak = samples.length === 0 ? 0 : Math.max(...samples);
  const quietest = samples.length === 0 ? 0 : Math.min(...samples);
  const floor = Math.min(quietest, peak * FLOOR_SHARE);
  return { floor, span: Math.max(peak - floor, MIN_SPAN) };
}

/**
 * A raw 0–1 sample as the 0–1 the drawing works in.
 *
 * Linear once normalised. The fixed-gain version needed a concave curve to lift
 * quiet input off the rail; the normaliser does that job now, and a curve on
 * top of it lifts room tone along with the speech.
 */
export function shapeLevel(sample: number, range: Normaliser): number {
  return clampLevel((clampLevel(sample) - range.floor) / range.span);
}

/**
 * A shaped sample's row width, as a percentage of the meter.
 *
 * Zero rather than a floor: the resting dash is a `min-width` in the
 * stylesheet, so silence and "no capture running" draw the same mark without
 * this having to know the card's width in pixels.
 */
export function barWidth(shaped: number, age: number): number {
  const taper = 1 - (AGE_TAPER * age) / Math.max(1, HISTORY - 1);
  return Math.round(MAX_BAR_PERCENT * clampLevel(shaped) * taper * 10) / 10;
}

export function barTone(shaped: number): BarTone {
  if (shaped >= TONE_HIGH) return "high";
  if (shaped >= TONE_MID) return "mid";
  return "low";
}
