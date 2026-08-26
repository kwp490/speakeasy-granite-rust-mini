import { useState, type KeyboardEvent } from "react";

import { messages } from "../catalog";
import { Advanced } from "./Advanced";
import { Audio } from "./Audio";
import { General } from "./General";
import { OutputPrivacy } from "./OutputPrivacy";
import { TranscriptLogPage } from "./TranscriptLogPage";
import { Transcription } from "./Transcription";
import { useProfile } from "./useProfile";

/**
 * The settings workspace (UI-GUIDE "Information architecture").
 *
 * Five pages behind a nav rail, which is a vertical `tablist`: exactly one page is
 * visible at a time, which is what the tab pattern describes. The old horizontal
 * tab strip used ArrowLeft/ArrowRight; a vertical rail uses ArrowUp/ArrowDown plus
 * Home/End, and declares `aria-orientation` so the pattern is not merely implied.
 *
 * This file is the shell and nothing else. It used to be 960 lines holding every
 * control in the product, which is why a change to the model list could break the
 * hotkey field. Each page now owns its own data; the profile is shared, because it
 * is one document in the backend and three independent copies would drift.
 *
 * One scroll region, never two: the rail is fixed and the content column scrolls.
 *
 * The rail used to have a setup wizard living above it, shown until onboarding
 * was complete. Setup is the installer's job now and there is no in-app wizard
 * to return to, so the shell is only ever the rail and one page.
 *
 * The transcript log is its own page rather than a section at the bottom of
 * Output. It is the only place a delivered transcript can be read back, which
 * makes it the thing people come here for most often, and it is what the dock's
 * pin control detaches into a window of its own.
 */
type SettingsGroup = "general" | "audio" | "transcription" | "output" | "log" | "advanced";

const settingsGroups: ReadonlyArray<{ id: SettingsGroup; label: string }> = [
  { id: "general", label: messages.settingsGroups.general },
  { id: "audio", label: messages.settingsGroups.audio },
  { id: "transcription", label: messages.settingsGroups.transcription },
  { id: "output", label: messages.settingsGroups.output },
  { id: "log", label: messages.settingsGroups.log },
  { id: "advanced", label: messages.settingsGroups.advanced },
];

export function SettingsApp() {
  const [activeGroup, setActiveGroup] = useState<SettingsGroup>("general");
  const profile = useProfile();

  function focusGroup(index: number) {
    const next = settingsGroups[index];
    if (next === undefined) return;
    setActiveGroup(next.id);
    document.getElementById(`settings-tab-${next.id}`)?.focus();
  }

  function onNavKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    const count = settingsGroups.length;
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        focusGroup((index + 1) % count);
        break;
      case "ArrowUp":
        event.preventDefault();
        focusGroup((index - 1 + count) % count);
        break;
      case "Home":
        event.preventDefault();
        focusGroup(0);
        break;
      case "End":
        event.preventDefault();
        focusGroup(count - 1);
        break;
      default:
        break;
    }
  }

  return (
    <main aria-labelledby="app-title" className="settings" data-testid="desktop-scaffold">
      <header className="settings-header">
        <p className="eyebrow">{messages.version}</p>
        <h1 id="app-title">{messages.settingsHeading}</h1>
      </header>

      <div className="settings-body">
        <nav
          aria-label={messages.settingsNav}
          aria-orientation="vertical"
          className="settings-rail"
          role="tablist"
        >
          {settingsGroups.map((group, index) => (
            <button
              aria-controls={`settings-panel-${group.id}`}
              aria-selected={activeGroup === group.id}
              id={`settings-tab-${group.id}`}
              key={group.id}
              onClick={() => setActiveGroup(group.id)}
              onKeyDown={(event) => onNavKeyDown(event, index)}
              role="tab"
              tabIndex={activeGroup === group.id ? 0 : -1}
              type="button"
            >
              {group.label}
            </button>
          ))}
        </nav>

        <div className="settings-content">
          {/*
            One banner for the whole workspace, because the profile feeds three
            pages and a null one renders every control fed from it at its own
            default -- unchecked boxes and a delivery preference nobody chose. Put
            here rather than on each page so it is seen whichever page is open,
            and so it says the *profile* is unread rather than implying six
            separate settings are off.
          */}
          {profile.unavailable && <p className="warning">{messages.profileUnavailable}</p>}
          <section
            aria-labelledby="settings-tab-general"
            hidden={activeGroup !== "general"}
            id="settings-panel-general"
            role="tabpanel"
            tabIndex={0}
          >
            <h2>{messages.settingsGroups.general}</h2>
            <General profile={profile} />
          </section>
          <section
            aria-labelledby="settings-tab-audio"
            hidden={activeGroup !== "audio"}
            id="settings-panel-audio"
            role="tabpanel"
            tabIndex={0}
          >
            <h2>{messages.settingsGroups.audio}</h2>
            {/* Mounted only while visible: the input meter polls, and a hidden
                page has no business sampling the microphone level. */}
            {activeGroup === "audio" && (
              <Audio preferredId={profile.profile?.preferred_capture_device_id ?? ""} />
            )}
          </section>
          <section
            aria-labelledby="settings-tab-transcription"
            hidden={activeGroup !== "transcription"}
            id="settings-panel-transcription"
            role="tabpanel"
            tabIndex={0}
          >
            <h2>{messages.settingsGroups.transcription}</h2>
            <Transcription />
          </section>
          <section
            aria-labelledby="settings-tab-output"
            hidden={activeGroup !== "output"}
            id="settings-panel-output"
            role="tabpanel"
            tabIndex={0}
          >
            <h2>{messages.settingsGroups.output}</h2>
            {activeGroup === "output" && <OutputPrivacy profile={profile} />}
          </section>
          <section
            aria-labelledby="settings-tab-log"
            hidden={activeGroup !== "log"}
            id="settings-panel-log"
            role="tabpanel"
            tabIndex={0}
          >
            <h2>{messages.settingsGroups.log}</h2>
            {/* Mounted only while visible: the log polls for new entries, and a
                hidden page has no business doing that. Same rule as Audio. */}
            {activeGroup === "log" && <TranscriptLogPage profile={profile} />}
          </section>
          <section
            aria-labelledby="settings-tab-advanced"
            hidden={activeGroup !== "advanced"}
            id="settings-panel-advanced"
            role="tabpanel"
            tabIndex={0}
          >
            <h2>{messages.settingsGroups.advanced}</h2>
            <Advanced profile={profile} />
          </section>
        </div>
      </div>
    </main>
  );
}
