import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import { formatError } from "./format";
import { TranscriptLog } from "./TranscriptLog";
import type { ProfileController } from "./useProfile";
import { useMutation } from "./useMutation";

/**
 * The transcript log page: every delivered transcript with Copy, plus the two
 * controls that decide what happens to them.
 *
 * This is the only place a delivered transcript can be read back. The large
 * transcriber HUD used to show the last one with its own Copy button, and the
 * recoverable result view kept the text when a paste was refused; both left
 * with that window, so a transcript that missed its target is recoverable here
 * or nowhere. That is why the log is its own page rather than a section at the
 * bottom of Output, where it used to sit.
 *
 * **Pin** detaches the list into its own small always-on-top window so it stays
 * visible while the user works elsewhere. That window is declared in
 * `tauri.conf.json` and only ever shown or hidden — never built on demand,
 * which deadlocks the whole app's IPC — and it is non-focusable, because
 * anything SpeakEasy puts in the foreground becomes the delivery target for the
 * next dictation.
 *
 * **Retention** is the on-disk history setting, moved here from Output because
 * this is the list it governs. Off by default, and off means the transcripts
 * were never written to disk rather than deleted on the way out — a distinction
 * `SessionTranscriptCoordinator::seed_from_history` explains, and one that
 * survives the process being killed where a delete-on-exit would not.
 *
 * The disclosure gate is kept exactly as Output had it. Turning retention on
 * writes plaintext transcripts to disk, so it stays behind an explicit
 * acknowledgement rather than a single click, and the retention-days control
 * stays hidden while retention is off — a retention period for a feature that
 * is off states nothing true.
 */
export function TranscriptLogPage({ profile }: { profile: ProfileController }) {
  const [historyEnabled, setHistoryEnabled] = useState(false);
  const [historyDisclosure, setHistoryDisclosure] = useState(false);
  const [retentionDays, setRetentionDays] = useState(30);
  const exportHistory = useMutation<string>();
  const deleteHistory = useMutation<void>();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [pinAction, setPinAction] = useState("");

  // Adopt the stored choice once the profile arrives, so the controls start
  // from what is actually saved rather than from a local default.
  useEffect(() => {
    if (profile.profile === null) return;
    setHistoryEnabled(profile.profile.history_enabled);
    setHistoryDisclosure(profile.profile.history_plaintext_disclosure_accepted);
    setRetentionDays(profile.profile.history_retention_days);
  }, [profile.profile]);

  async function pin() {
    setPinAction("");
    try {
      await invoke("transcript_log_pin");
      setPinAction(messages.transcriptLogPinned);
    } catch (error: unknown) {
      setPinAction(formatError(String(error)));
    }
  }

  return (
    <>
      <section aria-labelledby="log-pin">
        <h3 id="log-pin">{messages.transcriptLogPinSection}</h3>
        <p className="setting-detail">{messages.transcriptLogPinDetail}</p>
        <div className="actions">
          <button onClick={() => void pin()} type="button">
            {messages.transcriptLogPin}
          </button>
          <output aria-live="polite">{pinAction}</output>
        </div>
      </section>

      <TranscriptLog />

      <section aria-labelledby="log-retention">
        <h3 id="log-retention">{messages.transcriptLogRetention}</h3>
        <p className="setting-detail">{messages.transcriptLogRetentionDetail}</p>
        <fieldset>
          <legend>{messages.transcriptLogRetention}</legend>
          <label className="confirmation">
            <input
              checked={!historyEnabled}
              name="history"
              onChange={() => setHistoryEnabled(false)}
              type="radio"
            />
            {messages.transcriptLogClearOnClose}
          </label>
          <label className="confirmation">
            <input
              checked={historyEnabled}
              name="history"
              onChange={() => setHistoryEnabled(true)}
              type="radio"
            />
            {messages.transcriptLogRetain}
          </label>
          {historyEnabled && (
            <>
              <p className="warning">{messages.historyDisclosure}</p>
              <label>
                <span>{messages.retentionDays}</span>
                <input
                  max="365"
                  min="1"
                  onChange={(event) => setRetentionDays(Number(event.target.value))}
                  type="number"
                  value={retentionDays}
                />
              </label>
              <label className="confirmation">
                <input
                  checked={historyDisclosure}
                  onChange={(event) => setHistoryDisclosure(event.target.checked)}
                  type="checkbox"
                />
                {messages.acceptHistoryDisclosure}
              </label>
            </>
          )}
          <button
            disabled={historyEnabled && !historyDisclosure}
            onClick={() =>
              void profile.setHistory({
                enabled: historyEnabled,
                retentionDays,
                disclosureAccepted: historyDisclosure,
              })
            }
            type="button"
          >
            {messages.saveHistory}
          </button>
          {profile.profile?.history_enabled === true && (
            <div className="actions">
              <button
                disabled={exportHistory.pending}
                onClick={() => {
                  void exportHistory.run(
                    () => invoke<string>("history_export", { disclosureAccepted: true }),
                    (path) => path,
                  );
                }}
                type="button"
              >
                {exportHistory.pending ? messages.working : messages.exportHistory}
              </button>
              <label className="confirmation">
                <input
                  checked={confirmDelete}
                  onChange={(event) => setConfirmDelete(event.target.checked)}
                  type="checkbox"
                />
                {messages.confirmDeleteHistory}
              </label>
              <button
                className="destructive"
                disabled={!confirmDelete || deleteHistory.pending}
                onClick={() => {
                  void deleteHistory
                    .run(
                      () => invoke("history_delete_all", { confirmed: true }),
                      () => messages.deleted,
                    )
                    .then((deleted) => {
                      // The confirmation is cleared only if the deletion
                      // actually happened. It used to clear either way, so a
                      // refused delete looked exactly like a completed one --
                      // the box unticked itself and the user had no reason to
                      // think their transcripts were still on disk.
                      if (deleted !== null) setConfirmDelete(false);
                    });
                }}
                type="button"
              >
                {deleteHistory.pending ? messages.working : messages.deleteHistory}
              </button>
              <output aria-live="polite">
                {exportHistory.error ??
                  deleteHistory.error ??
                  exportHistory.message ??
                  deleteHistory.message}
              </output>
            </div>
          )}
        </fieldset>
      </section>
    </>
  );
}
