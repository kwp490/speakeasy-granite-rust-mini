import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import {
  formatEngineReason,
  formatError,
  formatFinalSourceGuidance,
  formatFinalSourceReason,
  formatProviderIntegrity,
  formatState,
} from "./format";
import { readWithRetry } from "./readWithRetry";
import { awaitEngineReady } from "./engineReady";
import { SettingExpander, SettingGroup, SettingRow, StatusText } from "./Rows";
import { useMutation } from "./useMutation";
import type {
  DiagnosticsStatus,
  GpuStatus,
  ModelCatalogItem,
  ModelInstallStatus,
  PersonalizationImportPreview,
  PersonalizationStatus,
  RecoverableResult,
} from "./types";

/**
 * Transcription: why the last dictation failed, where the engine runs, the
 * speech model, and vocabulary.
 *
 * The model is read, never installed, here. Setup provisions it and verifies
 * it before the app opens; exact provenance lives in Advanced → Technical
 * details.
 */
/**
 * How long to keep re-reading the engine row while the worker warms.
 *
 * 30 x 1 s. A cold Granite load measured 2-5 s on this hardware, and the ceiling
 * is generous rather than tuned -- the poll stops as soon as the device is
 * reported, so the only run that reaches the ceiling is one where Granite never
 * warms, and that is a state to stop asking about rather than one to wait for.
 */
const ENGINE_WARM_READS = 30;
const ENGINE_WARM_READ_INTERVAL_MS = 1_000;

/**
 * The gap between the end of one model-status read and the start of the next.
 *
 * A gap rather than a period: the poll is self-scheduling, so at most one
 * request is ever outstanding.
 */
const MODEL_POLL_GAP_MS = 750;

export function Transcription() {
  const [models, setModels] = useState<ModelCatalogItem[]>([]);
  const [gpu, setGpu] = useState<GpuStatus | null>(null);
  const [lastFailure, setLastFailure] = useState<string | null>(null);
  const [result, setResult] = useState<RecoverableResult | null>(null);
  const [resultUnavailable, setResultUnavailable] = useState(false);
  const [retryAction, setRetryAction] = useState("");
  const [modelStatus, setModelStatus] = useState<ModelInstallStatus>({
    state: "verifying",
    error: null,
  });
  const [personalization, setPersonalization] = useState<PersonalizationStatus | null>(null);
  /**
   * Set when the personalization read never succeeded. "Could not be read" and
   * "your list is empty" are different facts, and they looked identical here
   * until 2026-08-20.
   */
  const [personalizationUnavailable, setPersonalizationUnavailable] = useState(false);
  /**
   * How many times the engine row has been re-read while the worker was still
   * coming up. The device and the provider-integrity line are both
   * `not_configured` until the launch warm has spoken, seconds after this page
   * mounts; read once, the fault disclosure would never be rendered.
   */
  const [warmReads, setWarmReads] = useState(0);
  const [observedTerm, setObservedTerm] = useState("");
  const [correctedTerm, setCorrectedTerm] = useState("");
  const [snippetName, setSnippetName] = useState("");
  const [snippetBody, setSnippetBody] = useState("");
  const [personalizationJson, setPersonalizationJson] = useState("");
  const [personalizationPreview, setPersonalizationPreview] =
    useState<PersonalizationImportPreview | null>(null);
  const [personalizationAction, setPersonalizationAction] = useState("");
  /** The model-status poll stopped answering. See the poll's own comment. */
  const [pollUnavailable, setPollUnavailable] = useState(false);
  const switchProvider = useMutation<void>();
  // The export and the two destructive personalization commands. Each reports
  // its own refusal, and neither destructive one announces a deletion it has not
  // been told happened.
  const exportPersonalization = useMutation<string>();
  const personalizationWrite = useMutation<PersonalizationStatus>();

  useEffect(() => {
    void refreshCatalog();
    void invoke<ModelInstallStatus>("model_install_status")
      .then(setModelStatus)
      .catch(() => {
        setModelStatus({ state: "failed", error: "model_status_unavailable" });
      });
    // Retried: a read that lost the race against `setup` managing
    // `PersonalizationCoordinator` left this list empty for the life of the
    // process, which reads as "you have no vocabulary".
    void readWithRetry<PersonalizationStatus>("personalization_status").then(
      (status) => {
        setPersonalization(status);
        setPersonalizationUnavailable(false);
      },
      () => {
        setPersonalizationUnavailable(true);
      },
    );
    // Retried, because this read carries the failure banner: losing it hides
    // the reason a dictation produced nothing, which is the one thing this page
    // owes a user whose transcript vanished.
    void readWithRetry<DiagnosticsStatus>("diagnostics_status").then(
      (status) => setLastFailure(status.final_source_reason),
      () => {
        // Diagnostics being unavailable is not itself a dictation failure, and
        // reporting it as one here would invent a problem. Advanced is where an
        // unreadable diagnostics surface shows up.
      },
    );
    // Whether the failed dictation's audio is still retained, so Try again can
    // be offered. `result_status` stands behind two coordinators, so it is
    // retried too; a lost race used to disable the one control that could have
    // recovered the audio.
    void readWithRetry<RecoverableResult>("result_status").then(
      (status) => {
        setResult(status);
        setResultUnavailable(false);
      },
      () => {
        setResultUnavailable(true);
      },
    );
  }, []);

  useEffect(() => {
    if (
      modelStatus.state !== "verifying" &&
      modelStatus.state !== "downloading" &&
      modelStatus.state !== "installing"
    ) {
      // A finished verification changes which engine resolves, and neither the
      // catalog nor the engine row is re-read by the poll itself.
      if (modelStatus.state === "verified_on_disk") {
        void refreshCatalog();
      }
      return;
    }
    // Self-scheduling, so 750 ms is the gap between the end of one read and the
    // start of the next; this read reaches the model coordinator's lock.
    //
    // The rejection handler does **not** set `state: "failed"`: a poll that
    // could not be read says nothing about the model, so it reports that the
    // *status* is unreadable and clears itself as soon as one arrives.
    let stopped = false;
    let timer = 0;
    const schedule = () => {
      if (stopped) return;
      timer = window.setTimeout(poll, MODEL_POLL_GAP_MS);
    };
    const poll = () => {
      invoke<ModelInstallStatus>("model_install_status")
        .then(
          (status) => {
            if (stopped) return;
            setModelStatus(status);
            setPollUnavailable(false);
          },
          () => {
            if (stopped) return;
            setPollUnavailable(true);
          },
        )
        .finally(schedule);
    };
    schedule();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, [modelStatus.state]);

  useEffect(() => {
    if (gpu === null || gpu.active_device !== "not_configured") return;
    if (warmReads >= ENGINE_WARM_READS) return;
    const timer = window.setTimeout(() => {
      setWarmReads((reads) => reads + 1);
      void refreshCatalog();
    }, ENGINE_WARM_READ_INTERVAL_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [gpu, warmReads]);

  /**
   * Re-reads the catalog *and* the engine row together, both through the retry:
   * this is called from mount, where a refusal would otherwise put "no model"
   * on screen about a machine with the weights on disk.
   */
  async function refreshCatalog() {
    try {
      setModels(await readWithRetry<ModelCatalogItem[]>("model_catalog"));
      setGpu(await readWithRetry<GpuStatus>("gpu_status"));
    } catch (error) {
      // Leaves whatever was last read on screen rather than blanking the row.
      setModelStatus({ state: "failed", error: String(error) });
    }
  }

  async function retryTranscription() {
    setRetryAction(messages.retryStarted);
    try {
      await invoke("dictation_retry");
      setResult(await readWithRetry<RecoverableResult>("result_status"));
      setRetryAction("");
    } catch {
      // The failure is reported before the status re-read, which can fail too;
      // a rejection there must not swallow the message the user needs.
      setRetryAction(messages.retryFailed);
      try {
        setResult(await readWithRetry<RecoverableResult>("result_status"));
      } catch {
        // The banner keeps what it last read.
      }
    }
  }

  async function recordCorrection() {
    try {
      setPersonalization(
        await invoke<PersonalizationStatus>("correction_record", {
          id: `correction-${Date.now()}`,
          locale: "en-US",
          observed: observedTerm,
          corrected: correctedTerm,
        }),
      );
      setObservedTerm("");
      setCorrectedTerm("");
      setPersonalizationAction(messages.personalizationSaved);
    } catch {
      setPersonalizationAction(messages.personalizationRejected);
    }
  }

  async function saveSnippet() {
    try {
      setPersonalization(
        await invoke<PersonalizationStatus>("snippet_save", {
          id: `snippet-${snippetName}`,
          name: snippetName,
          body: snippetBody,
        }),
      );
      setSnippetName("");
      setSnippetBody("");
      setPersonalizationAction(messages.personalizationSaved);
    } catch {
      setPersonalizationAction(messages.personalizationRejected);
    }
  }

  async function deletePersonalization(kind: "dictionary" | "snippet", id: string) {
    const next = await personalizationWrite.run(
      () => invoke<PersonalizationStatus>("personalization_delete", { kind, id }),
      () => messages.deleted,
    );
    if (next !== null) setPersonalization(next);
  }

  async function previewPersonalizationImport() {
    try {
      setPersonalizationPreview(
        await invoke<PersonalizationImportPreview>("personalization_import_preview", {
          json: personalizationJson,
        }),
      );
      setPersonalizationAction("");
    } catch {
      setPersonalizationPreview(null);
      setPersonalizationAction(messages.personalizationRejected);
    }
  }

  async function commitPersonalizationImport() {
    if (personalizationPreview === null) return;
    try {
      setPersonalization(
        await invoke<PersonalizationStatus>("personalization_import_commit", {
          fingerprint: personalizationPreview.fingerprint_sha256,
          policy: "keep_existing",
        }),
      );
      setPersonalizationPreview(null);
      setPersonalizationJson("");
      setPersonalizationAction(messages.personalizationSaved);
    } catch {
      setPersonalizationAction(messages.personalizationRejected);
    }
  }

  async function resetPersonalization() {
    const next = await personalizationWrite.run(
      () => invoke<PersonalizationStatus>("personalization_reset", { confirmed: true }),
      () => messages.deleted,
    );
    // "Deleted" only if it was. A refused reset must not look identical to one
    // that emptied the vocabulary.
    if (next === null) return;
    setPersonalization(next);
    setPersonalizationPreview(null);
    setPersonalizationAction("");
  }

  const installed = models.find((model) => model.installed);
  const onGraphicsCard =
    gpu?.active_device === "cuda" || gpu?.active_device === "cuda_unverified";
  const showFailure =
    lastFailure !== null || result?.error_code != null || result?.retry_available === true;
  const vocabularyMessage =
    exportPersonalization.error ??
    personalizationWrite.error ??
    exportPersonalization.message ??
    personalizationWrite.message ??
    personalizationAction;

  return (
    <>
      {/* The last failure, first on the page. With one engine and no fallback,
          a dictation that went wrong produced no text at all, so the user
          arriving here has already lost something and is looking for why.
          Absent entirely when the last dictation succeeded: an empty "no
          problems" panel is a permanent invitation to worry. */}
      {showFailure && (
        <div className="infobar" data-tone="bad" data-testid="transcription-status">
          <span aria-hidden="true" className="infobar-icon">
            !
          </span>
          <div className="infobar-body">
            <strong>{messages.lastDictationFailed}</strong>
            {lastFailure !== null && (
              <>
                <span role="alert">{formatFinalSourceReason(lastFailure)}</span>
                <span className="infobar-muted">{formatFinalSourceGuidance(lastFailure)}</span>
              </>
            )}
            {result?.error_code != null && (
              <span role="alert">
                {messages.resultFailed} {formatError(result.error_code)}
              </span>
            )}
            <output aria-live="polite">{retryAction}</output>
          </div>
          {result?.retry_available === true && (
            <button onClick={() => void retryTranscription()} type="button">
              {messages.retryTranscription}
            </button>
          )}
        </div>
      )}
      {resultUnavailable && <p className="warning">{messages.resultStatusUnavailable}</p>}

      <SettingGroup label={messages.engineGroup}>
        {/* The **device**, not the pack. The pack's own provider reads `cpu` on
            a machine whose graphics-card worker offloads that same GGUF, so it
            is not sent and cannot be rendered here. */}
        <SettingRow
          control={
            <>
              <span className="setting-row-value" data-testid="engine-disclosure">
                <bdi>
                  {gpu === null
                    ? formatState("unknown")
                    : gpu.pack_installed
                      ? formatState(gpu.active_device)
                      : messages.engineNone}
                </bdi>
              </span>
              {/* Only on an install that staged both workers. A processor-only
                  install has nothing to switch to, and a control that can never
                  do anything is not shown. Success is the engine reporting
                  `ready`, never the command returning. */}
              {gpu?.alternate_provider_available === true && (
                <button
                  disabled={switchProvider.pending}
                  onClick={() => {
                    const target = onGraphicsCard ? "cpu" : "cuda";
                    void switchProvider.run(
                      async () => {
                        await invoke("runtime_switch_engine_provider", { provider: target });
                        const engine = await awaitEngineReady();
                        if (engine !== "ready") throw engine;
                      },
                      () => messages.engineProviderSwitched,
                    );
                  }}
                  type="button"
                >
                  {switchProvider.pending
                    ? messages.engineProviderSwitching
                    : onGraphicsCard
                      ? messages.switchToCpu
                      : messages.switchToGpu}
                </button>
              )}
            </>
          }
          detail={
            gpu === null ? undefined : (
              // Its own element, never joined to the device: a reason about the
              // installation and a device are two facts that disagree on any
              // machine running a graphics-card worker against the single
              // processor-named pack.
              <span data-testid="engine-reason">{formatEngineReason(gpu.engine_reason)}</span>
            )
          }
          title={messages.engineDisclosure}
        >
          {/* Shown only when it says something. `ok` and `unrecorded` are the
              quiet answers and have no copy. */}
          {gpu !== null && formatProviderIntegrity(gpu.provider_integrity) !== null && (
            <p
              className={gpu.provider_fault ? "warning" : "row-note"}
              data-testid="provider-integrity"
            >
              {formatProviderIntegrity(gpu.provider_integrity)}
            </p>
          )}
          {(switchProvider.error ?? switchProvider.message) !== null && (
            <output aria-live="polite" className="row-note">
              {switchProvider.error ?? switchProvider.message}
            </output>
          )}
        </SettingRow>
        <SettingRow
          control={
            <span className="setting-row-value">
              <bdi>{installed?.display_name ?? formatState("absent")}</bdi>
            </span>
          }
          detail={
            <StatusText
              testId="model-state"
              tone={modelStatus.state === "verified_on_disk" ? "ok" : modelStatus.state === "failed" ? "bad" : "neutral"}
            >
              {formatState(modelStatus.state)}
            </StatusText>
          }
          title={messages.speechModel}
        >
          {modelStatus.error !== null && (
            <p className="row-note" role="alert">
              {messages.modelCheckFailed} {formatError(modelStatus.error)}
            </p>
          )}
          {pollUnavailable && <p className="row-note warning">{messages.modelStatusPollUnavailable}</p>}
        </SettingRow>
        <SettingRow
          control={<span className="setting-row-value">{messages.languageValue}</span>}
          title={messages.languageSection}
        />
      </SettingGroup>

      <SettingGroup label={messages.vocabularyGroup}>
        <SettingExpander
          detail={messages.hotwordLimitation}
          title={messages.wordCorrections}
          value={personalization?.dictionary.length ?? "—"}
        >
          {personalizationUnavailable && (
            <p className="warning">{messages.personalizationUnavailable}</p>
          )}
          <table className="pairs">
            <thead>
              <tr>
                <th scope="col">{messages.correctionObserved}</th>
                <th scope="col">{messages.correctionCorrected}</th>
                <th scope="col">
                  <span className="sr-only">{messages.delete}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {personalization?.dictionary.map((entry) => (
                <tr key={entry.id}>
                  <td>
                    <bdi>{entry.source}</bdi>
                  </td>
                  <td>
                    <bdi>{entry.replacement}</bdi>
                  </td>
                  <td>
                    <button
                      className="link destructive"
                      onClick={() => void deletePersonalization("dictionary", entry.id)}
                      type="button"
                    >
                      {messages.delete}
                    </button>
                  </td>
                </tr>
              ))}
              <tr>
                <td>
                  <input
                    aria-label={messages.correctionObserved}
                    onChange={(event) => setObservedTerm(event.target.value)}
                    value={observedTerm}
                  />
                </td>
                <td>
                  <input
                    aria-label={messages.correctionCorrected}
                    onChange={(event) => setCorrectedTerm(event.target.value)}
                    value={correctedTerm}
                  />
                </td>
                <td>
                  <button
                    disabled={observedTerm === "" || correctedTerm === ""}
                    onClick={() => void recordCorrection()}
                    type="button"
                  >
                    {messages.recordCorrection}
                  </button>
                </td>
              </tr>
            </tbody>
          </table>
          <p className="row-note">{messages.contactsDisabled}</p>
        </SettingExpander>

        <SettingExpander
          detail={messages.snippetGrammar}
          title={messages.snippets}
          value={personalization?.snippets.length ?? "—"}
        >
          <ul className="plain-list">
            {personalization?.snippets.map((snippet) => (
              <li className="snippet" key={snippet.id}>
                <div className="snippet-text">
                  <strong>
                    <bdi>{snippet.name}</bdi>
                  </strong>
                  <pre>{snippet.body}</pre>
                </div>
                <button
                  className="link destructive"
                  onClick={() => void deletePersonalization("snippet", snippet.id)}
                  type="button"
                >
                  {messages.delete}
                </button>
              </li>
            ))}
          </ul>
          <div className="stacked-fields">
            <label>
              <span>{messages.snippetName}</span>
              <input onChange={(event) => setSnippetName(event.target.value)} value={snippetName} />
            </label>
            <label>
              <span>{messages.snippetBody}</span>
              <textarea
                onChange={(event) => setSnippetBody(event.target.value)}
                value={snippetBody}
              />
            </label>
            <button
              disabled={snippetName === "" || snippetBody === ""}
              onClick={() => void saveSnippet()}
              type="button"
            >
              {messages.saveSnippet}
            </button>
          </div>
        </SettingExpander>

        <SettingExpander detail={messages.vocabularyBackupDetail} title={messages.vocabularyBackup}>
          <div className="stacked-fields">
            <label>
              <span>{messages.personalizationJson}</span>
              <textarea
                onChange={(event) => setPersonalizationJson(event.target.value)}
                value={personalizationJson}
              />
            </label>
            {personalizationPreview !== null && (
              <p>
                {messages.personalizationImportSummary(
                  personalizationPreview.dictionary_count,
                  personalizationPreview.snippet_count,
                  personalizationPreview.conflicts,
                )}
              </p>
            )}
            <div className="actions">
              <button
                disabled={personalizationJson === ""}
                onClick={() => void previewPersonalizationImport()}
                type="button"
              >
                {messages.previewPersonalizationImport}
              </button>
              <button
                disabled={personalizationPreview === null}
                onClick={() => void commitPersonalizationImport()}
                type="button"
              >
                {messages.commitPersonalizationImport}
              </button>
              <button
                disabled={exportPersonalization.pending}
                onClick={() => {
                  void exportPersonalization.run(
                    () => invoke<string>("personalization_export"),
                    (fileName) => fileName,
                  );
                }}
                type="button"
              >
                {exportPersonalization.pending ? messages.working : messages.exportPersonalization}
              </button>
              <button
                className="destructive"
                disabled={personalizationWrite.pending}
                onClick={() => void resetPersonalization()}
                type="button"
              >
                {personalizationWrite.pending ? messages.working : messages.resetPersonalization}
              </button>
            </div>
          </div>
        </SettingExpander>
      </SettingGroup>
      {vocabularyMessage !== "" && (
        <output aria-live="polite" className="page-note">
          {vocabularyMessage}
        </output>
      )}
    </>
  );
}
