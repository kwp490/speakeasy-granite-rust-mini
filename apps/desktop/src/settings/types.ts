/**
 * The IPC views the settings workspace reads.
 *
 * These mirror the `Serialize` structs in `src-tauri/src/lib.rs`. They used to
 * sit at the bottom of a 960-line `SettingsApp.tsx`; they live here so five
 * pages can share them without one importing another.
 */

export type SafeDeliveryPreference = "result_view_only" | "explicit_copy";

export type ModelCatalogItem = {
  id: string;
  revision: string;
  display_name: string;
  archive_bytes: number;
  installed_bytes: number;
  confirmation_required: boolean;
  source_repository: string;
  source_revision: string;
  license_name: string;
  license_spdx: string | null;
  license_url: string;
  runtime: string;
  provider: string;
  capabilities: string[];
  hardware_evidence: string;
  /** Whether the app can fetch this pack at all. A pack with no archive URL
   *  installs only from an archive supplied on disk. */
  downloadable: boolean;
  /** Whether *this* pack is on disk. Per row: install state is per pack. */
  installed: boolean;
};

/**
 * Which engine dictation will actually run on, and why.
 *
 * `active_provider` is deliberately not the same question as whether a GPU was
 * detected: an admissible card whose pack was never installed runs on CPU, and
 * `engine_reason` is the only thing that says so.
 */
export type GpuStatus = {
  status: string;
  qualified: boolean;
  admissible: boolean;
  adapter_name: string | null;
  compute_capability: string | null;
  total_vram_bytes: number | null;
  free_vram_bytes: number | null;
  driver_version: string | null;
  minimum_compute_capability: string;
  active_provider: string | null;
  engine_reason: string;
  /**
   * The device the worker is actually running on, which is what the disclosure
   * shows. Distinct from `active_provider`, which names the selected *pack* —
   * there is one Granite GGUF and a graphics-card worker offloads that same
   * file, so the pack reads `cpu` on a machine holding the card. Displaying the
   * pack under "Dictation runs on" was a mislabel of exactly that case.
   */
  active_device: string;
  /**
   * Whether what setup recorded still describes what is running. `ok` and
   * `unrecorded` are quiet; anything else is disclosed.
   */
  provider_integrity: string;
  /** Whether that is a condition someone has to act on. Decided in Rust. */
  provider_fault: boolean;
};


export type ModelHardware = {
  operating_system: string;
  operating_system_build: string | null;
  logical_processors: number;
  total_memory_bytes: number | null;
};

export type ModelInstallStatus = {
  state: string;
  error: string | null;
  bytes_downloaded?: number | null;
  bytes_total?: number | null;
};

export type ProfileStatus = {
  schema_version: number;
  startup_with_windows: boolean;
  history_enabled: boolean;
  history_retention_days: number;
  history_plaintext_disclosure_accepted: boolean;
  delivery_preference: SafeDeliveryPreference;
  recording_feedback_enabled: boolean;
  disk_logging_enabled: boolean;
  /**
   * The microphone dictation will actually record from, or `null` when nothing is
   * stored and the OS default applies. The Audio page shows this rather than the
   * OS default: on this host they differ, and the page was naming a device the
   * next dictation would not have used.
   */
  preferred_capture_device_id: string | null;
};

export type ImportPreview = {
  nonce: string;
  source_fingerprint: string;
  settings_available: boolean;
  preset_names: string[];
  warnings: string[];
  running_v1: boolean;
};

export type CollisionPolicy = "keep_v2" | "replace_from_v1" | "rename_v1";

export type ImportReport = {
  source_fingerprint: string;
  settings_written: boolean;
  presets_written: number;
  collisions_resolved: string[];
};

export type DiagnosticsStatus = {
  schema_version: number;
  engine: string;
  worker: string;
  runtime: string;
  provider: string;
  rtf_median: number | null;
  rtf_p95: number | null;
  latency_p50_ms: number | null;
  latency_p95_ms: number | null;
  audio_overflow_count: number;
  device: string;
  vad: string;
  delivery_capability: string;
  delivery_reason: string;
  model_id: string;
  model_revision: string;
  model_source: string;
  final_source_reason: string | null;
  recent_reason_codes: string[];
  logs_sanitized: boolean;
};

export type DiagnosticsExport = {
  file_name: string;
  categories: string[];
  contains_sensitive_content: boolean;
};

export type CredentialStatus = {
  openai_legacy: string;
  remote_legacy: string;
  values_exposed: false;
};

export type ResetPreview = {
  nonce: string;
  categories: string[];
  excludes_v1: boolean;
  excludes_custom_models: boolean;
  excludes_credentials: boolean;
};

export type DictionaryEntry = {
  id: string;
  source: string;
  replacement: string;
};

export type Snippet = {
  id: string;
  name: string;
  body: string;
};

export type PersonalizationStatus = {
  schema_version: number;
  transform_pipeline_version: number;
  locale_status: string;
  hotword_path: string;
  contacts_import_enabled: false;
  dictionary: DictionaryEntry[];
  snippets: Snippet[];
};

export type PersonalizationImportPreview = {
  fingerprint_sha256: string;
  dictionary_count: number;
  snippet_count: number;
  conflicts: number;
  contacts_imported: false;
};

export type HotkeyStatus = {
  binding: string;
  mode: "toggle" | "push_to_talk" | "hands_free";
  registration: string;
  enabled: boolean;
  active: boolean;
};

export type CaptureDevice = {
  id: string;
  name: string;
  is_default: boolean;
  supported: boolean;
};

export type CaptureWizardStatus = {
  state: string;
  device_name: string | null;
  captured_samples: number | null;
  error_code: string | null;
  can_stop: boolean;
  can_transcribe: boolean;
  can_retry: boolean;
};

export type RecoverableResult = {
  state: string;
  text: string | null;
  provenance: string | null;
  input_samples: number | null;
  final_segments: number | null;
  draft_revisions: number | null;
  error_code: string | null;
  retry_available: boolean;
};

/**
 * One finished transcript from this run of the app.
 *
 * In memory only. There is no id that outlives the process and nothing to
 * delete, because nothing was stored.
 */
export type SessionTranscriptEntry = {
  id: string;
  text: string;
  provenance: string;
  recorded_unix_ms: number;
};

/** Level snapshot for the Audio page's input meter. Non-mutating. */
export type CaptureLevel = {
  level: number;
  active: boolean;
  device_diagnostic: string;
};

/**
 * Everything the Audio page samples on its timer, in one answer.
 *
 * `CaptureLevel` plus the three `CaptureWizardStatus` fields the device-health
 * panel renders. Both of those types are still used elsewhere; this exists so
 * the 10 Hz page makes one call instead of two, and so its two halves cannot
 * describe different moments.
 */
export type CaptureAudioSnapshot = CaptureLevel & {
  state: string;
  device_name: string | null;
  error_code: string | null;
};
