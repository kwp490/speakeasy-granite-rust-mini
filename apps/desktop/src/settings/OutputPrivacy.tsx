import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { messages } from "../catalog";
import { formatError, formatState } from "./format";
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
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const [retryAction, setRetryAction] = useState("");

  useEffect(() => {
    void invoke<RecoverableResult>("result_status").then(setResult);
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
      setResult(await invoke<RecoverableResult>("result_status"));
      setRetryAction("");
    } catch {
      setResult(await invoke<RecoverableResult>("result_status"));
      setRetryAction(messages.retryFailed);
    }
  }

  const delivery = profile.profile?.delivery_preference ?? "result_view_only";

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
        removal of the capture controls (decision 6). It does not deliver: the user
        is looking at this window, so the focused app is SpeakEasy itself.
      */}
      <section aria-labelledby="output-last">
        <h3 id="output-last">{messages.lastTranscriptSection}</h3>
        <dl className="fact-grid">
          <div>
            <dt>{messages.transcriptStatus}</dt>
            <dd>{formatState(result?.state ?? "empty")}</dd>
          </div>
          {result?.provenance != null && (
            <div>
              <dt>{messages.provenance}</dt>
              <dd>{formatState(result.provenance)}</dd>
            </div>
          )}
        </dl>
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
            checked={profile.profile?.disk_logging_enabled ?? true}
            onChange={(event) => void profile.setDiskLogging(event.target.checked)}
            type="checkbox"
          />
          {messages.diagnosticLogging}
        </label>
        <p className="setting-detail">{messages.diagnosticLoggingDetail}</p>
      </section>

      <section aria-labelledby="output-protected">
        <h3 id="output-protected">{messages.protectedTargets}</h3>
        <p className="setting-detail">{messages.protectedTargetsDetail}</p>
      </section>
    </>
  );
}
