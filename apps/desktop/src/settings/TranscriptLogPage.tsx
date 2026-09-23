import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import { formatError } from "./format";
import { SettingGroup, SettingRow, Switch } from "./Rows";
import { TranscriptLog } from "./TranscriptLog";
import type { ProfileController } from "./useProfile";
import { useMutation } from "./useMutation";

/** The retention periods offered. A stored value outside them is shown too. */
const RETENTION_DAYS = [7, 30, 90, 365];

/**
 * History: every finished transcript with Copy, and what happens to them.
 *
 * This is the only place a finished transcript can be read back, so a
 * transcript that missed its target is recoverable here or nowhere.
 *
 * **Pin** detaches the list into its own small always-on-top window. That
 * window is declared in `tauri.conf.json` and only ever shown or hidden --
 * never built on demand, which deadlocks the app's IPC -- and it is
 * non-focusable, because anything SpeakEasy puts in the foreground becomes the
 * delivery target for the next dictation.
 *
 * **Saved history** is off by default, and off means the transcripts were never
 * written to disk rather than deleted on the way out. Turning it on writes
 * plain text to disk, so it asks first: the switch opens a confirmation that
 * names the data, the place and the period, and only the confirmation writes.
 */
export function TranscriptLogPage({ profile }: { profile: ProfileController }) {
  const exportHistory = useMutation<string>();
  const deleteHistory = useMutation<void>();
  const [consentOpen, setConsentOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [pinAction, setPinAction] = useState("");
  const consentButton = useRef<HTMLButtonElement>(null);

  const stored = profile.profile;
  const enabled = stored?.history_enabled ?? false;
  const retentionDays = stored?.history_retention_days ?? 30;
  const retentionOptions = RETENTION_DAYS.includes(retentionDays)
    ? RETENTION_DAYS
    : [...RETENTION_DAYS, retentionDays].sort((a, b) => a - b);

  useEffect(() => {
    if (consentOpen) consentButton.current?.focus();
  }, [consentOpen]);

  async function pin() {
    setPinAction("");
    try {
      await invoke("transcript_log_pin");
      setPinAction(messages.transcriptLogPinned);
    } catch (error: unknown) {
      setPinAction(formatError(String(error)));
    }
  }

  function setHistory(next: { enabled: boolean; retentionDays: number; accepted?: boolean }) {
    void profile.setHistory({
      enabled: next.enabled,
      retentionDays: next.retentionDays,
      disclosureAccepted:
        next.accepted ?? stored?.history_plaintext_disclosure_accepted ?? false,
    });
  }

  return (
    <>
      <div className="page-actions">
        <button onClick={() => void pin()} type="button">
          {messages.transcriptLogPin}
        </button>
        <output aria-live="polite">{pinAction}</output>
      </div>

      <TranscriptLog />

      <SettingGroup label={messages.savedHistoryGroup}>
        <SettingRow
          control={
            <Switch
              checked={enabled}
              describedBy="history-keep-detail"
              disabled={stored === null}
              label={messages.historyKeep}
              onChange={(next) => {
                if (next) setConsentOpen(true);
                else setHistory({ enabled: false, retentionDays });
              }}
            />
          }
          detail={messages.historyKeepDetail}
          detailId="history-keep-detail"
          title={messages.historyKeep}
        >
          {consentOpen && (
            <div
              aria-labelledby="history-consent-title"
              className="confirm-panel"
              role="group"
            >
              <strong id="history-consent-title">{messages.historyConsentTitle}</strong>
              <p>{messages.historyDisclosure(retentionDays)}</p>
              <div className="actions">
                <button
                  className="primary"
                  onClick={() => {
                    setConsentOpen(false);
                    setHistory({ enabled: true, retentionDays, accepted: true });
                  }}
                  ref={consentButton}
                  type="button"
                >
                  {messages.historyConsentConfirm}
                </button>
                <button onClick={() => setConsentOpen(false)} type="button">
                  {messages.cancel}
                </button>
              </div>
            </div>
          )}
        </SettingRow>
        <SettingRow
          control={
            <select
              disabled={!enabled}
              id="history-retention"
              onChange={(event) =>
                setHistory({ enabled, retentionDays: Number(event.target.value) })
              }
              value={retentionDays}
            >
              {retentionOptions.map((days) => (
                <option key={days} value={days}>
                  {messages.historyRetentionDays(days)}
                </option>
              ))}
            </select>
          }
          labelFor="history-retention"
          title={messages.historyRetention}
        />
        <SettingRow
          control={
            <>
              <button
                disabled={!enabled || exportHistory.pending}
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
              <button
                className="destructive"
                disabled={!enabled || confirmDelete}
                onClick={() => {
                  deleteHistory.reset();
                  setConfirmDelete(true);
                }}
                type="button"
              >
                {messages.deleteHistory}
              </button>
            </>
          }
          detail={messages.historyStoredDetail}
          title={messages.historyStored}
        >
          {confirmDelete && (
            <div
              aria-labelledby="history-delete-title"
              className="confirm-panel"
              data-tone="bad"
              role="group"
            >
              <strong id="history-delete-title">{messages.deleteHistoryConfirmTitle}</strong>
              <p>{messages.deleteHistoryConfirmDetail}</p>
              <div className="actions">
                <button
                  className="destructive"
                  disabled={deleteHistory.pending}
                  onClick={() => {
                    void deleteHistory
                      .run(
                        () => invoke("history_delete_all", { confirmed: true }),
                        () => messages.deleted,
                      )
                      .then((deleted) => {
                        // Closed only if the deletion actually happened. A
                        // refusal keeps the confirmation open beside its reason,
                        // so it cannot look like a completed deletion.
                        if (deleted !== null) setConfirmDelete(false);
                      });
                  }}
                  type="button"
                >
                  {deleteHistory.pending ? messages.working : messages.deleteHistoryNow}
                </button>
                <button
                  disabled={deleteHistory.pending}
                  onClick={() => setConfirmDelete(false)}
                  type="button"
                >
                  {messages.cancel}
                </button>
              </div>
            </div>
          )}
          {(exportHistory.error ??
            deleteHistory.error ??
            exportHistory.message ??
            deleteHistory.message) !== null && (
            <output aria-live="polite" className="row-note">
              {exportHistory.error ??
                deleteHistory.error ??
                exportHistory.message ??
                deleteHistory.message}
            </output>
          )}
        </SettingRow>
      </SettingGroup>
    </>
  );
}
