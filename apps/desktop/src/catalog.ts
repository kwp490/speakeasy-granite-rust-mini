export const catalogMetadata = {
  locale: "en-US",
  reviewStatus: "source-language-owner-review-required",
  translatedLocales: [] as const,
} as const;

export const messages = {
  productName: "SpeakEasy",
  version: __SPEAKEASY_PRODUCT_VERSION__,
  settings: "Settings",

  // ── The compact transcriber ──────────────────────────────────────────────
  // Everyday register (UI-GUIDE "Two vocabulary registers"): plain language, no
  // contract vocabulary. The three
  // streaming tiers and the qualification wording are the exceptions — they
  // stay precise everywhere, because truthful disclosure depends on them.
  transcriber: "SpeakEasy transcriber",
  transcriberStates: {
    setupRequired: "Setup needed",
    loadingModel: "Loading model",
    idle: "Ready",
    starting: "Starting…",
    listening: "Listening",
    stopping: "Stopping…",
    transcribing: "Transcribing…",
    deliveredInserted: "Text inserted",
    deliveredCopied: "Copied to clipboard",
    deliveredHeld: "Transcript ready",
    deliveredRefused: "Not inserted",
    failed: "Stopped safely",
  },
  // The dock's engine indicator. Everyday register, and every string states
  // *both* facts in words — where Granite runs and whether it is up — because
  // the dock never takes keyboard focus, so the accessible name is the whole of
  // what a screen reader is given. "GPU ready" would be neither fact stated.
  //
  // `GPU` rather than `CUDA` and `CPU` rather than `cpu`: the wire codes are
  // the engine's vocabulary and these are the user's (UI-GUIDE "Two vocabulary
  // registers"). The map is deliberately not exhaustive over the backend's
  // codes — `unknown`, `not_configured` and `granite_state_unavailable` are not
  // devices, and `engineDeviceUnknown` is what the chip shows instead of
  // inventing one.
  engineDevices: {
    cpu: "CPU",
    cuda: "GPU",
  },
  // An em dash, not "Unknown": the chip is 52px of card and this sits beside a
  // colour and a shape that already say what is going on. Spelling out a word
  // here would push the chip past the card.
  engineDeviceUnknown: "—",
  engineChipReady: (device: string) =>
    device === "—"
      ? "Speech engine ready"
      : `Speech engine ready, running on the ${device === "GPU" ? "graphics card" : "processor"}`,
  engineChipWarming: "Speech engine is still loading. Dictation will wait for it.",
  // Not an error, and worded so it does not read as one: nothing has failed on
  // a machine that has yet to install the model.
  engineChipUnconfigured: "No speech engine is installed yet. Finish setup to dictate.",
  engineChipFailed: (reason: string) => `Speech engine unavailable. ${reason}`,
  // The one control (UI-GUIDE "Main window and focus"). Its label and colour
  // are the transcriber's primary state readout, which is why every entry here
  // names a state rather than an action: "Recording" is what is happening, and
  // pressing it is what stops it. Sentence case throughout, matching the rest
  // of this catalog.
  recordButton: {
    loadingModel: "Loading model",
    runSetup: "Set up SpeakEasy",
    ready: "Start recording",
    // Not "Recording": capture has not begun yet, and claiming it had would be
    // the same lie UI-GUIDE's truthful-disclosure rule forbids about delivery.
    starting: "Starting…",
    recording: "Recording",
    processing: "Processing…",
    tryAgain: "Try again",
  },
  modelLoadingHint: "Preparing the transcription model — this takes a moment",
  // `startDictation`, `stopAndTranscribe`, `done`, `tryAgain` and `runSetup`
  // lived here until the one control absorbed all five. Keeping them would leave
  // five plausible names for a button that no longer answers to any of them.
  cancelDictation: "Cancel",
  // Clears a finished outcome. The state table always specified this for
  // `complete`; the reducer and its tests were written and then never wired to
  // anything, so a finished transcript sat on screen until the next dictation.
  dismissResult: "Done",
  // The transcriber's own copy control. Its command resolves the text in Rust,
  // so this button names no transcript — it asks for the last one.
  copyTranscript: "Copy",
  // Past tense, and it expires: the button reports what happened rather than
  // what it offers, and only for as long as this window can vouch for it.
  copiedConfirmation: "Copied",
  openSettings: "Open settings",
  // The caption under the gear. Kept shorter than `openSettings`, which stays the
  // accessible name: the visible word is a substring of it, so the two agree
  // (WCAG 2.5.3) while the name still says what pressing it does.
  settingsButton: "Settings",
  minimizeTranscriber: "Minimize",
  closeTranscriber: "Close SpeakEasy",
  // Both names state the current setting rather than the action, because the
  // button is a toggle that shows its own state (WCAG 4.1.2 needs the state
  // conveyed, and `aria-pressed` carries it here too).
  stayOnTopOn: "Staying on top of other windows",
  stayOnTopOff: "Not staying on top of other windows",
  switchToSideDock: "Switch to side dock",
  // The side dock's one command. The deleted default HUD's record button named
  // the *state* it was in ("Recording") because it was also that window's state
  // readout; the dock has a level meter and a clock for that, so its button is
  // free to name the action instead. Short by necessity — the dock is 130px
  // wide — and `stopDictationName` is the longer accessible name it agrees
  // with (WCAG 2.5.3: the visible word is a substring of it).
  stopDictation: "Stop",
  stopDictationName: "Stop recording",
  chooseMicrophone: "Choose microphone",
  shortcutHint: (binding: string) => `${binding} to start`,
  shortcutUnavailable: "No shortcut is active — use Start recording",
  elapsedAndDevice: (elapsed: string, device: string) =>
    device === "" ? elapsed : `${elapsed} · ${device}`,
  capturedSeconds: (seconds: string) => `${seconds} seconds captured`,
  // Truthful about what happened, per UI-GUIDE's truthful-disclosure rule: a
  // refusal is never dressed up as an insertion. `deliveredInsertedDetail`,
  // `deliveredHeldDetail` and `deliveredRefusedDetail` used to sit here as a
  // second, longer telling of whatever the state line already said — three
  // prose restatements stacked above the transcript they described. The state
  // line and the Copy button carry it now, so only the refusal keeps a longer
  // form: it is the one outcome where *why* changes what the user does next.
  deliveredRefusedStatus: "Not inserted — that app refused it",
  // Why a dictation produced no text. Keyed by
  // `speakeasy_worker::FinalSourceReason::code()`.
  //
  // Every one of these used to end "so the live version was kept", which was
  // true when a streaming transcript was standing by to deliver instead. There
  // is no live version and no second engine, so each of these now describes a
  // dictation that produced nothing at all — and has to say what to do about
  // it. A reason with no action is a reason the user cannot use.
  //
  // Two registers on purpose: `finalSourceReasons` is the one-line form for the
  // toast and the dock, and `finalSourceGuidance` is what the Status page shows
  // underneath it. The short form never promises detail the long form does not
  // deliver.
  lastDictationFailed: "Your last dictation produced no text",
  finalSourceReasonUnknown: "The transcription did not complete, and nothing was pasted.",
  finalSourceReasons: {
    granite_implausible:
      "The transcription produced more text than the recording could contain, so it was discarded.",
    granite_empty: "The transcription produced no text.",
    granite_failed: "The transcription could not complete.",
    granite_unavailable: "The transcription engine is not installed.",
    granite_quarantined: "The transcription engine was paused after repeated failures.",
    no_speech: "No speech was found in the recording.",
  },
  finalSourceGuidanceUnknown:
    "Nothing was pasted. Try the dictation again; if it keeps happening, open Advanced and export the diagnostic log.",
  finalSourceGuidance: {
    // The signature of a speech model answering its prompt instead of
    // transcribing. Naming that plainly matters: the recording was fine, the
    // transcript was not, so "try again" is genuinely the right advice rather
    // than a shrug.
    granite_implausible:
      "This usually means the engine wrote text it did not hear, which the app refuses to paste. The recording itself was fine — say it again and it will normally succeed. If every dictation does this, the model files are likely damaged: reinstall from Transcription.",
    granite_empty:
      "The engine ran but wrote nothing. Check that the right microphone is selected in Audio and that its level moves while you speak.",
    granite_failed:
      "The engine stopped partway. Try again — a single failure is usually transient. If it repeats, restart SpeakEasy Mini, and if it still repeats, reinstall the model from Transcription.",
    granite_unavailable:
      "No transcription engine is installed, so nothing can be transcribed. Setup installs one and verifies it before the app ever opens, so this normally means the installation was changed afterwards. Run the installer again.",
    granite_quarantined:
      "The engine failed several times in a row and was paused so it could not keep failing silently. Restart SpeakEasy Mini to clear it.",
    // Not a malfunction, so it must not read as one.
    no_speech:
      "The recording held no speech. If you did speak, check the microphone selected in Audio — the level meter there should move while you talk.",
  },
  // Short enough for the status line's side slot, which is where the ceiling
  // lives now. It used to be a full sentence in a notice row of its own, which
  // appeared mid-dictation and pushed the transcript down while it was being
  // read.
  ceilingShort: (minutes: string) => `Auto-stops in ${minutes} min`,
  setupReasons: {
    setup_incomplete: "Finish setup before dictating.",
    model_missing: "Install the transcription model before dictating.",
    microphone_missing: "Connect a microphone before dictating.",
    shortcut_unavailable: "Choose a working keyboard shortcut before dictating.",
  },
  // ── The settings workspace ───────────────────────────────────────────────
  // Six pages behind a nav rail (UI-GUIDE "Information architecture").
  // Everyday register throughout except Advanced, which is where contract
  // vocabulary belongs (UI-GUIDE "Two vocabulary registers").
  settingsGroups: {
    general: "General",
    audio: "Audio",
    transcription: "Transcription",
    output: "Output & Privacy",
    log: "Transcript log",
    advanced: "Advanced",
  },
  settingsNav: "Settings pages",

  // Transcript log
  transcriptLogPinSection: "Keep the log visible",
  transcriptLogPinDetail:
    "Opens the log in a small window that stays on top of other windows, so you can read past transcripts while you work. It never takes keyboard focus, so it will not affect where the next dictation is pasted. Close that window to unpin it.",
  transcriptLogPin: "Pin the log",
  transcriptLogPinned: "The log is pinned.",
  transcriptLogUnpin: "Close the pinned log",
  transcriptLogRetention: "Keep transcripts",
  transcriptLogRetentionDetail:
    "Transcripts are held in memory while the app runs. Keeping them writes them to disk instead, so they are still here the next time you open the app.",
  transcriptLogClearOnClose: "Clear when I close the app",
  transcriptLogRetain: "Keep them between sessions",
  settingsHeading: "SpeakEasy Mini settings",

  // General
  shortcutSection: "Keyboard shortcut",
  shortcutStates: {
    registered: "Shortcut active",
    conflict: "Another app is already using this shortcut",
    disabled: "Shortcut turned off",
    pending: "Shortcut not registered yet",
    unknown: "Shortcut state unknown",
  },
  // Shown only after the retried read has given up. "Shortcut state unknown" says
  // the app does not know; this says what to do about it, and says the shortcut
  // itself is unaffected -- because the panel someone reads this in is the one
  // they opened believing their shortcut was broken.
  shortcutStateUnavailable:
    "The shortcut's state could not be read, so it is not shown above. The shortcut itself is unaffected — reopen this window to try again.",
  changeShortcut: "Change shortcut",
  shortcutDetail:
    "Press the shortcut to start dictating and press it again to stop. It works whether or not this window is open. A recording runs for up to 2 minutes, then auto-stops and transcribes.",
  dockSection: "Dock",
  // Section headings, distinct from the control labels beneath them. Reusing the
  // control's own label as its heading printed the same sentence twice.
  recordingFeedbackSection: "Recording sounds",
  dockAlwaysOnTop:
    "The dock stays on top of other windows so it is reachable while you work, and clings to whichever screen edge you drag it to. Drag it to move it; right-click it for settings.",
  keyboardPathsSection: "Keyboard access",
  keyboardPathsDetail:
    "The dock never takes keyboard focus, so it cannot be operated by keyboard. The shortcut starts and stops dictation; these controls cover everything else it offers.",
  quitApp: "Quit SpeakEasy Mini",
  quitAppDetail: "Quitting during a dictation asks first and never discards a recording silently.",
  startupSection: "Windows startup",

  // Audio
  audioDeviceSection: "Microphone",
  audioDeviceDetail:
    "Dictation records from this microphone, whether it is started from the transcriber or the shortcut.",
  recordingBehaviorSection: "Recording behavior",
  recordingBehaviorDetail:
    "Press the record button or shortcut to start and press it again to stop. SpeakEasy does not use automatic voice detection. A recording runs for up to 2 minutes, then auto-stops and transcribes.",
  refreshDevices: "Refresh microphone list",
  deviceSaved: "Microphone saved.",
  deviceSaveFailed: "That microphone is no longer available. Refresh the list and choose another.",
  noDevices: "Windows is not offering any usable microphone. Check the cable and microphone privacy permission.",
  unsupportedDeviceSuffix: " (unsupported format)",
  inputLevelSection: "Input level",
  inputLevelWhileDictating: "The level moves while a dictation is running. Start one from the transcriber or the shortcut to see it.",
  deviceHealthSection: "Microphone status",
  captureStateLabel: "Recording state",

  // Transcription
  languageSection: "Language",
  languageDetail: "English (United States) only. No other language is qualified, so none is offered.",
  modelSection: "Transcription model",
  modelReadiness: "Readiness",
  technicalDetails: "Technical details",
  technicalDetailsHint: "Exact package facts. Select a value to copy it.",
  showRawValues: "Show raw values",
  rawValuesHint: "The exact identifiers used in logs and diagnostics, before display names are applied.",

  // Output & Privacy
  sessionLog: "This session's transcripts",
  sessionLogDetail:
    "Every finished transcript from this run of SpeakEasy, newest first. This list is held in memory only: it is never written to disk, and it disappears when SpeakEasy closes.",
  sessionLogEmpty: "Finished transcripts will appear here.",
  sessionLogCount: (count: number) =>
    count === 1 ? "1 transcript this session" : `${count} transcripts this session`,
  copyEntry: "Copy",
  lastTranscriptSection: "Last transcript",
  // The recoverable result's own state, not the microphone's. Labelling it
  // "Recording state" made "Ready" read as "ready to record".
  transcriptStatus: "Status",
  retryTranscription: "Transcribe the retained audio again",
  retryUnavailable: "No retained audio is available to transcribe again.",
  retryStarted: "Transcribing the retained audio again.",
  retryFailed: "That did not complete. The audio is still retained, so you can try again.",
  protectedTargets: "Protected targets",
  protectedTargetsDetail:
    "Password fields, the Windows secure desktop, elevated windows, read-only targets and terminals never receive inserted text. The transcript stays here instead.",

  // Advanced
  runtimeSection: "Runtime",
  performanceSection: "Performance",
  credentialsSection: "Credentials",
  maintenanceSection: "Maintenance",
  restartEngine: "Restart transcription engine",
  engineRestarted: "The transcription engine was restarted.",
  engineRestartFailed: "The engine could not be restarted. Try again after any running dictation finishes.",
  aboutSection: "About",
  aboutDetail:
    "SpeakEasy transcribes on this device with a local model. There is no analytics, no crash upload and no cloud sync.",
  /**
   * Contract identifier to display name (UI-GUIDE "Information architecture",
   * the Advanced group). Everyday surfaces read these;
   * Advanced shows the raw identifier alongside, behind Show raw values, because
   * that is the vocabulary logs and diagnostics use.
   */
  displayNames: {
    supervised_process: "Supervised process",
    sherpa_onnx: "sherpa-onnx",
    manual_stop_only: "Manual stop",
    commit_on_finish: "Insert after transcription finishes",
    live_qualified: "Live transcript (qualified)",
    final_only: "Final transcript only",
    result_view_only: "Private result view only",
    explicit_copy: "Result view with explicit copy",
    clipboard_paste: "Paste from the clipboard",
    clipboard_only: "Clipboard only",
    selected_device_only: "Only the microphone you selected",
    hotkey_auto_paste_enabled: "Inserting into the focused app is turned on",
    bundled_trusted_manifest: "Bundled trusted manifest",
  },
  back: "Back",
  continue: "Continue",
  completeSetup: "Complete setup",
  stepProgress: (current: number, total: number) => `Step ${current} of ${total}`,
  stepDone: "Done",
  stepCurrent: "Current step",
  tryItPending: "No transcript yet.",
  tryItReady: "A transcript is ready, so dictation is working on this computer.",
  closeSetup: "Close setup",
  importNotFound: "No v1 source was detected.",
  importFound: "A read-only v1 import preview is available.",
  runningV1Warning: "SpeakEasy v1 is running. Close it before import so the source cannot change.",
  importCategories: "Import categories",
  importSettings: "Settings",
  importPresets: "Presets",
  collisionPolicy: "When a v2 item already exists",
  keepV2: "Keep the v2 item",
  renameV1: "Import the v1 item with a new name",
  replaceFromV1: "Replace the v2 item from the read-only v1 source",
  credentialPresenceOnly: "Credential checks report presence and access state only. Secret values are never imported into this window.",
  importSelected: "Import selected categories",
  importComplete: "The selected v1 categories were imported. The v1 source was not changed.",
  importFailed: "Import stopped safely. Review the source warning and preview again.",
  sharedProgramDataWarning: "The v1 source is machine-wide. Review the preview to confirm it belongs to this profile.",
  corruptSettingsWarning: "Unreadable v1 settings were excluded.",
  corruptPresetWarning: "An unreadable v1 preset was excluded.",
  importWarning: "The import preview contains a warning.",
  importReportSummary: (settings: boolean, presets: number, collisions: number) =>
    `Report: settings ${settings ? "written" : "not written"}; ${presets} presets written; ${collisions} collisions resolved.`,
  privacyDefaults: "Private defaults",
  privacyDefaultsDetail: "History is session-only. Audio, transcripts, credentials, and logs are not persisted by default.",
  hotkeyRegistration: "Hotkey registration",
  hotkeyBinding: "Global hotkey",
  hotkeyMode: "Activation mode",
  hotkeyModeToggle: "Toggle (press to start, press again to stop)",
  hotkeyModePushToTalk: "Push to talk (hold to record)",
  hotkeyModeHandsFree: "Hands-free (press to start, automatic stop is not implemented; use Stop)",
  hotkeyEnabledLabel: "Enable the global hotkey",
  saveHotkey: "Save hotkey",
  hotkeySaved: "Hotkey saved.",
  hotkeySaveFailed: "Hotkey could not be saved. Check the binding and try again.",
  hotkeyDetail: "Works even when this window is closed. The transcriber's record button does exactly the same thing.",
  recordingFeedback: "Play a Windows sound when recording starts and stops",
  recordingFeedbackDetail: "Visual recording status is always shown. Windows sound settings control audible volume.",
  diagnosticLogging: "Keep a local diagnostic log",
  diagnosticLoggingDetail: "Sanitized event names and error codes only, never transcript text or audio. Stays on this device and is never uploaded.",
  startupWithWindows: "Start SpeakEasy with Windows",
  history: "Persisted history",
  historyOff: "Session-only (recommended)",
  historyOn: "Store transcript history",
  historyDisclosure: "History is plaintext in your per-user app data. Secure targets are always excluded.",
  retentionDays: "Retention in days",
  acceptHistoryDisclosure: "I understand the plaintext-at-rest disclosure",
  saveHistory: "Save history choice",
  deleteHistory: "Delete persisted history",
  confirmDeleteHistory: "I understand this permanently deletes every stored transcript",
  deleted: "Deleted",
  exportHistory: "Export persisted history",
  deliveryChoice: "Delivery choice",
  resultViewOnly: "Private result view only",
  explicitCopy: "Result view with explicit copy",
  deliveryChoiceDetail: "This choice covers the transcript kept in this window. Dictation started from the transcriber or the shortcut always inserts its final transcript into the app you were using, unless that app refuses inserted text.",
  copyLastTranscript: "Copy the last transcript",
  modelSource: "Source",
  modelRevision: "Revision",
  modelLicense: "License",
  modelCapabilities: "Capabilities",
  modelHardwareEvidence: "Hardware evidence",
  downloadSize: "Download",
  installedSize: "Installed",
  progress: "Progress",
  diagnostics: "Diagnostics",
  engine: "Engine",
  worker: "Worker",
  runtime: "Runtime",
  provider: "Provider",
  performance: "Performance",
  rtfP95: "Real-time factor p95",
  latencyP50: "Latency p50",
  latencyP95: "Latency p95",
  noMeasuredValue: "Not measured on this profile",
  audioOverflow: "Audio overflow count",
  deviceStatus: "Device status",
  deliveryCapability: "Delivery capability",
  deliveryReason: "Delivery reason",
  finalSource: "Final source disclosure",
  modelProvenance: "Model provenance",
  sanitizedLogs: "Logs and export are sanitized",
  exportDiagnostics: "Export sanitized diagnostics",
  diagnosticsExported: "Sanitized diagnostics exported:",
  credentialStatus: "Credential status",
  legacyOpenAiCredential: "Legacy OpenAI credential",
  legacyRemoteCredential: "Legacy remote credential",
  credentialPresent: "Present in the primary legacy service",
  credentialLegacyService: "Present in the fallback legacy service",
  credentialMissing: "Missing",
  credentialAccessDenied: "Access denied",
  credentialUnavailable: "Credential Manager unavailable",
  credentialsNeverShown: "Credential values are never shown or returned to this window.",
  resetProfile: "Reset fresh profile",
  previewReset: "Preview reset",
  resetExclusions: "Reset excludes v1, custom models, and credentials.",
  resetNow: "Reset v2 settings, history, personalization, and logs",
  resetCategorySettings: "v2 settings",
  resetCategoryHistory: "v2 history",
  resetCategoryPersonalization: "v2 personalization",
  resetCategoryLogs: "v2 logs",
  resetCategoryOther: "other v2-owned data",
  // Capture controls are gone from settings entirely (decision 6): dictation
  // happens only from the transcriber and the global shortcut, so there is one
  // controller and no second start path to diverge from it.
  microphone: "Microphone",
  selectMicrophone: "Select a microphone",
  defaultDeviceSuffix: " (default)",
  captureFailed: "Recording or transcription stopped safely:",
  installBeforeCapture: "Install and verify the local model before dictating.",
  vad: "Voice activity",
  level: "Level",
  inputLevel: "Microphone input level",
  liveTranscript: "Live transcript",
  mutableHypothesis: "Changing",
  stableDisplay: "Stable display",
  stableDisplayDisclosure: "Display-stable text may still change in the final transcript and is never written to another application.",
  finalTranscript: "Final",
  finalOnlyDisclosure: "Live partial text is not shown. The final transcript appears once you stop recording.",
  liveQualifiedDisclosure: "Live transcription is qualified for the supported en-US CPU and CUDA configurations. The final transcript is what gets written.",
  localModels: "Local models",
  provisioning: "Provisioning",
  build: "build",
  unknown: "unknown",
  logicalProcessors: "logical processors",
  ram: "RAM",
  inventoryOnly: "Detected only, not runtime-qualified.",
  personalization: "Personalization",
  localeQualification: "Only limited en-US normalization and sentence capitalization are qualified. Other locales remain unchanged.",
  hotwordLimitation: "Protected terms are applied after the transcript is finished, correcting the spelling and the spacing of words that were recognised. They do not change what the model hears, so a word that is misheard stays misheard. Adding them to the model's prompt instead was measured and rejected: it recognised more names and returned the whole dictation without any sentence punctuation.",
  contactsDisabled: "Contacts import is disabled. No contact source is read or scraped.",
  correctionObserved: "Recognized text",
  correctionCorrected: "Always replace with",
  recordCorrection: "Save explicit correction",
  dictionaryEntries: "Dictionary and protected terms",
  snippetName: "Snippet trigger name",
  snippetBody: "Inert text expansion",
  saveSnippet: "Save text-only snippet",
  snippets: "Snippets",
  snippetGrammar: "Say “snippet name” as the whole finished utterance. Say “literal snippet name” to escape it. Snippets never run mid-partial or send Enter/actions.",
  delete: "Delete",
  personalizationJson: "Personalization JSON",
  previewPersonalizationImport: "Preview JSON import",
  commitPersonalizationImport: "Commit reviewed import",
  personalizationImportSummary: (dictionary: number, snippets: number, conflicts: number) =>
    `Preview: ${dictionary} dictionary entries; ${snippets} snippets; ${conflicts} conflicts.`,
  exportPersonalization: "Export personalization",
  resetPersonalization: "Reset dictionary and snippets",
  personalizationUnavailable:
    "Your dictionary and snippets could not be read, so this list is not showing them. Nothing has been lost — reopen this window to try again.",
  runtimeStatusUnavailable:
    "The runtime facts could not be read, so they are not shown here. Nothing is wrong with the engine — reopen this window to try again.",
  resultStatusUnavailable:
    "The last transcript's status could not be read, so it is not shown above. Nothing has been lost — reopen this window to try again.",
  profileUnavailable:
    "Your settings could not be read, so the controls below are showing their defaults rather than your choices. Nothing has been changed — reopen this window to try again.",
  personalizationSaved: "Personalization saved.",
  personalizationRejected: "The change was rejected. Check conflicts, limits, or forbidden action placeholders.",
  confirmInstall: "Confirm download and local installation",
  install: "Install",
  cancel: "Cancel",
  remove: "Remove",
  installationFailed: "Installation stopped safely:",
  packNotDownloadable:
    "This model is not published for download yet, so it cannot be installed from here.",
  /**
   * The notice shown when the safety ceiling ends a recording.
   *
   * Owner-approved wording, 2026-08-25. **It leads with the transcript being
   * safe**, because that is the question a user asks first when a recording
   * stops without them asking it to — and because the honest answer is
   * reassuring, which the previous behaviour's answer was not: the ceiling used
   * to discard the recording outright.
   *
   * The number is named rather than called "the limit", so the sentence is
   * usable the *next* time the user starts dictating, which is the whole point
   * of telling them at all. The last clause says what to do, because "you hit a
   * limit" with no next step is a complaint rather than an instruction.
   */
  captureLimitNoticeTitle: "Recording reached the 2-minute maximum",
  captureLimitNoticeBody:
    "Recording stopped automatically and your transcript was delivered. Anything said after the 2-minute mark was not recorded — start another dictation to continue.",
  captureLimitNoticeDismiss: "Got it",
  engineDisclosure: "Dictation runs on:",
  /**
   * Whether what setup recorded still describes what is running.
   *
   * `ok` and `unrecorded` have no copy on purpose: they are the quiet answers,
   * and a line that appears on every launch to say nothing is wrong is a line
   * people stop reading. Only the three disclosures below are ever shown, and
   * only one of them is a fault.
   *
   * `gpu_install_not_operational` is the actionable one, and it is the condition
   * that produced this whole surface: setup used to record a graphics-card
   * installation from an unchecked radio button, the app then correctly ran on
   * the processor, and the disagreement existed only as three fields of one log
   * line that nothing compared. It names what to do, and it does not claim
   * dictation is broken, because it is not: the same model produces the same
   * transcript on the processor.
   *
   * The remedy changed on 2026-08-21 and the copy had to change with it. It said
   * "reinstall", because a reinstall was the only thing that re-ran the proof;
   * the bootstrapper's `--verify-provider` verb now runs the identical check
   * against an installed build in seconds. Leaving the old wording in place
   * would have had the app recommending the expensive remedy while a cheap one
   * shipped beside it. The two ordinary causes come first, because they are what
   * a user can actually act on and neither needs a command line.
   *
   * `gpu_record_unconfirmed` is the one it used to swallow. A driver that will
   * not answer NVML is not evidence of anything, and until 2026-08-21 it landed
   * in the fault above and told the user their dictation had moved to the
   * processor. This says what is known, says nothing is known to be wrong, and
   * puts the remedy last because there may be nothing to remedy.
   */
  providerIntegrity: {
    gpu_install_not_operational:
      "This installation was recorded as using the graphics card, and dictation is running on the processor instead. Transcripts are unaffected; the speed is not. Update the graphics driver and close anything else using the card, then re-check the engine — running setup with --verify-provider re-proves it without reinstalling.",
    running_beyond_record:
      "Dictation is running on the graphics card, which is more than this installation was recorded as providing. Nothing is wrong — the graphics-card engine was staged after setup ran.",
    gpu_record_unconfirmed:
      "This installation was recorded as using the graphics card, and this run could not be confirmed — the graphics driver did not answer. Dictation is unaffected and is most likely still on the card. If it keeps happening, update the graphics driver, then re-check with setup's --verify-provider.",
  },
  engineNone: "Nothing yet",
  engineReasonUnknown: "The reason is unavailable.",
  gpuRetest: "Re-test graphics-card engine",
  gpuQualified: "The graphics-card engine has passed its local execution check.",
  gpuNotQualified: "The graphics-card engine is detected but has not passed its local execution check yet.",
  /**
   * Why this machine landed on this engine.
   *
   * `cpu_gpu_pack_not_installed` is the one that has to exist: without it a
   * user with a perfectly good graphics card sees "runs on processor" and has
   * no way to learn that the GPU model was simply never installed.
   *
   * **Every one of these describes the installation, and none of them says what
   * is running.** That is the change of 2026-08-21, and the defect was found by
   * reading the rendered window rather than the code. They were clauses appended
   * to the device line with an em-dash, so Settings said, verbatim, `Dictation
   * runs on: Graphics card (GPU) — ... so the processor model is being
   * used.` The device was right, the code was right, and the sentence was false:
   * a reason about the *pack* and a device are two facts that disagree on any
   * machine running a graphics-card worker against the single processor-named
   * pack, which `ARCHITECTURE.md` under "Which provider runs, and how you find
   * out" had predicted and nobody had looked at. Two things keep it fixed —
   * these strings are scoped to what the installation includes, and the page
   * renders them as their own sentence instead of joining them to the device.
   *
   * `probe_preferred` was reworded too, though it was not the reported defect.
   * "The best engine this hardware supports" becomes false the day a
   * graphics-card pack is preferred and the driver refuses it, and keeping a
   * latent copy of the bug just fixed is not worth the smaller diff.
   */
  engineReasons: {
    probe_preferred: "This installation includes the best engine this hardware supports.",
    cpu_gpu_pack_not_installed:
      "This computer's graphics card is supported, but this installation includes only the processor model.",
    cpu_gpu_runtime_missing:
      "This computer's graphics card is supported, but this installation does not include graphics-card acceleration.",
    // Not "nothing is installed": a pack can be on disk and still unrunnable
    // here — an installed graphics-card model on an installation that has no
    // graphics-card acceleration is exactly that. This says what is true of
    // the outcome in both cases without asserting the disk is empty.
    no_pack_installed: "No transcription model is ready to run on this computer.",
  },
  /**
   * The graphics-card acceleration offer.
   *
   * Deliberately never says "CUDA", "cuDNN" or "execution provider" in the
   * headline copy — those are the names of the files, not of the benefit. The
   * component breakdown under Technical details is where the real names live,
   * because a user comparing a download against their remaining disk needs the
   * actual figures.
   */
  localResult: "Local result",
  recoverableResult: "Recoverable transcript",
  noResult: "Your completed local transcript will remain available here.",
  provenance: "Source",
  resultFailed: "Transcription stopped safely:",
  copy: "Copy",
  copied: "Copied",
  copyFailed: "Copy failed. The result remains available.",
  retry: "Retry",
  recoverWorker: "Recover transcription worker",
  yes: "Yes",
  no: "No",
  millisecondSuffix: " ms",
  unknownState: "Unknown",
  errorUnknown: "The operation stopped safely. Review the current status and try again.",
  errors: {
    capture_device_enumeration_failed: "Windows could not list microphones. Check microphone privacy permission and reconnect the selected device.",
    capture_status_unavailable: "Recording status is still starting. Wait a moment and try again.",
    capture_device_unavailable: "The selected microphone is unavailable. Choose an available microphone and retry.",
    capture_device_format_unsupported: "The selected microphone format is not supported. Choose another input device.",
    capture_start_failed: "Recording could not start. Check Windows microphone permission and the selected device.",
    capture_already_active: "A recording is already active. Stop it before starting another.",
    // The press that lands between a recording ending and its transcript
    // arriving -- most often the habitual second press of a toggle after the
    // two-minute maximum stopped the recording on its own. One dictation at a
    // time, so it is refused rather than queued. The copy has to say the earlier
    // transcript is safe, because the visible effect is a shortcut that did
    // nothing, which reads as a broken shortcut.
    dictation_still_finishing:
      "The previous recording is still being transcribed. Its text is on its way — wait for it to arrive, then start the next one.",
    capture_not_active: "No active recording could be stopped.",
    capture_empty: "No usable audio was captured. Check mute and the Windows input meter, then retry.",
    // The five conditions below annotate audio that **was** delivered, and the
    // wording has to match that. Until 2026-08-25 every one of them made
    // `run_capture` return `Err`, which discarded the recording -- and the
    // buffer's byte limit bound 3.5 s inside the two-minute ceiling, so every
    // maximum-length dictation raised one and was destroyed. Only the first of
    // them had copy at all; the other four fell through to `errorUnknown`
    // ("The operation stopped safely"), which is how a user came to lose two
    // minutes of speech and be told nothing about why.
    capture_queue_overflow:
      "Audio processing could not keep up, so parts of the recording may be missing from the transcript.",
    capture_discontinuity:
      "The audio stream was interrupted, so a moment of the recording may be missing from the transcript.",
    capture_duration_limit:
      "The recording filled its audio buffer, so the last part of it may be missing from the transcript.",
    capture_byte_limit:
      "The recording filled its audio buffer, so the last part of it may be missing from the transcript.",
    capture_buffer_limit:
      "The recording filled its audio buffer, so the last part of it may be missing from the transcript.",
    // Genuine failures: nothing was delivered.
    capture_device_fault:
      "The microphone stopped responding during recording. Reconnect it or choose another device, then retry.",
    capture_finish_failed:
      "The recording could not be finished. Retry, and choose another microphone if it happens again.",
    capture_duration_out_of_range:
      "Recording reached the 2-minute safety limit. It stopped and is being transcribed now.",
    // Absent until 2026-08-10, so this fell through to `errorUnknown` — a
    // missing-install-files condition reported as "the operation stopped
    // safely", which is indistinguishable from a transcription regression and
    // cost a debugging round to tell apart. It is also what every dictation
    // under `npm run tauri -- dev` returns, because nothing stages `proof/`
    // for a dev run.
    runtime_resources_unavailable:
      "This installation is missing its transcription components. Reinstall the app.",
    runtime_adapter_failed: "Local transcription did not complete. The captured audio remains available for Retry.",
    runtime_deadline_exceeded: "Local transcription timed out. The captured audio remains available for Retry.",
    runtime_worker_out_of_memory: "Local transcription ran out of memory. Close other large applications and retry.",
    runtime_worker_quarantined: "Transcription paused after repeated worker failures. Use Recover transcription worker before retrying.",
    runtime_not_ready: "The local transcription model is not ready. Verify the installed model and retry.",
    model_status_unavailable: "Model status is still starting. Wait a moment and retry.",
    catalog_unavailable: "The trusted local model catalog could not be read.",
    installed_reverification_failed: "The installed model failed verification. Reinstall the local model.",
    // Codes the backend could already return with nothing to say about them:
    // every one of these rendered as errorUnknown, and the install path's
    // rejections were not caught at all, so they rendered as nothing.
    pack_is_not_downloadable:
      "This model is not available for download yet. Install the other listed model to start dictating.",
    pack_not_admitted: "That model is not in the trusted local catalog.",
    confirmation_required: "Confirm the installation before it can start.",
    install_busy: "Another model installation is already running. Wait for it to finish.",
    install_not_active: "No model installation is running.",
    model_state_unavailable: "Model installation state is unavailable. Wait a moment and retry.",
    cuda_runtime_state_unavailable:
      "Graphics-card acceleration state is unavailable. Wait a moment and retry.",
    gpu_not_admissible:
      "No supported graphics card was found, so graphics-card acceleration cannot be installed on this computer.",
    cuda_runtime_download_failed:
      "The graphics-card acceleration files could not be downloaded. Check the connection and try again; a retry resumes rather than restarting.",
    // Distinguished from a plain download failure because a length or digest
    // mismatch is not something to retry through — it means the bytes served
    // were not the bytes the catalog pins.
    cuda_runtime_verification_failed:
      "The downloaded graphics-card acceleration files did not match the trusted catalog and were refused. Nothing was installed.",
    cuda_runtime_insufficient_disk:
      "There is not enough free disk space to install graphics-card acceleration. Free up space and try again.",
    // The one failure here with a concrete user action attached: Windows locks a
    // library that is already loaded, so replacing it needs the app restarted.
    cuda_runtime_in_use:
      "Graphics-card acceleration is currently in use and cannot be replaced. Restart the app and try again.",
    cuda_runtime_incomplete:
      "The graphics-card acceleration files are incomplete, so the processor is still being used. Try installing again.",
    cuda_runtime_write_failed:
      "The graphics-card acceleration files could not be written. Check disk space and permissions, then try again.",
    cuda_runtime_cancelled: "The graphics-card acceleration installation was cancelled.",
    cuda_runtime_manifest_incomplete:
      "The trusted catalog does not describe graphics-card acceleration completely. Reinstall the app.",
    cuda_runtime_name_collision:
      "The trusted catalog describes conflicting graphics-card acceleration files. Reinstall the app.",
    remove_failed: "The model could not be removed. It may be in use by an active dictation.",
    streaming_pack_not_installed:
      "No transcription model is ready to run on this computer. Install one from Transcription settings.",
    streaming_model_load_failed: "The installed model could not be loaded. Reinstall the local model.",
    streaming_worker_unavailable: "The local transcription worker could not start. Retry, or use Recover transcription worker.",
    runtime_policy_invalid: "The local runtime configuration was refused. Reinstall the app.",
    engine_state_unavailable: "The transcription engine state is unavailable. Wait a moment and retry.",
    clipboard_write_failed: "Windows did not accept the explicit copy. The result remains available.",
    clipboard_writer_unavailable: "The explicit copy service is unavailable. The result remains available.",
    clipboard_sequence_unavailable: "Windows did not confirm the copy. Check the clipboard before relying on it.",
    session_transcript_unavailable: "This session's transcript list could not be read. The transcripts themselves are unaffected.",
    session_transcript_entry_unavailable: "That transcript is no longer in this session's list.",
    profile_recovery_required: "Settings need recovery. Use the local Repair shortcut before changing this profile.",
    history_recovery_required: "Optional history needs recovery. Dictation remains available without history.",
    // Warm states the dock's engine indicator can render. Every one of these is
    // reachable from `GraniteEngineCoordinator::warm_state`, and a code with no
    // entry here falls through to `errorUnknown` -- "The operation stopped
    // safely" -- which is the generic non-answer a lost dictation once got.
    //
    // `granite_worker_missing` is deliberately separate from "no model yet".
    // The latter is not an error at all and never reaches this table; it is
    // `engineChipUnconfigured`, in the amber state, because a machine part-way
    // through setup has not failed at anything.
    granite_worker_missing:
      "The transcription engine is missing from this installation. Reinstall the app.",
    granite_model_files_unverified:
      "The installed model failed verification, so it was not loaded. Reinstall the local model.",
    granite_quarantined:
      "Transcription is paused after repeated engine failures. Use Restart engine in Advanced settings.",
    granite_state_unavailable:
      "The transcription engine state is unavailable. Wait a moment and retry.",
    engine_unavailable: "The transcription engine is still starting. Wait a moment and retry.",
    runtime_stale_response:
      "The transcription engine did not answer in time while loading. Use Restart engine in Advanced settings.",
    runtime_invalid_data: "The transcription engine returned something unusable. Reinstall the local model.",
    runtime_invalid_transition:
      "The transcription engine was asked for something out of order. Use Restart engine in Advanced settings.",
    runtime_queue_full: "The transcription engine is busy. Wait for the current dictation to finish.",
    runtime_cancelled: "Loading the transcription engine was cancelled.",
    // Not a fault in the install, and worded so it does not read as one: this
    // computer is below the 8 GiB floor Granite needs, which is a fact about
    // the machine rather than something to repair. Found by the scaffold test
    // rather than by review -- it was the one publishable warm state with no
    // copy, and it would have rendered as "The operation stopped safely".
    memory_below_granite_floor:
      "This computer has too little memory to run the speech engine, which needs 8 GB.",
  },
  degradationReasons: {
    "degradation.microphone_denied": "Microphone access was denied.",
    "degradation.microphone_unavailable": "The selected microphone is unavailable.",
    "degradation.audio_overflow": "Audio processing could not keep up.",
    "degradation.streaming_unavailable": "Live transcription is unavailable.",
    "degradation.final_engine_failed": "Final transcription did not complete.",
    "degradation.worker_quarantined": "Transcription was paused after repeated worker failures.",
    "degradation.accelerator_lost": "The optional accelerator became unavailable.",
    "degradation.model_unavailable": "The local model is missing or failed verification.",
    "degradation.target_changed": "The original text target changed.",
    "degradation.target_refused": "This text target cannot be used safely.",
    "degradation.clipboard_unavailable": "The clipboard could not be used safely.",
    "degradation.optional_network_unavailable": "The optional network service is unavailable.",
    "degradation.optional_storage_unavailable": "Optional storage is unavailable.",
    "degradation.lifecycle_interrupted": "Windows interrupted this dictation session.",
    "degradation.shutdown_in_progress": "SpeakEasy is shutting down.",
    "degradation.protocol_too_new": "An optional component uses a newer protocol.",
    "degradation.disk_full": "There is not enough free disk space.",
    "degradation.dictation_busy": "This operation waits until dictation finishes.",
  },
  degradationActions: {
    "action.review_microphone_permission": "Review microphone permission and retry.",
    "action.choose_microphone": "Choose an available microphone and retry.",
    "action.finish_and_review": "Finish and review the recoverable result.",
    "action.continue_final_only": "Continue in final-only mode.",
    "action.review_recoverable_result": "Review the last recoverable result.",
    "action.recover_worker": "Use manual worker recovery before retrying.",
    "action.retry_on_cpu": "Retry using the private CPU provider.",
    "action.verify_model": "Verify or reinstall the local model.",
    "action.open_result_view": "Open the recoverable result.",
    "action.open_private_result": "Review the result privately in SpeakEasy.",
    "action.retry_copy": "Retry the explicit copy action.",
    "action.use_local_result": "Continue with the raw local result.",
    "action.continue_without_history": "Continue without optional history.",
    "action.retry_after_resume": "Retry after Windows resumes the session.",
    "action.restart_later": "Restart after dictation is safe.",
    "action.update_optional_component": "Update the optional component.",
    "action.free_disk_space": "Free disk space and retry.",
    "action.retry_after_dictation": "Retry after dictation finishes.",
  },
  states: {
    starting: "Starting",
    idle: "Idle",
    unavailable: "Unavailable",
    ready: "Ready",
    arming: "Arming",
    capturing: "Capturing",
    captured: "Ready to transcribe",
    complete: "Complete",
    draining: "Draining",
    finalizing: "Finalizing",
    running: "Running",
    delivering: "Delivering",
    delivered: "Delivered",
    failed: "Failed",
    result_view_only: "Result view only",
    verifying: "Verifying installed model",
    absent: "Absent",
    downloading: "Downloading",
    installing: "Installing",
    verified_on_disk: "Verified on disk",
    cancelled: "Cancelled",
    installed: "Installed",
    partial: "Partly installed",
    cpu: "Processor (CPU)",
    cuda: "Graphics card (GPU)",
    // A graphics-card engine whose context could not be confirmed. Its own
    // label rather than either of the two above, because it is neither: calling
    // it "Graphics card" is the unverified claim, and calling it "Processor"
    // would report a fault on a machine that is very likely using its card.
    cuda_unverified: "Graphics card (GPU), unconfirmed",
    not_configured: "Not started yet",
    unknown: "Not reported",
    empty: "No result",
    finalized_stream: "Finalized streaming output",
    last_valid_draft: "Last valid draft",
    final_only: "Final only",
    live_qualified: "Live transcript (qualified)",
    streaming: "Streaming",
    true_online: "True streaming",
    manual_stop_only: "Manual stop only",
    pending: "Not yet registered",
    disabled: "Disabled",
    registered: "Registered",
    conflict: "Registration conflict; another app may be using this binding",
  },
} as const;
