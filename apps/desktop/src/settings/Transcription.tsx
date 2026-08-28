import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { Disclosure } from "../components/Disclosure";
import { messages } from "../catalog";
import {
  formatBytes,
  formatEngineReason,
  formatProviderIntegrity,
  formatError,
  formatFinalSourceGuidance,
  formatFinalSourceReason,
  formatState,
} from "./format";
import { readWithRetry } from "./readWithRetry";
import type {
  DiagnosticsStatus,
  GpuStatus,
  ModelCatalogItem,
  ModelHardware,
  ModelInstallStatus,
  PersonalizationImportPreview,
  PersonalizationStatus,
} from "./types";

/**
 * Transcription: language, the local model, and personalization.
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
/**
 * How long to keep re-reading the engine disclosure while the worker warms.
 *
 * 30 x 1 s. A cold Granite load measured 2-5 s on this hardware, and the ceiling
 * is generous rather than tuned -- the poll stops as soon as the device is
 * reported, so the only run that reaches the ceiling is one where Granite never
 * warms, and that is a state to stop asking about rather than one to wait for.
 */
const ENGINE_WARM_READS = 30;
const ENGINE_WARM_READ_INTERVAL_MS = 1_000;

export function Transcription() {
  const [models, setModels] = useState<ModelCatalogItem[]>([]);
  const [hardware, setHardware] = useState<ModelHardware | null>(null);
  const [gpu, setGpu] = useState<GpuStatus | null>(null);
  const [lastFailure, setLastFailure] = useState<string | null>(null);
  const [modelStatus, setModelStatus] = useState<ModelInstallStatus>({
    state: "verifying",
    error: null,
  });
  const [confirmed, setConfirmed] = useState(false);
  const [personalization, setPersonalization] = useState<PersonalizationStatus | null>(null);
  /**
   * Set when the personalization read never succeeded.
   *
   * Its own flag rather than folding into `personalizationAction`, because it
   * belongs beside the list that is missing rather than beside the import
   * controls — and because "could not be read" and "your list is empty" are
   * different facts that looked identical here until 2026-08-20.
   */
  const [personalizationUnavailable, setPersonalizationUnavailable] = useState(false);
  /**
   * How many times the engine disclosure has been re-read while the worker was
   * still coming up.
   *
   * The device and the provider-integrity line are both `not_configured` until
   * the launch warm has spoken, and that happens seconds *after* this page
   * mounts -- a cold Granite load is 2-5 s on this hardware. Read once on mount,
   * the page therefore reported "Not started yet" and no integrity line for the
   * life of the window, which for the fault case means the one disclosure that
   * exists to be seen is never rendered.
   *
   * Bounded, and it stops the moment the device is reported. An unbounded poll
   * would keep asking forever on a machine where Granite is not configured at
   * all, which is an ordinary state rather than a wait.
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

  useEffect(() => {
    void refreshCatalog();
    void readWithRetry<ModelHardware>("model_hardware").then(setHardware, () => {
      // Same startup race as the personalization read below. Left unset rather
      // than defaulted: the hardware panel renders nothing without it, which is
      // honest, where invented values would not be.
    });
    void invoke<ModelInstallStatus>("model_install_status")
      .then(setModelStatus)
      .catch(() => {
        setModelStatus({ state: "failed", error: "model_status_unavailable" });
      });
    // Retried, because this read used to be fired once with no `catch` and
    // dropped its rejection. A read that lost the race against `setup` managing
    // `PersonalizationCoordinator` left this list empty for the life of the
    // process, which reads as "you have no protected terms" — the exact way
    // setup's vocabulary appeared to be discarded while sitting correctly on
    // disk.
    void readWithRetry<PersonalizationStatus>("personalization_status").then(
      (status) => {
        setPersonalization(status);
        setPersonalizationUnavailable(false);
      },
      () => {
        setPersonalizationUnavailable(true);
      },
    );
    // Read once on mount rather than polled. The reason only changes when a
    // dictation finishes, and this page is not open during one — settings never
    // has focus while the user is dictating, because taking focus would change
    // where the transcript is pasted.
    //
    // Retried, because "read once, not polled" is exactly the shape that cannot
    // recover from a lost startup race, and this read carries the *failure
    // panel*: losing it hides the reason a dictation produced nothing, which is
    // the one thing this page owes a user whose transcript vanished.
    void readWithRetry<DiagnosticsStatus>("diagnostics_status").then(
      (status) => setLastFailure(status.final_source_reason),
      () => {
        // Diagnostics being unavailable is not itself a dictation failure, and
        // reporting it as one here would invent a problem. The panel stays
        // hidden; Advanced is where an unreadable diagnostics surface shows up.
      },
    );
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

  const installing =
    modelStatus.state === "verifying" ||
    modelStatus.state === "downloading" ||
    modelStatus.state === "installing";

  /**
   * Re-reads the catalog *and* the engine disclosure together.
   *
   * Both change when a pack is installed or removed: `installed` per row, and
   * which engine dictation resolves to. Reading them at the same moment keeps
   * the page from claiming a GPU engine next to a GPU pack it just deleted.
   */
  async function refreshCatalog() {
    try {
      // Both through the retry, including the calls that follow an install or a
      // removal. This is called from mount as well, and there it was the worst of
      // the reads that could lose the startup race: a refusal landed in the
      // `catch` below, which sets `modelStatus` to failed and puts the raw error
      // string on screen next to an empty model list — "no models exist", said
      // by an error path, about a machine with 2.14 GB of weights on disk. A
      // genuine `catalog_unavailable` still reports, five seconds later.
      setModels(await readWithRetry<ModelCatalogItem[]>("model_catalog"));
      setGpu(await readWithRetry<GpuStatus>("gpu_status"));
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
      {/* The last failure, first on the page.
          With one engine and no fallback, a dictation that went wrong produced
          no text at all — so the user arriving here has already lost something
          and is looking for why. Putting the model catalog above that answer
          would make them scroll past four disclosures to reach it.
          Absent entirely when the last dictation succeeded: an empty "no
          problems" panel is a permanent invitation to worry. */}
      {lastFailure !== null && (
        <section aria-labelledby="transcription-status" className="status-panel">
          <h3 id="transcription-status">{messages.lastDictationFailed}</h3>
          <p role="alert">{formatFinalSourceReason(lastFailure)}</p>
          <p className="setting-detail">{formatFinalSourceGuidance(lastFailure)}</p>
        </section>
      )}

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
            {/* The **device**, not the pack. `active_provider` names the
                selected pack, and there is one Granite GGUF that a graphics-card
                worker offloads unchanged — so the pack reads `cpu` on a machine
                holding the card, and showing it here said the wrong thing about
                every such machine. */}
            <p className="setting-detail" data-testid="engine-disclosure">
              {messages.engineDisclosure}{" "}
              <bdi>
                {gpu.active_provider === null
                  ? messages.engineNone
                  : formatState(gpu.active_device)}
              </bdi>
            </p>
            {/* Its own sentence, in its own element, and never joined to the
                line above. It used to hang off the device after an em-dash,
                which reads as one sentence about one fact -- and these are two
                facts that disagree on any machine running a graphics-card
                worker against the single processor-named pack. The rendered
                result was `Dictation runs on: Graphics card (GPU) -- ... so the
                processor model is being used.` Rewording alone would have left
                the next reason free to do it again. */}
            <p className="setting-detail" data-testid="engine-reason">
              {formatEngineReason(gpu.engine_reason)}
            </p>
            {/* Shown only when it says something. `ok` and `unrecorded` are the
                quiet answers and have no copy, so this renders nothing on a
                normal launch — which is the requirement: never hide the active
                provider, and never narrate it either. */}
            {formatProviderIntegrity(gpu.provider_integrity) !== null && (
              <p
                className={gpu.provider_fault ? "warning" : "setting-detail"}
                data-testid="provider-integrity"
              >
                {formatProviderIntegrity(gpu.provider_integrity)}
              </p>
            )}
            <article className="model-row" data-testid="gpu-controls">
              {/* A qualification sentence sat here and could only ever be the
                  negative one. `GpuQualificationCoordinator::record` -- the only
                  thing that promotes a card from "admissible" to "proven" -- was
                  deleted on 2026-08-21 because Granite had no GPU path to smoke,
                  and its own note said it "comes back with the CUDA worker, not
                  before". The CUDA worker shipped on 2026-08-26 and nothing
                  brought it back, so this line told every graphics-card user
                  that the engine "has not passed its local execution check yet"
                  -- beside a device line reading Graphics card (GPU), with a
                  button that implied a remedy no amount of pressing could reach.
                  Found 2026-08-28.
                  Removed rather than reworded, because the question it asked is
                  already answered above by two facts that *are* reachable: the
                  device line, which reads `cuda` only where NVML confirmed the
                  worker's own pid holds a context, and the provider-integrity
                  line, which speaks up when the record and the run disagree.
                  Restoring the promotion needs an `ExecutionEvidence` with a
                  real `inference_sample_count`, which nothing at warm time has
                  -- and inventing one would be the manufactured claim this whole
                  area exists to prevent. Recorded as an open gap in
                  `docs/handoff/CURRENT.md`.
                  The button below stays and is not cosmetic: `gpu_retest`
                  invalidates the engine and re-warms it, so its effect lands on
                  both lines above. */}
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
        {/* The graphics-card acceleration offer stood here.

            It fetched ONNX Runtime's CUDA execution provider on demand — 2.97 GB
            of libraries the streaming engine needed. Granite does not use them,
            and its own GPU support is a compile-time feature of the worker
            binary rather than anything this page can download, so an offer here
            could only ever have installed DLLs and changed nothing observable.
            Setup fetches the CUDA worker and its two libraries together, as the
            single unit they are. */}
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
          {personalizationUnavailable && (
            <p className="warning">{messages.personalizationUnavailable}</p>
          )}
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
