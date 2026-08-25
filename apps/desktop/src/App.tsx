import { getCurrentWindow } from "@tauri-apps/api/window";

import { CaptureNoticeApp } from "./hud/CaptureNoticeApp";
import { HudDockApp } from "./hud/HudDockApp";
import { PinnedLogApp } from "./hud/PinnedLogApp";
import { SettingsApp } from "./settings/SettingsApp";

/**
 * Every window loads this one entry point and branches on the window label.
 *
 * The router owns no hooks. It used to early-return the transcriber *before*
 * thirty `useState` calls, which is a rules-of-hooks violation — safe only
 * because a webview's window label never changes — and is why the file it came
 * from grew to a thousand lines. Nothing warns about it:
 * `eslint-plugin-react-hooks` is not installed here.
 *
 * There were three windows and three branches. The large transcriber HUD is
 * gone: it existed to show words appearing as you spoke, and nothing appears
 * as you speak any more. `notice` joined on 2026-08-25.
 *
 * The fallback is Settings, so a label with no branch renders a settings
 * window. That is the safe wrong answer rather than a blank one, but it does
 * mean a misspelled label here fails by showing the wrong window rather than by
 * failing — check the branch, not just that the window appeared.
 */
export function App() {
  const label = getCurrentWindow().label;
  if (label === "hud-dock") return <HudDockApp />;
  if (label === "log") return <PinnedLogApp />;
  if (label === "notice") return <CaptureNoticeApp />;
  return <SettingsApp />;
}
