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
 * whole UI lived in one file. Once it was split up, reading one file would let a
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

/**
 * Every `.rs` in the workspace, as `[repo-relative path, body]` pairs, skipping
 * `target/`.
 *
 * Pairs rather than one concatenation because the rules that need this are about
 * *where* something appears, not merely whether it does — the single-spawn rule
 * below has to be able to name the file that broke it.
 */
async function readAllRustSources() {
  const root = fileURLToPath(new URL("../../../", import.meta.url));
  const entries = await readdir(root, { recursive: true, withFileTypes: true });
  const files = entries
    .filter((entry) => entry.isFile() && entry.name.endsWith(".rs"))
    .map((entry) => join(entry.parentPath, entry.name))
    .filter((file) => !file.slice(root.length).replace(/\\/g, "/").startsWith("target/"));
  return Promise.all(
    files.map(async (file) => [
      file.slice(root.length).replace(/\\/g, "/"),
      await readFile(file, "utf8"),
    ]),
  );
}

/**
 * Every `.rs` in the desktop crate, concatenated.
 *
 * The rules that want this are about the *crate*, not a file: `lib.rs`
 * `include!`s its siblings into one namespace, so a function can move between
 * them without anything changing. Seven tests read `const backend = ""` instead
 * from the fork until 2026-08-28, which is not a weaker version of this — a
 * `match` against the empty string fails, so those tests could only ever have
 * been skipped.
 */
async function readDesktopBackend() {
  const root = fileURLToPath(new URL("../src-tauri/src/", import.meta.url));
  const entries = await readdir(root, { recursive: true, withFileTypes: true });
  const files = entries
    .filter((entry) => entry.isFile() && entry.name.endsWith(".rs"))
    .map((entry) => join(entry.parentPath, entry.name));
  const sources = await Promise.all(files.map((file) => readFile(file, "utf8")));
  return sources.join("\n");
}

/**
 * The body of a Rust `fn` by name, brace-balanced.
 *
 * Not a character window. `watch_for_unattended_capture_end` was asserted to
 * reach `transcribe_and_deliver` within 2,400 characters of its head; the
 * function grew to 3,878 when the notice window landed on 2026-08-25, and the
 * assertion would have failed on a function that was still correct. A window is
 * a guess about how long a function is allowed to get.
 */
function rustFunctionBody(source, name) {
  const declaration = new RegExp(String.raw`\bfn\s+${name}\b`).exec(source);
  if (declaration === null) return null;
  const brace = source.indexOf("{", declaration.index);
  if (brace === -1) return null;
  return balancedFrom(source, brace, "{", "}");
}

/** Every `.tsx` in `src/`. Use for rules about **markup**. */
function readComponents() {
  return readSources([".tsx"]);
}

/**
 * Every `.ts` and `.tsx` in `src/`. Use for rules about **wiring** — which
 * commands are invoked, which catalog strings are read.
 *
 * The settings rewrite moved the profile mutators and the catalog lookups out
 * of components and into `useProfile.ts` and `format.ts`, and six assertions that
 * read only `.tsx` stopped seeing the thing they were guarding. That is the same
 * failure the tsx-only reader was introduced to prevent, one directory deeper: a
 * guarantee moves into a new file and quietly stops being checked.
 */
function readAllSources() {
  return readSources([".ts", ".tsx"]);
}

/**
 * Every `.ts` and `.tsx` under `src/` as `[src-relative path, body]` pairs.
 *
 * Pairs rather than one concatenation because the startup-race rule below is
 * about *where* a read lives: a command read through the retry in one file and
 * bare in another is the exact state that let one defect recur in a second
 * location, and a concatenation cannot tell the two apart.
 */
async function readFrontendSources() {
  const root = fileURLToPath(new URL("../src/", import.meta.url));
  const entries = await readdir(root, { recursive: true, withFileTypes: true });
  const files = entries
    .filter(
      (entry) => entry.isFile() && (entry.name.endsWith(".ts") || entry.name.endsWith(".tsx")),
    )
    .map((entry) => join(entry.parentPath, entry.name));
  return Promise.all(
    files.map(async (file) => [
      file.slice(root.length).replace(/\\/g, "/"),
      await readFile(file, "utf8"),
    ]),
  );
}

/**
 * The source between the delimiter at `start` and its match, exclusive.
 *
 * Counts delimiters without lexing strings or comments, which is sound only
 * because every construct it is pointed at is balanced. The instrument
 * self-checks in the startup-race test exist to notice if that stops being true,
 * rather than to assume it: a scanner that silently stops matching reads exactly
 * like a tree with nothing left to find.
 */
function balancedFrom(source, start, open, close) {
  let depth = 0;
  for (let index = start; index < source.length; index += 1) {
    if (source[index] === open) depth += 1;
    else if (source[index] === close) {
      depth -= 1;
      if (depth === 0) return source.slice(start + 1, index);
    }
  }
  return null;
}

/** The argument list of every `useEffect(...)` call in one file. */
function effectBodies(source) {
  const bodies = [];
  const marker = /useEffect\(/g;
  let match;
  while ((match = marker.exec(source)) !== null) {
    const body = balancedFrom(source, match.index + "useEffect".length, "(", ")");
    if (body !== null) bodies.push(body);
  }
  return bodies;
}

/**
 * The body of a function declared in this file, by name.
 *
 * Covers the three forms this tree uses — `function f()`, `const f = () =>` and
 * `const f = useCallback(() =>` — because the read that mattered most was behind
 * one: `refreshCatalog` holds `model_catalog` and `gpu_status`, and the mount
 * effect only calls it. A rule that read effect bodies alone would have passed
 * the page that renders "no models exist" out of an error path.
 */
function localFunctionBody(source, name) {
  const declaration = new RegExp(String.raw`(?:async\s+function|function|const)\s+${name}\b`).exec(
    source,
  );
  if (declaration === null) return null;
  const brace = source.indexOf("{", declaration.index);
  if (brace === -1) return null;
  return balancedFrom(source, brace, "{", "}");
}

/** Command names passed to `invoke` or to `readWithRetry` inside one snippet. */
function commandsIn(snippet, caller) {
  const names = [];
  const marker = new RegExp(String.raw`${caller}(?:<[^>(]*>)?\(\s*"([a-z_0-9]+)"`, "g");
  let match;
  while ((match = marker.exec(snippet)) !== null) names.push(match[1]);
  return names;
}

/** Identifiers called as bare functions inside one snippet. */
function calleesIn(snippet) {
  const names = new Set();
  const marker = /(?:^|[^.\w$])([a-zA-Z_$][\w$]*)\s*\(/g;
  let match;
  while ((match = marker.exec(snippet)) !== null) names.add(match[1]);
  return [...names];
}

/**
 * Every `#[tauri::command]` whose signature takes a `tauri::State`, read out of
 * the Rust source rather than listed here.
 *
 * Derived, because the previous version of the startup-race rule named one
 * command in one file — and the defect it was written for then recurred in a
 * second file, with a second command, while the test stayed green. A `State`
 * parameter is precisely what Tauri refuses when the coordinator behind it is
 * not managed yet ("state not managed for field `state` on command …"), so this
 * list is the hazard itself rather than a record of where it has been seen.
 */
async function raceProneCommands() {
  const root = new URL("../src-tauri/src/commands/", import.meta.url);
  const names = await readdir(root);
  const commands = new Set();
  for (const name of names.filter((file) => file.endsWith(".rs"))) {
    const source = await readFile(new URL(name, root), "utf8");
    const marker = /#\[tauri::command\]\s*\n(?:#\[[^\]]*\]\s*\n)*fn\s+(\w+)\s*\(/g;
    let match;
    while ((match = marker.exec(source)) !== null) {
      const signature = balancedFrom(source, marker.lastIndex - 1, "(", ")");
      if (signature !== null && signature.includes("tauri::State<")) commands.add(match[1]);
    }
  }
  return commands;
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
  // Unconditional on release now. It used to carry a `not(feature =
  // "proof-mode")` arm so a smoke-test build could keep a console for its
  // eprintln! output; that feature was removed on 2026-08-19 having never been
  // built by any script, and this pattern narrowed with it. The invariant is
  // unchanged — a release build must declare the attribute — so only the shape
  // being matched moved.
  const main = await readFile(new URL("../src-tauri/src/main.rs", import.meta.url), "utf8");
  assert.match(
    main,
    /#!\[cfg_attr\(not\(debug_assertions\), windows_subsystem = "windows"\)\]/,
    "release builds must declare windows_subsystem so no console is allocated",
  );
  // And nothing re-introduces a configuration that opts back out of it.
  assert.doesNotMatch(main, /proof-mode"\)\s*\),\s*windows_subsystem/);

  // The other half of the same defect. The workers are console binaries, so a
  // windowed parent leaves them no console to inherit and Windows gives each its
  // own visible window — which then takes the foreground and becomes the delivery
  // target. Fixing only the attribute above traded one stray console for one per
  // worker, so both assertions belong together.
  //
  // The spawn moved to `speakeasy-windows` on 2026-08-19 so `apps/bootstrapper`
  // could run setup's smoke test through it rather than writing a second one.
  // This assertion followed it deliberately: a second spawn is exactly how the
  // flag goes missing again, and it is checked here because this is the suite
  // that already owns the foreground-target rule.
  const worker = await readFile(
    new URL("../../../crates/speakeasy-windows/src/worker_process.rs", import.meta.url),
    "utf8",
  );
  assert.match(
    worker,
    /CREATE_NO_WINDOW: u32 = 0x0800_0000;\s*command\.creation_flags\(CREATE_NO_WINDOW\)/,
    "workers must be spawned with CREATE_NO_WINDOW or each opens a console window",
  );

  // And there is still only one place that sets it. A second would be a second
  // place for it to be forgotten, and the symptom is a dictation delivered into
  // a console window rather than an error.
  //
  // `CREATE_NO_WINDOW` specifically, not `creation_flags`: the bootstrapper sets
  // `DETACHED_PROCESS` in `relaunch_detached` for a different job — re-launching
  // *itself* to draw the wizard while the parent exits so a script's capture
  // returns. That one is console-free by having no console at all, which is why
  // it is a different constant and why this rule must not sweep it up.
  const rustSources = await readAllRustSources();
  const noWindowSites = rustSources
    .filter(([, body]) => /creation_flags\(CREATE_NO_WINDOW\)/.test(body))
    .map(([name]) => name);
  assert.deepEqual(
    noWindowSites,
    ["crates/speakeasy-windows/src/worker_process.rs"],
    "every worker spawn must go through the one function that sets CREATE_NO_WINDOW",
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

test("nothing slow or blocking sits between the key press and the microphone", async () => {
  const backend = await readDesktopBackend();
  const target = await readFile(
    new URL("../../../crates/speakeasy-windows/src/target.rs", import.meta.url),
    "utf8",
  );

  // Instrument self-check. Every assertion here is a `doesNotMatch` or a match
  // on source, and both report the same thing about a file that was not read as
  // about a file that is correct. This test spent from the fork to 2026-08-28
  // reading `const backend = ""`, where `doesNotMatch` passes vacuously.
  assert.match(backend, /fn on_window_event\(/, "the desktop backend did not load");
  assert.match(target, /INSPECT_DEADLINE/, "target.rs did not load");

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

  // Four assertions stood here about the streaming engine's non-blocking warm:
  // `try_ready` against `ensure_ready` in `build_capture_tap`, and that live and
  // delivered selection asked the provider question the same way rather than
  // reading the ONNX provider DLL. Every symbol in them left with the streaming
  // engine, and there is no live text to delay a recording for now — Granite
  // runs once, after the recording stops. Restoring them was not possible; the
  // remaining assertions are the ones that still describe this product.

  // A UIA call reaches into another process and can hang there. An unbounded
  // recv took the caller with it.
  assert.match(target, /recv_timeout\(Self::INSPECT_DEADLINE\)/);
  assert.doesNotMatch(
    target,
    /result\s*\n?\s*\.recv\(\)/,
    "inspect must not wait without a deadline",
  );
});

test("startup model verification is explicit and failure-visible", async () => {
  const app = await readAllSources();
  const catalog = await readFile(new URL("../src/catalog.ts", import.meta.url), "utf8");
  const backend = await readDesktopBackend();

  // Instrument self-check: this read `const backend = ""` from the fork until
  // 2026-08-28, where every `match` below fails and every `doesNotMatch` passes.
  assert.match(backend, /fn model_install_status\(/, "the desktop backend did not load");

  assert.match(app, /state: "verifying"/);
  assert.match(app, /invoke<ModelInstallStatus>\("model_install_status"\)\.then/);
  assert.match(app, /model_status_unavailable/);
  assert.doesNotMatch(app, /invoke<string\[\]>\("model_verified"\)/);
  assert.match(catalog, /verifying: "Verifying installed model"/);
  // Startup readiness must resolve *the pack dictation will actually load*, not
  // merely some pack. This used to be `.any()` over the whole manifest, which
  // meant installing the CPU pack on a CUDA-capable machine reported "verified
  // on disk" while the resolver went on picking the uninstalled CUDA pack — the
  // app claimed ready and failed every dictation. Pinning the resolver call
  // here is what keeps the two from drifting apart again.
  const readiness = rustFunctionBody(backend, "readiness");
  assert.ok(readiness, "fn readiness must be findable");
  assert.match(readiness, /granite_selection\(/);
  // …and it must do that **without hashing**. `readiness` runs twice on a
  // configured launch — once synchronously inside `ModelCoordinator::new` on
  // the `setup` path, once after the warm — so a `reverify` here was two full
  // reads of the 2.30 GB pack, and with the warm's own `verify_pack_files` a
  // launch took three, about 6.90 GB, before the app was usable. The warm's is
  // the one worth keeping: it is taken immediately before the worker is handed
  // the `model_root`, and it already runs on its own thread.
  assert.doesNotMatch(readiness, /\.reverify\(/);
  assert.match(readiness, /\.is_present\(&selection\.install_spec\)/);
  // Presence reports `installed_unverified`, never `verifying` and never
  // `verified_on_disk`. Both of those are claims about bytes nobody has read:
  // `verifying` says a pass is running, and readiness runs on the `setup` path
  // where none is, which left the model line saying "Verifying installed model"
  // for the life of the process. `verified_on_disk` is the warm's to award.
  assert.match(readiness, /"installed_unverified"/);
  assert.doesNotMatch(readiness, /"verifying"/);
  assert.doesNotMatch(readiness, /"verified_on_disk"/);
  // The promotion is guarded on the pack the digest pass actually hashed. The
  // warm is what discovers whether the worker is CUDA-capable, and that answer
  // changes which pack resolves, so "the warm said ready" is not "these bytes
  // were checked" — matching on the id alone would promote the same pack at a
  // different revision.
  assert.match(
    backend,
    /id == identity\.0 && revision == identity\.1/,
  );
  assert.match(backend, /WarmVerification::Verified \{ \.\. \} => \("verified_on_disk", None\)/);
  // …and readiness must be reachable more than once. The CUDA worker's
  // availability changes which pack resolves without touching a pack, so an
  // answer computed only in `new` goes stale and the app says "Setup needed"
  // until it is relaunched — the relaunch the "re-resolve per warm" decision
  // rules out.
  assert.match(backend, /fn new\(root: PathBuf, cuda_worker_available: bool\)[\s\S]{0,200}?readiness\(&root, cuda_worker_available\)/);
  assert.match(
    backend,
    /fn settle_after_warm\([\s\S]{0,200}?readiness\(&self\.root, cuda_worker_available\)/,
  );
  // An installed pack is installed whether or not its bytes have been read, and
  // the dock must not call either "Setup needed". A pass that is genuinely
  // running gets its own reason, so the refusal says what is happening and
  // clears itself.
  assert.match(backend, /"verified_on_disk" \| "installed_unverified" => \{\}/);
  assert.match(backend, /"verifying" => return Ok\(Some\("model_verifying"\)\)/);
  // The same refusal reaches the global shortcut, through the one function both
  // controllers share — two copies of this rule is how `can_start` came to
  // refuse a press the shortcut accepted.
  assert.match(backend, /fn start_dictation[\s\S]{0,2500}?return Err\("model_verifying"\)/);
  assert.match(app, /model_verifying/);
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
  // `hud` left with the large transcriber and `log` was never added, so this
  // list named a window that does not exist and omitted one that does. In
  // Tauri v2 a window matched by no capability gets no permissions at all, so
  // the pinned log's own close button was calling `transcript_log_unpin`
  // against a capability that did not cover it. Corrected 2026-08-25 alongside
  // `notice`, which needs `core:default` to receive its event and to call its
  // dismiss command.
  assert.deepEqual(capability.windows, ["main", "hud-dock", "log", "notice"]);
  assert.equal(capability.permissions.includes("global-shortcut:default"), false);
});

test("every window is declared, and none of them can take the foreground", async () => {
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  );
  const hudComponents = await readHudComponents();
  const catalog = await readFile(new URL("../src/catalog.ts", import.meta.url), "utf8");
  const styles = await readFile(new URL("../src/styles.css", import.meta.url), "utf8");

  // Four windows, and the `hud` transcriber is not among them. It showed words
  // as they were spoken; nothing is spoken into a transcript any more.
  //
  // `notice` joined on 2026-08-25: the dock is 62 px wide and cannot say
  // "your recording hit the two-minute maximum", and a Windows toast was
  // specified and rejected because it displays nothing while reporting success
  // when it has no AUMID.
  assert.deepEqual(
    config.app.windows.map((window) => window.label).sort(),
    ["hud-dock", "log", "main", "notice"],
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

  // The notice is shown *while a transcript is being delivered*, which is the
  // one moment `deliver_final_text` is reading the foreground window. Its
  // `focus: false` is asserted with every other window above; these are the
  // rest of the properties that keep it out of the way.
  const notice = config.app.windows.find((window) => window.label === "notice");
  assert.equal(notice.alwaysOnTop, true);
  assert.equal(notice.skipTaskbar, true);
  assert.equal(notice.resizable, false);
  assert.equal(notice.visible, false, "the notice is shown by the ceiling, not at launch");

  // The declared size and the stylesheet's account of it, compared. The notice
  // shipped at 360x172 needing 188 CSS px, so its only control sat 16 px below
  // the fold behind a scrollbar, on every machine, and nothing could see it:
  // `height: 100vh` with `justify-content: space-between` describes a box that
  // looks correctly filled at any content height. This cannot measure the running
  // window -- only CDP can, and the raise to 192 was confirmed there at zero
  // overflow with the button 2 px clear of the fold, not the 4 px the estimate
  // predicted -- but it can stop the stylesheet's own comment from drifting
  // away from the config it claims to match, which is how the wrong number
  // would next be believed.
  assert.match(
    styles,
    new RegExp(`The capture-limit notice[\\s\\S]*?\\n   ${notice.width}x${notice.height} logical`),
    "styles.css must name the notice's declared size, and the same one",
  );
  assert.equal(
    notice.minHeight,
    notice.height,
    "a non-resizable window whose minimum disagrees with its size has two answers",
  );
  assert.equal(notice.minWidth, notice.width);

  // Still absolute: no OS-input or delivery command from a
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
    // `capture_devices` was here for `MicPicker`, the large HUD's device list.
    // The component was deleted on 2026-08-28 and both device commands went
    // main-only with it: a window that no longer enumerates devices should not
    // keep the authority to.
    "capture_hud_status",
    // Added 2026-08-25 with the notice window. It takes a warning off the
    // user's screen and does nothing else -- no delivery, no OS input, no
    // transcript -- and the Rust side refuses it from any window but `notice`.
    "capture_notice_dismiss",
    "capture_transcribe_cancel",
    "dictation_start",
    "dictation_stop",
    "hud_dock_context_menu",
    "hud_dock_placement_configure",
    "hud_transcript_copy",
    // Added 2026-08-28 with the dock's gear button. No new authority: the same
    // command was already reachable from this window through
    // `hud_dock_context_menu`'s Settings item, and the Rust side has always
    // gated it on `require_main_or_hud_window`. What changed is that a control
    // the user can see now does it, instead of only a native popup they had to
    // guess to right-click for. It touches window state and nothing else, by
    // contract, so it cannot disturb a dictation in flight.
    "open_settings_window",
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

  // The dock never takes keyboard focus, and skipTaskbar because a window that
  // cannot be activated has no use for a taskbar button. It shared the rule
  // with the `hud` transcriber until that window left with the large HUD.
  assert.equal(window_.focus, false);
  assert.equal(window_.alwaysOnTop, true);
  assert.equal(window_.skipTaskbar, true);
  assert.equal(window_.resizable, false);
  // 400 since 2026-08-28, from 360. The 40px pays for the action row's button
  // being present in every state; the meter is the only `1fr`, so it took all of
  // it and went 112 -> 152.
  assert.equal(window_.height, 400);
  assert.equal(window_.minHeight, window_.height);

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

  // Stop is on the dock (UI-GUIDE "Main window and focus"). Hands-free is one of three
  // activation modes and has no key that ends a recording, so the presentation
  // the user moved away to is not allowed to be the only way out.
  assert.match(dock, /onClick=\{onStop\}/);
  assert.match(dock, /data-testid="hud-dock-stop"/);
  assert.match(dock, /messages\.stopDictation\b/);

  // And Start is on it too, since 2026-08-28. The button is present in every
  // state, so the dock can begin a dictation as well as end one — it previously
  // rendered only while listening, which left the surface whose whole promise is
  // staying reachable offering a control that appeared once you no longer needed
  // to find it.
  assert.match(dock, /onClick=\{onStart\}/);
  assert.match(dock, /data-testid="hud-dock-start"/);
  assert.doesNotMatch(
    dock,
    /\{listening && \(/,
    "the dock's button must not be conditional on listening: it is present in every state",
  );

  // The gear opens Settings without going through the right-click menu, which is
  // discoverable only by someone who already guessed to try it.
  assert.match(dock, /data-testid="hud-dock-settings"/);
  assert.match(dock, /invoke\("open_settings_window"\)/);

  // Both state-dependent slots keep their height, so a dictation starting
  // cannot resize the waveform's box under it.
  assert.match(styles, /\.hud-dock-status \{[^}]*height: 16px;/);
  assert.match(styles, /\.hud-dock-action \{[^}]*height: 28px;/);

  // The waveform sits below the wordmark and above the engine chip. Asserted as
  // *order* rather than as geometry because the chip's placement is the whole
  // point: above the meter it was a filled horizontal pill cutting across a
  // 52px-wide vertical column, severing the mark from the waveform (owner,
  // 2026-08-28).
  assert.ok(
    dock.indexOf("hud-dock-level-wrap") < dock.indexOf("<EngineChip"),
    "the engine chip must render below the waveform, not between it and the wordmark",
  );

  // The five fixed rows, the card's padding and the five gaps have to leave the
  // waveform a positive number of pixels — the window cannot grow to absorb an
  // overrun, it just clips. Computed rather than pinned, so moving any one row
  // is caught here instead of in a screenshot.
  //
  // **`hud-dock-engine` and the fifth gap were missing from this until
  // 2026-08-28**, and their absence is why this test could not see the row it
  // was written to protect: it summed four rows and four gaps, computed 134, and
  // asserted 134 — while the running window's waveform had been 112 since the
  // engine row shipped two days earlier. A 22px blind spot, in the one check
  // whose comment claims that moving any row is caught here. Every row that
  // declares a height is in the sum now, and the gap count is derived from the
  // row count rather than written down, so adding a seventh row cannot repeat it.
  const height = (selector) =>
    Number(
      new RegExp(`\\.${selector} \\{[^}]*height: (\\d+)px;`).exec(styles)?.[1] ??
        Number.NaN,
    );
  const CARD_GUTTER = 5;
  const SPACE_2 = 8;
  const fixedRowSelectors = [
    "hud-dock-chrome",
    "hud-dock-wordmark",
    "hud-dock-engine",
    "hud-dock-status",
    "hud-dock-action",
  ];
  const fixedRows = fixedRowSelectors.reduce((total, selector) => total + height(selector), 0);
  // The waveform is the sixth row and the only `1fr`, so the gaps between six
  // rows is five.
  const gaps = fixedRowSelectors.length;
  const cardHeight = window_.height - CARD_GUTTER * 2;
  const waveform = cardHeight - fixedRows - SPACE_2 * 2 - SPACE_2 * gaps;
  assert.ok(Number.isFinite(fixedRows), "every fixed dock row must declare a height");
  assert.equal(waveform, 152, "the waveform gets whatever the fixed rows do not");

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
  // the resting state differs in shape *and* colour (never colour alone).
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
  //
  // Asserted as "the resting label is reachable from `idle` and nothing else"
  // rather than by naming the busy states, which is how this read until
  // 2026-08-28. The button is present in every state now, so the hazard inverted:
  // it is no longer that the dock says nothing while Granite works, it is that a
  // permanent button says `Ready` while Granite loads two gigabytes — the exact
  // defect 1.7.0 fixed one row up, in a louder place to repeat it.
  assert.match(
    dock,
    /kind === "idle" \|\| kind === "delivered" \|\| kind === "failed"/,
    "the dock's resting label must be keyed off idle, not off `not listening`",
  );
  assert.doesNotMatch(
    dock,
    /kind !== "listening"/,
    "a resting label keyed off `not listening` claims Ready while the model loads",
  );
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
  // to the same system colour (UI-GUIDE "Contrast, themes, and motion").
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

test("the engine chip never claims a device the worker has not reported", async () => {
  const dock = await readFile(new URL("../src/hud/HudDockApp.tsx", import.meta.url), "utf8");
  const catalog = await readFile(new URL("../src/catalog.ts", import.meta.url), "utf8");
  const styles = await readFile(new URL("../src/styles.css", import.meta.url), "utf8");

  // Instrument self-check. Every assertion below is a `match` on source, and a
  // file read from the wrong path matches nothing while reporting the same
  // failure a real regression would.
  assert.match(dock, /hud-dock-engine-chip/, "the dock source does not contain the chip");

  // `device()` answers `not_configured` before any warm and `unknown` for a
  // pre-v2 worker, and during the load the worker has usually not finished its
  // handshake. None of those three is a device. Rendering one as `GPU` is the
  // overreach that once put `device=cuda` in a support log for a worker running
  // entirely on the processor, and it is the single most likely way to ship a
  // confident, wrong indicator.
  for (const code of ["unknown", "not_configured", "granite_state_unavailable"]) {
    assert.match(
      dock,
      new RegExp(`NOT_A_DEVICE[\\s\\S]{0,200}"${code}"`),
      `${code} is not a device and must be held back from the chip's label`,
    );
  }
  assert.match(
    dock,
    /NOT_A_DEVICE\.has\(device\)\s*\?\s*messages\.engineDeviceUnknown/,
    "the chip must fall back to the placeholder rather than print a non-device code",
  );

  // Shape, not only hue: under forced-colors every fill flattens to one system
  // colour, and UI-GUIDE "Contrast, themes, and motion" forbids colour alone.
  // The failed state draws an SVG triangle where the others draw a round pip.
  assert.match(
    dock,
    /health === "failed" \? <AlertPip \/> : <span className="hud-dock-engine-pip"/,
    "the failed state must differ from the others in shape, not only in colour",
  );

  // Red means exactly one thing in this window -- active capture -- so the
  // chip's failure state spends `--hud-danger`, which is what the dock's
  // existing failure glyph already paints in.
  const chipRules = styles.slice(
    styles.indexOf(".hud-dock-engine {"),
    styles.indexOf(".hud-dock-level-wrap {"),
  );
  assert.ok(chipRules.length > 400, "the chip's stylesheet block was not located");
  // `var(--recording)`, not the bare token name: the first cut of this matched
  // the comment two lines above the rules explaining that the token is
  // deliberately *not* spent here, so the check failed on the presence of its
  // own justification. Spending a token is `var(...)`; naming it is prose.
  assert.doesNotMatch(
    chipRules,
    /var\(--recording\)/,
    "the engine chip must not spend --recording; red is reserved for active capture",
  );
  assert.match(chipRules, /@media \(forced-colors: active\)/);

  // No code may fall through to `errorUnknown` -- "The operation stopped
  // safely" -- which is the generic non-answer a lost dictation once got.
  // Every warm state the coordinator can publish as a failure needs copy.
  const engine = await readFile(
    new URL("../src-tauri/src/granite_engine.rs", import.meta.url),
    "utf8",
  );
  const published = [...engine.matchAll(/record_warm_state\("([a-z_0-9]+)"\)/g)].map((m) => m[1]);
  assert.ok(published.length >= 4, `only found ${published.length} published warm states`);
  for (const code of published) {
    // `cold`, `warming` and `ready` are states rather than failures and are
    // named by their own strings; everything else reaches `formatError`.
    if (["cold", "warming", "ready", "not_configured"].includes(code)) continue;
    assert.match(
      catalog,
      new RegExp(`\\b${code}:`),
      `the warm state ${code} can reach the chip and has no catalog entry, so it would render as errorUnknown`,
    );
  }

  // A cold engine is not merely a status line: it refuses the start press,
  // because `dictation_start` would block on the load's mutex for up to a
  // minute with the window frozen. That premise is why a warm that fails costs
  // the user dictation rather than cosmetics.
  //
  // Which warm states count as "still loading" is written down **twice** —
  // `ENGINE_LOADING` in `transcriberState.ts` decides the session state,
  // `ENGINE_PENDING` here decides the chip's colour — and `ENGINE_PENDING`'s own
  // comment says it is "kept in step with" the other. Nothing checked that. Two
  // hand-maintained copies of one set drift in the direction where the chip
  // says loading and the button is live, or the reverse.
  //
  // The graphics-card-runtime test pinned the first half and was deleted on
  // 2026-08-28 with the runtime download it described; this is what was worth
  // keeping out of it.
  const transcriberState = await readFile(
    new URL("../src/state/transcriberState.ts", import.meta.url),
    "utf8",
  );
  const loadingStates = (source, name) =>
    new RegExp(String.raw`const ${name}: ReadonlySet<string> = new Set\(\[([^\]]*)\]\)`).exec(
      source,
    );
  const sessionSide = loadingStates(transcriberState, "ENGINE_LOADING");
  const chipSide = loadingStates(dock, "ENGINE_PENDING");
  assert.ok(sessionSide, "ENGINE_LOADING must be findable in transcriberState.ts");
  assert.ok(chipSide, "ENGINE_PENDING must be findable in HudDockApp.tsx");
  assert.equal(
    chipSide[1].trim(),
    sessionSide[1].trim(),
    "the chip's loading states and the session's must be the same set; the comment says they are kept in step",
  );
  assert.match(sessionSide[1], /"cold"/);
  assert.match(sessionSide[1], /"warming"/);

  // …and a press that lands during the load is refused rather than queued.
  assert.match(
    transcriberState,
    /if \(current\.state\.kind === "loading_model"\) return current;/,
    "a start press during the model load must be refused, not blocked on the mutex",
  );
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
// gone. What it was guarding is not — every copy stays backend-owned
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

  // Still exactly five here; the transcript log is the sixth group and is
  // asserted separately.
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

  // The guided-test path is gone, commands and
  // all. `capture_stop` stopped without delivering, so a dictation started from
  // settings silently skipped the paste the shortcut performed — the exact
  // two-controllers failure the single-controller rule exists to prevent.
  for (const command of ["capture_start", "capture_stop", "capture_transcribe"]) {
    assert.equal(schema.commands[command], undefined);
    assert.doesNotMatch(app, new RegExp(`invoke\\("${command}"`));
  }

  // The duration slider went with them.
  assert.doesNotMatch(app, /maximumSeconds|type="range"/);
  assert.doesNotMatch(catalog, /maximumDuration|startCapture|stopCapture/);

  // What did *not* go: retrying a transcription whose audio is still retained.
  // That is recovery, not a guided test, and dropping it would lose a capability
  // the user has today, and nothing is silently dropped.
  assert.match(app, /invoke\("dictation_retry"\)/);
  // And it must not deliver — settings is the focused app while the user reads it.

  // One dictation at a time, stated once. The dock's `can_start` refused a press
  // while the previous dictation was still finishing and the global shortcut did
  // not, so the same key was accepted by one controller and declined by the
  // other — observed 490 ms after a ceiling stop, which opened a second
  // recording that queued 36.6 s and pasted wherever the user had moved on to.
  // Both paths converge on `start_dictation`, so the guard lives there, and the
  // definition of "still finishing" lives in one function both readers call.
  const rust = new Map(await readAllRustSources());
  const views = rust.get("apps/desktop/src-tauri/src/views.rs");
  const capture = rust.get("apps/desktop/src-tauri/src/commands/capture.rs");
  const coordinators = rust.get("apps/desktop/src-tauri/src/coordinators.rs");

  assert.match(coordinators, /fn hud_session_with_delivery\(/);
  assert.match(coordinators, /fn dictation_is_finishing\(/);
  assert.match(views, /if dictation_is_finishing\(app\) \{/);
  assert.match(views, /"dictation_still_finishing"/);
  assert.match(capture, /hud_session_with_delivery\(capture_view\.state\.as_str\(\)/);

  // No second statement of the promotion. `capture_hud_status` held its own copy
  // — `session == "complete" && delivery_pending` — and a copy is what let the
  // two controllers disagree in the first place.
  assert.doesNotMatch(
    capture,
    /session == "complete"/,
    "capture_hud_status must not re-implement the delivery promotion",
  );

  // The refusal is logged and has copy. "The shortcut did nothing" is
  // indistinguishable from a broken shortcut, so the log is what separates them
  // and the catalog entry is what a user gets instead of `errorUnknown`.
  assert.match(views, /\("result", "dictation_still_finishing"\)/);
  assert.match(catalog, /\n\s*dictation_still_finishing:/);
});

test("the session transcript log copies text and writes nothing to disk", async () => {
  const app = await readAllSources();
  const schema = JSON.parse(
    await readFile(new URL("../src/ipc/phase9.schema.json", import.meta.url), "utf8"),
  );

  assert.match(app, /invoke<SessionTranscriptEntry\[\]>\("session_transcript_log"\)/);
  assert.match(app, /invoke<number>\("session_transcript_copy", \{ id \}\)/);

  // Clipboard authority is still refused to the dock, which is
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

  // UI-GUIDE "Accessibility and input": the dock never takes focus, so it is
  // not keyboard operable — a
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
// transcriber now carries session controls, so it is no longer
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

  // Exactly the session-controls allowlist. Adding a command here is a deliberate decision
  // about what a no-activate window is allowed to do, not a formality.
  const dockReachable = Object.entries(schema.commands)
    .filter(([, spec]) => spec.window.split("|").includes("hud-dock"))
    .map(([name]) => name)
    .sort();
  assert.deepEqual(dockReachable, [
    // `capture_device_configure` and `capture_devices` left this list on
    // 2026-08-28 with `MicPicker`, the only thing in the dock that used them.
    "capture_hud_status",
    "capture_transcribe_cancel",
    "capture_wizard_status",
    "dictation_start",
    "dictation_stop",
    "hotkey_status",
    "hud_dock_context_menu",
    "hud_dock_placement_configure",
    // The amendment to the clipboard rule. It reaches the clipboard, which nothing else
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

test("the HUD's session controls share one implementation with the global shortcut", async () => {
  const backend = await readDesktopBackend();

  // Instrument self-check. From the fork to 2026-08-28 this read
  // `const backend = ""` behind a `test.skip`, on the recorded grounds that "the
  // shared implementation is exercised by the Rust desktop tests" — a
  // placeholder that could not have passed if it ran.
  assert.match(backend, /fn start_dictation\(/, "the desktop backend did not load");

  // The regression this exists to prevent is a dictation started from the
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

  // There is one delivery path, and this used to say so by counting
  // `deliver_final_text(&app, &text, source_reason)` and asserting the count was
  // 1. Wrong shape twice over: the `source_reason` argument left with the
  // streaming fallback, so the count silently became 0, and a count of a call
  // *string* passes just as happily when a second caller copies the line.
  //
  // The rule is about reachability, and the path is longer than "stop_dictation
  // delivers". `stop_dictation` and the ceiling watcher both call
  // `transcribe_and_deliver`, which does not deliver at all — it submits the
  // audio to `OrderedFinalizationQueue`. The queue's single consumer is
  // `process_finalization_job`, and that is the only function that calls
  // `deliver_final_text`: twice, for the two arms of one match, a transcript and
  // the empty string a no-speech result delivers.
  //
  // Asserted by naming the enclosing function rather than by counting, so it
  // survives a signature change and still fails if a second caller appears.
  const consumer = rustFunctionBody(backend, "process_finalization_job");
  assert.ok(consumer, "fn process_finalization_job must be findable");
  const inConsumer = [...consumer.matchAll(/deliver_final_text\(/g)].length;
  assert.ok(inConsumer > 0, "the queue consumer must be what delivers");
  const everywhere = [...backend.matchAll(/deliver_final_text\(/g)].length;
  assert.equal(
    everywhere - inConsumer,
    1,
    "deliver_final_text must be called only from process_finalization_job; the one remainder is its own definition",
  );
  // …and that consumer is reached only from the queue, wired once at the
  // composition root. One consumer is what stops two utterances racing.
  assert.match(backend, /move \|job\| process_finalization_job\(&finalization_app, job\)/);
  assert.equal([...backend.matchAll(/process_finalization_job\(/g)].length, 2);

  // Both button commands route through the shortcut's own session and debounce
  // state, so a click and a key press compete for one session.
  assert.match(backend, /fn request_start\(&self\)[\s\S]{0,900}?self\.accept_press\(\)/);
  assert.match(backend, /fn request_stop\(&self\)[\s\S]{0,900}?self\.accept_press\(\)/);
});

test("window close means different things for the dock and for settings", async () => {
  const backend = await readDesktopBackend();
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  );
  const confirm = await readFile(
    new URL("../../../crates/speakeasy-windows/src/confirm.rs", import.meta.url),
    "utf8",
  );
  const nativeCatalog = await readFile(
    new URL("../src-tauri/src/native_catalog.rs", import.meta.url),
    "utf8",
  );

  // Instrument self-check: all three read `""` from the fork until 2026-08-28.
  assert.match(backend, /fn on_window_event\(/, "the desktop backend did not load");
  assert.match(confirm, /Confirmation/, "confirm.rs did not load");
  assert.match(nativeCatalog, /pub const/, "native_catalog.rs did not load");

  // There was no window-event handler at all before the redesign; the only exit
  // path was the tray.
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

  // Every label the handler matches is a window that exists. It matched `"hud"`
  // from the fork until 2026-08-28, for a window the config has not declared
  // since — an arm that compiles, never fires, and reads to the next person as
  // proof the window is there. The `_ => {}` arm is correct and stays: an
  // unrecognised label arriving at runtime must not panic, which is exactly what
  // makes a dead arm invisible, so the only place to catch it is here. Same
  // shape as `every_menu_id_that_is_built_has_a_handler` on the Rust side.
  const closeHandler = rustFunctionBody(backend, "on_window_event");
  assert.ok(closeHandler, "fn on_window_event must be findable");
  //
  // Whole arm *patterns*, not `"label" =>`: the dead arm was `"hud" | "hud-dock"
  // => {`, where only the last alternative is followed by the arrow. A rule that
  // reads one label per arm sees the live half and misses the dead one, which is
  // how this was written the first time.
  const declared = new Set(config.app.windows.map((window) => window.label));
  const matched = [...closeHandler.matchAll(/^\s*("[^"]+"(?:\s*\|\s*"[^"]+")*)\s*=>/gm)].flatMap(
    (arm) => [...arm[1].matchAll(/"([^"]+)"/g)].map((label) => label[1]),
  );
  assert.ok(matched.length >= 2, `expected the handler to match some labels, found ${matched.length}`);
  for (const label of matched) {
    assert.ok(declared.has(label), `on_window_event matches "${label}", which no window declares`);
  }

  // Settings hides and never destroys, never quits, never touches a dictation.
  assert.match(backend, /"main" =>[\s\S]{0,200}?prevent_close\(\)[\s\S]{0,120}?window\.hide\(\)/);
  // Closing the dock quits the whole app, gracefully. This named `"hud"` until
  // 2026-08-28 — a window label that has not existed since the fork, matched by
  // an arm the source still carried.
  assert.match(
    backend,
    /"hud-dock" =>[\s\S]{0,320}?request_quit\(&app\)[\s\S]{0,160}?shutdown_gracefully\(&app\)/,
  );

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
  // object to clean up after a hard exit. It was the streaming coordinator that
  // was shut down here; there is one engine, and it is the one holding two
  // gigabytes of weights and a child process.
  const shutdown = rustFunctionBody(backend, "shutdown_gracefully");
  assert.ok(shutdown, "fn shutdown_gracefully must be findable");
  assert.match(shutdown, /GraniteEngineCoordinator>\(\)\.shutdown\(\)/);
});

test("the dock is the surface a relaunch and a restore bring back", async () => {
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  );
  const backend = await readDesktopBackend();
  const hudComponents = await readHudComponents();
  const main = config.app.windows.find((window) => window.label === "main");

  // Instrument self-check: `backend` read `""` from the fork until 2026-08-28,
  // and this test also looked up a `hud` window that has not been declared since
  // the fork — `hud.visible` would have thrown on `undefined` had it ever run.
  assert.match(backend, /fn show_dock\(/, "the desktop backend did not load");
  assert.ok(main, "the main window must be declared");

  // Settings starts hidden; the dock is what a launch puts on screen. Which
  // windows exist, and that none of them can take the foreground, is asserted by
  // "every window is declared, and none of them can take the foreground" — this
  // deliberately does not repeat `main.focus`, which that test already covers
  // for every window rather than for this one.
  assert.equal(main.visible, false);

  /*
   * The settings window's minimum must fit the work area at 200% scaling,
   * which is the tightest case a common display produces: a 1920x1080 panel at
   * 200% leaves 960x516 *logical* pixels once the taskbar is accounted for.
   *
   * The redesign brief specified a 560 px minimum height. Measured against that
   * work area it is
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

  // A second launch means "give me SpeakEasy", which is the dock. This showed
  // `main` until it was actually run: relaunching with the dock minimized left
  // it minimized and popped settings instead. The function was
  // `show_transcriber` while the large HUD existed.
  assert.match(
    backend,
    /single_instance::init\([\s\S]{0,600}?show_dock\(app\)/,
    "a second launch must restore the dock, not the settings workspace",
  );
  // And it must not focus it — no-activate is load-bearing for delivery
  // targeting, so a relaunch cannot be allowed to steal the foreground. Anything
  // this app puts in the foreground becomes the next dictation's paste target.
  const showDock = rustFunctionBody(backend, "show_dock");
  assert.ok(showDock, "fn show_dock must be findable");
  assert.match(showDock, /unminimize\(\)/);
  assert.doesNotMatch(
    showDock,
    /set_focus\(\)/,
    "the dock is no-activate; showing it must never focus it",
  );

  // The backend still reports the microphone the next dictation will really use:
  // a fresh profile stores no preference but records from a real device, so a
  // picker that read "Select a microphone" was stating something untrue.
  assert.match(backend, /preferred_device_id:/);
  // The picker that consumed it is gone. `MicPicker.tsx` was the large HUD's
  // device list and no file imported it after the fork; a 62 px dock has nowhere
  // to put one, and Settings → Audio already offers the choice through
  // `capture_device_configure`. Two assertions stood here about its JSX
  // (`resolveDevice`, `preferredId={model.preferredDeviceId}`) and passed only
  // because this test never ran — the second named a call site that did not
  // exist. Deleted 2026-08-28.
  //
  // What replaces them is the rule that made the component redundant: a device
  // the user can choose has to be choosable somewhere they can reach with a
  // keyboard, and the dock is deliberately not keyboard operable.
  const audio = await readFile(new URL("../src/settings/Audio.tsx", import.meta.url), "utf8");
  assert.match(audio, /invoke\("capture_device_configure"/);
  assert.doesNotMatch(
    hudComponents,
    /capture_device_configure/,
    "choosing a device belongs in settings; the dock cannot take keyboard focus",
  );
});

test("a dictation that hits the duration ceiling delivers instead of vanishing", async () => {
  const backend = await readDesktopBackend();
  const capture = await readFile(
    new URL("../src-tauri/src/capture_wizard.rs", import.meta.url),
    "utf8",
  );

  // Instrument self-check: both read `""` from the fork until 2026-08-28.
  assert.match(backend, /fn stop_dictation\(/, "the desktop backend did not load");
  assert.match(capture, /MAX_CAPTURE_SECONDS/, "capture_wizard.rs did not load");

  // Hitting the ceiling must stop exactly as a user stop does — it must
  // not discard. Both paths reach the same function.
  //
  // Read as function *bodies* rather than as a character window from each head.
  // The watcher's call was asserted to appear within 2,400 characters; the
  // function grew to 3,878 when the notice window landed on 2026-08-25, so the
  // bound would have failed on a function that was still correct. A window is a
  // guess about how long a function is allowed to get.
  const watcher = rustFunctionBody(backend, "watch_for_unattended_capture_end");
  const stop = rustFunctionBody(backend, "stop_dictation");
  assert.ok(watcher, "fn watch_for_unattended_capture_end must be findable");
  assert.ok(stop, "fn stop_dictation must be findable");
  assert.match(backend, /fn transcribe_and_deliver\(/);
  assert.match(stop, /transcribe_and_deliver\(app\)/);
  assert.match(watcher, /transcribe_and_deliver\(&app\)/);

  // The watcher must not race the user's own stop into a second transcription.
  assert.match(watcher, /stop_was_requested\(\)/);
  assert.match(capture, /fn stop_was_requested\(&self\)/);

  // The shipped product ceiling is two minutes, and an unattended capture
  // follows the same transcription path as an explicit stop.
  assert.doesNotMatch(backend, /HOTKEY_MAXIMUM_CAPTURE_SECONDS/);
  assert.match(backend, /DICTATION_CEILING_SECONDS: u32 = capture_wizard::MAX_CAPTURE_SECONDS/);
  assert.match(capture, /pub const MAX_CAPTURE_SECONDS: u32 = 2 \* 60/);
});

test("colour and type go through the design tokens, not through literals", async () => {
  const styles = await readFile(new URL("../src/styles.css", import.meta.url), "utf8");

  // Every colour is a token. Literals are allowed only where tokens are
  // *defined* — the `:root` block and the dark-scheme override.
  const rootBlock = /^:root \{[\s\S]*?^\}/m.exec(styles)?.[0] ?? "";
  const darkBlock = /@media \(prefers-color-scheme: dark\) \{[\s\S]*?\n\}/.exec(styles)?.[0] ?? "";
  assert.notEqual(rootBlock, "");
  assert.notEqual(darkBlock, "");
  const elsewhere = styles.replace(rootBlock, "").replace(darkBlock, "");
  const literals = elsewhere.match(/#[0-9a-fA-F]{3,8}\b/g) ?? [];
  assert.deepEqual(literals, [], "colour literals belong in the token blocks only");

  // The old identity's interface font is gone. One sans-serif for the interface,
  // and serif only where transcript text earns it.
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

  // All three media blocks stay.
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

test("no effect can read a race-prone command without retrying or polling", async () => {
  // This rule replaced one that named `personalization_status` in
  // `Transcription.tsx` and asserted it went through the retry. That test was
  // green on the day the same defect was found in `General.tsx`, on a different
  // command, in a different file -- reporting a registered, working shortcut as
  // "Shortcut not registered yet" for the life of the process (2026-08-25). The
  // recorded lesson of the first occurrence was "one reader carried a retry and
  // nothing else did", and the fix for it repeated exactly that, because the
  // check was a record of where the bug had been seen rather than a rule about
  // where it can happen.
  //
  // So nothing here is named by hand. The hazard is derived from the Rust
  // signatures, the readers are found by scanning every `useEffect` in the tree,
  // and a command is only cleared two ways:
  //
  // - **retried**, through `readWithRetry`, which bounds the attempts and still
  //   rejects so the caller has to say something; or
  // - **polled**, because an effect that re-reads on an interval heals itself on
  //   the next tick and the first refusal costs nothing.
  //
  // A read that is neither is the defect, whichever file it is in.
  const racy = await raceProneCommands();
  const sources = await readFrontendSources();

  // Instrument self-checks. Every assertion below is of the form "nothing was
  // found", which is also what a broken scanner reports -- and two of the three
  // components here (balanced-delimiter scanning, one-level call resolution)
  // fail silently rather than throwing. So each has to be shown working against
  // a case whose answer is known.
  for (const command of ["hotkey_status", "profile_status", "personalization_status"]) {
    assert.ok(racy.has(command), `${command} takes a State and must be derived as race-prone`);
  }
  for (const command of ["model_install_status", "capture_devices", "capture_hud_status"]) {
    assert.ok(
      !racy.has(command),
      `${command} resolves its own state with try_state and is not refused this way`,
    );
  }

  const findings = [];
  const polled = [];
  const retried = new Set();
  for (const [path, source] of sources) {
    for (const command of commandsIn(source, "readWithRetry")) retried.add(command);
    for (const effect of effectBodies(source)) {
      // An effect that installs an interval re-reads until it succeeds. That is
      // the same guarantee the retry gives, arrived at from the other direction.
      const selfHealing = /setInterval\(/.test(effect);
      const reachable = [effect];
      for (const callee of calleesIn(effect)) {
        const body = localFunctionBody(source, callee);
        if (body !== null) reachable.push(body);
      }
      for (const snippet of reachable) {
        for (const command of commandsIn(snippet, "invoke")) {
          if (!racy.has(command)) continue;
          if (selfHealing) polled.push(`${path}:${command}`);
          else findings.push(`${path} reads ${command} on mount without a retry`);
        }
      }
    }
  }

  // The scanner has to be shown reaching through a helper and reaching the poll
  // exemption, or the two branches above are decoration.
  assert.ok(
    retried.has("model_catalog"),
    "the one-level call resolution must reach refreshCatalog's reads",
  );
  assert.ok(
    polled.includes("settings/TranscriptLog.tsx:session_transcript_log"),
    "the poll exemption must be reached by the page that polls",
  );
  assert.deepEqual(findings, []);

  // Once a command is read through the retry, no file may also read it bare.
  // Both defects were a single unguarded call sitting beside guarded ones.
  for (const [path, source] of sources) {
    for (const command of commandsIn(source, "invoke")) {
      assert.ok(
        !retried.has(command),
        `${path} reads ${command} with a bare invoke; it is retried elsewhere`,
      );
    }
  }

  // The failure the rule was first written for cost a user their installer
  // vocabulary: three words correct in `personalization.json` and an empty
  // dictionary list on screen. An empty list is not a blank page anyone reports
  // -- it says "you have no protected terms" -- so a read that never succeeds
  // has to say what actually happened, in the place the missing answer would
  // have been.
  const wiring = new Map(sources);
  const catalog = wiring.get("catalog.ts");
  for (const [path, message] of [
    ["settings/Transcription.tsx", "personalizationUnavailable"],
    ["settings/General.tsx", "shortcutStateUnavailable"],
    ["settings/Advanced.tsx", "runtimeStatusUnavailable"],
    ["settings/OutputPrivacy.tsx", "resultStatusUnavailable"],
    ["settings/SettingsApp.tsx", "profileUnavailable"],
  ]) {
    assert.match(wiring.get(path), new RegExp(`messages\\.${message}`), `${path} must say so`);
    assert.match(catalog, new RegExp(`\\n\\s*${message}:`), `${message} needs catalog prose`);
  }

  // The deeper half of the same defect, and the half a retry cannot fix. A
  // fallback that renders as a *claim about the system* turns an unanswered read
  // into a statement of fact: `pending` is "registration has not been attempted",
  // whose copy reads "Shortcut not registered yet", and `empty` is "No result".
  // Both are real backend values that mean something specific. `undefined` means
  // the page does not know, and only the `unknown` codes say that.
  assert.match(wiring.get("settings/General.tsx"), /registration \?\? "unknown"/);
  assert.match(wiring.get("settings/OutputPrivacy.tsx"), /result\?\.state \?\? "unknown"/);

  // General's binding field held `Ctrl+Alt+L` -- SpeakEasy's shortcut, inherited
  // by the fork and never rebranded. That made the lost read destructive rather
  // than merely wrong: the remedy the panel implied would have rebound this
  // app's working `Ctrl+Alt+P` to the other product's shortcut, on a machine
  // where both are installed. No page may hold a shortcut as a *value* -- the
  // backend owns the default -- and Save is disabled until the status has been
  // read. Quoted only: the comment recording this may name both shortcuts, and
  // it has to, because the number that was wrong is the whole point.
  assert.doesNotMatch(await readAllSources(), /"Ctrl\+Alt\+[A-Z]"/);
  assert.match(wiring.get("settings/General.tsx"), /disabled=\{hotkey === null\}/);

  // A refusal is not the only way to lose this race, and the other way is the
  // one that reproduced on every launch. `HotkeyCoordinator` starts at
  // `registration: "pending"` and `register_activation_hotkey` runs at the *end*
  // of `setup`, after the tray is built -- while all three eagerly mounted pages
  // (General, Transcription, Advanced) have already read. So the read *succeeded*
  // and returned a value true for one moment of the process, which rendered as
  // "Shortcut not registered yet" for the life of it. Indistinguishable from a
  // refusal on screen; separated by reloading the window and watching the same
  // page report "Shortcut active" from the same backend.
  assert.match(wiring.get("settings/General.tsx"), /registration !== "pending"/);

  // The retry has to be bounded. Retrying forever trades a wrong answer for a
  // permanent spinner, and the rejection still has to reach the caller.
  const helper = wiring.get("settings/readWithRetry.ts");
  assert.match(helper, /const ATTEMPTS = \d+;/);
  assert.match(helper, /throw lastError;/);

  // An unsettled answer that is still unsettled after every attempt is returned,
  // not thrown. A startup value that survives five seconds has stopped being
  // transient and is the truth -- `pending` then really does mean the shortcut
  // was never registered, which is the one case that copy is for. Only a refusal
  // has nothing to report.
  assert.match(helper, /if \(read\) return lastValue as T;/);

  // One retry, not two. `useProfile.ts` carried a hand-rolled copy of the same
  // 20 x 250 ms, and `readWithRetry`'s own comment named the risk: one page
  // recovering from a startup the other reported as broken.
  assert.match(wiring.get("settings/useProfile.ts"), /readWithRetry<ProfileStatus>/);
  assert.doesNotMatch(wiring.get("settings/useProfile.ts"), /ATTEMPTS/);
});

test("the provider a machine is recorded as running on is proved, never chosen", async () => {
  // The reported failure: `engine=cpu_gpu_runtime_missing device=cpu
  // installed=cuda`. Setup wrote a graphics-card installation down from a radio
  // button it had never disabled, the app correctly ran on the processor, and the
  // disagreement lived only as three fields of one log line nothing compared.
  //
  // Pinned against source because every part of this is a *structural* rule --
  // where a value may come from, and what has to be true before it is written --
  // and a structural rule is exactly what review keeps missing.
  const bootstrapper = new URL("../../../apps/bootstrapper/src/", import.meta.url);
  const wizard = await readFile(new URL("wizard.rs", bootstrapper), "utf8");
  const seed = await readFile(new URL("seed.rs", bootstrapper), "utf8");
  const build = await readFile(
    new URL("../../../scripts/Build-LocalInstaller.ps1", import.meta.url),
    "utf8",
  );

  // The option that cannot be installed is shown and *disabled*. Hiding it reads
  // as setup not having looked at the card; leaving it enabled is a control that
  // installs something else and says nothing.
  assert.match(wizard, /GRAPHICS_CARD_OPTION/);
  assert.match(wizard, /EnableWindow\(download::graphics_card_configuration_available\(\)\.is_ok\(\)\)/);

  // The marker is written from the engine check's verdict, and nowhere else.
  assert.match(wizard, /seed::record_installed_provider/);
  assert.doesNotMatch(
    wizard,
    /provider: if self\.provider\.selected_index/,
    "the installed-configuration record must not come from the radio group again",
  );
  // And it is not one of the seeds, which are written three pages earlier from
  // what the user chose.
  assert.doesNotMatch(seed, /\(PROVIDER, answers\./);

  // Packaging refuses to assemble a graphics-card worker with no libraries
  // beside it -- the failure that does not degrade, because Windows cannot
  // resolve the imports and the engine never starts.
  assert.match(build, /Assert-GraniteWorkerPayloadIsCoherent/);
});

test("the active provider is reported as the device, never as the pack", async () => {
  // There is one Granite GGUF and a graphics-card worker offloads that same
  // file, so the selected pack reads `cpu` on a machine holding the card.
  // "Dictation runs on: Processor (CPU)" was rendered from the pack.
  const page = await readFile(
    new URL("../src/settings/Transcription.tsx", import.meta.url),
    "utf8",
  );
  const catalog = await readFile(new URL("../src/catalog.ts", import.meta.url), "utf8");

  assert.match(page, /formatState\(gpu\.active_device\)/);
  assert.doesNotMatch(
    page,
    /messages\.engineDisclosure[\s\S]{0,200}formatState\(gpu\.active_provider\)/,
    "the disclosure must read the device, not the pack's provider",
  );
  // A graphics-card engine whose context could not be confirmed gets its own
  // label. Calling it `cuda` is the unverified claim; calling it `cpu` reports a
  // fault on a machine that is probably using its card.
  assert.match(catalog, /cuda_unverified:/);
  // The disagreement has copy, and it is only shown when it says something.
  assert.match(catalog, /gpu_install_not_operational:/);
  assert.match(page, /formatProviderIntegrity\(gpu\.provider_integrity\)/);

  // And it is re-read while the worker warms. Both the device and the integrity
  // line are `not_configured` until the launch warm has spoken, seconds after
  // this page mounts -- read once, the fault disclosure is never rendered at all.
  assert.match(page, /ENGINE_WARM_READS/);
  assert.match(page, /active_device !== "not_configured"/);
});

test("desktop exposes connected activation settings and friendly catalog errors", async () => {
  const app = await readAllSources();
  const catalog = await readFile(new URL("../src/catalog.ts", import.meta.url), "utf8");

  assert.match(catalog, /locale: "en-US"/);
  assert.match(catalog, /translatedLocales: \[\]/);
  assert.doesNotMatch(catalog, /globalActivationUnavailable/);
  assert.doesNotMatch(app, /messages\.globalActivationUnavailable/);
  assert.match(app, /invoke\("hotkey_configure"/);
  // Through the retry, not bare: this read lost the startup race and reported a
  // working shortcut as unregistered. See the startup-race rule above, which is
  // where that is actually enforced -- this assertion only pins that the page
  // still reads the status at all.
  assert.match(app, /readWithRetry<HotkeyStatus>\("hotkey_status"\)/);

  // Registration is reported in plain language, not as a contract term
  // (UI-GUIDE "Two vocabulary registers").
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
  for (const file of [
    "wizard.rs",
    "install.rs",
    "uninstall.rs",
    "probe.rs",
    // Added 2026-08-19 with the modules themselves. `payload.rs` is the one
    // most likely to grow a sentence by accident: every way it fails is a
    // sentence somebody wants to show a user, and it cannot reach the catalog
    // because `bin/pack-payload.rs` compiles it too. It returns a typed error
    // and `catalog::describe_payload_failure` does the talking.
    "payload.rs",
    "seed.rs",
  ]) {
    const source = await readFile(new URL(file, bootstrapper), "utf8");
    for (const [index, line] of source.split(/\r?\n/).entries()) {
      if (line.trim().startsWith("//")) continue;
      // Panic and assertion messages are developer diagnostics, not copy. They
      // are never rendered to a user, and holding them at arm's length in a
      // catalog would put the explanation of a crash somewhere other than the
      // code that crashes.
      //
      // This exemption is per-line like the rule it sits inside, so it only
      // sees a message on the same line as its macro. `rustfmt` splits a long
      // `assert_eq!` across lines and leaves the message alone on one, where
      // this cannot tell it from copy. That is a known false positive and the
      // deliberate cost of not matching across lines: keep assertion messages
      // under 60 characters and put the reasoning in the test's doc comment,
      // which is where this codebase keeps it anyway.
      if (/\.expect\(|panic!|unreachable!|assert/.test(line)) continue;
      assert.ok(
        !sentence.test(line),
        `${file}:${index + 1} holds a user-facing sentence; it belongs in catalog.rs`,
      );
    }
  }
});

test("the token budget covers the longest dictation the ceiling allows", async () => {
  // The hazard, in full: Granite's generation loop stops on reaching
  // `max_new_tokens` with no error and no end-of-generation token, so nothing
  // downstream can tell "the model finished" from "the model was cut off
  // mid-clause". `is_plausible` cannot catch it either -- the gate is one-sided
  // and only rejects transcripts that are too *long*.
  //
  // It has been unreachable since the fork, and the reason is arithmetic nobody
  // did for months: the capture ceiling caps a dictation at about a fifth of the
  // budget. Both numbers were sitting in the tree, correct, and the risk was
  // inherited by copying rather than re-derived. Confirmed empirically on
  // 2026-08-25 by a 120.183 s dictation -- the longest this product can make --
  // which transcribed complete with a six-word tripwire intact.
  //
  // This is what makes it stay unreachable. `capture_wizard.rs` already records
  // wanting a thirty-minute ceiling, which is far past the budget and would
  // truncate silently somewhere in the middle, and nothing in the tree connected
  // the two constants. They cannot be compared in Rust: they live in
  // `speakeasy-desktop` and `speakeasy-granite`, and the desktop crate
  // deliberately does not depend on the one that compiles llama.cpp. So they are
  // compared here, as source.
  const ceilingSource = await readFile(
    new URL("../src-tauri/src/capture_wizard.rs", import.meta.url),
    "utf8",
  );
  const graniteSource = await readFile(
    new URL("../../../crates/speakeasy-granite/src/lib.rs", import.meta.url),
    "utf8",
  );

  const ceiling = /pub const MAX_CAPTURE_SECONDS: u32 = (\d+) \* (\d+);/.exec(ceilingSource);
  assert.ok(ceiling !== null, "MAX_CAPTURE_SECONDS must be readable from capture_wizard.rs");
  const seconds = Number(ceiling[1]) * Number(ceiling[2]);

  const budget = /max_new_tokens: (\d[\d_]*),/.exec(graniteSource);
  assert.ok(budget !== null, "max_new_tokens must be readable from GraniteOptions::default");
  const tokens = Number(budget[1].replace(/_/g, ""));

  // Both rates are deliberately pessimistic against the only real measurement
  // this product has -- 312 words in 120.183 s (156 wpm) needing ~400 tokens
  // (1.29 tokens per word), measured on an installed release build. A fast
  // dictator reaches 200 wpm, and 1.5 tokens per word leaves room for a speaker
  // whose vocabulary tokenises worse than that clip's. Erring high here means
  // the check complains before a real user is truncated, which is the only
  // useful direction for a failure that produces no error of its own.
  const WORDS_PER_MINUTE = 200;
  const TOKENS_PER_WORD = 1.5;
  const needed = Math.ceil((seconds / 60) * WORDS_PER_MINUTE * TOKENS_PER_WORD);

  assert.ok(
    tokens >= needed,
    `a ${seconds} s ceiling can need ${needed} tokens and max_new_tokens is ${tokens}; ` +
      "raising the ceiling means raising the budget with it, or a long dictation " +
      "is silently cut off mid-clause",
  );
});

test("setup's download policy and the app's are the same policy", async () => {
  // Two copies of one policy, in crates that deliberately do not depend on each
  // other, and until 2026-08-28 nothing compared them. `download.rs` claimed
  // `the_policy_matches_the_app` pinned them together; no test by that name has
  // ever existed, so the comment was describing a guarantee rather than
  // recording one — which is the most expensive kind of dead citation, because
  // it reads as coverage.
  //
  // What a drift costs: a host pinned in the manifest but absent from one
  // policy's redirect list fails `validate_url` at transfer time, on a user's
  // machine, after setup or the app has already promised to fetch it. The
  // manifest and the two policies are edited by different changes in different
  // files, so nothing else would notice.
  //
  // Compared as source for the same reason the token-budget rule above is:
  // `apps/bootstrapper` and `apps/desktop` share no crate, and adding a
  // dependency between them to share five hostnames would be the larger change.
  const setup = await readFile(
    new URL("../../../apps/bootstrapper/src/download.rs", import.meta.url),
    "utf8",
  );
  const app = await readFile(
    new URL("../src-tauri/src/commands/models.rs", import.meta.url),
    "utf8",
  );

  // The literal a function returns, with comments and whitespace removed. The
  // app's copy carries a comment inside `redirect_hosts` explaining the Xet CDN
  // hop, so a raw string compare would fail on prose.
  const policyLiteral = (source, name) => {
    const body = rustFunctionBody(source, name);
    assert.ok(body, `fn ${name} must be findable`);
    const literal = /DownloadPolicy\s*\{[\s\S]*\}/.exec(body);
    assert.ok(literal, `fn ${name} must return a DownloadPolicy literal`);
    return literal[0]
      .replace(/\/\/[^\n]*/g, "")
      .replace(/\s+/g, "")
      .replace(/,\}/g, "}");
  };

  const setupPolicy = policyLiteral(setup, "policy");
  const appPolicy = policyLiteral(app, "model_download_policy");

  // Instrument self-check: every assertion here is an equality between two
  // extractions, and two failed extractions are equal to each other. Both must
  // contain something only a real policy contains.
  for (const [label, extracted] of [
    ["setup", setupPolicy],
    ["app", appPolicy],
  ]) {
    assert.match(extracted, /redirect_hosts:vec!\[/, `${label}'s policy did not extract`);
    assert.match(extracted, /"huggingface\.co"/, `${label}'s policy lost its hosts`);
    assert.match(extracted, /maximum_retries:/, `${label}'s policy lost its retry count`);
  }

  assert.equal(
    appPolicy,
    setupPolicy,
    "the bootstrapper's download policy and the app's have drifted; a host, deadline or " +
      "retry count added on one side and not the other fails on a user's download rather " +
      "than here",
  );
});

test("every catalog entry is reachable from something that renders it", async () => {
  // 59 of 296 entries were referenced nowhere on 2026-08-27 — 20% of the app's
  // user-facing copy — and eight of them were the deleted streaming engine's
  // truthful-disclosure strings. `liveQualifiedDisclosure` read "Live
  // transcription is qualified for the supported en-US CPU and CUDA
  // configurations. The final transcript is what gets written." There is no live
  // transcription in this fork. That is the hazard the rest of the sweep is
  // incidental to: copy sitting in the file, ready for someone to wire up in
  // good faith, making a claim the product cannot back.
  //
  // This runs in the gate so the file cannot regrow.
  const src = fileURLToPath(new URL("../src/", import.meta.url));
  const catalogPath = join(src, "catalog.ts");

  // `src/` alone is not the corpus. The scaffold suite asserts catalog copy from
  // `tests/*.mjs`, and the Rust side reads `catalog.ts` as source — a key
  // referenced only from either would read dead, and deleting it would break the
  // test that existed to protect it. Two entries were nearly lost that way.
  const roots = [
    src,
    fileURLToPath(new URL("./", import.meta.url)),
    fileURLToPath(new URL("../src-tauri/src/", import.meta.url)),
  ];
  const files = [];
  for (const root of roots) {
    const entries = await readdir(root, { recursive: true, withFileTypes: true });
    for (const entry of entries) {
      if (!entry.isFile() || !/\.(ts|tsx|mjs|rs)$/.test(entry.name)) continue;
      const full = join(entry.parentPath, entry.name);
      if (full !== catalogPath) files.push(full);
    }
  }
  // Comment-only lines are dropped before searching. **This test's own prose is
  // in the corpus**, so naming a dead key while explaining why it is dead marks
  // it live — which is exactly what happened on the first run: the paragraph
  // above quotes `liveQualifiedDisclosure`, and that alone resurrected it. Any
  // comment anywhere in the tree does the same, so a key surviving only in
  // somebody's explanation of it would read reachable forever.
  //
  // Whole-line comments only (`//`, `*`, `/*`). Stripping to end-of-line from
  // any `//` would cut a URL in a string literal and take whatever followed it
  // on that line with it, and losing a real reference is the direction that
  // deletes copy the app renders.
  const withoutComments = (text) =>
    text
      .split("\n")
      .filter((line) => !/^\s*(\/\/|\*|\/\*)/.test(line))
      .join("\n");
  const corpus = withoutComments(
    (await Promise.all(files.map((file) => readFile(file, "utf8")))).join("\n"),
  );
  const source = await readFile(catalogPath, "utf8");

  // Top-level keys only: two-space indent, an identifier, a colon. Nested tables
  // are reached through their parent, which this sees.
  const keys = [...source.matchAll(/^ {2}([A-Za-z][A-Za-z0-9_]*)\s*:/gm)].map((match) => match[1]);
  const used = (key) => new RegExp(String.raw`\b${key}\b`).test(corpus);

  // Instrument self-check, and it is not optional here: every assertion below is
  // "nothing was found", which is exactly what a scanner reading an empty corpus
  // reports. The first version of this claimed all 296 keys were dead, because
  // its regex did not survive shell quoting — a clean, confident, wrong answer.
  assert.ok(files.length > 30, `corpus is too small to be real: ${files.length} files`);
  assert.ok(corpus.length > 100_000, `corpus is too small to be real: ${corpus.length} bytes`);
  assert.ok(keys.length > 200, `only ${keys.length} top-level keys parsed; the key regex is wrong`);
  for (const live of ["transcriber", "transcriberStates", "errors"]) {
    assert.ok(used(live), `self-check: "${live}" is rendered and must read live`);
  }
  // Deleted in Phase 2. If the key parser is matching something it should not,
  // this is where it shows.
  assert.ok(
    !keys.includes("transcriberHeader"),
    "self-check: transcriberHeader was deleted and must not parse as a key",
  );

  // Deliberately one-directional: a key whose name is also a common identifier
  // (`copy`, `install`, `cancel`, `engine`) reads live because something *else*
  // uses that word, so this under-reports. It may miss a dead key; it must never
  // call a live one dead, because acting on that deletes copy the app renders.
  const dead = keys.filter((key) => !used(key));
  assert.deepEqual(
    dead,
    [],
    `catalog entries nothing references: ${dead.join(", ")} — delete them, or wire up whatever was supposed to render them`,
  );
});

test("Advanced's diagnostics report the running device, not the selected pack", async () => {
  // Advanced rendered `PROVIDER: Processor (CPU)` on a machine whose worker held
  // 2,365 MiB of VRAM and transcribed 24 s of speech in 1,424 ms, because the
  // field was filled from the selected pack's provider capability -- which is
  // `Cpu` on every machine, since there is one Granite pack and the
  // graphics-card worker offloads that same GGUF. The owner read the page and
  // reasonably concluded dictation had fallen back to the processor.
  //
  // `gpu_status` had been corrected for precisely this and this command had not,
  // so the rule lives here rather than in either call site: whatever fills a
  // user-facing provider field reads the device the worker reported.
  const source = await readFile(
    fileURLToPath(new URL("../src-tauri/src/commands/profile.rs", import.meta.url)),
    "utf8",
  );
  const body = source.slice(
    source.indexOf("fn diagnostics_status("),
    source.indexOf("fn diagnostics_export("),
  );
  // Instrument self-check. Both assertions below are "the source does not say
  // X", which is also what slicing the wrong range reports.
  assert.ok(body.length > 1_000, `diagnostics_status body did not slice: ${body.length} bytes`);
  assert.match(body, /DiagnosticsView \{/, "self-check: the sliced body must build the view");

  assert.match(
    body,
    /provider: granite\.device\(\)/,
    "diagnostics_status must fill `provider` from the device the worker runs on",
  );
  assert.ok(
    !/provider: *selection\.capabilities\.provider/.test(body) &&
      !/"provider_unresolved"/.test(body),
    "diagnostics_status must not report the pack's provider as the active one",
  );
});

test("nothing claims the graphics card passed an execution check that cannot pass", async () => {
  // `GpuQualificationCoordinator::record` is the only thing that promotes a card
  // from admissible to proven, and it was deleted on 2026-08-21 with a note
  // saying it "comes back with the CUDA worker, not before". The worker shipped
  // 2026-08-26 and it did not come back, so `qualified` is unreachable -- and
  // the sentence it drove told every graphics-card user the engine "has not
  // passed its local execution check yet", underneath a device line reading
  // Graphics card (GPU).
  //
  // This is one-directional on purpose: it forbids *rendering* an unreachable
  // qualification claim, not keeping `qualified` in the view payload, where
  // `admissible_execution_untested` is an honest answer. Restoring the promotion
  // with real `ExecutionEvidence` is what makes this test's premise expire; if
  // that happens, delete this test in the same change rather than working
  // around it.
  const rendered = await readSources([".tsx"]);
  assert.ok(rendered.length > 50_000, `corpus is too small to be real: ${rendered.length} bytes`);
  assert.match(rendered, /provider_integrity/, "self-check: the corpus must include the GPU panel");

  assert.ok(
    !/\bgpu\.qualified\b/.test(rendered),
    "no component may render `gpu.qualified`: nothing can promote it, so it is always false",
  );
  const catalog = await readFile(
    fileURLToPath(new URL("../src/catalog.ts", import.meta.url)),
    "utf8",
  );
  assert.ok(
    !/local execution check/.test(catalog),
    "the execution-check copy is unreachable and must not sit in the catalog waiting to be wired up",
  );
});

test("Advanced mounts only while its own tab is active", async () => {
  // Every field on Advanced is a fact about *now* -- the engine reason, the
  // device, the RTF and latency percentiles, the overflow count. Mounted eagerly
  // it read them once at launch, before the resident worker had answered
  // `Hello`, and showed `cpu_gpu_runtime_missing` for the life of the process; a
  // reload against the same backend returned `cpu_gpu_pack_not_installed`, which
  // is how the stale read was told apart from a refused one.
  //
  // `readWithRetry` cannot carry this: the early value is a legitimate terminal
  // answer on a machine with no CUDA worker, so no `settled` predicate can
  // separate "not yet" from "not ever" without spinning on every processor
  // install. Mounting on activation is what makes the read late enough, and it
  // is the rule the log and Audio pages already follow.
  const source = await readFile(
    fileURLToPath(new URL("../src/settings/SettingsApp.tsx", import.meta.url)),
    "utf8",
  );
  assert.match(source, /<Advanced/, "self-check: SettingsApp must render Advanced at all");
  assert.match(
    source,
    /activeGroup === "advanced" && <Advanced/,
    "Advanced must be mounted only while its tab is active, or its reads race the worker's Hello",
  );
});
