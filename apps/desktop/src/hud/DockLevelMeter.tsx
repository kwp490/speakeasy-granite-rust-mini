import { useEffect, useRef, useState } from "react";

import { messages } from "../catalog";
import { HISTORY, ROWS, barTone, barWidth, clampLevel, shapeLevel } from "./levelShaping";

/**
 * Input level for the side dock, drawn as a symmetric waveform.
 *
 * The default HUD paints the level as its record button's fill (`LevelMeter`),
 * which needs a wide horizontal box to read as anything. The dock has a narrow
 * column instead, so the same value is drawn as a stack of centred horizontal
 * bars: one row per sample, the newest in the middle, older samples spreading
 * outward in both directions. That is why the shape is symmetric — both halves
 * are the same history, mirrored, not two channels.
 *
 * How a sample becomes a width and a colour is `levelShaping.ts`; this file is
 * only the history and the elements.
 *
 * What is on screen is decoration (`aria-hidden`). The value itself stays on a
 * native `<meter>`, visually hidden but in the accessibility tree, for the same
 * reason `LevelMeter` keeps one: the element is what carries the accessible
 * name and what a screen reader can actually read. Under `forced-colors:
 * active` the bars are dropped exactly as the HUD's fill is — recording is
 * still signalled there by the elapsed clock and the Stop button.
 *
 * Samples on its own 10 Hz timer rather than on `level` changing. Two
 * consecutive polls that report the same float are common during silence and a
 * steady tone, and an effect keyed on the value would freeze the waveform for
 * exactly as long as the input held still.
 */
const SAMPLE_INTERVAL_MS = 100;

export function DockLevelMeter({ level, active }: { level: number; active: boolean }) {
  const history = useLevelHistory(level, active);

  return (
    <div className="hud-dock-level" data-active={active} data-testid="hud-dock-level">
      <meter
        aria-label={messages.inputLevel}
        className="sr-only"
        max={1}
        min={0}
        value={clampLevel(level)}
      />
      <div aria-hidden="true" className="hud-dock-level-bars">
        {Array.from({ length: ROWS }, (_row, index) => {
          const age = Math.abs(index - (HISTORY - 1));
          const shaped = shapeLevel(history[age] ?? 0);
          return (
            <span
              className="hud-dock-level-bar"
              data-tone={barTone(shaped)}
              key={index}
              style={{ width: `${barWidth(shaped, age)}%` }}
            />
          );
        })}
      </div>
    </div>
  );
}

/**
 * The last `HISTORY` samples, newest first.
 *
 * Cleared rather than left to decay when capture stops: a waveform that keeps
 * showing the last thing it heard is a claim that the microphone is still open.
 */
function useLevelHistory(level: number, active: boolean): number[] {
  const latest = useRef(level);
  latest.current = level;
  const [history, setHistory] = useState<number[]>(() => new Array<number>(HISTORY).fill(0));

  useEffect(() => {
    if (!active) {
      setHistory(new Array<number>(HISTORY).fill(0));
      return;
    }
    const timer = window.setInterval(() => {
      setHistory((previous) => [clampLevel(latest.current), ...previous.slice(0, HISTORY - 1)]);
    }, SAMPLE_INTERVAL_MS);
    return () => {
      window.clearInterval(timer);
    };
  }, [active]);

  return history;
}
