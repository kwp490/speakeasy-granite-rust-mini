import { messages } from "../catalog";

/**
 * Catalog lookups shared by the six settings pages.
 *
 * Every one of these turns a backend code into catalog prose. None of them ever
 * returns a raw identifier as user-facing text — that is what `displayName`
 * exists for, and what the Advanced page's Show raw values disclosure is for
 * when the identifier itself is the thing worth seeing (§9.5).
 */

export function formatBytes(bytes: number | null): string {
  return bytes === null ? messages.unknown : `${(bytes / 1_073_741_824).toFixed(1)} GiB`;
}

export function formatState(state: string): string {
  return messages.states[state as keyof typeof messages.states] ?? messages.unknownState;
}

export function formatError(code: string): string {
  return messages.errors[code as keyof typeof messages.errors] ?? messages.errorUnknown;
}

/** Why dictation landed on the engine it did (`GpuStatus.engine_reason`). */
export function formatEngineReason(reason: string): string {
  const reasons = messages.engineReasons;
  return reasons[reason as keyof typeof reasons] ?? messages.engineReasonUnknown;
}

/**
 * Contract identifier to display name (§9.5).
 *
 * Falls back to the state table, and then to **the identifier itself** — never to
 * "Unknown". Falling back to Unknown was a real defect: the runtime version,
 * device policy and delivery reason all have values outside §9.5's six-row table,
 * and the Advanced page reported every one of them as "Unknown" while the raw
 * panel two lines below showed `sherpa_onnx_c_api_1_13_4`. Telling a user a value is
 * unknown when the app knows it is worse than showing the identifier, and Advanced
 * is precisely where contract vocabulary is allowed to appear (§12).
 *
 * Only a genuinely absent value is Unknown.
 */
export function displayName(value: string): string {
  if (value === "") return messages.unknown;
  const names = messages.displayNames;
  if (value in names) return names[value as keyof typeof names];
  return messages.states[value as keyof typeof messages.states] ?? value;
}

/** Plain-language shortcut registration (§9.1) — never "HOTKEY REGISTRATION". */
export function formatShortcutState(registration: string): string {
  const states = messages.shortcutStates;
  if (registration in states) return states[registration as keyof typeof states];
  return states.unknown;
}

export function formatImportWarning(warning: string): string {
  if (warning === "v1_running_source_may_change") return messages.runningV1Warning;
  if (warning === "shared_programdata_user_ambiguity") return messages.sharedProgramDataWarning;
  if (warning === "corrupt_settings") return messages.corruptSettingsWarning;
  if (warning.startsWith("corrupt_preset:")) return messages.corruptPresetWarning;
  return messages.importWarning;
}

export function formatResetCategory(category: string): string {
  if (category === "v2_settings") return messages.resetCategorySettings;
  if (category === "v2_history") return messages.resetCategoryHistory;
  if (category === "v2_personalization") return messages.resetCategoryPersonalization;
  if (category === "v2_logs") return messages.resetCategoryLogs;
  return messages.resetCategoryOther;
}

export function formatCredentialStatus(status: string): string {
  if (status === "primary_service") return messages.credentialPresent;
  if (status === "legacy_service") return messages.credentialLegacyService;
  if (status === "missing") return messages.credentialMissing;
  if (status === "access_denied") return messages.credentialAccessDenied;
  return messages.credentialUnavailable;
}

/**
 * Locale-aware wall-clock time for a session-log entry.
 *
 * Time only, not a date: the log covers one run of the app, so the date is
 * always today and repeating it would be noise.
 */
export function formatTimeOfDay(unixMs: number): string {
  return new Date(unixMs).toLocaleTimeString();
}

/**
 * Why the last dictation produced no text, as a sentence.
 *
 * Keyed by `speakeasy_worker::FinalSourceReason::code()`, plus the `runtime_*`
 * codes that can fail a dictation before the engine is reached. Falls back to
 * a real sentence rather than the code, because this is read by someone whose
 * dictation just vanished — a bare `granite_implausible` tells them nothing
 * they can act on.
 */
export function formatFinalSourceReason(code: string): string {
  const reasons = messages.finalSourceReasons;
  return (
    reasons[code as keyof typeof reasons] ??
    messages.errors[code as keyof typeof messages.errors] ??
    messages.finalSourceReasonUnknown
  );
}

/** What to do about it. Same keys as `formatFinalSourceReason`. */
export function formatFinalSourceGuidance(code: string): string {
  const guidance = messages.finalSourceGuidance;
  return guidance[code as keyof typeof guidance] ?? messages.finalSourceGuidanceUnknown;
}
