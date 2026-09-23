import { messages } from "../catalog";

/**
 * Catalog lookups shared by the settings pages.
 *
 * Every one of these turns a backend code into catalog prose. None of them ever
 * returns a raw identifier as user-facing text — that is what `displayName`
 * exists for, and what Advanced → Technical details is for
 * when the identifier itself is the thing worth seeing.
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
/**
 * The disclosure for a provider record that no longer matches what is running.
 *
 * `null` for the quiet answers -- `ok`, `unrecorded`, and any code this build
 * does not know -- so the caller renders nothing. Deliberately not a fallback
 * to "Unknown": this line only exists to report a specific disagreement, and a
 * placeholder in its place would be a line saying something is wrong without
 * saying what.
 */
export function formatProviderIntegrity(code: string): string | null {
  const disclosures: Record<string, string> = messages.providerIntegrity;
  return disclosures[code] ?? null;
}

export function formatEngineReason(reason: string): string {
  const reasons = messages.engineReasons;
  return reasons[reason as keyof typeof reasons] ?? messages.engineReasonUnknown;
}

/**
 * Contract identifier to display name.
 *
 * Falls back to the state table, and then to **the identifier itself** — never to
 * "Unknown". Falling back to Unknown was a real defect: the runtime version,
 * device policy and delivery reason all have values outside the table below,
 * and the Advanced page reported every one of them as "Unknown" while the raw
 * panel two lines below showed `sherpa_onnx_c_api_1_13_4`. Telling a user a value is
 * unknown when the app knows it is worse than showing the identifier, and Advanced
 * is precisely where contract vocabulary is allowed to appear (UI-GUIDE
 * "Two vocabulary registers").
 *
 * Only a genuinely absent value is Unknown.
 */
export function displayName(value: string): string {
  if (value === "") return messages.unknown;
  const names = messages.displayNames;
  if (value in names) return names[value as keyof typeof names];
  return messages.states[value as keyof typeof messages.states] ?? value;
}

/** Plain-language shortcut registration — never "HOTKEY REGISTRATION". */
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

/** Modifier keys by `KeyboardEvent.key`, which never form a shortcut alone. */
const MODIFIER_KEYS = new Set(["Control", "Alt", "Shift", "Meta", "OS", "AltGraph"]);

/**
 * The binding a key press names, in the form `hotkey_configure` accepts, or
 * `null` when the press is not a shortcut.
 *
 * Built from `code` rather than `key`, because `key` is the character the
 * layout produces -- Shift+2 is `@` on one keyboard and `"` on another -- and
 * the global-shortcut parser wants the physical key. A shortcut needs Ctrl, Alt
 * or Windows: Shift alone would swallow ordinary typing in every application.
 */
export function bindingFromKey(event: {
  key: string;
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}): string | null {
  if (MODIFIER_KEYS.has(event.key)) return null;
  if (!event.ctrlKey && !event.altKey && !event.metaKey) return null;
  let key = event.code;
  if (/^Key[A-Z]$/.test(key)) key = key.slice(3);
  else if (/^Digit[0-9]$/.test(key)) key = key.slice(5);
  else if (key === "") return null;
  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  if (event.metaKey) parts.push("Super");
  parts.push(key);
  return parts.join("+");
}
