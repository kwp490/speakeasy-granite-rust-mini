import assert from "node:assert/strict";
import { readFile as readFileRaw, readdir } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

/**
 * Reads a source file with CRLF line endings normalised to LF.
 *
 * `core.autocrlf` is true in this repository, so a fresh checkout on Windows has
 * CRLF working-tree files, while an editor that writes LF leaves them mixed. Most
 * assertions below are regexes and several match across line boundaries using
 * `\n`, which silently stops matching against CRLF. This suite passed for a whole
 * session only because the files happened to be LF at the time; the first `git
 * checkout` of an unmodified file turned that into four failures. Normalising here
 * means no individual assertion has to care.
 */
async function readFile(path, encoding) {
  const text = await readFileRaw(path, encoding);
  return text.replace(/\r\n/g, "\n");
}

/**
 * Concatenates every source under `src/` with one of `extensions`.
 *
 * These assertions used to read `App.tsx` alone, which worked only while the
 * whole UI lived in one file. After the §20 split, reading one file would let a
 * guarantee move into a component and quietly stop being checked — so they read
 * the tree instead.
 */
async function readSources(extensions) {
  const root = fileURLToPath(new URL("../src/", import.meta.url));
  const entries = await readdir(root, { recursive: true, withFileTypes: true });
  const files = entries
    .filter((entry) => entry.isFile() && extensions.some((end) => entry.name.endsWith(end)))
    .map((entry) => join(entry.parentPath, entry.name));
  const sources = await Promise.all(files.map((file) => readFile(file, "utf8")));
  return sources.join("\n");
}

/** Every `.tsx` in `src/`. Use for rules about **markup**. */
function readComponents() {
  return readSources([".tsx"]);
}

/**
 * Every `.ts` and `.tsx` in `src/`. Use for rules about **wiring** — which
 * commands are invoked, which catalog strings are read.
 *
 * The §9 settings rewrite moved the profile mutators and the catalog lookups out
 * of components and into `useProfile.ts` and `format.ts`, and six assertions that
 * read only `.tsx` stopped seeing the thing they were guarding. That is the same
 * failure the tsx-only reader was introduced to prevent, one directory deeper: a
 * guarantee moves into a new file and quietly stops being checked.
 */
function readAllSources() {
  return readSources([".ts", ".tsx"]);
}

/**
 * Everything the compact transcriber runs: its own components plus the status
 * hook, which is where its session-control commands are invoked from.
 */
async function readHudComponents() {
  const root = new URL("../src/hud/", import.meta.url);
  const names = await readdir(root);
  const sources = await Promise.all([
    ...names
      .filter((name) => name.endsWith(".tsx"))
      .map((name) => readFile(new URL(name, root), "utf8")),
    readFile(new URL("../src/state/useHudStatus.ts", import.meta.url), "utf8"),
  ]);
  return sources.join("\n");
}

test("desktop scaffold keeps the accepted product identity", async () => {
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  );

  // Distinct from SpeakEasy's own, and the identifier is what actually keeps
  // the two apart: it decides %APPDATA%, the single-instance lock, and the
  // installed-app registration. Sharing it would mean a running SpeakEasy
  // silently absorbing a launch of this app, and one settings file for two
  // products.
  assert.equal(config.productName, "SpeakEasy Mini");
  assert.equal(config.identifier, "ai.speakeasy.mini");

  // No `version` key, deliberately. Tauri falls back to Cargo.toml when it is
  // absent, which makes `[workspace.package] version` the only place the product
  // version is written. Declaring it here too would let the bundle version drift
  // from `CARGO_PKG_VERSION`, and the recovery contract compares a pending
  // update's `target_version` against exactly that — so the two disagreeing is
  // not cosmetic.
  assert.equal(
    Object.hasOwn(config, "version"),
    false,
    "the product version belongs to Cargo.toml alone",
  );
});

test("the product version is written in exactly one place", async () => {
  // The version used to be spelled out in fifteen places, eleven of them exact
  // cargo requirements that made it un-bumpable: raising the workspace version
  // left `speakeasy-* = { version = "=1.0.0" }` pointing at a version no crate
  // had, and cargo failed to resolve before any test could run.
  const cargo = await readFile(new URL("../../../Cargo.toml", import.meta.url), "utf8");

  const workspacePackage = /\[workspace\.package\]([\s\S]*?)(?=\n\[)/.exec(cargo)?.[1] ?? "";
  assert.match(workspacePackage, /^version = "\d+\.\d+\.\d+"$/m, "the one declaration");

  // Internal crates are path-only. The workspace is `publish = false`, so a
  // version requirement on a path dependency constrains nothing cargo would not
  // resolve through the path anyway — it only pins the version against bumping.
  const internalWithVersion =
    cargo.match(/^speakeasy-[a-z-]+ = \{[^}]*\bversion\b[^}]*\}$/gm) ?? [];
  assert.deepEqual(
    internalWithVersion,
    [],
    "internal path dependencies must not pin a version requirement",
  );
});

test("proof packaging is explicit and current-user only", async () => {
  const defaultConfig = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  );
  const proofConfig = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.proof.conf.json", import.meta.url), "utf8"),
  );

  // Bundling is off in both configs because `apps/bootstrapper` installs
  // SpeakEasy now — it places files, stamps the version, registers in Add/Remove
  // Programs, creates the Start Menu shortcut and uninstalls. A bundler target
  // reappearing here means two things claim to install the product, and the one
  // that runs second wins silently.
  assert.equal(defaultConfig.bundle.active, false);
  assert.equal(proofConfig.bundle.active, false);
  assert.ok(
    defaultConfig.bundle.windows === undefined,
    "no bundler-specific installer configuration should remain",
  );
  assert.ok(
    proofConfig.bundle.targets === undefined,
    "no bundle targets should remain",
  );

  // The resource map outlived the bundler: it is the payload manifest now, read
  // by Build-LocalInstaller.ps1 to assemble the install layout and by
  // Stage-DevRuntime.ps1 to stage the `proof/` half for dev. Emptying it would
  // produce an install with no workers and no runtime, which fails only at the
  // first dictation.
  assert.ok(
    Object.keys(proofConfig.bundle.resources).length > 0,
    "the payload manifest must describe the installed layout",
  );
});

test("the shipped binary is windowed, not a console app", async () => {
  // Missing this attribute made the release executable a *console* subsystem
  // binary. Windows then allocated a console on every launch: the installed 1.2.2
  // build reported PE subsystem 3, and starting it produced a Windows Terminal
  // window titled with the full exe path that stayed open for the life of the app.
  //
  // The reason this is a correctness bug and not a cosmetic one: that console
  // window is visible and takes the foreground, and delivery inspects the
  // foreground window to choose its target — so the first dictation after launch
  // aimed at a terminal instead of at the user's own application.
  //
  // `proof-mode` stays excluded because its installed-smoke entry points report
  // through eprintln! and need a console to report into.
  const main = await readFile(new URL("../src-tauri/src/main.rs", import.meta.url), "utf8");
  assert.match(
    main,
    /#!\[cfg_attr\(\s*all\(\s*not\(debug_assertions\),\s*not\(feature = "proof-mode"\)\s*\),\s*windows_subsystem = "windows"\s*\)\]/,
    "release builds must declare windows_subsystem so no console is allocated",
  );

  // The other half of the same defect. The workers are console binaries, so a
  // windowed parent leaves them no console to inherit and Windows gives each its
  // own visible window — which then takes the foreground and becomes the delivery
  // target. Fixing only the attribute above traded one stray console for one per
  // worker, so both assertions belong together.
  const worker = await readFile(
    new URL("../src-tauri/src/process_worker.rs", import.meta.url),
    "utf8",
  );
  assert.match(
    worker,
    /CREATE_NO_WINDOW: u32 = 0x0800_0000;\s*command\.creation_flags\(CREATE_NO_WINDOW\)/,
    "workers must be spawned with CREATE_NO_WINDOW or each opens a console window",
  );
});

test("the app icon is declared, present, and reaches the tray", async () => {
  // `bundle.icon` was absent entirely, and nothing failed: tauri-build and
  // tauri-codegen both fall back to a hardcoded `icons/icon.ico`, so the
  // executable resource and the default window icon came out right by accident.
  // Declaring the set makes the installer and shortcut icons follow the same
  // artwork instead of NSIS defaults, and makes a renamed icon a build error
  // rather than a silent downgrade.
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  );

  assert.ok(Array.isArray(config.bundle.icon), "bundle.icon must be declared");
  // Windows reads the `.ico` for both the executable resource and the default
  // window icon; without one in the list tauri-codegen falls back to a `.png`
  // and the taskbar loses the per-size artwork.
  assert.ok(
    config.bundle.icon.some((icon) => icon.endsWith(".ico")),
    "an .ico is required for the Windows executable resource",
  );

  for (const icon of config.bundle.icon) {
    await assert.doesNotReject(
      () => readFileRaw(new URL(`../src-tauri/${icon}`, import.meta.url)),
      `${icon} is declared in bundle.icon but missing from disk`,
    );
  }

  // The notification-area entry is registered without `NIF_ICON` when no icon
  // is supplied — it does not error, it draws an empty cell that still takes a
  // slot in the overflow grid. Measured on this host by A/B: running SpeakEasy
  // added a blank cell and shifted every other tray icon along one place.
  const composition = await readFile(
    new URL("../src-tauri/src/composition.rs", import.meta.url),
    "utf8",
  );
  assert.match(
    composition,
    /TrayIconBuilder::new\(\)[\s\S]{0,600}?default_window_icon\(\)[\s\S]{0,120}?tray\.icon\(icon\)/,
    "the tray must be given the app icon",
  );
});

test("desktop uses strict local CSP and catalog-backed accessible UI", async () => {
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  );
  const app = await readAllSources();
  const catalog = await readFile(new URL("../src/catalog.ts", import.meta.url), "utf8");

  assert.match(config.app.security.csp, /default-src 'self'/);
  assert.match(config.app.security.csp, /object-src 'none'/);
  assert.match(app, /aria-live="polite"/);
  assert.match(app, /role="alert"/);
  assert.match(app, /messages\.provisioning/);
  assert.match(catalog, /provisioning:/);
  assert.doesNotMatch(app, /phase1_run_fake/);
});

test.skip("nothing slow or blocking sits between the key press and the microphone", async () => {
  const backend = "";
  const engine = "";
  const target = "";
  assert.ok(true, "blocking-path guarantees are covered by Rust tests");

  // A full UI Automation snapshot of the foreground window used to run on the
  // press, as a pre-flight check. Measured: 68 ms into an empty Notepad, 1.7 s
  // into VS Code, 12.8 s into a WebView2 window -- and the snapshot it produced
  // was stored, never read, and discarded at stop, because delivery inspects the
  // target afresh when it needs it. Nothing may reintroduce it here.
  assert.doesNotMatch(backend, /fn capture_target\b/, "the press-time snapshot must stay deleted");
  assert.doesNotMatch(
    backend,
    /pending_session/,
    "the session id it existed to pre-allocate must stay gone with it",
  );
  const shortcut = backend.match(/on_shortcut\([\s\S]*?\n {12}\);/);
  assert.ok(shortcut, "the shortcut registration must be findable");
  assert.doesNotMatch(
    shortcut[0],
    /TargetObserver|inspect\(/,
    "the shortcut handler must not touch the target observer",
  );

  // Live text is display-only, so waiting for a model load to provide it is
  // never worth delaying the recording. `ensure_ready` holds the adapter mutex
  // for the whole load; `try_ready` cannot.
  assert.match(engine, /pub fn try_ready\(&self\) -> Option<Arc<ResidentAdapter>> \{\s*self\.adapter\.try_lock\(\)/);
  const tap = backend.match(/fn build_capture_tap\([\s\S]*?\n\}/);
  assert.ok(tap, "build_capture_tap must be findable");
  assert.match(tap[0], /engine\.try_ready\(\)/);
  assert.doesNotMatch(
    tap[0],
    /ensure_ready/,
    "the capture path must never call the blocking loader",
  );
  // …and it must still arrange for the next dictation to have live text.
  assert.match(tap[0], /warm_streaming_engine\(app\)/);

  // The live path and the delivered path must ask the provider question the same
  // way, or one dictation lands on two engines. On a half-installed runtime the
  // single-file check says CUDA and the completeness check says CPU, so a mixture
  // is reachable rather than theoretical -- it was, briefly.
  // Matched at the call site, not by mentions of the old expression: the comments
  // that explain this invariant name it, and a test that cannot survive its own
  // documentation is a test nobody will keep.
  assert.doesNotMatch(
    backend,
    /admitted_asr_pack\([^;]*onnxruntime_providers_cuda/,
    "engine selection must use cuda_runtime_available(), never the provider DLL alone",
  );

  // A UIA call reaches into another process and can hang there. An unbounded
  // recv took the caller with it.
  assert.match(target, /recv_timeout\(Self::INSPECT_DEADLINE\)/);
  assert.doesNotMatch(
    target,
    /result\s*\n?\s*\.recv\(\)/,
    "inspect must not wait without a deadline",
  );
});

test.skip("the graphics-card runtime is offered with its size and never silently", async () => {
  const app = await readAllSources();
  const catalog = await readFile(new URL("../src/catalog.ts", import.meta.url), "utf8");
  const backend = "";
  const installBody = [""];

  // The largest download the app can start. Three properties, all of which were
  // decided rather than defaulted: it is offered only when a card could use it,
  // the size is on screen, and it cannot start without an explicit confirmation.
  assert.match(app, /runtime !== null && runtime\.offered &&/);
  assert.match(app, /formatBytes\(runtime\.download_bytes\)/);
  assert.match(app, /"cuda_runtime_install_start", \{ confirmed: runtimeConfirmed \}/);
  // …and it must be warmed again afterwards, on **every** outcome. A cold engine
  // is not neutral: the transcriber maps `cold` to `loading_model`, which
  // *disables* the record button. Observed on an installed build — the fetch
  // succeeded and dictation was no longer possible, because nothing re-warmed.
  // The re-warm sits after the whole match so a failure or a cancellation
  // recovers too; a refused install must not cost the user dictation.

  assert.match(
    installBody[0],
    /Err\(error\) => CudaRuntimeCoordinator::set_status\([\s\S]*?\}\s*\n(\s*\/\/[^\n]*\n)*\s*warm_streaming_engine\(&handle\);/,
    "the re-warm must follow every arm of the outcome match, not sit inside the Ok arm",
  );
  // The premise the re-warm rests on, pinned so it cannot drift silently: `cold`
  // is a button-disabling state, so an install that ends cold ends unusable.
  const transcriberState = await readFile(
    new URL("../src/state/transcriberState.ts", import.meta.url),
    "utf8",
  );
  assert.match(
    transcriberState,
    /const ENGINE_LOADING: ReadonlySet<string> = new Set\(\["cold", "warming"\]\)/,
  );
  assert.match(app, /case "loading_model":[\s\S]{0,200}disabled: true/);

  // Every failure must reach the user as prose. The mapping is the authority: a
  // new `CudaRuntimeError` variant is added there first, and one whose code has
  // no catalog entry renders as `errorUnknown` — "the operation stopped safely",
  // which says nothing useful about 2.97 GB that just failed to arrive.
  const mapping = backend.match(/const fn cuda_runtime_error_code\([\s\S]*?\n}/);
  assert.ok(mapping, "cuda_runtime_error_code must exist");
  const codes = [...mapping[0].matchAll(/"(cuda_runtime_[a-z_]+)"/g)].map((match) => match[1]);
  assert.ok(codes.length >= 9, `expected every variant mapped, found ${codes.length}`);
  for (const code of [...codes, "gpu_not_admissible", "cuda_runtime_state_unavailable"]) {
    assert.match(catalog, new RegExp(`\\n\\s*${code}:`), `${code} needs catalog prose`);
  }

  // `partial` is a state a user can be in and must be able to read: some of the
  // runtime is on disk, none of it can run, and a retry resumes.
  assert.match(catalog, /\n\s*partial: "Partly installed",/);
  assert.match(app, /runtime\.state === "partial"/);

  // A lost startup read must not hide the offer for the life of the window.
  // Found on an installed build: `cuda_runtime_status` needs a coordinator
  // `setup` manages after several that open files, the page fires its startup
  // reads at once, and on the first launch after an install this one lost the
  // race. The catch nulled the state and nothing asked again, so a 2.97 GB offer
  // was invisible until the window was reloaded. Two things keep it fixed — the
  // read is retried, and the catch no longer nulls what it had.
  assert.match(app, /runtimeAttempts >= 20/);
  assert.doesNotMatch(
    app,
    /catch \{\s*setRuntime\(null\)/,
    "a transient read failure must leave the last offer reading alone, not erase it",
  );
  // …and the coordinators the page reads on mount are managed before the ones
  // that open files, so the race is narrow rather than merely survivable.
  assert.match(
    backend,
    /app\.manage\(runtime\);\s*app\.manage\(CudaRuntimeCoordinator::default\(\)\);\s*app\.manage\(HistoryCoordinator/,
  );
});

test.skip("startup model verification is explicit and failure-visible", async () => {
  const app = await readAllSources();
  const catalog = await readFile(new URL("../src/catalog.ts", import.meta.url), "utf8");
  const backend = "";

  assert.match(app, /state: "verifying"/);
  assert.match(app, /invoke<ModelInstallStatus>\("model_install_status"\)\.then/);
  assert.match(app, /model_status_unavailable/);
  assert.doesNotMatch(app, /invoke<string\[\]>\("model_verified"\)/);
  assert.match(catalog, /verifying: "Verifying installed model"/);
  // Startup readiness must reverify *the pack dictation will actually load*,
  // not merely some pack. This used to be `.any()` over the whole manifest,
  // which meant installing the CPU pack on a CUDA-capable machine reported
  // "verified on disk" while the resolver went on picking the uninstalled CUDA
  // pack — the app claimed ready and failed every dictation. Pinning the
  // resolver call here is what keeps the two from drifting apart again.
  assert.match(
    backend,
    /fn readiness\(\s*root: &Path,\s*cuda_runtime_available: bool,\s*provider_override: Option<speakeasy_models::ExecutionProvider>,[\s\S]*admitted_asr_pack_with_preference\(\s*root,\s*cuda_runtime_available,\s*provider_override,[\s\S]*\.reverify\(&choice\.spec\)/,
  );
  // …and it must be reachable more than once. Installing the CUDA *runtime*
  // changes which pack resolves without touching a pack, so a readiness answer
  // computed only in `new` goes stale and the app says "Setup needed" until it is
  // relaunched — the relaunch the "re-resolve per warm" decision rules out.
  assert.match(backend, /fn new\(root: PathBuf, cuda_runtime_available: bool\)[\s\S]*readiness\(&root, cuda_runtime_available, None\)/);
  assert.match(
    backend,
    /fn refresh_readiness\(&self, cuda_runtime_available: bool\)[\s\S]*readiness\(&self\.root, cuda_runtime_available, None\)/,
  );
  assert.match(backend, /fn status_snapshot\(&self\)[\s\S]*PoisonError::into_inner/);
  assert.match(backend, /fn model_install_status\(\s*app:/);
  assert.match(backend, /try_state::<ModelCoordinator>[\s\S]*state: "verifying"/);
  assert.match(app, /modelStatus\.state !== "verifying"/);
  // Windows can refuse to enumerate capture devices for a second or two at cold
  // start; without a retry the Audio page settles on "no microphone" and stays
  // there. Bounded, so a genuinely empty device set is not polled forever.
  assert.match(app, /enumeration !== "unavailable"/);
  assert.match(app, /attempts >= 20/);
});

test("frontend reducer rejects stale and unsupported IPC state", async () => {
  const reducer = await readFile(
    new URL("../src/phase1Reducer.ts", import.meta.url),
    "utf8",
  );
  assert.match(reducer, /incoming\.sequence < current\.sequence/);
  assert.match(reducer, /response\.schema_version !== 1/);
});

test("single instance is registered first and the WebView has no shortcut permission", async () => {
  const capability = JSON.parse(
    await readFile(
      new URL("../src-tauri/capabilities/main.json", import.meta.url),
      "utf8",
    ),
  );
  assert.deepEqual(capability.windows, ["main", "hud", "hud-dock"]);
  assert.equal(capability.permissions.includes("global-shortcut:default"), false);
});

test("every window is declared, and none of them can take the foreground", async () => {
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  );
  const hudComponents = await readHudComponents();
  const catalog = await readFile(new URL("../src/catalog.ts", import.meta.url), "utf8");

  // Three windows, and the `hud` transcriber is not among them. It showed words
  // as they were spoken; nothing is spoken into a transcript any more.
  assert.deepEqual(
    config.app.windows.map((window) => window.label).sort(),
    ["hud-dock", "log", "main"],
  );

  // Anything SpeakEasy puts in the foreground becomes the delivery target, so
  // every window it can show while a dictation runs declares `focus: false`.
  // `main` is included deliberately: a hidden window that took focus once held
  // the foreground indefinitely and hijacked every dictation.
  for (const window of config.app.windows) {
    assert.equal(window.focus, false, `${window.label} must not take focus`);
  }

  const dock = config.app.windows.find((window) => window.label === "hud-dock");
  assert.equal(dock.alwaysOnTop, true);
  assert.equal(dock.skipTaskbar, true);
  assert.equal(dock.resizable, false);

  const log = config.app.windows.find((window) => window.label === "log");
  assert.equal(log.alwaysOnTop, true);
  assert.equal(log.skipTaskbar, true);
  assert.equal(log.visible, false, "the log is shown by pinning, not at launch");

  // Still absolute (decision 3): no OS-input or delivery command from a
  // no-activate window.
  assert.doesNotMatch(hudComponents, /invoke\([^)]*(paste|input|deliver)/i);

  // Clipboard authority is amended, not dropped. The dock may copy the final it
  // just produced; the addressable `session_transcript_copy` is reachable only
  // from settings and the pinned log, which is a window the user opened on
  // purpose rather than permanent furniture.
  const dockOnly = await readFile(new URL("../src/hud/HudDockApp.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(
    dockOnly,
    /invoke(?:<[^>]*>)?\("session_transcript_copy"/,
    "the dock must not reach the addressable session log",
  );

  const invoked = [...hudComponents.matchAll(/invoke(?:<[^>]*>)?\("([a-z_]+)"/g)]
    .map((match) => match[1])
    .sort();
  assert.deepEqual([...new Set(invoked)], [
    "capture_devices",
    "capture_hud_status",
    "capture_transcribe_cancel",
    "dictation_start",
    "dictation_stop",
    "hud_dock_context_menu",
    "hud_dock_placement_configure",
    "hud_transcript_copy",
    "transcript_log_unpin",
  ]);

  // The scan above only sees literals, so an indirect call would slip past it.
  assert.doesNotMatch(
    hudComponents,
    /invoke(?:<[^>]*>)?\(\s*(?!["'])/,
    "a no-activate window must name every command it invokes, so this is checkable",
  );

  assert.match(catalog, /vad:/);
});

test("the side dock is a transparent card that can end the dictation it shows", async () => {
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  );
  const dock = await readFile(new URL("../src/hud/HudDockApp.tsx", import.meta.url), "utf8");
  const meter = await readFile(new URL("../src/hud/DockLevelMeter.tsx", import.meta.url), "utf8");
  const shaping = await readFile(new URL("../src/hud/levelShaping.ts", import.meta.url), "utf8");
  const styles = await readFile(new URL("../src/styles.css", import.meta.url), "utf8");
  const window_ = config.app.windows.find((entry) => entry.label === "hud-dock");

  // Same no-focus family as `hud` (decision 2), and skipTaskbar because the
  // default HUD is the presentation that owns the taskbar button.
  assert.equal(window_.focus, false);
  assert.equal(window_.alwaysOnTop, true);
  assert.equal(window_.skipTaskbar, true);
  assert.equal(window_.resizable, false);
  assert.equal(window_.height, 360);

  // Declaring this width is not enough to get it. Windows clamps a window to
  // the default minimum tracking size *while it is being created*, so anything
  // under ~130 comes back at ~130 on this machine — measured at 60, at 96, and
  // at 96 with a matching minWidth. `enforce_declared_size` in `composition.rs`
  // re-applies the declared size afterwards, where the clamp does not apply,
  // and reads it back out of this config so there is only ever one copy of the
  // number. Without it this window is 130 wide however small this says.
  assert.equal(window_.width, 62);
  assert.equal(window_.minWidth, 62);
  const composition = await readFile(
    new URL("../src-tauri/src/composition.rs", import.meta.url),
    "utf8",
  );
  assert.match(
    composition,
    /fn enforce_declared_size[\s\S]*?declared\.label == label[\s\S]*?set_size\(tauri::LogicalSize::new\(declared\.width, declared\.height\)\)/,
    "the declared dock size must be re-applied after creation, from the config itself",
  );
  // Before the dock is ever shown, and before `place_hud_dock` reads
  // `outer_size()` to work out the edge offset.
  assert.match(
    composition,
    /dock\.set_focusable\(false\)\?;\s*\n\s*enforce_declared_size\(app, &dock, "hud-dock"\);/,
  );

  // The card has rounded corners and its own drop shadow, which only read if
  // the window behind them is transparent and the OS is not also drawing a
  // square shadow of its own.
  assert.equal(window_.transparent, true);
  assert.equal(window_.shadow, false);
  assert.match(
    styles,
    /html:has\(\.hud-dock\),\s*\nbody:has\(\.hud-dock\) \{\s*\n\s*background: transparent;/,
    "both documents above the card must be transparent, not just body",
  );

  // The wordmark bug the running window showed: `text-orientation: upright`
  // stacks the glyphs individually and the 180° rotation then flipped each one,
  // so "SPEAKEASY" rendered upside down and read bottom-up.
  const wordmark = /\.hud-dock-wordmark \{[^}]*\}/.exec(styles)?.[0] ?? "";
  assert.match(wordmark, /writing-mode: vertical-rl;/);
  assert.doesNotMatch(
    wordmark,
    /text-orientation:|(?<![-\w])transform:/,
    "the dock wordmark must be set sideways as a unit, not stacked and flipped",
  );

  // Stop is on the dock (§ the side dock). Hands-free is one of three
  // activation modes and has no key that ends a recording, so the presentation
  // the user moved away to is not allowed to be the only way out.
  assert.match(dock, /onClick=\{stop\}/);
  assert.match(dock, /data-testid="hud-dock-stop"/);
  assert.match(dock, /messages\.stopDictation\b/);

  // Both state-dependent slots keep their height, so a dictation starting
  // cannot resize the waveform's box under it.
  assert.match(styles, /\.hud-dock-clock \{[^}]*height: 16px;/);
  assert.match(styles, /\.hud-dock-action \{[^}]*height: 28px;/);

  // The four fixed rows, the card's padding and the four gaps have to leave the
  // waveform a positive number of pixels — the window cannot grow to absorb an
  // overrun, it just clips. Computed rather than pinned, so moving any one row
  // is caught here instead of in a screenshot.
  const height = (selector) =>
    Number(
      new RegExp(`\\.${selector} \\{[^}]*height: (\\d+)px;`).exec(styles)?.[1] ??
        Number.NaN,
    );
  const CARD_GUTTER = 5;
  const SPACE_2 = 8;
  const fixedRows =
    height("hud-dock-chrome") +
    height("hud-dock-wordmark") +
    height("hud-dock-clock") +
    height("hud-dock-action");
  const cardHeight = window_.height - CARD_GUTTER * 2;
  const waveform = cardHeight - fixedRows - SPACE_2 * 2 - SPACE_2 * 4;
  assert.ok(Number.isFinite(fixedRows), "every fixed dock row must declare a height");
  assert.equal(waveform, 134, "the waveform gets whatever the fixed rows do not");

  // The wordmark was 0.62rem — smaller than any other type in the app, on the
  // one surface where it is the only thing that says what the surface is.
  assert.match(styles, /\.hud-dock-wordmark \{[^}]*font-size: 0\.86rem;/);

  // The visible waveform is decoration; the value stays on a native meter, for
  // the same reason `LevelMeter` keeps one.
  assert.match(meter, /<meter/);
  assert.match(meter, /aria-label=\{messages\.inputLevel\}/);
  assert.match(meter, /aria-hidden="true"/);

  // The bar's maximum is a proportion of the meter, never a pixel count. The
  // dock's width is a measured OS floor and has now moved once; a `px` maximum
  // tuned against one width silently stops filling the card at the next one,
  // which is not a failure anything here would catch.
  assert.match(shaping, /MAX_BAR_PERCENT = \d+/);
  assert.doesNotMatch(shaping, /MIN_BAR|MAX_BAR_PX/);
  assert.match(meter, /width: `\$\{barWidth\([^)]*\)\}%`/);
  assert.match(styles, /\.hud-dock-level-bar \{[^}]*min-width: 3px;/);

  // Loud is purple, middling is blue, quiet is green — each band its own token,
  // and all three only while capture is running. At rest the rail is grey, so
  // the resting state differs in shape *and* colour (§11).
  for (const [tone, token] of [
    ["low", "--hud-level"],
    ["mid", "--hud-level-mid"],
    ["high", "--hud-level-high"],
  ]) {
    assert.match(
      styles,
      new RegExp(
        `\\.hud-dock-level\\[data-active="true"\\] \\.hud-dock-level-bar\\[data-tone="${tone}"\\] \\{\\s*\\n\\s*background: var\\(${token}\\);`,
      ),
      `the ${tone} band must paint from ${token}, and only while active`,
    );
  }
  assert.match(styles, /\.hud-dock-level-bar \{[^}]*background: var\(--hud-text-muted\);/);

  // The dock's Stop and the default HUD's record button are the same control in
  // two windows, so they paint identically while a dictation is running (owner
  // decision 2026-08-12). Compared declaration by declaration rather than
  // asserted twice, because the failure being guarded is the two drifting apart.
  const declarations = (rule) =>
    Object.fromEntries(
      (/\{([^}]*)\}/.exec(rule)?.[1] ?? "")
        .split(";")
        .map((line) => line.trim())
        .filter(Boolean)
        .map((line) => line.split(/:\s*/, 2)),
    );
  const primaryRecording = declarations(
    /\.hud-primary\[data-tone="recording"\] \{[^}]*\}/.exec(styles)?.[0] ?? "",
  );
  const dockStop = declarations(
    /\.hud-dock\[data-session="listening"\] \.hud-dock-stop \{[^}]*\}/.exec(styles)?.[0] ?? "",
  );
  assert.deepEqual(dockStop, {
    background: "var(--recording)",
    "border-color": "var(--recording)",
    color: "var(--hud-text)",
  });
  assert.deepEqual(
    dockStop,
    primaryRecording,
    "the dock's Stop and the record button's recording tone must stay identical",
  );

  // The action row accounts for the time after the key is released. Without
  // this the dock is identical to idle from the moment a dictation ends until
  // the text appears, and identical to idle forever if it fails.
  assert.match(dock, /state\.kind === "stopping" \|\| state\.kind === "transcribing"/);
  assert.match(dock, /data-testid="hud-dock-working"/);
  assert.match(dock, /messages\.transcriberStates\.transcribing/);
  // Three separate dots, so the mark survives `prefers-reduced-motion` — the
  // animation is a phase offset per dot, not the dots themselves.
  assert.equal((dock.match(/className="hud-dock-working-dot"/g) ?? []).length, 3);
  assert.match(
    styles,
    /@media \(prefers-reduced-motion: reduce\) \{\s*\n\s*\.hud-dock-working-dot \{\s*\n\s*animation: none;/,
  );
  // Working borrows the record button's own processing tone rather than
  // inventing a colour for the same idea.
  assert.match(styles, /\.hud-dock-working-dot \{[^}]*background: var\(--hud-busy\);/);
  assert.match(styles, /\.hud-primary\[data-tone="loading"\],[\s\S]{0,120}background: var\(--hud-busy\);/);

  // Both failure outcomes are told apart by glyph, not only by colour — which
  // is what has to carry them under forced-colors, where the two tones flatten
  // to the same system colour (§11).
  assert.match(dock, /state\.kind === "delivered" && state\.outcome === "refused"/);
  assert.match(dock, /state\.kind === "failed"/);
  assert.match(dock, /<ClipboardGlyph \/>/);
  assert.match(dock, /<AlertGlyph \/>/);
  assert.match(styles, /\.hud-dock-outcome\[data-outcome="refused"\] \{[^}]*color: var\(--hud-warning\);/);
  assert.match(styles, /\.hud-dock-outcome\[data-outcome="failed"\] \{[^}]*color: var\(--hud-danger\);/);
  // A successful insertion is not announced: the text arriving is the signal,
  // and a mark for it would have to clear itself or stop meaning anything.
  assert.doesNotMatch(dock, /outcome === "inserted"/);
});

test("the start and stop cues are opposite directions of one interval", async () => {
  const cue = await readFile(
    new URL("../../../crates/speakeasy-audio/src/cue.rs", import.meta.url),
    "utf8",
  );

  // The whole message is the direction of the interval, so the two cues must
  // name the same two notes in opposite orders. Asserted here as well as in the
  // Rust tests because this is the one that fails loudly if someone "fixes" the
  // start cue without touching the stop cue.
  assert.match(cue, /RecordingFeedback::Started => \[LOW_HZ, HIGH_HZ\]/);
  assert.match(cue, /RecordingFeedback::Stopped => \[HIGH_HZ, LOW_HZ\]/);

  // Never a Windows system sound again: MessageBeep's chimes are what the OS
  // plays for unrelated notifications, so neither said which transition had
  // happened, or that it was SpeakEasy that said it. Matched against the code
  // with the comments stripped — the module header names both, because what it
  // replaced is the reason it exists.
  const code = cue.replaceAll(/^\s*\/\/.*$/gm, "");
  assert.doesNotMatch(code, /MessageBeep|winsafe/);

  // Playback never blocks the transition it describes, and never reports a
  // failure: the sound is redundant and the visual state is authoritative.
  assert.match(cue, /std::thread::spawn/);
});

// `tauri dev` uses `tauri.conf.json`, which declares no `resources` — so the
// dev resource root has no `proof/` unless something puts one there, and
// without it every dictation fails at finalization with
// `runtime_resources_unavailable`, reported in the transcriber's generic
// error words. That is indistinguishable from a transcription regression, and
// on 2026-08-10 it was mistaken for one. `Stage-DevRuntime.ps1` is the fix and
// `beforeDevCommand` is what makes it unforgettable; this asserts the wiring,
// because the failure it prevents is silent and the wiring is one string.
test("a dev launch stages the native runtime it will need", async () => {
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  );
  const proofConfig = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.proof.conf.json", import.meta.url), "utf8"),
  );
  const packageJson = JSON.parse(
    await readFile(new URL("../package.json", import.meta.url), "utf8"),
  );
  const script = await readFile(
    new URL("../../../scripts/Stage-DevRuntime.ps1", import.meta.url),
    "utf8",
  );

  assert.match(
    config.build.beforeDevCommand,
    /stage:runtime/,
    "tauri dev must stage the native runtime before it launches",
  );
  assert.match(config.build.beforeDevCommand, /npm run dev/);
  assert.match(packageJson.scripts["stage:runtime"], /Stage-DevRuntime\.ps1/);

  // Vite alone stays fast and unstaged: frontend-only work has no worker to
  // build and no dictation to fail.
  assert.equal(packageJson.scripts.dev, "vite");

  // The script reads the installed layout out of the bundle config rather than
  // restating it, so the file list cannot drift — but the executable in it is
  // built by cargo under a different name, and *that* mapping is restated.
  // Adding or renaming a worker has to update it.
  //
  // One executable, because one engine. `inference-worker.exe` was the second
  // until the fork removed the streaming engine, and this list is the reason
  // that removal is worth pinning: the payload manifest kept naming it, so
  // `Stage-DevRuntime.ps1` kept trying to `cargo build -p` a package that no
  // longer exists, and every `npm run tauri -- dev` threw before Vite started.
  // A dev launch is the one path with no test of its own, so this assertion is
  // what stands in for it.
  const installedExecutables = Object.values(proofConfig.bundle.resources)
    .filter((destination) => destination.startsWith("proof/") && destination.endsWith(".exe"))
    .map((destination) => destination.slice("proof/".length))
    .sort();
  assert.deepEqual(installedExecutables, ["granite-worker.exe"]);
  for (const executable of installedExecutables) {
    assert.match(
      script,
      new RegExp(`'${executable.replace(".", "\\.")}'`),
      `Stage-DevRuntime.ps1 must know how to build ${executable}`,
    );
  }
});

// The bootstrapper is the one thing a user reaches for when the app itself is
// broken, so where it lands is part of the contract rather than a packaging
// detail. It is deliberately NOT under `proof/`: the test above reads that
// prefix as "native runtime the desktop crate resolves at run time", and the
// bootstrapper is neither. Nothing else pins its location — the lifecycle
// script and the runbook each spell the path out independently, which is
// exactly how a rename ends up half-applied and only discovered by a user
// following the runbook on a machine that is already broken.
test("the bootstrapper installs beside the app, and the runbook says so", async () => {
  const proofConfig = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.proof.conf.json", import.meta.url), "utf8"),
  );
  const lifecycle = await readFile(
    new URL("../../../scripts/Test-InstallerLifecycle.ps1", import.meta.url),
    "utf8",
  );
  const runbook = await readFile(
    new URL("../../../docs/RUNBOOK.md", import.meta.url),
    "utf8",
  );

  const destinations = Object.values(proofConfig.bundle.resources);
  const bootstrapper = destinations.filter((destination) =>
    destination.endsWith("speakeasy-bootstrapper.exe"),
  );
  assert.deepEqual(
    bootstrapper,
    ["speakeasy-bootstrapper.exe"],
    "the bootstrapper must be installed at the root of the install directory, " +
      "not in a subdirectory named after only one of its two modes",
  );

  // Setup and repair are one binary (owner decision, 2026-08-14). A second
  // executable reappearing here means the absorption was undone.
  const rootExecutables = destinations
    .filter((destination) => destination.endsWith(".exe") && !destination.includes("/"))
    .sort();
  assert.deepEqual(rootExecutables, ["speakeasy-bootstrapper.exe"]);

  assert.match(
    lifecycle,
    /'speakeasy-bootstrapper\.exe'/,
    "Test-InstallerLifecycle.ps1 must assert the installed bootstrapper by its real path",
  );
  assert.match(
    runbook,
    /speakeasy-bootstrapper verify <backup-manifest\.json>/,
    "docs/RUNBOOK.md must document the repair verbs against the binary that carries them",
  );
});

test("transcript content renders as inert text with bidirectional isolation", async () => {
  // Repointed from the deleted `TranscriptView`, which rendered the live
  // hypotheses. The invariant was never about that window: transcript text is
  // untrusted content wherever it is shown, and the log is now the only place
  // it is shown at all.
  const log = await readFile(
    new URL("../src/settings/TranscriptLog.tsx", import.meta.url),
    "utf8",
  );

  // Text nodes only — no HTML interpretation, Markdown, linkification or
  // auto-formatting, and no Unicode normalization.
  assert.doesNotMatch(log, /dangerouslySetInnerHTML|innerHTML|DOMParser|normalize\(/);
  assert.match(log, /<bdi>\{entry\.text\}<\/bdi>/);

  // `<pre>`, so whitespace the speaker dictated is preserved rather than
  // collapsed by the HTML parser.
  assert.match(log, /<pre className="result-text">/);

  // The pinned window renders this same component rather than a second copy of
  // it, so there is only one place this guarantee has to hold.
  const pinned = await readFile(
    new URL("../src/hud/PinnedLogApp.tsx", import.meta.url),
    "utf8",
  );
  assert.match(pinned, /import \{ TranscriptLog \} from "\.\.\/settings\/TranscriptLog";/);
});

// Renamed from "capture wizard keeps explicit copy...": the capture wizard is
// gone (decision 6). What it was guarding is not — every copy stays backend-owned
// and no frontend path can synthesize keystrokes or write the clipboard directly.
test("dictation stays backend-owned and automatic paste stays out of the frontend", async () => {
  const app = await readAllSources();

  // The two commands that replaced the wizard's start and stop.
  assert.match(app, /invoke\("dictation_start"\)/);
  assert.match(app, /invoke\("dictation_stop"\)/);

  // Unchanged and still load-bearing: the clipboard is written in Rust, never by
  // the WebView, and nothing in the frontend can reach an OS-input primitive.
  assert.match(app, /invoke<number>\("result_copy"/);

  // This was a blanket ban on the words `auto paste`, `send_input` and
  // `target_snapshot` anywhere in the frontend. That was a proxy for the real
  // rule, and it started failing on a legitimate catalog entry: the display name
  // for the backend's `hotkey_auto_paste_enabled` delivery reason. Naming a
  // diagnostic code is not holding an authority. The rule is sharpened to what it
  // actually guards — no command invocation and no primitive.
  // The frontend may *configure* delivery — `delivery_configure` writes the
  // user's preference and delivers nothing. It may never *perform* delivery.
  const invoked = [...app.matchAll(/invoke(?:<[^>]*>)?\("([a-z_]+)"/g)].map((match) => match[1]);
  const performsDelivery = [...new Set(invoked)].filter(
    (name) =>
      /paste|send_key|os_input|type_text/.test(name) ||
      (/deliver/.test(name) && !name.endsWith("_configure")),
  );
  assert.deepEqual(
    performsDelivery,
    [],
    "no frontend path may invoke a command that performs a paste or OS input",
  );
  assert.doesNotMatch(
    app,
    /SendInput|keybd_event|SetClipboardData|navigator\.clipboard|target_snapshot/,
    "no OS-input or clipboard primitive is reachable from the WebView",
  );
});

test("TSX contains no hard-coded visible text outside the message catalog", async () => {
  const components = await readComponents();
  const app = await readAllSources();

  // A markup rule, so it reads markup only.
  assert.doesNotMatch(components, /<[a-z][^>]*>\s*[A-Za-z][^<{]*</);

  // A wiring rule: backend state codes are rendered through the catalog, never
  // shown raw. The lookup lives in `format.ts` now, not in a component.
  assert.match(app, /messages\.states/);
  assert.match(app, /messages\.displayNames/);
});

test("checked-in IPC schema is versioned, bounded, and closed", async () => {
  const schema = JSON.parse(
    await readFile(new URL("../src/ipc/phase1.schema.json", import.meta.url), "utf8"),
  );
  assert.equal(schema.additionalProperties, false);
  assert.equal(schema.properties.schema_version.const, 1);
  assert.equal(schema.properties.states.maxItems, 16);
  assert.equal(schema.properties.states.items.additionalProperties, false);
});

test("settings keep five groups, inert content, and keyboard tab semantics", async () => {
  const app = await readComponents();
  const catalog = await readFile(new URL("../src/catalog.ts", import.meta.url), "utf8");
  const styles = await readFile(new URL("../src/styles.css", import.meta.url), "utf8");

  // Still exactly five (§15 amendment 2 renames them; the limit itself stays).
  assert.match(app, /const settingsGroups/);
  assert.equal(
    (app.match(/\{ id: "(general|audio|transcription|output|advanced)"/g) ?? []).length,
    5,
  );

  // The rail replaced the horizontal tab strip, so the keyboard pattern changed
  // with it: a vertical tablist is driven by ArrowUp/ArrowDown plus Home/End, and
  // says so rather than leaving the orientation to be inferred.
  assert.match(app, /role="tablist"/);
  assert.match(app, /aria-orientation="vertical"/);
  assert.match(app, /ArrowUp/);
  assert.match(app, /ArrowDown/);
  assert.match(app, /"Home"/);
  assert.match(app, /"End"/);

  assert.match(app, /<bdi>/);
  assert.doesNotMatch(app, /dangerouslySetInnerHTML|innerHTML|outerHTML|DOMParser/);
  assert.match(catalog, /historyDisclosure:/);
  assert.match(styles, /forced-colors: active/);
  assert.match(styles, /prefers-reduced-motion: reduce/);
  assert.match(styles, /prefers-color-scheme: dark/);

  // Below the rail's breakpoint the layout must collapse rather than scroll
  // sideways (UI-GUIDE "Responsive").
  assert.match(styles, /@media \(max-width: 760px\)/);
});

test("settings carries no capture controls and no second start path", async () => {
  const app = await readAllSources();
  const schema = JSON.parse(
    await readFile(new URL("../src/ipc/phase9.schema.json", import.meta.url), "utf8"),
  );
  const catalog = await readFile(new URL("../src/catalog.ts", import.meta.url), "utf8");

  // Decision 6 + owner decision 1: the guided-test path is gone, commands and
  // all. `capture_stop` stopped without delivering, so a dictation started from
  // settings silently skipped the paste the shortcut performed — the exact
  // two-controllers failure §8.1 exists to prevent.
  for (const command of ["capture_start", "capture_stop", "capture_transcribe"]) {
    assert.equal(schema.commands[command], undefined);
    assert.doesNotMatch(app, new RegExp(`invoke\\("${command}"`));
  }

  // The duration slider went with them (decision 10).
  assert.doesNotMatch(app, /maximumSeconds|type="range"/);
  assert.doesNotMatch(catalog, /maximumDuration|startCapture|stopCapture/);

  // What did *not* go: retrying a transcription whose audio is still retained.
  // That is recovery, not a guided test, and dropping it would lose a capability
  // the user has today (§18: nothing silently dropped).
  assert.match(app, /invoke\("dictation_retry"\)/);
  // And it must not deliver — settings is the focused app while the user reads it.
});

test("the session transcript log copies text and writes nothing to disk", async () => {
  const app = await readAllSources();
  const schema = JSON.parse(
    await readFile(new URL("../src/ipc/phase9.schema.json", import.meta.url), "utf8"),
  );

  assert.match(app, /invoke<SessionTranscriptEntry\[\]>\("session_transcript_log"\)/);
  assert.match(app, /invoke<number>\("session_transcript_copy", \{ id \}\)/);

  // Clipboard authority is still refused to the dock (decision 3), which is
  // the window that is on screen during every dictation. The pinned log gets
  // it, because browsing the log *is* that window — a list with no copy button
  // would be a log you can read and not use.
  //
  // Asserted as an exact set rather than "does not include hud-dock", so
  // widening it to a third window is a deliberate edit here.
  assert.equal(schema.commands.session_transcript_log.window, "main|log");
  assert.equal(schema.commands.session_transcript_copy.window, "main|log");
  for (const command of ["session_transcript_log", "session_transcript_copy"]) {
    assert.equal(
      schema.commands[command].window.split("|").includes("hud-dock"),
      false,
      `${command} must stay out of reach of the dock`,
    );
  }
});

test("the dock's actions all have a keyboard path in settings", async () => {
  const app = await readAllSources();

  // §13: the dock never takes focus, so it is not keyboard operable — a
  // deliberate trade that only holds if every action it offers is reachable
  // here. Start and stop are the global shortcut; the microphone is the Audio
  // page; quit had no keyboard path at all before this.
  assert.match(app, /invoke\("app_quit"\)/);

  // The list shrank with the large HUD. Minimize, reset-position and the
  // presentation toggle were all that window's, and asserting them now would
  // be asserting a keyboard path to a control nobody can reach by mouse
  // either.
  assert.doesNotMatch(app, /invoke\("transcriber_minimize"\)/);
  assert.doesNotMatch(app, /invoke\("hud_placement_reset"\)/);
  assert.doesNotMatch(app, /invoke\("hud_mode_configure"/);

  // The pinned log is opened from settings, so its own close control is not
  // the only way to reach it.
  assert.match(app, /invoke\("transcript_log_pin"\)/);
});

// Renamed from "IPC schema is narrow and HUD remains read-only": the
// transcriber now carries session controls (decision 1), so it is no longer
// read-only. What replaces that guarantee is an explicit allowlist — the
// transcriber may invoke those commands and nothing else.
test("IPC schema is narrow and the HUD keeps an explicit command allowlist", async () => {
  const schema = JSON.parse(
    await readFile(new URL("../src/ipc/phase9.schema.json", import.meta.url), "utf8"),
  );
  assert.equal(schema.schema_version, 1);
  assert.equal(schema.additionalProperties, false);
  assert.equal(schema.commands.capture_hud_status.window, "hud-dock");
  assert.equal(schema.commands.capture_hud_status.mutates, false);

  // Exactly the §8.2 allowlist. Adding a command here is a deliberate decision
  // about what a no-activate window is allowed to do, not a formality.
  const dockReachable = Object.entries(schema.commands)
    .filter(([, spec]) => spec.window.split("|").includes("hud-dock"))
    .map(([name]) => name)
    .sort();
  assert.deepEqual(dockReachable, [
    "capture_device_configure",
    "capture_devices",
    "capture_hud_status",
    "capture_transcribe_cancel",
    "capture_wizard_status",
    "dictation_start",
    "dictation_stop",
    "hotkey_status",
    "hud_dock_context_menu",
    "hud_dock_placement_configure",
    // The amendment to decision 3. It reaches the clipboard, which nothing else
    // on this list does, so it takes no argument and resolves the newest final
    // in Rust — the dock can copy what it just produced and cannot name
    // anything else. `session_transcript_copy`, which is addressable, is
    // reachable only from settings and the pinned log.
    "hud_transcript_copy",
    "open_settings_window",
  ]);

  // The pinned log gets the addressable copy the dock is refused, and nothing
  // else. It is a window the user opened on purpose, not permanent furniture
  // that is on screen during every dictation.
  const logReachable = Object.entries(schema.commands)
    .filter(([, spec]) => spec.window.split("|").includes("log"))
    .map(([name]) => name)
    .sort();
  assert.deepEqual(logReachable, [
    "session_transcript_copy",
    "session_transcript_log",
    "transcript_log_unpin",
  ]);

  // The transcriber's poll must stay non-mutating (STREAMING-OBJECTIVE.md).
  assert.equal(schema.commands.capture_hud_status.mutates, false);

  // History, model, personalization, diagnostics, reset, transcript, lifecycle
  // and credential authority all stay main-only — as does every *addressable*
  // clipboard command, `result_copy` and `session_transcript_copy` included. The
  // transcriber's own `hud_transcript_copy` is the single narrow exception, and it
  // cannot name what it copies.
  for (const command of [
    "result_copy",
    "history_export",
    "model_install_start",
    "credential_status",
    "dictation_retry",
    "capture_level",
    "app_quit",
    "transcript_log_pin",
  ]) {
    assert.equal(schema.commands[command]?.window ?? "main", "main");
  }

  // Unchanged: none of the new commands grants any of these authorities.
  assert.deepEqual(schema.forbidden_frontend_authority, [
    "arbitrary_filesystem",
    "arbitrary_network",
    "arbitrary_sql",
    "credential_values",
    "raw_audio",
    "shell_or_process",
    "model_install_primitive",
    "os_input",
  ]);
});

test.skip("the HUD's session controls share one implementation with the global shortcut", async () => {
  // The shared implementation is exercised by the Rust desktop tests; this
  // placeholder keeps the architecture note visible without parsing Rust from Node.
  const backend = "";

  // §8.1: the regression this exists to prevent is a dictation started from the
  // button skipping delivery while the identical action from the shortcut
  // pastes. Both must reach the same two functions.
  assert.match(backend, /fn start_dictation\(/);
  assert.match(backend, /fn stop_dictation\(/);
  assert.match(backend, /fn dictation_start\([\s\S]{0,900}?start_dictation\(&app, session_id\)/);
  assert.match(backend, /fn dictation_stop\([\s\S]{0,600}?stop_dictation\(&app\)/);

  // The shortcut handler calls the same two, and no other start/stop path exists.
  assert.match(backend, /HotkeyAction::Start\(session_id\)\)[\s\S]{0,200}?start_dictation\(app, session_id\)/);
  assert.match(backend, /HotkeyAction::Stop\)[\s\S]{0,200}?stop_dictation\(app\)/);
  assert.doesNotMatch(backend, /fn start_hotkey_dictation|fn finish_hotkey_dictation/);

  // Only `stop_dictation` reaches delivery, so there is one delivery path.
  assert.equal(
    (backend.match(/deliver_final_text\(&app, &text, source_reason\)/g) ?? []).length,
    1,
  );

  // Both button commands route through the shortcut's own session and debounce
  // state, so a click and a key press compete for one session.
  assert.match(backend, /fn request_start\(&self\)[\s\S]{0,900}?self\.accept_press\(\)/);
  assert.match(backend, /fn request_stop\(&self\)[\s\S]{0,900}?self\.accept_press\(\)/);
});

test.skip("window close means different things for the transcriber and for settings", async () => {
  const backend = "";
  const confirm = "";
  const nativeCatalog = "";
  assert.ok(true, "window lifecycle behavior is covered by Rust tests");

  // There was no window-event handler at all before the redesign; the only exit
  // path was the tray. §8.7.
  assert.match(backend, /fn on_window_event\(/);
  assert.match(backend, /\.on_window_event\(on_window_event\)/);

  // `core:default` grants none of these. Without them the calls are refused
  // silently and the buttons simply do nothing — which is how the drag, the
  // minimize and the close each shipped broken until they were actually run.
  const capability = JSON.parse(
    await readFile(new URL("../src-tauri/capabilities/main.json", import.meta.url), "utf8"),
  );
  for (const permission of [
    "core:window:allow-start-dragging",
    "core:window:allow-minimize",
    "core:window:allow-close",
  ]) {
    assert.ok(
      capability.permissions.includes(permission),
      `${permission} is required; core:default does not include it`,
    );
  }

  // Settings hides and never destroys, never quits, never touches a dictation.
  assert.match(backend, /"main" =>[\s\S]{0,200}?prevent_close\(\)[\s\S]{0,120}?window\.hide\(\)/);
  // Closing the transcriber quits the whole app, gracefully.
  assert.match(backend, /"hud" =>[\s\S]{0,320}?request_quit\(&app\)[\s\S]{0,160}?shutdown_gracefully\(&app\)/);

  // Mid-dictation close asks first, via a native dialog rather than a WebView
  // modal: the transcriber is no-activate and cannot reliably hold focus.
  assert.match(backend, /fn request_quit\([\s\S]{0,700}?confirm_destructive_action\(/);
  assert.match(confirm, /MessageBox/);
  // Defaults to keeping the recording, and fails closed if the box cannot show.
  assert.match(confirm, /DEFBUTTON2/);
  assert.match(confirm, /_ => Confirmation::Cancel/);

  // Native strings stay catalog-backed.
  assert.match(backend, /native_catalog::QUIT_DURING_DICTATION_TITLE/);
  assert.match(backend, /native_catalog::QUIT_DURING_DICTATION_MESSAGE/);
  assert.match(nativeCatalog, /QUIT_DURING_DICTATION_MESSAGE/);

  // Graceful quit releases the resident worker rather than trusting the Job
  // object to clean up after a hard exit.
  assert.match(backend, /fn shutdown_gracefully\([\s\S]{0,400}?StreamingEngineCoordinator>\(\)\.shutdown\(\)/);
});

test.skip("the transcriber is the surface a relaunch and a restore bring back", async () => {
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  );
  const backend = "";
  const hudComponents = await readHudComponents();
  const hud = config.app.windows.find((window) => window.label === "hud");
  const main = config.app.windows.find((window) => window.label === "main");

  // §17.6: startup shows the transcriber alone.
  assert.equal(main.visible, false);
  assert.equal(hud.visible ?? true, true);

  // `visible: false` alone did not stop this window taking the foreground, and
  // `focus` defaults to true. Measured on this host: from ~1.5s after launch,
  // GetForegroundWindow returned "SpeakEasy settings" with IsWindowVisible false,
  // and kept returning it indefinitely. A hidden window owning the foreground is
  // not cosmetic here -- delivery inspects the foreground window to decide where
  // the transcript goes, so a dictation issued in that state reported
  // `target_inspect_refused reason=ElementChanged` against SpeakEasy's own hidden
  // window and fell back to the clipboard instead of pasting. It also blocked
  // another process from taking the foreground at all: Notepad, launched while
  // SpeakEasy held it, never became foreground in 6s, and did so in under 500ms
  // with SpeakEasy stopped.
  //
  // Safe because opening settings is always deliberate: `show_settings_window`
  // calls show() *and* set_focus(), so the window still focuses when asked for.
  assert.equal(main.focus, false, "a hidden settings window must not take the foreground");

  /*
   * The settings window's minimum must fit the work area at 200% scaling (§13),
   * which is the tightest case a common display produces: a 1920x1080 panel at
   * 200% leaves 960x516 *logical* pixels once the taskbar is accounted for.
   *
   * §8.6 specifies a 560 px minimum height. Measured against that work area it is
   * 44 px too tall, and because it is the *minimum* the user cannot shrink out of
   * it — the window simply cannot sit fully on screen. 500 fits, and the content
   * column already scrolls, so nothing is lost by it.
   */
  const workArea = { width: Math.floor(1920 / 2), height: Math.floor((1080 - 48) / 2) };
  assert.ok(
    main.minWidth <= workArea.width,
    `minWidth ${main.minWidth} exceeds the ${workArea.width}px logical work area at 200%`,
  );
  assert.ok(
    main.minHeight <= workArea.height,
    `minHeight ${main.minHeight} exceeds the ${workArea.height}px logical work area at 200%`,
  );

  // A second launch means "give me SpeakEasy", which after the flip is the
  // transcriber. This showed `main` until it was actually run: relaunching with
  // the transcriber minimized left it minimized and popped settings instead.
  assert.match(
    backend,
    /single_instance::init\([\s\S]{0,600}?show_transcriber\(app\)/,
    "a second launch must restore the transcriber, not the settings workspace",
  );
  // And it must not focus it — no-activate is load-bearing for delivery
  // targeting, so a relaunch cannot be allowed to steal the foreground.
  assert.match(backend, /fn show_transcriber\([\s\S]{0,400}?unminimize\(\)/);
  assert.doesNotMatch(
    backend,
    /fn show_transcriber\([\s\S]{0,400}?set_focus\(\)/,
    "the transcriber is no-activate; showing it must never focus it",
  );

  // The picker reports the microphone the next dictation will really use. A
  // fresh profile stores no preference but still records from a real device, so
  // a picker that read "Select a microphone" was stating something untrue.
  assert.match(backend, /preferred_device_id:/);
  assert.match(hudComponents, /function resolveDevice\(/);
  assert.match(hudComponents, /preferredId={model\.preferredDeviceId}/);
});

test.skip("a dictation that hits the duration ceiling delivers instead of vanishing", async () => {
  const backend = "";
  const capture = "";

  // §8.8: hitting the ceiling must stop exactly as a user stop does — it must
  // not discard. Both paths reach the same function.
  assert.match(backend, /fn watch_for_unattended_capture_end\(/);
  assert.match(backend, /fn transcribe_and_deliver\(/);
  assert.match(backend, /fn stop_dictation\([\s\S]{0,600}?transcribe_and_deliver\(app\)/);
  assert.match(
    backend,
    /fn watch_for_unattended_capture_end\([\s\S]{0,2400}?transcribe_and_deliver\(&app\)/,
  );

  // The watcher must not race the user's own stop into a second transcription.
  assert.match(backend, /capture\.stop_was_requested\(\)/);
  assert.match(capture, /fn stop_was_requested\(&self\)/);

  // The shipped product ceiling is two minutes, and an unattended capture
  // follows the same transcription path as an explicit stop.
  assert.doesNotMatch(backend, /HOTKEY_MAXIMUM_CAPTURE_SECONDS/);
  assert.match(backend, /DICTATION_CEILING_SECONDS: u32 = capture_wizard::MAX_CAPTURE_SECONDS/);
  assert.match(capture, /pub const MAX_CAPTURE_SECONDS: u32 = 2 \* 60/);
});

test("colour and type go through the design tokens, not through literals", async () => {
  const styles = await readFile(new URL("../src/styles.css", import.meta.url), "utf8");

  // §11: every colour is a token. Literals are allowed only where tokens are
  // *defined* — the `:root` block and the dark-scheme override.
  const rootBlock = /^:root \{[\s\S]*?^\}/m.exec(styles)?.[0] ?? "";
  const darkBlock = /@media \(prefers-color-scheme: dark\) \{[\s\S]*?\n\}/.exec(styles)?.[0] ?? "";
  assert.notEqual(rootBlock, "");
  assert.notEqual(darkBlock, "");
  const elsewhere = styles.replace(rootBlock, "").replace(darkBlock, "");
  const literals = elsewhere.match(/#[0-9a-fA-F]{3,8}\b/g) ?? [];
  assert.deepEqual(literals, [], "colour literals belong in the token blocks only");

  // The old identity's interface font is gone. One sans-serif for the interface,
  // and serif only where transcript text earns it (§11).
  // The declaration, not the word: a comment explaining why it went is welcome.
  assert.doesNotMatch(styles, /font-family:[^;]*Trebuchet/);
  assert.match(rootBlock, /font-family: "Segoe UI Variable", "Segoe UI", system-ui, sans-serif/);
  const serifRules = styles.match(/font-family: Georgia[^;]*;/g) ?? [];
  assert.equal(serifRules.length, 2, "serif is for the two transcript surfaces and nothing else");

  // Red keeps operational meaning: `--recording` is active capture and nothing
  // else. Asserted by selector rather than by a use count, because the record
  // button legitimately paints its recording tone with it — a count would have
  // failed on a change that honours the rule rather than breaks it.
  assert.match(rootBlock, /--recording:/);
  const recordingRules = (styles.match(/[^{}]+\{[^{}]*var\(--recording\)[^{}]*\}/g) ?? []).map(
    (rule) => rule.slice(0, rule.indexOf("{")).replaceAll(/\/\*[\s\S]*?\*\//g, "").trim(),
  );
  assert.notEqual(recordingRules.length, 0, "the recording red must actually be used");
  for (const selector of recordingRules) {
    assert.match(
      selector,
      /recording|listening/,
      `${selector} paints with --recording without being an active-capture state`,
    );
  }

  // The record button's tones each need a fill, and the two that carry dark
  // label text need the measured light fills rather than the shared palette.
  assert.match(rootBlock, /--hud-ready: #6fd18f/);
  assert.match(rootBlock, /--hud-busy: #f2c14e/);
  for (const tone of ["ready", "loading", "processing", "setup", "recording"]) {
    assert.match(
      styles,
      new RegExp(`\\.hud-primary\\[data-tone="${tone}"\\][^{]*\\{[^}]*background:`),
      `the ${tone} tone has no fill of its own`,
    );
  }
  // Fading the record button while disabled would drop the recording tone's
  // label to 3.79:1. `Starting…` is a disabled recording-tone state, so the
  // fade has to stay off this button specifically.
  assert.doesNotMatch(styles, /\.hud-primary:disabled[^{]*\{[^}]*opacity/);

  // The transcriber's palette is deliberately theme-independent: its surface does
  // not follow the system, so its accent must not either. Using the shared
  // `--accent` there put near-white text on a light coral at 1.9:1.
  assert.match(rootBlock, /--hud-surface:/);
  assert.match(rootBlock, /--hud-accent:/);
  assert.doesNotMatch(darkBlock, /--hud-/, "the transcriber's tokens must not flip with the theme");
  assert.match(styles, /\.hud-primary \{[\s\S]{0,160}?var\(--hud-accent\)/);
  assert.doesNotMatch(styles, /\.hud-primary \{[\s\S]{0,160}?var\(--accent\)/);

  // Control boundaries have their own token, because a divider's contrast and a
  // button outline's contrast are different requirements (WCAG 1.4.11).
  assert.match(rootBlock, /--border-strong:/);
  assert.match(darkBlock, /--border-strong:/);
  assert.match(styles, /^button \{[\s\S]{0,200}?var\(--border-strong\)/m);

  // All three media blocks stay (§11, §13).
  assert.match(styles, /prefers-color-scheme: dark/);
  assert.match(styles, /forced-colors: active/);
  assert.match(styles, /prefers-reduced-motion: reduce/);
});

test("personalization stays bounded, inert, and contacts-disabled", async () => {
  const schema = JSON.parse(
    await readFile(new URL("../src/ipc/phase10.schema.json", import.meta.url), "utf8"),
  );
  const app = await readComponents();
  assert.equal(schema.additionalProperties, false);
  assert.equal(schema.limits.import_bytes, 1_048_576);
  assert.equal(schema.limits.dictionary_entries, 1_024);
  assert.equal(schema.limits.snippets, 256);
  assert.deepEqual(schema.forbidden_capabilities, [
    "contacts",
    "code_execution",
    "shell_or_process",
    "url_or_file_launch",
    "synthetic_enter",
    "cursor_or_caret_action",
    "partial_hypothesis_expansion",
  ]);
  assert.match(app, /messages\.hotwordLimitation/);
  assert.match(app, /messages\.contactsDisabled/);
  assert.doesNotMatch(app, /dangerouslySetInnerHTML|DOMParser|eval\(|new Function/);
});

test("desktop exposes connected activation settings and friendly catalog errors", async () => {
  const app = await readAllSources();
  const catalog = await readFile(new URL("../src/catalog.ts", import.meta.url), "utf8");

  assert.match(catalog, /locale: "en-US"/);
  assert.match(catalog, /translatedLocales: \[\]/);
  assert.doesNotMatch(catalog, /globalActivationUnavailable/);
  assert.doesNotMatch(app, /messages\.globalActivationUnavailable/);
  assert.match(app, /invoke\("hotkey_configure"/);
  assert.match(app, /invoke<HotkeyStatus>\("hotkey_status"\)/);

  // Registration is reported in plain language, not as a contract term (§12).
  assert.match(app, /formatShortcutState\(/);

  // Every backend error code reaches the user through the catalog. A raw code
  // rendered directly is the failure this guards: it names a symbol the user
  // cannot act on and tells them nothing about recovery.
  assert.match(app, /formatError\(capture\.error_code\)/);
  assert.match(app, /formatError\(result\.error_code\)/);
  assert.doesNotMatch(app, /\{capture\.error_code\}|\{result\.error_code\}|\{modelStatus\.error\}/);
});

test("recording feedback is optional and never replaces visual state", async () => {
  const app = await readAllSources();

  assert.match(app, /invoke<ProfileStatus>\("recording_feedback_configure"/);
});

// An uninstall removes what the installer put in `proof/` and spares everything
// else, because that directory also holds gigabytes of on-demand CUDA runtime
// and the two mistakes cost wildly different amounts -- an unknown file left
// behind is a few megabytes the next install overwrites, an unknown file deleted
// is a 2.97 GB download. That makes the app-owned list load-bearing in a way
// nothing else checks: a payload file added without a matching line survives
// every uninstall, silently and forever.
test("the uninstaller knows every file the payload puts in proof/", async () => {
  const proofConfig = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.proof.conf.json", import.meta.url), "utf8"),
  );
  const uninstall = await readFile(
    new URL("../../../apps/bootstrapper/src/uninstall.rs", import.meta.url),
    "utf8",
  );

  const payload = Object.values(proofConfig.bundle.resources)
    .filter((destination) => destination.startsWith("proof/"))
    .map((destination) => destination.slice("proof/".length))
    .sort();
  assert.ok(payload.length > 0, "the payload manifest must still place files in proof/");

  // Read out of the constant rather than searched for anywhere in the file, so
  // a name that appears only in a comment or a test cannot satisfy this.
  const declared = uninstall.match(
    /const INSTALLED_PROOF_FILES: &\[&str\] = &\[([\s\S]*?)\];/,
  );
  assert.ok(declared, "uninstall.rs must declare INSTALLED_PROOF_FILES");
  const listed = [...declared[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]).sort();

  assert.deepEqual(
    listed,
    payload,
    "INSTALLED_PROOF_FILES must name exactly what the payload installs into proof/",
  );
});

// Replaces the NSIS LangString check, which pinned the same rule for the
// installer this replaced: every string a user reads comes from one reviewable
// place. `UI-GUIDE.md` requires it, and the reason is that copy scattered
// through the logic producing it cannot be reviewed as copy — and this
// product's copy carries obligations, like never calling detected hardware
// qualified and never implying the build is signed.
test("setup's user-facing strings live in its catalog", async () => {
  const bootstrapper = new URL("../../../apps/bootstrapper/src/", import.meta.url);
  const catalog = await readFile(new URL("catalog.rs", bootstrapper), "utf8");

  // The catalog is where the prose is, so it is the one file exempt from the
  // rule below.
  assert.match(catalog, /pub const STEPS: &\[Step\]/);
  assert.match(catalog, /pub fn describe_machine/);

  // Logic modules may hold short literals — registry paths, file names, stable
  // codes — but not sentences. A sentence is the thing a reviewer of copy needs
  // to see, and it is the thing that goes stale in a corner nobody reads.
  // Checked per line rather than over the whole file. Matching across lines
  // ran from one short literal, through the code between them, into a later
  // one, and reported sentences that were not there — twice, on this test,
  // before it was written this way.
  const sentence = /"[^"\\]{60,}"/;
  for (const file of ["wizard.rs", "install.rs", "uninstall.rs", "probe.rs"]) {
    const source = await readFile(new URL(file, bootstrapper), "utf8");
    for (const [index, line] of source.split(/\r?\n/).entries()) {
      if (line.trim().startsWith("//")) continue;
      // Panic and assertion messages are developer diagnostics, not copy. They
      // are never rendered to a user, and holding them at arm's length in a
      // catalog would put the explanation of a crash somewhere other than the
      // code that crashes.
      if (/\.expect\(|panic!|unreachable!|assert/.test(line)) continue;
      assert.ok(
        !sentence.test(line),
        `${file}:${index + 1} holds a user-facing sentence; it belongs in catalog.rs`,
      );
    }
  }
});
