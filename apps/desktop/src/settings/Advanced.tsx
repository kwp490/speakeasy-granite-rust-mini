import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import { displayName, formatResetCategory } from "./format";
import { readWithRetry } from "./readWithRetry";
import { awaitEngineReady } from "./engineReady";
import { SettingExpander, SettingGroup, SettingRow, StatusText, Switch } from "./Rows";
import type { DiagnosticsExport, DiagnosticsStatus, ProfileStatus, ResetPreview } from "./types";
import type { ProfileController } from "./useProfile";
import { useMutation } from "./useMutation";

/**
 * Advanced: engine status and restart, diagnostics, reset and quit.
 *
 * The summary rows use display names ("Processor (CPU)"). **Technical details**
 * holds the untranslated identifiers, because those are what the diagnostic log
 * and an exported bundle contain, and a user comparing the two needs the same
 * strings (UI-GUIDE "Two vocabulary registers").
 *
 * `Not measured` is neutral here, not an error: nothing on this host has been
 * qualified, and saying so plainly is the whole discipline.
 */
export function Advanced({ profile }: { profile: ProfileController }) {
  const [diagnostics, setDiagnostics] = useState<DiagnosticsStatus | null>(null);
  const [statusUnavailable, setStatusUnavailable] = useState(false);
  const exportDiagnostics = useMutation<DiagnosticsExport>();
  const previewReset = useMutation<ResetPreview>();
  // `reset_commit` is the destructive half of the pair and needs the visible
  // failure state more than the preview does: a refusal that says nothing reads
  // as a button that does nothing rather than as a reset that did not happen.
  const resetCommit = useMutation<ProfileStatus>();
  const [resetPreview, setResetPreview] = useState<ResetPreview | null>(null);
  const restartEngine = useMutation<void>();

  // Retried, with a rejection handler: six coordinators stand behind
  // `diagnostics_status`, so this is the read most exposed to the startup race,
  // and a lost one must say so rather than leave the page empty.
  useEffect(() => {
    void readWithRetry<DiagnosticsStatus>("diagnostics_status").then(
      (status) => {
        setDiagnostics(status);
        setStatusUnavailable(false);
      },
      () => {
        setStatusUnavailable(true);
      },
    );
  }, []);

  async function commitReset() {
    if (resetPreview === null) return;
    const nonce = resetPreview.nonce;
    const next = await resetCommit.run(() => invoke<ProfileStatus>("reset_commit", { nonce }));
    // The panel closes only if the reset happened. Closing it either way would
    // make a refusal look exactly like a success.
    if (next !== null) {
      profile.replace(next);
      setResetPreview(null);
    }
  }

  const measured = (value: number | null, suffix = "") =>
    value === null ? messages.noMeasuredValue : `${value}${suffix}`;
  const seconds = (ms: number | null) => (ms === null ? null : `${(ms / 1000).toFixed(1)} s`);
  const typical = seconds(diagnostics?.latency_p50_ms ?? null);
  const slowest = seconds(diagnostics?.latency_p95_ms ?? null);

  return (
    <>
      {/* An empty page is not neutral here. It exists to answer "what is it
          running on", so nothing under the heading would read as "nothing is
          running" rather than as "the read did not arrive". */}
      {statusUnavailable && <p className="warning">{messages.runtimeStatusUnavailable}</p>}

      <SettingGroup label={messages.engineGroup}>
        <SettingRow
          control={
            <>
              <button
                disabled={restartEngine.pending}
                onClick={() => {
                  void restartEngine.run(
                    async () => {
                      await invoke("runtime_recover");
                      // The command starts the warm and returns; it cannot hold
                      // an IPC call for a 2 GB load. Success is the engine
                      // reporting `ready`, never this command returning.
                      const engine = await awaitEngineReady();
                      // The bare code, because that is what `invoke` rejects
                      // with and what `formatError` maps to catalog prose.
                      if (engine !== "ready") throw engine;
                    },
                    () => messages.engineRestarted,
                  );
                }}
                type="button"
              >
                {restartEngine.pending ? messages.engineRestarting : messages.restartEngine}
              </button>
              <output aria-live="polite" className="sr-only">
                {restartEngine.error ?? restartEngine.message}
              </output>
            </>
          }
          detail={
            diagnostics === null ? undefined : (
              <StatusText tone="neutral">
                <bdi>{displayName(diagnostics.device)}</bdi>
              </StatusText>
            )
          }
          title={messages.engineStatus}
        >
          {(restartEngine.error ?? restartEngine.message) !== null && (
            <p aria-hidden="true" className="row-note">
              {restartEngine.error ?? restartEngine.message}
            </p>
          )}
        </SettingRow>
        <SettingRow
          control={
            <span className="setting-row-value">
              {typical === null || slowest === null
                ? messages.noMeasuredValue
                : messages.speedValue(typical, slowest)}
            </span>
          }
          detail={messages.speedDetail}
          title={messages.speed}
        />
      </SettingGroup>

      <SettingGroup label={messages.diagnosticsGroup}>
        <SettingRow
          control={
            <Switch
              checked={profile.profile?.disk_logging_enabled ?? true}
              describedBy="advanced-logging-detail"
              label={messages.diagnosticLogging}
              onChange={(next) => void profile.setDiskLogging(next)}
            />
          }
          detail={messages.diagnosticLoggingDetail}
          detailId="advanced-logging-detail"
          title={messages.diagnosticLogging}
        />
        <SettingRow
          control={
            <button
              disabled={exportDiagnostics.pending}
              onClick={() => {
                void exportDiagnostics.run(
                  () => invoke<DiagnosticsExport>("diagnostics_export"),
                  (exported) => `${messages.diagnosticsExported} ${exported.file_name}`,
                );
              }}
              type="button"
            >
              {exportDiagnostics.pending ? messages.working : messages.exportDiagnosticsButton}
            </button>
          }
          title={messages.exportDiagnostics}
        >
          {(exportDiagnostics.error ?? exportDiagnostics.message) !== null && (
            <output aria-live="polite" className="row-note">
              {exportDiagnostics.error ?? exportDiagnostics.message}
            </output>
          )}
        </SettingRow>
        <SettingExpander detail={messages.technicalDetailsHint} title={messages.technicalDetails}>
          {diagnostics === null ? (
            <p className="row-note">{messages.runtimeStatusUnavailable}</p>
          ) : (
            <dl className="raw-values" data-testid="raw-values">
              {(
                [
                  [messages.engine, diagnostics.engine],
                  [messages.worker, diagnostics.worker],
                  [messages.runtime, diagnostics.runtime],
                  [messages.provider, diagnostics.provider],
                  [messages.deviceStatus, diagnostics.device],
                  [messages.vad, diagnostics.vad],
                  [messages.deliveryCapability, diagnostics.delivery_capability],
                  [messages.deliveryReason, diagnostics.delivery_reason],
                  [messages.finalSource, diagnostics.final_source_reason ?? messages.noMeasuredValue],
                  [
                    messages.modelProvenance,
                    `${diagnostics.model_id}@${diagnostics.model_revision} · ${diagnostics.model_source}`,
                  ],
                  [messages.performance, measured(diagnostics.rtf_median)],
                  [messages.rtfP95, measured(diagnostics.rtf_p95)],
                  [messages.latencyP50, measured(diagnostics.latency_p50_ms, messages.millisecondSuffix)],
                  [messages.latencyP95, measured(diagnostics.latency_p95_ms, messages.millisecondSuffix)],
                  [messages.audioOverflow, String(diagnostics.audio_overflow_count)],
                  [messages.sanitizedLogs, diagnostics.logs_sanitized ? messages.yes : messages.no],
                ] as const
              ).map(([term, value]) => (
                <div key={term}>
                  <dt>{term}</dt>
                  <dd className="exact-value" title={value}>
                    <bdi>{value}</bdi>
                  </dd>
                </div>
              ))}
            </dl>
          )}
        </SettingExpander>
      </SettingGroup>

      <SettingGroup label={messages.resetQuitGroup}>
        <SettingRow
          control={
            resetPreview === null ? (
              <button
                className="destructive"
                disabled={previewReset.pending}
                onClick={() => {
                  void previewReset
                    .run(() => invoke<ResetPreview>("reset_preview"))
                    .then((preview) => {
                      // Only on success. A refused preview must not open the
                      // destructive panel behind it.
                      if (preview !== null) setResetPreview(preview);
                    });
                }}
                type="button"
              >
                {previewReset.pending ? messages.working : messages.previewReset}
              </button>
            ) : undefined
          }
          detail={messages.resetExclusions}
          title={messages.resetSettings}
        >
          {previewReset.error !== null && (
            <output aria-live="polite" className="row-note">
              {previewReset.error}
            </output>
          )}
          {resetPreview !== null && (
            <div className="confirm-panel" data-tone="bad" role="group" aria-label={messages.resetSettings}>
              <p>
                {messages.resetPreviewLead}{" "}
                {resetPreview.categories.map(formatResetCategory).join(", ")}.
              </p>
              <div className="actions">
                <button
                  className="destructive"
                  disabled={resetCommit.pending}
                  onClick={() => void commitReset()}
                  type="button"
                >
                  {resetCommit.pending ? messages.working : messages.resetNow}
                </button>
                <button
                  disabled={resetCommit.pending}
                  onClick={() => {
                    resetCommit.reset();
                    setResetPreview(null);
                  }}
                  type="button"
                >
                  {messages.cancel}
                </button>
              </div>
              <output aria-live="polite">{resetCommit.error}</output>
            </div>
          )}
        </SettingRow>
        {/* The dock never takes keyboard focus, so it is not keyboard operable;
            every action it offers needs a path that is (UI-GUIDE "Accessibility
            and input"). The shortcut covers start and stop, the Microphone page
            covers the microphone, and this covers quitting. */}
        <SettingRow
          control={
            <button
              onClick={() => {
                void invoke("app_quit");
              }}
              type="button"
            >
              {messages.quitAppButton}
            </button>
          }
          detail={messages.quitAppDetail}
          title={messages.quitApp}
        />
      </SettingGroup>

      <p className="page-footer">
        {messages.settingsProductName} {messages.version.replace(/^v/i, "")} · {messages.aboutDetail}
      </p>
    </>
  );
}
