import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { Disclosure } from "../components/Disclosure";
import { messages } from "../catalog";
import {
  formatBytes,
  formatEngineReason,
  formatError,
  formatRuntimeComponent,
  formatState,
} from "./format";
import type {
  CudaRuntimeStatus,
  GpuStatus,
  ModelCatalogItem,
  ModelHardware,
  ModelInstallStatus,
  PersonalizationImportPreview,
  PersonalizationStatus,
} from "./types";

/**
 * Transcription (§9.3): language, the local model, and personalization.
 *
 * Package internals — sizes, source repository, revision, license, provider,
 * capabilities, hardware evidence — sit behind a collapsed **Technical details**
 * disclosure rather than in front of every user. None of them are removed: the
 * exact provenance of an installed model is a promise this product makes, it just
 * is not what someone came to this page to read.
 *
 * There is no Copy button on those values by design. Copying would need a command
 * that writes arbitrary frontend text to the clipboard, which is a broader
 * authority than anything the app grants today; the values are selectable
 * instead, and Ctrl+C is a native `WebView` operation that needs no permission.
 */
export function Transcription() {
  const [models, setModels] = useState<ModelCatalogItem[]>([]);
  const [hardware, setHardware] = useState<ModelHardware | null>(null);
  const [gpu, setGpu] = useState<GpuStatus | null>(null);
  const [modelStatus, setModelStatus] = useState<ModelInstallStatus>({
    state: "verifying",
    error: null,
  });
  const [confirmed, setConfirmed] = useState(false);
  const [runtime, setRuntime] = useState<CudaRuntimeStatus | null>(null);
  const [runtimeAttempts, setRuntimeAttempts] = useState(0);
  const [runtimeConfirmed, setRuntimeConfirmed] = useState(false);
  const [personalization, setPersonalization] = useState<PersonalizationStatus | null>(null);
  const [observedTerm, setObservedTerm] = useState("");
  const [correctedTerm, setCorrectedTerm] = useState("");
  const [snippetName, setSnippetName] = useState("");
  const [snippetBody, setSnippetBody] = useState("");
  const [personalizationJson, setPersonalizationJson] = useState("");
  const [personalizationPreview, setPersonalizationPreview] =
    useState<PersonalizationImportPreview | null>(null);
  const [personalizationAction, setPersonalizationAction] = useState("");

  useEffect(() => {
    void refreshCatalog();
    void invoke<ModelHardware>("model_hardware").then(setHardware);
    void invoke<ModelInstallStatus>("model_install_status")
      .then(setModelStatus)
      .catch(() => {
        setModelStatus({ state: "failed", error: "model_status_unavailable" });
      });
    void invoke<PersonalizationStatus>("personalization_status").then(setPersonalization);
  }, []);

  useEffect(() => {
    if (
      modelStatus.state !== "verifying" &&
      modelStatus.state !== "downloading" &&
      modelStatus.state !== "installing"
    ) {
      // A finished install changes which packs are on disk and therefore which
      // engine resolves, and neither is re-read by the poll itself.
      if (modelStatus.state === "verified_on_disk") {
        void refreshCatalog();
      }
      return;
    }
    const timer = window.setInterval(() => {
      void invoke<ModelInstallStatus>("model_install_status").then(setModelStatus);
    }, 750);
    return () => {
      window.clearInterval(timer);
    };
  }, [modelStatus.state]);

  const installing =
    modelStatus.state === "verifying" ||
    modelStatus.state === "downloading" ||
    modelStatus.state === "installing";

  const runtimeInstalling =
    runtime?.state === "downloading" || runtime?.state === "installing";

  /**
   * The runtime download is polled on its own timer, not folded into the model
   * poll, because the two are independent transfers with independent states.
   *
   * On completion it re-reads the catalog: installing the runtime changes which
   * engine resolves, so the disclosure above has to be re-read or it goes on
   * saying "this installation does not include graphics-card acceleration"
   * next to a runtime that is now installed.
   */
  /**
   * Reads the offer on a bounded retry rather than exactly once.
   *
   * Retrying is what makes "no offer shown" mean *no supported card* rather than
   * *asked a moment too early* — see the note in `readRuntime`. Bounded so a
   * machine that genuinely cannot answer is not polled forever, and matching the
   * bound the Audio page already uses for device enumeration, which exists for
   * the same reason.
   */
  useEffect(() => {
    if (runtime !== null || runtimeAttempts >= 20) {
      return;
    }
    const timer = window.setTimeout(() => {
      setRuntimeAttempts((attempts) => attempts + 1);
      void readRuntime();
    }, 250);
    return () => {
      window.clearTimeout(timer);
    };
  }, [runtime, runtimeAttempts]);

  useEffect(() => {
    if (!runtimeInstalling) {
      return;
    }
    const timer = window.setInterval(() => {
      void invoke<CudaRuntimeStatus>("cuda_runtime_status")
        .then((next) => {
          setRuntime(next);
          if (next.state === "installed") {
            void refreshCatalog();
          }
        })
        .catch(() => {
          /* A dropped poll leaves the last reading on screen. */
        });
    }, 750);
    return () => {
      window.clearInterval(timer);
    };
  }, [runtimeInstalling]);

  /**
   * Re-reads the catalog *and* the engine disclosure together.
   *
   * Both change when a pack is installed or removed: `installed` per row, and
   * which engine dictation resolves to. Reading them at the same moment keeps
   * the page from claiming a GPU engine next to a GPU pack it just deleted.
   */
  async function refreshCatalog() {
    try {
      setModels(await invoke<ModelCatalogItem[]>("model_catalog"));
      setGpu(await invoke<GpuStatus>("gpu_status"));
    } catch (error) {
      // Leaves whatever was last read on screen rather than blanking the page.
      // A stale row is recoverable; an empty model list reads as "no models
      // exist", which would be a lie told by an error path.
      setModelStatus({ state: "failed", error: String(error) });
    }
  }

  /**
   * A rejected `invoke` used to be an unhandled promise rejection: no `catch`,
   * and the caller was `onClick={() => void installModel(model)}`. Clicking
   * Install on a pack with no archive URL rejected with
   * `pack_is_not_downloadable` and the button appeared to do nothing at all.
   */
  async function installModel(model: ModelCatalogItem) {
    try {
      await invoke("model_install_start", {
        id: model.id,
        revision: model.revision,
        confirmed,
      });
      setModelStatus({ state: "downloading", error: null });
    } catch (error) {
      setModelStatus({ state: "failed", error: String(error) });
    }
  }

  async function readRuntime() {
    try {
      setRuntime(await invoke<CudaRuntimeStatus>("cuda_runtime_status"));
    } catch {
      // Deliberately leaves the previous reading alone rather than nulling it.
      //
      // Nulling here was a real defect, found on an installed build: this
      // command needs a coordinator Tauri's `setup` manages after several that
      // open files, the page fires its startup reads at once, and on the first
      // launch after an install this read lost that race. One transient failure
      // then hid a 2.97 GB offer permanently, because nothing asked again until
      // the window was reloaded. The retry effect above asks again.
      //
      // While it stays null nothing is rendered, which is the right fail-closed
      // behaviour: an offer this page cannot price — no size, no file count —
      // must not be shown at all.
    }
  }

  /**
   * Starts the runtime fetch. Same shape as `installModel`, including the
   * `catch`: without one a refusal such as `gpu_not_admissible` would be an
   * unhandled rejection and the button would appear to do nothing.
   */
  async function installRuntime() {
    try {
      await invoke("cuda_runtime_install_start", { confirmed: runtimeConfirmed });
      setRuntime((previous) =>
        previous === null ? previous : { ...previous, state: "downloading", error: null },
      );
    } catch (error) {
      setRuntime((previous) =>
        previous === null ? previous : { ...previous, state: "failed", error: String(error) },
      );
    }
  }

  async function retestGpu() {
    try {
      await invoke("gpu_retest");
      window.setTimeout(() => void refreshCatalog(), 1_000);
    } catch (error) {
      setModelStatus({ state: "failed", error: String(error) });
    }
  }

  async function removeModel(model: ModelCatalogItem) {
    try {
      await invoke("model_remove", { id: model.id, revision: model.revision });
      setModelStatus({ state: "absent", error: null });
    } catch (error) {
      setModelStatus({ state: "failed", error: String(error) });
    }
    await refreshCatalog();
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
    setPersonalization(await invoke<PersonalizationStatus>("personalization_delete", { kind, id }));
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
      setPersonalizationAction(messages.personalizationSaved);
    } catch {
      setPersonalizationAction(messages.personalizationRejected);
    }
  }

  async function resetPersonalization() {
    setPersonalization(
      await invoke<PersonalizationStatus>("personalization_reset", { confirmed: true }),
    );
    setPersonalizationPreview(null);
    setPersonalizationAction(messages.deleted);
  }

  return (
    <>
      <section aria-labelledby="transcription-language">
        <h3 id="transcription-language">{messages.languageSection}</h3>
        <p className="setting-detail">{messages.languageDetail}</p>
      </section>

      <section aria-labelledby="transcription-model">
        <div className="section-heading">
          <h3 id="transcription-model">{messages.modelSection}</h3>
          <output
            aria-label={messages.provisioning}
            aria-live="polite"
            data-state={modelStatus.state}
            data-testid="model-state"
          >
            {formatState(modelStatus.state)}
          </output>
        </div>
        {/* Which engine this machine landed on, and why. Users now land on
            different engines by hardware and by what they have installed, so
            the product says so once, here, rather than leaving them to infer
            it. `engine_reason` is the load-bearing half: "running on CPU"
            reads identically whether there is no GPU or there is a good one
            whose pack was never installed. */}
        {gpu !== null && (
          <>
            <p className="setting-detail" data-testid="engine-disclosure">
              {messages.engineDisclosure}{" "}
              <bdi>
                {gpu.active_provider === null
                  ? messages.engineNone
                  : formatState(gpu.active_provider)}
              </bdi>{" "}
              — {formatEngineReason(gpu.engine_reason)}
            </p>
            <article className="model-row" data-testid="gpu-controls">
              <p className="setting-detail">
                {gpu.qualified ? messages.gpuQualified : messages.gpuNotQualified}
              </p>
              {/* Auto / Use processor / Use graphics card used to sit here.
                  Granite's provider is not a preference: the GPU path exists
                  only where a CUDA-capable worker binary is installed, and no
                  setting can conjure one. A control offering a choice the
                  machine cannot honour reports a state the engine will not be
                  in, so what is left is the engine's own answer above and a
                  way to ask it again. */}
              <div className="actions">
                <button onClick={() => void retestGpu()} type="button">
                  {messages.gpuRetest}
                </button>
              </div>
            </article>
          </>
        )}
        {/* The graphics-card acceleration offer.

            Shown only when the probe admitted a card (`offered`), because
            fetching 2.97 GB of graphics-card libraries for a machine that cannot
            execute a single node on them is pure cost. Never started without
            `runtimeConfirmed`, and the size is on screen next to that checkbox
            rather than behind a disclosure — this is the largest download the
            app can initiate, so the confirmation and the number it applies to
            have to be visible at the same moment. */}
        {runtime !== null && runtime.offered && (
          <article className="model-row" data-testid="gpu-runtime-offer">
            <h4>
              <bdi>{messages.gpuRuntimeSection}</bdi>
            </h4>
            <dl className="fact-grid">
              <div>
                <dt>{messages.modelReadiness}</dt>
                <dd>{formatState(runtime.state)}</dd>
              </div>
              <div>
                <dt>{messages.downloadSize}</dt>
                <dd>{formatBytes(runtime.download_bytes)}</dd>
              </div>
              <div>
                <dt>{messages.installedSize}</dt>
                <dd>{formatBytes(runtime.installed_bytes)}</dd>
              </div>
            </dl>
            <p>
              {runtime.state === "installed"
                ? messages.gpuRuntimeInstalled
                : runtime.state === "partial"
                  ? messages.gpuRuntimePartial
                  : messages.gpuRuntimeAbsent}
            </p>
            <Disclosure hint={messages.technicalDetailsHint} summary={messages.technicalDetails}>
              <dl className="fact-grid">
                <div>
                  <dt>{messages.gpuRuntimeFiles}</dt>
                  <dd className="exact-value">{runtime.file_count}</dd>
                </div>
                {/* Which halves are already down. The two are fetched and
                    verified separately, so a resumed install can legitimately
                    have one and not the other — and neither alone can run. */}
                <div>
                  <dt>{messages.gpuRuntimeComponents}</dt>
                  <dd>
                    {runtime.installed_components.length === 0
                      ? messages.engineNone
                      : runtime.installed_components
                          .map((component) => formatRuntimeComponent(component))
                          .join(", ")}
                  </dd>
                </div>
              </dl>
            </Disclosure>
            {runtime.state !== "installed" && (
              <label className="confirmation">
                <input
                  checked={runtimeConfirmed}
                  onChange={(event) => setRuntimeConfirmed(event.target.checked)}
                  type="checkbox"
                />
                {messages.gpuRuntimeConfirm}
              </label>
            )}
            <div className="actions">
              <button
                disabled={
                  !runtimeConfirmed ||
                  runtimeInstalling ||
                  installing ||
                  runtime.state === "installed"
                }
                onClick={() => void installRuntime()}
                type="button"
              >
                {messages.gpuRuntimeInstall}
              </button>
              <button
                disabled={!runtimeInstalling}
                onClick={() => void invoke("cuda_runtime_install_cancel")}
                type="button"
              >
                {messages.cancel}
              </button>
            </div>
            {runtime.bytes_total != null && (
              <label className="setting-field">
                <span>{messages.progress}</span>
                <progress max={runtime.bytes_total} value={runtime.bytes_downloaded ?? 0} />
              </label>
            )}
            {runtime.error !== null && (
              <p role="alert">
                {messages.gpuRuntimeFailed} {formatError(runtime.error)}
              </p>
            )}
          </article>
        )}
        {models.map((model) => (
          <article className="model-row" key={`${model.id}@${model.revision}`}>
            <h4>
              <bdi>{model.display_name}</bdi>
            </h4>
            <dl className="fact-grid">
              <div>
                <dt>{messages.modelReadiness}</dt>
                {/* Per row. This used to render the single global coordinator
                    state against every pack, so both admitted packs claimed the
                    same installedness and only one of them could be right. */}
                <dd>{formatState(model.installed ? "verified_on_disk" : "absent")}</dd>
              </div>
            </dl>
            {!model.downloadable && !model.installed && (
              <p className="warning">{messages.packNotDownloadable}</p>
            )}

            <Disclosure hint={messages.technicalDetailsHint} summary={messages.technicalDetails}>
              <dl className="fact-grid">
                <div>
                  <dt>{messages.downloadSize}</dt>
                  <dd>{formatBytes(model.archive_bytes)}</dd>
                </div>
                <div>
                  <dt>{messages.installedSize}</dt>
                  <dd>{formatBytes(model.installed_bytes)}</dd>
                </div>
                <div>
                  <dt>{messages.modelSource}</dt>
                  <dd className="exact-value" title={model.source_repository}>
                    <bdi>{model.source_repository}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.modelRevision}</dt>
                  <dd className="exact-value" title={model.source_revision}>
                    <bdi>{model.source_revision}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.modelLicense}</dt>
                  <dd className="exact-value" title={model.license_spdx ?? model.license_name}>
                    <bdi>{model.license_spdx ?? model.license_name}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.provider}</dt>
                  <dd className="exact-value" title={model.provider}>
                    <bdi>{model.provider}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.runtime}</dt>
                  <dd className="exact-value" title={model.runtime}>
                    <bdi>{model.runtime}</bdi>
                  </dd>
                </div>
                <div>
                  <dt>{messages.modelCapabilities}</dt>
                  <dd>
                    {model.capabilities.map((capability) => (
                      <span className="tag" key={capability}>
                        <bdi>{capability}</bdi>
                      </span>
                    ))}
                  </dd>
                </div>
                <div>
                  <dt>{messages.modelHardwareEvidence}</dt>
                  <dd className="exact-value" title={model.hardware_evidence}>
                    <bdi>{model.hardware_evidence}</bdi>
                  </dd>
                </div>
              </dl>
              {hardware !== null && (
                <p className="setting-detail">
                  <bdi>{hardware.operating_system}</bdi> {messages.build}{" "}
                  <bdi>{hardware.operating_system_build ?? messages.unknown}</bdi> ·{" "}
                  {hardware.logical_processors} {messages.logicalProcessors} ·{" "}
                  {formatBytes(hardware.total_memory_bytes)} {messages.ram} —{" "}
                  {messages.inventoryOnly}
                </p>
              )}
            </Disclosure>

            <label className="confirmation">
              <input
                checked={confirmed}
                onChange={(event) => setConfirmed(event.target.checked)}
                type="checkbox"
              />
              {messages.confirmInstall}
            </label>
            <div className="actions">
              <button
                disabled={!confirmed || installing || !model.downloadable || model.installed}
                onClick={() => void installModel(model)}
                type="button"
              >
                {messages.install}
              </button>
              <button
                disabled={modelStatus.state !== "downloading" && modelStatus.state !== "installing"}
                onClick={() => void invoke("model_install_cancel")}
                type="button"
              >
                {messages.cancel}
              </button>
              <button
                className="destructive"
                disabled={!model.installed || installing}
                onClick={() => void removeModel(model)}
                type="button"
              >
                {messages.remove}
              </button>
            </div>
          </article>
        ))}
        {modelStatus.bytes_total != null && (
          <label className="setting-field">
            <span>{messages.progress}</span>
            <progress max={modelStatus.bytes_total} value={modelStatus.bytes_downloaded ?? 0} />
          </label>
        )}
        {modelStatus.error !== null && (
          <p role="alert">
            {messages.installationFailed} {formatError(modelStatus.error)}
          </p>
        )}
      </section>

      <section aria-labelledby="transcription-personalization">
        <h3 id="transcription-personalization">{messages.personalization}</h3>
        <p className="setting-detail">{messages.localeQualification}</p>
        <p className="warning">{messages.hotwordLimitation}</p>
        <p className="setting-detail">{messages.contactsDisabled}</p>

        <fieldset>
          <legend>{messages.dictionaryEntries}</legend>
          <label>
            <span>{messages.correctionObserved}</span>
            <input onChange={(event) => setObservedTerm(event.target.value)} value={observedTerm} />
          </label>
          <label>
            <span>{messages.correctionCorrected}</span>
            <input
              onChange={(event) => setCorrectedTerm(event.target.value)}
              value={correctedTerm}
            />
          </label>
          <button
            disabled={observedTerm === "" || correctedTerm === ""}
            onClick={() => void recordCorrection()}
            type="button"
          >
            {messages.recordCorrection}
          </button>
          <ul className="plain-list">
            {personalization?.dictionary.map((entry) => (
              <li key={entry.id}>
                <bdi>{entry.source}</bdi> → <bdi>{entry.replacement}</bdi>
                <button
                  className="destructive"
                  onClick={() => void deletePersonalization("dictionary", entry.id)}
                  type="button"
                >
                  {messages.delete}
                </button>
              </li>
            ))}
          </ul>
        </fieldset>

        <fieldset>
          <legend>{messages.snippets}</legend>
          <p className="setting-detail">{messages.snippetGrammar}</p>
          <label>
            <span>{messages.snippetName}</span>
            <input onChange={(event) => setSnippetName(event.target.value)} value={snippetName} />
          </label>
          <label>
            <span>{messages.snippetBody}</span>
            <textarea onChange={(event) => setSnippetBody(event.target.value)} value={snippetBody} />
          </label>
          <button
            disabled={snippetName === "" || snippetBody === ""}
            onClick={() => void saveSnippet()}
            type="button"
          >
            {messages.saveSnippet}
          </button>
          <ul className="plain-list">
            {personalization?.snippets.map((snippet) => (
              <li key={snippet.id}>
                <bdi>{snippet.name}</bdi>
                <pre>{snippet.body}</pre>
                <button
                  className="destructive"
                  onClick={() => void deletePersonalization("snippet", snippet.id)}
                  type="button"
                >
                  {messages.delete}
                </button>
              </li>
            ))}
          </ul>
        </fieldset>

        <fieldset>
          <legend>{messages.personalizationJson}</legend>
          <textarea
            onChange={(event) => setPersonalizationJson(event.target.value)}
            value={personalizationJson}
          />
          <div className="actions">
            <button onClick={() => void previewPersonalizationImport()} type="button">
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
              onClick={() => {
                void invoke<string>("personalization_export").then(setPersonalizationAction);
              }}
              type="button"
            >
              {messages.exportPersonalization}
            </button>
            <button className="destructive" onClick={() => void resetPersonalization()} type="button">
              {messages.resetPersonalization}
            </button>
          </div>
          {personalizationPreview !== null && (
            <p>
              {messages.personalizationImportSummary(
                personalizationPreview.dictionary_count,
                personalizationPreview.snippet_count,
                personalizationPreview.conflicts,
              )}
            </p>
          )}
          <output aria-live="polite">{personalizationAction}</output>
        </fieldset>
      </section>
    </>
  );
}
