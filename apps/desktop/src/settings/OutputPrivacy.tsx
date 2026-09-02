import { useEffect, useState, type ChangeEvent } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import { formatError, formatState } from "./format";
import { readWithRetry } from "./readWithRetry";
import type { RecoverableResult } from "./types";
import type { ProfileController } from "./useProfile";

/**
 * Output & Privacy.
 *
 * Two transcript surfaces sit here and they are deliberately distinct:
 *
 * - **This session's transcripts** — in memory, gone when the app closes.
 * - **Persisted history** — on disk, off by default, and gated behind an explicit
 *   plaintext-at-rest disclosure and acknowledgement.
 *
 * They are adjacent because a user looking for "where did my transcript go" will
 * look in one place, and clearly separated because confusing them would mean
 * believing something was stored when it was not, or vice versa.
 */
export function OutputPrivacy({ profile }: { profile: ProfileController }) {
  const [result, setResult] = useState<RecoverableResult | null>(null);
  const [resultUnavailable, setResultUnavailable] = useState(false);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const [retryAction, setRetryAction] = useState("");

  // Retried, and with a rejection handler, because it had neither. `result_status`
  // stands behind two coordinators, so a lost startup race left this page saying
  // "No result" -- and "no result" is the answer someone comes here to check after
  // a dictation they cannot find. It also disabled Retry, which is the one control
  // that could have recovered the audio it was denying existed.
  useEffect(() => {
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

  async function copyResult() {
    try {
      await invoke<number>("result_copy");
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  }

  async function retryTranscription() {
    setRetryAction(messages.retryStarted);
    try {
      await invoke("dictation_retry");
      setResult(await readWithRetry<RecoverableResult>("result_status"));
      setRetryAction("");
    } catch {
      setResult(await readWithRetry<RecoverableResult>("result_status"));
      setRetryAction(messages.retryFailed);
    }
  }

  const delivery = profile.profile?.delivery_preference ?? "result_view_only";

  function updateDiskLogging(event: ChangeEvent<HTMLInputElement>) {
    void profile.setDiskLogging(event.target.checked);
  }

  return (
    <>
      <section aria-labelledby="output-delivery">
        <h3 id="output-delivery">{messages.deliveryChoice}</h3>
        <label className="confirmation">
          <input
            checked={delivery === "result_view_only"}
            name="delivery"
            onChange={() => void profile.setDelivery("result_view_only")}
            type="radio"
          />
          {messages.resultViewOnly}
        </label>
        <label className="confirmation">
          <input
            checked={delivery === "explicit_copy"}
            name="delivery"
            onChange={() => void profile.setDelivery("explicit_copy")}
            type="radio"
          />
          {messages.explicitCopy}
        </label>
        <p className="setting-detail">{messages.deliveryChoiceDetail}</p>
        {delivery === "explicit_copy" && (
          <div className="actions">
            <button
              disabled={result?.text == null}
              onClick={() => void copyResult()}
              type="button"
            >
              {messages.copyLastTranscript}
            </button>
            <output aria-live="polite">
              {copyState === "copied"
                ? messages.copied
                : copyState === "failed"
                  ? messages.copyFailed
                  : ""}
            </output>
          </div>
        )}
      </section>

      {/*
        A transcription that failed keeps its audio retained in memory, and
        retrying it is recovery rather than a guided test — so it survived the
        removal of the capture controls. It does not deliver: the user is
        looking at this window, so the focused app is SpeakEasy itself.
      */}
      <section aria-labelledby="output-last">
        <h3 id="output-last">{messages.lastTranscriptSection}</h3>
        <dl className="fact-grid">
          {/*
            `unknown` ("Not reported"), never `empty` ("No result"). Both are real
            backend states; only one of them is a claim, and it is the wrong claim
            to make from a read that has not answered.
          */}
          <div>
            <dt>{messages.transcriptStatus}</dt>
            <dd>{formatState(result?.state ?? "unknown")}</dd>
          </div>
          {result?.provenance != null && (
            <div>
              <dt>{messages.provenance}</dt>
              <dd>{formatState(result.provenance)}</dd>
            </div>
          )}
        </dl>
        {resultUnavailable && <p className="warning">{messages.resultStatusUnavailable}</p>}
        {result?.error_code != null && (
          <p role="alert">
            {messages.resultFailed} {formatError(result.error_code)}
          </p>
        )}
        <div className="actions">
          <button
            disabled={result?.retry_available !== true}
            onClick={() => void retryTranscription()}
            type="button"
          >
            {messages.retryTranscription}
          </button>
          <output aria-live="polite">{retryAction}</output>
        </div>
        {result?.retry_available !== true && (
          <p className="setting-detail">{messages.retryUnavailable}</p>
        )}
      </section>

      <section aria-labelledby="output-logging">
        <h3 id="output-logging">{messages.diagnosticLogging}</h3>
        <label className="confirmation">
          <input
            aria-label={messages.diagnosticLogging}
            checked={profile.profile?.disk_logging_enabled ?? true}
            onChange={updateDiskLogging}
            type="checkbox"
          />
          <span>
            <strong>{messages.diagnosticLogging}</strong>
            <small>{messages.diagnosticLoggingDetail}</small>
          </span>
        </label>
      </section>

      <section aria-labelledby="output-protected">
        <h3 id="output-protected">{messages.protectedTargets}</h3>
        <p className="setting-detail">{messages.protectedTargetsDetail}</p>
      </section>
    </>
  );
}
