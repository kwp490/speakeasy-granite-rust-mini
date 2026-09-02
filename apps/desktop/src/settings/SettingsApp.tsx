import { useState, type KeyboardEvent } from "react";

import { messages } from "../catalog";
import { Advanced } from "./Advanced";
import { Audio } from "./Audio";
import { General } from "./General";
import { OutputPrivacy } from "./OutputPrivacy";
import { SettingsPageHeader } from "./SettingsPageHeader";
import { TranscriptLogPage } from "./TranscriptLogPage";
import { Transcription } from "./Transcription";
import { useProfile } from "./useProfile";

/**
 * The settings workspace (UI-GUIDE "Information architecture").
 *
 * Six pages behind a nav rail, which is a vertical `tablist`: exactly one page is
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
        <div aria-hidden="true" className="settings-mark">{messages.settingsMark}</div>
        <div className="settings-title">
          <h1 aria-label={messages.settingsHeading} id="app-title">
            {messages.settingsProductName}
          </h1>
          <p>{messages.settings}</p>
        </div>
        <p className="settings-version">{messages.versionLabel(messages.version)}</p>
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
          {/*
            And a refused *write*, here for the same reason. Every control fed
            from the profile renders the stored value, so a rejected toggle
            snaps back and looks like a switch that will not move; before this,
            nothing anywhere said why, and the rejection was unhandled. One
            banner rather than six inline messages: it is one document, one
            write at a time, and the user is looking at whichever page they
            just touched.
          */}
          {profile.write.error !== null && (
            <p className="warning" role="alert">
              {profile.write.error}
            </p>
          )}
          <section
            aria-labelledby="settings-tab-general"
            hidden={activeGroup !== "general"}
            id="settings-panel-general"
            role="tabpanel"
            tabIndex={0}
          >
            <SettingsPageHeader
              detail={messages.settingsPageDetails.general}
              eyebrow={messages.settingsPageEyebrows.general}
              title={messages.settingsGroups.general}
            />
            <General profile={profile} />
          </section>
          <section
            aria-labelledby="settings-tab-audio"
            hidden={activeGroup !== "audio"}
            id="settings-panel-audio"
            role="tabpanel"
            tabIndex={0}
          >
            <SettingsPageHeader
              detail={messages.settingsPageDetails.audio}
              eyebrow={messages.settingsPageEyebrows.audio}
              title={messages.settingsGroups.audio}
            />
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
            <SettingsPageHeader
              detail={messages.settingsPageDetails.transcription}
              eyebrow={messages.settingsPageEyebrows.transcription}
              title={messages.settingsGroups.transcription}
            />
            <Transcription />
          </section>
          <section
            aria-labelledby="settings-tab-output"
            hidden={activeGroup !== "output"}
            id="settings-panel-output"
            role="tabpanel"
            tabIndex={0}
          >
            <SettingsPageHeader
              detail={messages.settingsPageDetails.output}
              eyebrow={messages.settingsPageEyebrows.output}
              title={messages.settingsGroups.output}
            />
            {activeGroup === "output" && <OutputPrivacy profile={profile} />}
          </section>
          <section
            aria-labelledby="settings-tab-log"
            hidden={activeGroup !== "log"}
            id="settings-panel-log"
            role="tabpanel"
            tabIndex={0}
          >
            <SettingsPageHeader
              detail={messages.settingsPageDetails.log}
              eyebrow={messages.settingsPageEyebrows.log}
              title={messages.settingsGroups.log}
            />
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
            <SettingsPageHeader
              detail={messages.settingsPageDetails.advanced}
              eyebrow={messages.settingsPageEyebrows.advanced}
              title={messages.settingsGroups.advanced}
            />
            {/* Mounted only while visible, and here the reason is staleness
                rather than cost. Every field on this page is a fact about *now*:
                the engine reason, the device, the RTF and latency percentiles,
                the overflow count. Mounted eagerly it read them once, at launch,
                before the resident worker had answered `Hello` -- so `WORKER`
                showed `cpu_gpu_runtime_missing`, which is what pack selection
                returns while `cuda_worker_available()` is still conservatively
                false, and it stayed that way for the life of the process. A
                reload against the same backend returned
                `cpu_gpu_pack_not_installed`, which is how the stale read was
                told apart from a refused one (2026-08-28).
                `readWithRetry` cannot fix this one: the early value is a
                legitimate terminal answer on a machine with no CUDA worker, so
                no `settled` predicate can distinguish "not yet" from "not ever"
                without spinning on every processor install. Mounting on tab
                activation reads after `setup` and after the worker has spoken,
                and re-reads whenever somebody opens the page -- which is also
                the only way the performance figures stop being frozen at
                whatever they were when the window was created. Same rule as the
                log and Audio pages above. */}
            {activeGroup === "advanced" && <Advanced profile={profile} />}
          </section>
        </div>
      </div>
    </main>
  );
}
