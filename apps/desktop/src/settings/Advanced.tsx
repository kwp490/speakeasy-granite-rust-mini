import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { Disclosure } from "../components/Disclosure";
import { messages } from "../catalog";
import { displayName, formatCredentialStatus, formatResetCategory } from "./format";
import type {
  CredentialStatus,
  DiagnosticsExport,
  DiagnosticsStatus,
  ResetPreview,
} from "./types";
import type { ProfileController } from "./useProfile";

/**
 * Advanced: runtime status, performance, credentials, maintenance, About.
 *
 * This is the one page that keeps the product-contract vocabulary (UI-GUIDE
 * "Two vocabulary registers"), and it carries two things the other five do not:
 *
 * - The **display-name translation**, so the summary reads
 *   "ONNX Runtime · CPU" rather than `onnxruntime_cpu`.
 * - A **Show raw values** disclosure holding the untranslated identifiers, because
 *   those are what the diagnostic log and an exported bundle actually contain, and
 *   a user comparing the two needs to see the same strings.
 *
 * `Not measured` is neutral here, not an error: nothing on this host has been
 * qualified, and saying so plainly is the whole discipline.
 */
export function Advanced({ profile }: { profile: ProfileController }) {
  const [diagnostics, setDiagnostics] = useState<DiagnosticsStatus | null>(null);
  const [credentials, setCredentials] = useState<CredentialStatus | null>(null);
  const [exportAction, setExportAction] = useState("");
  const [resetPreview, setResetPreview] = useState<ResetPreview | null>(null);
  const [engineAction, setEngineAction] = useState("");

  useEffect(() => {
    void invoke<DiagnosticsStatus>("diagnostics_status").then(setDiagnostics);
    void invoke<CredentialStatus>("credential_status").then(setCredentials);
  }, []);

  async function restartEngine() {
    setEngineAction("");
    try {
      await invoke("runtime_recover");
      setEngineAction(messages.engineRestarted);
    } catch {
      setEngineAction(messages.engineRestartFailed);
    }
  }

  async function commitReset() {
    if (resetPreview === null) return;
    profile.replace(await invoke("reset_commit", { nonce: resetPreview.nonce }));
    setResetPreview(null);
  }

  const measured = (value: number | null, suffix = "") =>
    value === null ? messages.noMeasuredValue : `${value}${suffix}`;

  return (
    <>
      <section aria-labelledby="advanced-runtime">
        <h3 id="advanced-runtime">{messages.runtimeSection}</h3>
        {diagnostics !== null && (
          <>
            <dl className="fact-grid">
              <div>
                <dt>{messages.engine}</dt>
                <dd>
                  <bdi>{displayName(diagnostics.engine)}</bdi>
                </dd>
              </div>
              <div>
                <dt>{messages.worker}</dt>
                <dd>
                  <bdi>{displayName(diagnostics.worker)}</bdi>
                </dd>
              </div>
              <div>
                <dt>{messages.runtime}</dt>
                <dd>
                  <bdi>{displayName(diagnostics.runtime)}</bdi>
                </dd>
              </div>
              <div>
                <dt>{messages.provider}</dt>
                <dd>
                  <bdi>{displayName(diagnostics.provider)}</bdi>
                </dd>
              </div>
              <div>
                <dt>{messages.deviceStatus}</dt>
                <dd>
                  <bdi>{displayName(diagnostics.device)}</bdi>
                </dd>
              </div>
              <div>
                <dt>{messages.vad}</dt>
                <dd>
                  <bdi>{displayName(diagnostics.vad)}</bdi>
                </dd>
              </div>
              <div>
                <dt>{messages.deliveryCapability}</dt>
                <dd>
                  <bdi>{displayName(diagnostics.delivery_capability)}</bdi>
                </dd>
              </div>
              <div>
                <dt>{messages.deliveryReason}</dt>
                <dd>
                  <bdi>{displayName(diagnostics.delivery_reason)}</bdi>
                </dd>
              </div>
              <div>
                <dt>{messages.sanitizedLogs}</dt>
                <dd>{diagnostics.logs_sanitized ? messages.yes : messages.no}</dd>
              </div>
            </dl>

            <Disclosure hint={messages.rawValuesHint} summary={messages.showRawValues}>
              <dl className="fact-grid" data-testid="raw-values">
                <div>
                  <dt>{messages.engine}</dt>
                  <dd className="exact-value">
                    <bdi>{diagnostics.engine}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.worker}</dt>
                  <dd className="exact-value">
                    <bdi>{diagnostics.worker}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.runtime}</dt>
                  <dd className="exact-value">
                    <bdi>{diagnostics.runtime}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.provider}</dt>
                  <dd className="exact-value">
                    <bdi>{diagnostics.provider}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.deviceStatus}</dt>
                  <dd className="exact-value">
                    <bdi>{diagnostics.device}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.vad}</dt>
                  <dd className="exact-value">
                    <bdi>{diagnostics.vad}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.deliveryCapability}</dt>
                  <dd className="exact-value">
                    <bdi>{diagnostics.delivery_capability}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.deliveryReason}</dt>
                  <dd className="exact-value">
                    <bdi>{diagnostics.delivery_reason}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.finalSource}</dt>
                  <dd className="exact-value">
                    <bdi>{diagnostics.final_source_reason ?? messages.noMeasuredValue}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.modelProvenance}</dt>
                  <dd className="exact-value">
                    <bdi>
                      {diagnostics.model_id}@{diagnostics.model_revision} · {diagnostics.model_source}
                    </bdi>
                  </dd>
                </div>
              </dl>
            </Disclosure>
          </>
        )}
      </section>

      <section aria-labelledby="advanced-performance">
        <h3 id="advanced-performance">{messages.performanceSection}</h3>
        {diagnostics !== null && (
          <dl className="fact-grid">
            <div>
              <dt>{messages.performance}</dt>
              <dd>{measured(diagnostics.rtf_median)}</dd>
            </div>
            <div>
              <dt>{messages.rtfP95}</dt>
              <dd>{measured(diagnostics.rtf_p95)}</dd>
            </div>
            <div>
              <dt>{messages.latencyP50}</dt>
              <dd>{measured(diagnostics.latency_p50_ms, messages.millisecondSuffix)}</dd>
            </div>
            <div>
              <dt>{messages.latencyP95}</dt>
              <dd>{measured(diagnostics.latency_p95_ms, messages.millisecondSuffix)}</dd>
            </div>
            <div>
              <dt>{messages.audioOverflow}</dt>
              <dd>{diagnostics.audio_overflow_count}</dd>
            </div>
          </dl>
        )}
      </section>

      <section aria-labelledby="advanced-credentials">
        <h3 id="advanced-credentials">{messages.credentialsSection}</h3>
        {credentials !== null && (
          <dl className="fact-grid">
            <div>
              <dt>{messages.legacyOpenAiCredential}</dt>
              <dd>{formatCredentialStatus(credentials.openai_legacy)}</dd>
            </div>
            <div>
              <dt>{messages.legacyRemoteCredential}</dt>
              <dd>{formatCredentialStatus(credentials.remote_legacy)}</dd>
            </div>
          </dl>
        )}
        <p className="setting-detail">{messages.credentialsNeverShown}</p>
      </section>

      <section aria-labelledby="advanced-maintenance">
        <h3 id="advanced-maintenance">{messages.maintenanceSection}</h3>
        <div className="actions">
          <button
            onClick={() => {
              void invoke<DiagnosticsExport>("diagnostics_export").then((exported) => {
                setExportAction(`${messages.diagnosticsExported} ${exported.file_name}`);
              });
            }}
            type="button"
          >
            {messages.exportDiagnostics}
          </button>
          <output aria-live="polite">{exportAction}</output>
        </div>
        <div className="actions">
          <button onClick={() => void restartEngine()} type="button">
            {messages.restartEngine}
          </button>
          <output aria-live="polite">{engineAction}</output>
        </div>
        <p className="setting-detail">{messages.resetExclusions}</p>
        {resetPreview === null ? (
          <button
            onClick={() => {
              void invoke<ResetPreview>("reset_preview").then(setResetPreview);
            }}
            type="button"
          >
            {messages.previewReset}
          </button>
        ) : (
          <div className="warning-panel">
            <p>{resetPreview.categories.map(formatResetCategory).join(", ")}</p>
            <div className="actions">
              <button className="destructive" onClick={() => void commitReset()} type="button">
                {messages.resetNow}
              </button>
              <button onClick={() => setResetPreview(null)} type="button">
                {messages.cancel}
              </button>
            </div>
          </div>
        )}
      </section>

      <section aria-labelledby="advanced-about">
        <h3 id="advanced-about">{messages.aboutSection}</h3>
        <p className="setting-detail">
            {messages.productName} {messages.version}
        </p>
        <p className="setting-detail">{messages.aboutDetail}</p>
      </section>
    </>
  );
}
